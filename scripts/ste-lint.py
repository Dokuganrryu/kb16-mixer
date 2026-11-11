#!/usr/bin/env python3
"""ste-lint: check Markdown files for the mechanical rules of ASD-STE100.

This is a heuristic helper. It is NOT a certified STE checker. It finds the
rules that a program can see (sentence length, passive forms, banned words and
similar). It cannot check the STE dictionary, the meaning of a word, or the
order of the text. A person must still read the text.

Usage:
    python scripts/ste-lint.py [--summary] [--allow FILE] [--no-allow] [PATH ...]

PATH is a Markdown file or a folder. Without PATH, the tool checks all Markdown
files in the repository. Each finding is one line:

    file:line: rule (detail): sentence

The exit code is 1 when there are findings, 2 for a usage error, else 0.

The tool does not check: fenced code blocks, inline code, tables, link targets,
headings, front matter, link reference lines, text in double quotes, and block
quotes (except a block quote that starts with a CAUTION, WARNING or NOTE label).
Bold text counts as one name, because the docs put the labels of the user
interface in bold, and those labels are technical names.

Rules:
    sentence-length   more than 25 words in a sentence
    step-length       more than 20 words in a procedure step (a list item
                      that starts with a verb)
    contraction       a contraction, for example "don't" or "it's"
    semicolon         a semicolon in the text
    passive           a form of "be" and a past participle
    ing-form          a word that ends in "-ing" and is not in the allow-list
    two-instructions  more than one instruction in one sentence
    banned-word       a word or phrase that STE does not use
    noun-cluster      more than three nouns in a row (heuristic only)
    paragraph-length  more than 6 sentences in one paragraph

The allow-list file (default: scripts/ste-lint.allow) holds justified
exceptions. Each line is:

    <path glob> | <rule or *> | <regular expression or *> | <reason>

A line without a reason is an error.
"""

from __future__ import annotations

import argparse
import fnmatch
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path

MAX_WORDS = 25
MAX_STEP_WORDS = 20
MAX_PARAGRAPH_SENTENCES = 6
MAX_NOUNS = 3

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_ALLOW = ROOT / "scripts" / "ste-lint.allow"
SKIP_DIRS = {".git", "target", "node_modules", ".venv", "venv", "__pycache__"}

# Placeholders for the parts of a line that are names, not prose.
CODE = "CODE"
QUOTE = "QUOTE"
NAME = "NAME"
URL = "URL"
PLACEHOLDERS = {CODE, QUOTE, NAME, URL}


def _words(text: str) -> set[str]:
    return set(text.split())


BE_FORMS = _words("am is are was were be been being isn't aren't wasn't weren't")
# Words that can stand between "be" and the participle.
BE_GAP = _words(
    "not also never always only still then already usually automatically often "
    "now all normally first again both each fully thus so"
)
IRREGULAR_PARTICIPLES = _words(
    "written rewritten overwritten done undone made given taken shown known seen "
    "sent kept held built rebuilt found left lost broken chosen driven drawn "
    "frozen hidden stolen worn begun bound brought bought caught dealt fed felt "
    "fought forgotten got gotten grown heard laid led lent meant met paid said "
    "sold spent struck taught thought told understood won thrown mistaken "
    "withdrawn forbidden spoken woken ridden risen fallen eaten born set put "
    "cut split shut spread"
)
# Past participles that describe a state, not an action. "The pad is
# connected" says what is true now; nobody does an action in the sentence.
STATE_ADJECTIVES = _words(
    "connected disconnected supported unsupported installed uninstalled muted "
    "unmuted enabled disabled required selected focused pressed attached "
    "suspended locked unlocked signed unsigned closed open opened empty full "
    "valid invalid active idle"
)
# Words that end in "-ed" but are not past participles.
NOT_PARTICIPLES = _words("need red bed shed hundred embed speed feed seed indeed")

# Words that end in "-ing" but are not -ing forms of a verb, and technical
# names that this project uses (each one is in docs/glossary.md).
ING_ALLOWED = _words(
    "thing things nothing something anything everything string strings during "
    "ring rings bring king sing spring wing swing sting ping ceiling "
    "setting settings warning warnings ducking scaling padding heading headings "
    "morning evening"
)

