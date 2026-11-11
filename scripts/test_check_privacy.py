"""Unit tests for scripts/check-privacy.py. Run: python -m unittest scripts/test_check_privacy.py

The examples of secrets are built from parts, so that the check of this repository does not
find them in this file.
"""

import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "check_privacy", Path(__file__).resolve().parent / "check-privacy.py"
)
cp = importlib.util.module_from_spec(_SPEC)
sys.modules["check_privacy"] = cp
_SPEC.loader.exec_module(cp)

AT = "@"


def rules(line: str) -> list[str]:
    return cp.check_line(line)


class Email(unittest.TestCase):
    def test_personal_address_fails(self):
        self.assertEqual(rules("mail jane.doe" + AT + "gmail.com for help"), ["email"])

    def test_github_noreply_passes(self):
        self.assertEqual(rules("12345+someone" + AT + "users.noreply.github.com"), [])

    def test_example_domains_pass(self):
        for domain in ("example.com", "example.org", "mail.example.net", "pad.test", "x.invalid", "example"):
            self.assertEqual(rules("user" + AT + domain), [], domain)

    def test_tool_version_is_no_address(self):
        self.assertEqual(rules("tool: cargo-deny" + AT + "0.20.2 and uses: x/y" + AT + "3d3c42e5"), [])

    def test_lookalike_domain_fails(self):
        self.assertEqual(rules("user" + AT + "example.com.evil.io"), ["email"])


class ProfilePath(unittest.TestCase):
    def test_real_names_fail(self):
        for line in (
            "C:\\Users\\" + "alice\\AppData\\x",
            "c:/users/" + "alice/x",
            "/c/Users/" + "alice/Documents",
            "see /Users/" + "alice/code",
            "cd /home/" + "alice/kb16",
            '"C:\\\\Users\\\\' + 'alice\\\\x.exe"',
        ):
            self.assertEqual(rules(line), ["profile-path"], line)

    def test_placeholders_pass(self):
        for line in (
            "C:\\Users\\<name>\\...",
            "C:\\Users\\%USERNAME%\\AppData",
            "/home/runner/work",
            "/home/USER/x",
            "C:\\Users\\Public\\Desktop",
            "C:\\Users\\Default\\x",
            "/Users/$USER/x",
            "C:\\\\Users\\\\Someone\\\\x.exe",
        ):
            self.assertEqual(rules(line), [], line)

    def test_user_folder_without_a_name_passes(self):
        self.assertEqual(rules("C:\\Users\\"), [])


class SidAndTokens(unittest.TestCase):
    def test_sid_fails(self):
        self.assertEqual(rules("S-1-5" + "-21-1004336348-1177238915-682003330-1001"), ["windows-sid"])

    def test_well_known_sid_passes(self):
        self.assertEqual(rules("S-1-5-18 is LocalSystem"), [])

    def test_tokens_fail(self):
        for secret in (
            "gh" + "p_" + "a" * 36,
            "github" + "_pat_" + "B" * 30,
            "AK" + "IA" + "ABCDEFGHIJKLMNOP",
            "-" * 5 + "BEGIN OPENSSH PRIVATE KEY" + "-" * 5,
        ):
            self.assertEqual(rules("x = " + secret), ["token"], secret[:6])

    def test_prefix_alone_passes(self):
        self.assertEqual(rules("Tokens start with ghp_ or github_pat_."), [])


class Output(unittest.TestCase):
    def test_finding_names_file_line_rule_and_never_the_value(self):
        secret = "gh" + "p_" + "z" * 36
        out = cp.check_text("notes.md", "fine\nkey " + secret + "\n")
        self.assertEqual(out, ["notes.md:2: token"])
        self.assertNotIn(secret, " ".join(out))


class Repository(unittest.TestCase):
    """check_tree and check_history on a small real git repository."""

    def git(self, *args):
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.git("init", "-q")
        self.git("config", "commit.gpgsign", "false")

    def commit(self, email):
        self.git("-c", "user.name=T", "-c", "user.email=" + email, "commit", "-q", "--allow-empty", "-m", "x")

    def test_binary_and_untracked_files_are_skipped(self):
        (self.root / "blob.bin").write_bytes(b"\0" + ("x" + AT + "gmail.com").encode())
        (self.root / "ok.md").write_text("nothing here\n")
        (self.root / "loose.md").write_text("x" + AT + "gmail.com\n")
        self.git("add", "blob.bin", "ok.md")
        self.assertEqual(cp.check_tree(self.root), [])

    def test_tracked_text_file_is_checked(self):
        (self.root / "a.md").write_text("one\nx" + AT + "gmail.com\n")
        self.git("add", "a.md")
        self.assertEqual(cp.check_tree(self.root), ["a.md:2: email"])

    def test_history_allows_only_noreply_addresses(self):
        self.commit("1+someone" + AT + "users.noreply.github.com")
        self.assertEqual(cp.check_history(self.root, "HEAD"), [])
        self.commit("someone" + AT + "gmail.com")
        problems = cp.check_history(self.root, "HEAD")
        self.assertEqual(len(problems), 2)
        self.assertTrue(all("commit-email" in p and AT not in p for p in problems))

    def test_history_allows_the_bot_noreply_form(self):
        self.commit("49699333+dependabot[bot]" + AT + "users.noreply.github.com")
        self.assertEqual(cp.check_history(self.root, "HEAD"), [])


class CommitAddress(unittest.TestCase):
    def test_bot_form_is_accepted(self):
        self.assertTrue(cp.commit_address_allowed("49699333+dependabot[bot]" + AT + "users.noreply.github.com"))

    def test_only_the_exact_bot_form_is_accepted(self):
        for address in (
            "dependabot[bot]" + AT + "users.noreply.github.com",  # no numeric id
            "abc+dependabot[bot]" + AT + "users.noreply.github.com",  # id is not digits
            "1+dependabot[bot]" + AT + "gmail.com",  # another domain
            "1+dependabot[bot]" + AT + "users.noreply.github.com.evil.io",
            "1+[bot]" + AT + "users.noreply.github.com",  # no name
            "1+a b[bot]" + AT + "users.noreply.github.com",
        ):
            self.assertFalse(cp.commit_address_allowed(address), address)


if __name__ == "__main__":
    unittest.main()
