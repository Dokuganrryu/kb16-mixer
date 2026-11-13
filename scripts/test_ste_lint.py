"""Unit tests for scripts/ste-lint.py. Run: python -m unittest scripts/test_ste_lint.py"""

import contextlib
import importlib.util
import io
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "ste_lint", Path(__file__).resolve().parent / "ste-lint.py"
)
ste = importlib.util.module_from_spec(_SPEC)
sys.modules["ste_lint"] = ste
_SPEC.loader.exec_module(ste)


def rules(markdown: str) -> list[str]:
    return [f.rule for f in ste.check_text("test.md", markdown)]


class SentenceLength(unittest.TestCase):
    def test_short_sentence_passes(self):
        self.assertEqual(rules("The pad has 16 keys and 3 knobs.\n"), [])

    def test_long_description_fails(self):
        text = " ".join(["the"] * 26) + ".\n"
        self.assertEqual(rules(text), ["sentence-length"])

    def test_25_words_pass(self):
        self.assertEqual(rules(" ".join(["the"] * 25) + ".\n"), [])

    def test_long_procedure_step_fails(self):
        step = "1. Connect " + " ".join(["cable"] * 20) + ".\n"
        self.assertEqual(rules(step), ["step-length"])

    def test_list_item_that_is_not_a_step_has_the_description_limit(self):
        item = "- The " + " ".join(["cable"] * 21) + ".\n"
        self.assertEqual(rules(item), [])

    def test_inline_code_counts_as_one_word(self):
        text = "Run `" + " ".join(["x"] * 40) + "` again.\n"
        self.assertEqual(rules(text), [])


class Contractions(unittest.TestCase):
    def test_contraction_fails(self):
        self.assertEqual(rules("Do not unplug it. It doesn't stop.\n"), ["contraction"])

    def test_its_is_a_contraction(self):
        self.assertEqual(rules("It's off.\n"), ["contraction"])

    def test_possessive_passes(self):
        self.assertEqual(rules("The app's window opens.\n"), [])


class Semicolons(unittest.TestCase):
    def test_semicolon_fails(self):
        self.assertEqual(rules("The pad starts; the screen comes on.\n"), ["semicolon"])

    def test_semicolon_in_code_passes(self):
        self.assertEqual(rules("Type `a; b` in the box.\n"), [])


class Passive(unittest.TestCase):
    def test_passive_fails(self):
        self.assertEqual(rules("The page is written once.\n"), ["passive"])

    def test_irregular_participle_with_adverb_fails(self):
        self.assertEqual(rules("The name is also sent.\n"), ["passive"])

    def test_state_adjective_passes(self):
        self.assertEqual(rules("When the pad is connected, the light comes on.\n"), [])

    def test_active_voice_passes(self):
        self.assertEqual(rules("The firmware writes the page once.\n"), [])

    def test_red_is_not_a_participle(self):
        self.assertEqual(rules("The light is red.\n"), [])


class IngForms(unittest.TestCase):
    def test_ing_form_fails(self):
        self.assertEqual(rules("The app keeps running.\n"), ["ing-form"])

    def test_allowed_ing_words_pass(self):
        self.assertEqual(rules("The setting does nothing during start.\n"), [])


class TwoInstructions(unittest.TestCase):
    def test_and_then_fails(self):
        self.assertEqual(rules("Unplug the cable and then wait.\n"), ["two-instructions"])

    def test_comma_then_verb_fails(self):
        self.assertEqual(rules("Right-click the icon, then click Quit.\n"),
                         ["two-instructions"])

    def test_and_second_verb_fails(self):
        self.assertEqual(rules("Unplug the pad and plug it in again.\n"),
                         ["two-instructions"])

    def test_one_instruction_passes(self):
        self.assertEqual(rules("Unplug the pad. Plug it in again.\n"), [])

    def test_bold_label_is_not_a_verb(self):
        self.assertEqual(rules("- **Local access.** A program can open the pad and send data.\n"), [])

    def test_description_with_and_passes(self):
        self.assertEqual(rules("The app reads and writes the file.\n"), [])


class BannedWords(unittest.TestCase):
    def test_each_banned_word_fails(self):
        for text in ("Utilize the knob.", "We leverage it.", "Do it in order to win.",
                     "Send it via USB.", "A key, e.g. F13.", "A key, i.e. F13.",
                     "Keys, knobs, etc.", "Keys, knobs and so on.", "Please wait.",
                     "Just wait.", "Simply wait.", "You should be able to see it."):
            with self.subTest(text=text):
                self.assertIn("banned-word", rules(text + "\n"))

    def test_word_that_contains_a_banned_word_passes(self):
        self.assertEqual(rules("Adjust the trim. The justification follows.\n"), [])