BANNED = [
    ("utilize", r"\butili[sz](e|es|ed|ing|ation)\b"),
    ("leverage", r"\bleverag(e|es|ed|ing)\b"),
    ("in order to", r"\bin order to\b"),
    ("via", r"\bvia\b"),
    ("e.g.", r"(?<!\w)e\.\s?g\.(?!\w)"),
    ("i.e.", r"(?<!\w)i\.\s?e\.(?!\w)"),
    ("etc.", r"\betc\b\.?"),
    ("and so on", r"\band so on\b"),
    ("please", r"\bplease\b"),
    ("just", r"\bjust\b"),
    ("simply", r"\bsimply\b"),
    ("should be able to", r"\bshould be able to\b"),
]
BANNED_RE = [(name, re.compile(rx, re.IGNORECASE)) for name, rx in BANNED]

CONTRACTION_RE = re.compile(
    r"\b(\w+n['’]t|\w+['’](?:re|ve|ll|d|m)|"
    r"(?:it|that|there|here|what|let|he|she|who|where|how)['’]s)\b",
    re.IGNORECASE,
)

# Verbs that start an instruction. A list item that starts with one of these
# is a procedure step.
IMPERATIVES = _words(
    "accept add answer apply ask attach build change check choose clear click "
    "close connect copy create delete describe disconnect do double-click "
    "download drag edit enter extract fill find fix flash follow format give go "
    "hold install keep let look make move open paste plug press push put read "
    "record reload remove rename replace report restart right-click run save "
    "select send set start stop switch tap tell test tick try turn type unplug "
    "unzip update use wait write sign tag verify return release leave drop "
    "include avoid compare increase decrease pick mark list name note skip "
    "remember rewrite link adjust define upload sign-in log"
)
# Verbs that make a second instruction after "and" in an instruction.
SECOND_ACTIONS = _words(
    "click press open run select type unplug plug hold release close save "
    "restart install wait attach copy paste enter download extract remove add "
    "start stop connect disconnect push tap turn go write read set send make "
    "delete edit change check put use keep"
)
THEN_RE = re.compile(r"\b(and then|after that)\b|,\s*then\s+(\w+)", re.IGNORECASE)

# Words that cannot be part of a noun cluster. The check is a heuristic: it
# sees a run of words that are not in this list and are not verbs.
FUNCTION_WORDS = _words(
    "a an the this that these those each every all any some no not one two "
    "three four five six seven eight nine ten first second third last next "
    "other another same own such only more most less least many much few "
    "both either neither half quarter of in on at to for from with without by "
    "into onto over under up down out off about after before between through "
    "during above below near behind until while when if then than as because "
    "so thus or and but nor also too very again always never often once "
    "now still already even yet here there where which who whom whose what "
    "how why it its they them their he she we you your our us i me my "
    "is are was were be been being am has have had do does did can could "
    "may might must shall should will would cannot per via "
    "new old short long small big large real raw live normal full empty "
    "high low open closed quiet correct wrong separate fixed same true false "
    "left right top bottom front back ready free safe local private public "
    "slow fast quick soft hard white black red green blue amber main extra "
    "optional default current different special simple clear exact exactly "
    "approximately usually automatically only again later earlier today "
    "standard generic virtual physical external internal single whole "
    "complete possible available necessary important visible common basic "
    "general specific known unknown little"
)
VERBS = _words(
    "is are was be has have get gets got makes sends shows goes gives keeps "
    "takes holds lets puts sets runs reads writes uses needs says tells finds "
    "comes becomes stays starts stops changes moves opens closes types saves "
    "works plays counts asks answers waits follows checks builds contains "
    "means lists names marks applies accepts ignores rejects refuses restores "
    "handles knows learns turns selects mutes unmutes dims draws prints "
    "returns releases presses reports tracks occurs replaces types adds "
    "removes calls connects reconnects decides fails passes receives "
    "remembers sees seems stays tries wins"
)
# After these words, a base form such as "add" or "check" is a verb.
VERB_CONTEXT = _words(
    "to can cannot must will would could should may might not does do did "
    "you we they and or then"
)

TOKEN_RE = re.compile(r"[A-Za-z0-9][A-Za-z0-9'’\-+%./]*")


@dataclass
class Unit:
    """A paragraph or a list item, with the line number of each character."""

    text: str
    lines: list[int]
    is_item: bool


