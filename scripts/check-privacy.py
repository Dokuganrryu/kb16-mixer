"""Finds personal data and secrets in the files that git tracks.

    python scripts/check-privacy.py                 # all tracked text files
    python scripts/check-privacy.py --history HEAD  # also the commit addresses of HEAD and its parents

Rules (see "Personal data" in CONTRIBUTING.md):
  email         an e-mail address that is not a GitHub noreply address or a reserved example domain
  profile-path  C:\\Users\\<name>\\, /Users/<name>/ or /home/<name>/ with a real name, not a placeholder
  windows-sid   a Windows account SID (S-1-5-21-...)
  token         a GitHub token, an AWS access key or a PEM block
  commit-email  (--history) an author or committer address that is not a GitHub noreply address
                (the bot form <id>+<name>[bot]@users.noreply.github.com, as Dependabot uses, is accepted)

The output is "file:line: rule". It never prints the value that it found, so that a CI log
does not copy a secret. Binary files are skipped. Exit code 1 when it finds something.
Standard library only.
"""
import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The last label must be letters, so that "cargo-deny@0.20.2" (a tool version) is no address.
EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@((?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,})(?![A-Za-z0-9-])")
# RFC 2606 and RFC 6761 names that can never belong to a person.
EXAMPLE_DOMAINS = re.compile(r"(^|\.)(example\.(com|net|org)|example|test|invalid|localhost)$", re.I)
NOREPLY = re.compile(r"^[A-Za-z0-9._%+-]+@users\.noreply\.github\.com$", re.I)
# The address of a GitHub App, for example Dependabot: "<id>+<name>[bot]@users.noreply.github.com".
# Only the commit history needs it: in a file, the EMAIL pattern does not match "[bot]@".
BOT_NOREPLY = re.compile(r"^\d+\+[A-Za-z0-9-]+\[bot\]@users\.noreply\.github\.com$", re.I)
# GitHub writes this committer address on commits made in its web interface.
GITHUB_WEB_FLOW = "noreply" + "@github.com"

# A drive letter (C:\ or /c/) or nothing, then Users\<name>\ ; or /home/<name>/.
PROFILE = re.compile(
    r"(?:\b[A-Za-z]:|/[A-Za-z]|^|(?<=[\s\"'`(=]))[\\/]+(?:Users|home)[\\/]+([^\\/\s\"'`]+)[\\/]",
    re.I,
)
PLACEHOLDERS = {
    "user", "username", "name", "you", "your-name", "someone", "runner", "public", "default",
    "all users", "%username%", "$user", "${user}", "$env:username", "~",
}

# The patterns are split so that this file does not match its own rules.
TOKENS = re.compile(
    "|".join([
        "gh" + r"[pousr]_[A-Za-z0-9]{20,}",
        "github" + r"_pat_[A-Za-z0-9_]{20,}",
        "AK" + r"IA[0-9A-Z]{16}",
        "-" * 5 + r"BEGIN [A-Z0-9 ]+-----",
    ])
)
SID = re.compile("S-1-5" + r"-21-\d+-\d+-\d+")


def is_placeholder(name: str) -> bool:
    return name.lower() in PLACEHOLDERS or name.startswith("<") or name.startswith("%") or name.startswith("$")


def email_allowed(address: str, domain: str) -> bool:
    return bool(NOREPLY.match(address)) or bool(EXAMPLE_DOMAINS.search(domain))


def check_line(line: str) -> list[str]:
    found = []
    for m in EMAIL.finditer(line):
        if not email_allowed(m.group(0), m.group(1)):
            found.append("email")
    for m in PROFILE.finditer(line):
        if not is_placeholder(m.group(1)):
            found.append("profile-path")
    if SID.search(line):
        found.append("windows-sid")
    if TOKENS.search(line):
        found.append("token")
    return found


def check_text(name: str, text: str) -> list[str]:
    return [f"{name}:{n}: {rule}" for n, line in enumerate(text.splitlines(), 1) for rule in check_line(line)]


def read_text(path: Path):
    """The text of a file, or None for a binary file."""
    data = path.read_bytes()
    if b"\0" in data[:8192]:
        return None
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError:
        return None


def tracked_files(root: Path) -> list[str]:
    out = subprocess.run(["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True).stdout
    return [p for p in out.decode("utf-8").split("\0") if p]


def check_tree(root: Path) -> list[str]:
    problems = []
    for name in tracked_files(root):
        path = root / name
        if not path.is_file():  # deleted in the work tree but not yet in the index
            continue
        text = read_text(path)
        if text is not None:
            problems += check_text(name, text)
    return problems


def commit_address_allowed(address: str) -> bool:
    return bool(NOREPLY.match(address) or BOT_NOREPLY.match(address))


def check_history(root: Path, rev_range: str) -> list[str]:
    out = subprocess.run(["git", "log", "--format=%h%x00%ae%x00%ce", rev_range],
                         cwd=root, check=True, capture_output=True, text=True).stdout
    problems = []
    for line in out.splitlines():
        sha, author, committer = line.split("\0")
        if not commit_address_allowed(author):
            problems.append(f"commit {sha}: commit-email (author)")
        if not (commit_address_allowed(committer) or committer.lower() == GITHUB_WEB_FLOW):
            problems.append(f"commit {sha}: commit-email (committer)")
    return problems


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--history", metavar="RANGE", help="also check the commit addresses in this git range")
    a = p.parse_args(argv)
    try:
        problems = check_tree(ROOT)
        if a.history:
            problems += check_history(ROOT, a.history)
    except subprocess.CalledProcessError as e:
        print(f"check-privacy: git failed: {e.stderr}", file=sys.stderr)
        return 2
    for problem in problems:
        print(problem)
    print(f"check-privacy: {len(problems)} problem(s)", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