class Vocabulary(unittest.TestCase):
    def found(self, text: str) -> list[str]:
        return [f.detail for f in ste.check_text("test.md", text + "\n")
                if f.rule == "vocabulary"]

    def test_word_that_is_not_approved_fails_with_its_alternative(self):
        self.assertEqual(self.found("You need a cable."), ["need: use NECESSARY or MUST"])

    def test_each_form_of_a_word_fails(self):
        for text in ("It needs a cable.", "A cable was needed.", "The app fails.",
                     "The app failed.", "It takes 2 s.", "It took 2 s."):
            with self.subTest(text=text):
                self.assertEqual(len(self.found(text)), 1)

    def test_approved_word_passes(self):
        self.assertEqual(self.found("A cable is necessary. The app does not start."), [])

    def test_word_inside_a_longer_word_passes(self):
        self.assertEqual(self.found("The company sends an anyway notice. Knowledge helps."), [])

    def test_check_and_test_as_nouns_pass(self):
        self.assertEqual(self.found("Do a check of the pad. The test passes. Run the checks."), [])

    def test_check_and_test_as_verbs_fail(self):
        for text in ("Check the pad.", "You must test the pad.", "The author tested it."):
            with self.subTest(text=text):
                self.assertEqual(len(self.found(text)), 1)

    def test_not_tested_is_the_status_label(self):
        self.assertEqual(self.found("Windows 10 is not tested."), [])

    def test_two_word_entry_fails(self):
        self.assertEqual(self.found("Turn on the switch."), ["turn on/off: use START, STOP or SET"])

    def test_code_bold_and_quotes_pass(self):
        self.assertEqual(self.found('Run `--never`. Click **Restore**. It shows "need".'), [])

    def test_each_entry_has_an_alternative(self):
        for name, _, alternative in ste.VOCABULARY:
            with self.subTest(name=name):
                self.assertTrue(alternative.startswith("use "))


class NounClusters(unittest.TestCase):
    def test_four_nouns_fail(self):
        self.assertEqual(rules("Read the USB device release number.\n"), ["noun-cluster"])

    def test_three_nouns_pass(self):
        self.assertEqual(rules("Read the raw HID interface.\n"), [])

    def test_verb_breaks_the_run(self):
        self.assertEqual(rules("The app replaces environment variables.\n"), [])


class Paragraphs(unittest.TestCase):
    def test_seven_sentences_fail(self):
        self.assertEqual(rules("It runs. " * 7 + "\n"), ["paragraph-length"])

    def test_six_sentences_pass(self):
        self.assertEqual(rules("It runs. " * 6 + "\n"), [])

    def test_list_items_are_separate(self):
        self.assertEqual(rules("".join(f"- It runs {i}.\n" for i in range(8))), [])

    def test_lines_join_into_one_paragraph(self):
        self.assertEqual(rules("It runs.\n" * 7), ["paragraph-length"])


class Skipped(unittest.TestCase):
    BAD = "It's utilized; please run it via the app etc.\n"

    def test_bad_sentence_fails(self):
        self.assertTrue(rules(self.BAD))

    def test_code_block_is_skipped(self):
        self.assertEqual(rules("```\n" + self.BAD + "```\n"), [])

    def test_table_heading_and_front_matter_are_skipped(self):
        text = "---\ntitle: " + self.BAD + "---\n# " + self.BAD + "| " + self.BAD
        self.assertEqual(rules(text), [])

    def test_link_target_is_skipped(self):
        self.assertEqual(rules("See [the guide](please-utilize-this-via.md).\n"), [])

    def test_quoted_text_is_skipped(self):
        self.assertEqual(rules('The box shows "please wait; it\'s busy".\n'), [])

    def test_block_quote_is_skipped(self):
        self.assertEqual(rules("> " + self.BAD), [])

    def test_caution_block_quote_is_checked(self):
        self.assertIn("banned-word", rules("> **CAUTION:** Please wait.\n"))

    def test_line_number_is_the_start_of_the_sentence(self):
        found = ste.check_text("t.md", "# T\n\nIt runs.\nPlease wait.\n")
        self.assertEqual([(f.line, f.rule) for f in found], [(4, "banned-word")])


class AllowList(unittest.TestCase):
    def write(self, text: str) -> Path:
        handle = tempfile.NamedTemporaryFile("w", suffix=".allow", delete=False,
                                             encoding="utf-8")
        handle.write(text)
        handle.close()
        self.addCleanup(Path(handle.name).unlink)
        return Path(handle.name)

    def test_entry_without_reason_is_an_error(self):
        with self.assertRaises(ValueError):
            ste.load_allow(self.write("docs/*.md | passive | is written |\n"))

    def test_entry_matches_path_rule_and_text(self):
        (entry,) = ste.load_allow(self.write("docs/*.md | passive | is written | a quote\n"))
        hit = ste.Finding("docs/a.md", 1, "passive", "", "It is written.")
        self.assertTrue(entry.matches(hit))
        self.assertFalse(entry.matches(ste.Finding("README.md", 1, "passive", "", "It is written.")))
        self.assertFalse(entry.matches(ste.Finding("docs/a.md", 1, "semicolon", "", "It is written.")))

    def test_exit_code(self):
        bad = self.write("Please wait.\n")
        good = self.write("Wait.\n")
        with contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(ste.main(["--no-allow", str(good)]), 0)
            self.assertEqual(ste.main(["--no-allow", str(bad)]), 1)
        self.assertIn("banned-word (please): Please wait.", out.getvalue())


if __name__ == "__main__":
    unittest.main()