@dataclass
class Finding:
    path: str
    line: int
    rule: str
    detail: str
    sentence: str

    def __str__(self) -> str:
        detail = f" ({self.detail})" if self.detail else ""
        return f"{self.path}:{self.line}: {self.rule}{detail}: {self.sentence}"


@dataclass
class AllowEntry:
    glob: str
    rule: str
    pattern: re.Pattern[str] | None
    reason: str
    used: bool = False

    def matches(self, f: Finding) -> bool:
        if not fnmatch.fnmatch(f.path, self.glob):
            return False
        if self.rule not in ("*", f.rule):
            return False
        return self.pattern is None or bool(self.pattern.search(f.sentence))


def load_allow(path: Path) -> list[AllowEntry]:
    entries = []
    for number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        parts = [p.strip() for p in line.split("|")]
        if len(parts) != 4 or not parts[3]:
            raise ValueError(
                f"{path}:{number}: need '<glob> | <rule> | <regex> | <reason>'"
            )
        glob, rule, rx, reason = parts
        pattern = None if rx == "*" else re.compile(rx)
        entries.append(AllowEntry(glob, rule, pattern, reason))
    return entries


# ---------------------------------------------------------------- Markdown


FENCE_RE = re.compile(r"^\s*(```|~~~)")
LIST_RE = re.compile(r"^(\s*)([-*+]|\d+[.)])\s+(.*)$")
ADMONITION_RE = re.compile(r"^\s*>\s*(\*\*)?(\[!)?(CAUTION|WARNING|NOTE)\b", re.I)


def _end(code: str) -> str:
    """Keep the full stop of code that ends a sentence, so that the split works."""
    return code[-1] if code[-1:] in (".", "!", "?") else ""


def clean_inline(text: str) -> str:
    """Replace names, code and links with placeholders, and drop markup."""
    text = re.sub(r"<!--|-->", " ", text)
    text = re.sub(r"(`+)(.+?)\1", lambda m: f" {CODE}{_end(m.group(2))} ", text)
    text = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)
    text = re.sub(r"\[([^\]]*)\]\[[^\]]*\]", r"\1", text)
    text = re.sub(r"<https?://[^>]*>", URL, text)
    text = re.sub(r"https?://\S+", URL, text)
    text = re.sub(r"</?[A-Za-z][^>]*>", " ", text)
    text = re.sub(r"\*\*(.+?)\*\*|__(.+?)__", f" {NAME} ", text)
    text = re.sub(r"(?<!\w)[*_](\S[^*_]*?)[*_](?!\w)", r"\1", text)
    text = re.sub(r"\"[^\"]*\"|“[^”]*”", QUOTE, text)
    text = text.replace("\\|", "|")
    return text


def parse_units(source: str) -> list[Unit]:
    """Split a Markdown file into paragraphs and list items of prose."""
    lines = source.splitlines()
    units: list[Unit] = []
    current: Unit | None = None
    in_fence = False
    in_quote_skip = False
    start = 0
    if lines and lines[0].strip() == "---":
        for i in range(1, len(lines)):
            if lines[i].strip() == "---":
                start = i + 1
                break

    def close() -> None:
        nonlocal current
        if current is not None and current.text.strip():
            units.append(current)
        current = None

    def add(text: str, number: int, is_item: bool, new: bool) -> None:
        nonlocal current
        if new or current is None:
            close()
            current = Unit("", [], is_item)
        piece = clean_inline(text.strip())
        if current.text:
            current.text += " "
            current.lines.append(number)
        current.text += piece
        current.lines.extend([number] * len(piece))

    for index in range(start, len(lines)):
        number = index + 1
        line = lines[index]
        if FENCE_RE.match(line):
            in_fence = not in_fence
            close()
            continue
        if in_fence:
            continue
        stripped = line.strip()
        if not stripped:
            close()
            in_quote_skip = False
            continue
        if stripped.startswith(("#", "|")) or re.match(r"^\[[^\]]+\]:\s", stripped):
            close()
            continue
        if re.fullmatch(r"[-*_=\s]{3,}", stripped):
            close()
            continue
        if stripped.startswith(">"):
            if ADMONITION_RE.match(stripped):
                add(stripped.lstrip("> "), number, False, True)
                in_quote_skip = False
            elif in_quote_skip or current is None or not current.text:
                in_quote_skip = True
                close()
            else:
                add(stripped.lstrip("> "), number, False, False)
            continue
        item = LIST_RE.match(line)
        if item:
            add(item.group(3), number, True, True)
            continue
        if stripped.startswith("<") and not re.sub(r"<[^>]*>", "", stripped).strip():
            close()
            continue
        add(stripped, number, current.is_item if current else False, False)
    close()
    return units


SPLIT_RE = re.compile(r"(?<=[.!?])[\"')\]]*\s+(?=[A-Z0-9\"'(\[“])")


def sentences(unit: Unit) -> list[tuple[int, str]]:
    """Split a unit into sentences, each with the line where it starts."""
    out = []
    pos = 0
    text = unit.text
    for match in list(SPLIT_RE.finditer(text)) + [None]:
        end = match.end() if match else len(text)
        chunk = text[pos:end]
        lead = len(chunk) - len(chunk.lstrip())
        sentence = re.sub(r"\s+", " ", chunk).strip()
        if sentence and re.search(r"[A-Za-z]", sentence):
            index = min(pos + lead, len(unit.lines) - 1)
            out.append((unit.lines[index], sentence))
        pos = end
    return out


def tokens(sentence: str) -> list[str]:
    return [t.rstrip(".") for t in TOKEN_RE.findall(sentence)]


# ---------------------------------------------------------------- rules


def first_word(words: list[str]) -> str:
    """The first real word: a placeholder for bold text or code is not a verb."""
    real = [w for w in words if w not in PLACEHOLDERS]
    return real[0].lower() if real else ""


def is_participle(word: str) -> bool:
    w = word.lower().strip(".,:")
    if w in STATE_ADJECTIVES or w in NOT_PARTICIPLES or w.endswith("eed"):
        return False
    if w in IRREGULAR_PARTICIPLES:
        return True
    return len(w) > 4 and w.endswith("ed")


def check_passive(words: list[str]) -> str | None:
    lower = [w.lower() for w in words]
    for i, w in enumerate(lower):
        if w not in BE_FORMS:
            continue
        j = i + 1
        while j < len(lower) and lower[j] in BE_GAP:
            j += 1
        if j < len(lower) and is_participle(words[j]):
            return " ".join(words[i : j + 1])
    return None


def check_ing(words: list[str]) -> str | None:
    found = []
    for w in words:
        if w in PLACEHOLDERS:
            continue
        lw = w.lower().strip(".,:")
        base = lw[:-1] if lw.endswith("ings") else lw
        if len(base) > 4 and base.endswith("ing") and lw not in ING_ALLOWED:
            found.append(w)
    return ", ".join(found) if found else None


def check_two_instructions(sentence: str, words: list[str]) -> str | None:
    match = THEN_RE.search(sentence)
    if match:
        if match.group(1):
            return match.group(1).lower()
        if match.group(2) and match.group(2).lower() in IMPERATIVES:
            return f"then {match.group(2).lower()}"
    if first_word(words) in IMPERATIVES:
        lower = [w.lower() for w in words]
        for i, w in enumerate(lower[1:-1], 1):
            if w == "and" and lower[i + 1] in SECOND_ACTIONS:
                prev = lower[i - 1]
                if prev not in ("press", "click", "tap"):
                    return f"and {lower[i + 1]}"
    return None


def is_noun_like(word: str) -> bool:
    if word in PLACEHOLDERS:
        return False
    lw = word.lower()
    if lw in FUNCTION_WORDS or lw in VERBS:
        return False
    if not re.search(r"[a-z]", lw) or re.fullmatch(r"[\d.%+\-]+", lw):
        return False
    if lw.endswith(("ly", "ed", "ing", "ful", "less", "ous", "ive", "able", "ible")):
        return False
    return True


def check_noun_cluster(sentence: str) -> str | None:
    # A comma, colon or bracket ends a run, so only real word runs count.
    for part in re.split(r"[,;:()\[\]/]|\s[-–—]\s", sentence):
        run: list[str] = []
        prev = None
        for w in tokens(part):
            lw = w.lower()
            verb = lw in IMPERATIVES and (prev is None or prev in VERB_CONTEXT)
            prev = lw
            if is_noun_like(w) and not verb:
                # A word in "-s" inside a run is a verb or the head noun, so
                # the run ends after it ("the app replaces environment ...").
                if run and run[-1].lower().endswith("s") and not run[-1].endswith("ss"):
                    run = []
                run.append(w)
                if len(run) > MAX_NOUNS:
                    return " ".join(run)
            else:
                run = []
    return None


def check_unit(path: str, unit: Unit) -> list[Finding]:
    found: list[Finding] = []
    sents = sentences(unit)
    for line, sentence in sents:
        words = tokens(sentence)

        def hit(rule: str, detail: str = "") -> None:
            found.append(Finding(path, line, rule, detail, sentence))

        is_step = unit.is_item and first_word(words) in IMPERATIVES
        if is_step and len(words) > MAX_STEP_WORDS:
            hit("step-length", f"{len(words)} words")
        elif len(words) > MAX_WORDS:
            hit("sentence-length", f"{len(words)} words")
        m = CONTRACTION_RE.search(sentence)
        if m:
            hit("contraction", m.group(0))
        if ";" in sentence:
            hit("semicolon")
        p = check_passive(words)
        if p:
            hit("passive", p)
        g = check_ing(words)
        if g:
            hit("ing-form", g)
        t = check_two_instructions(sentence, words)
        if t:
            hit("two-instructions", t)
        for name, rx in BANNED_RE:
            if rx.search(sentence):
                hit("banned-word", name)
        n = check_noun_cluster(sentence)
        if n:
            hit("noun-cluster", n)
    if not unit.is_item and len(sents) > MAX_PARAGRAPH_SENTENCES:
        found.append(
            Finding(path, sents[0][0], "paragraph-length", f"{len(sents)} sentences",
                    sents[0][1])
        )
    return found


def check_text(path: str, source: str) -> list[Finding]:
    findings = []
    for unit in parse_units(source):
        findings.extend(check_unit(path, unit))
    return findings


# ---------------------------------------------------------------- driver


def markdown_files(paths: list[str]) -> list[Path]:
    targets = [Path(p) for p in paths] if paths else [ROOT]
    files: list[Path] = []
    for target in targets:
        if target.is_file():
            files.append(target)
        elif target.is_dir():
            for dirpath, dirnames, filenames in os.walk(target):
                dirnames[:] = sorted(d for d in dirnames if d not in SKIP_DIRS)
                files.extend(Path(dirpath) / f for f in sorted(filenames)
                             if f.lower().endswith(".md"))
        else:
            raise FileNotFoundError(str(target))
    return files


def display_path(path: Path) -> str:
    try:
        return path.resolve().relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="ste-lint",
        description=(
            "Check Markdown files for the mechanical rules of ASD-STE100 "
            "Simplified Technical English. This is a heuristic helper, not a "
            "certified STE checker: a person must still read the text."
        ),
        epilog=__doc__.split("Rules:", 1)[1].split("The allow-list", 1)[0],
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("paths", nargs="*", help="Markdown files or folders")
    parser.add_argument("--summary", action="store_true",
                        help="print counts per rule and per file, not each finding")
    parser.add_argument("--allow", type=Path, default=DEFAULT_ALLOW,
                        help="allow-list file (default: scripts/ste-lint.allow)")
    parser.add_argument("--no-allow", action="store_true",
                        help="do not use an allow-list")
    args = parser.parse_args(argv)
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    try:
        allow = [] if args.no_allow or not args.allow.exists() else load_allow(args.allow)
        files = markdown_files(args.paths)
    except (ValueError, FileNotFoundError, re.error) as err:
        print(f"ste-lint: {err}", file=sys.stderr)
        return 2

    findings: list[Finding] = []
    for file in files:
        name = display_path(file)
        for f in check_text(name, file.read_text(encoding="utf-8")):
            entry = next((e for e in allow if e.matches(f)), None)
            if entry:
                entry.used = True
            else:
                findings.append(f)

    if args.summary:
        by_rule: dict[str, int] = {}
        by_file: dict[str, int] = {}
        for f in findings:
            by_rule[f.rule] = by_rule.get(f.rule, 0) + 1
            by_file[f.path] = by_file.get(f.path, 0) + 1
        print(f"{len(findings)} finding(s) in {len(by_file)} of {len(files)} file(s)")
        print("\nPer rule:")
        for rule, count in sorted(by_rule.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {count:5}  {rule}")
        print("\nPer file:")
        for name, count in sorted(by_file.items(), key=lambda kv: (-kv[1], kv[0])):
            print(f"  {count:5}  {name}")
    else:
        for f in findings:
            print(f)
    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
