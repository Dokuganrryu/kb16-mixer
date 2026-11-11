"""Checks the relative links and the #anchors in the Markdown files of the repository.

    python scripts/check-links.py            # all Markdown files that git knows
    python scripts/check-links.py README.md  # some files

A link fails when its file is not in the repository (tracked, or new and not ignored), or when
its #anchor is not a heading (or an <a id>/<a name>) of the Markdown file that it points to.
Anchors follow the GitHub rules: lower case, punctuation removed, spaces become hyphens, and a
repeated heading gets -1, -2 and so on. Links with a scheme (https:, mailto:) are not checked.
Exit code 1 when a link fails. Standard library only.
"""
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parent.parent

FENCE = re.compile(r"^\s*(```|~~~)")
CODE_SPAN = re.compile(r"`+[^`]*`+")
INLINE = re.compile(r"!?\[(?:[^\[\]]|\[[^\]]*\])*\]\(\s*(<[^>]*>|[^)\s]+)(?:\s+\"[^\"]*\")?\s*\)")
REFERENCE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*(<[^>]*>|\S+)")
HEADING = re.compile(r"^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$")
HTML_ANCHOR = re.compile(r"<a\s+(?:id|name)=\"([^\"]+)\"", re.I)
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")


def prose_lines(text: str):
    """(line number, line) of each line outside fenced code blocks."""
    fenced = False
    for n, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if not fenced:
            yield n, line


def slug(heading: str) -> str:
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", heading)  # a link keeps only its text
    text = re.sub(r"<[^>]+>", "", text)
    text = text.replace("`", "").replace("*", "")
    text = re.sub(r"[^\w\- ]", "", text.lower())
    return text.replace(" ", "-")


def anchors(text: str) -> set[str]:
    found, seen = set(), {}
    for _, line in prose_lines(text):
        found.update(HTML_ANCHOR.findall(line))
        m = HEADING.match(line)
        if not m:
            continue
        base = slug(m.group(2))
        count = seen.get(base, 0)
        seen[base] = count + 1
        found.add(base if count == 0 else f"{base}-{count}")
    return found


def links(text: str):
    """(line number, target) of each link outside code."""
    for n, line in prose_lines(text):
        m = REFERENCE.match(line)
        if m:
            yield n, m.group(1)
            continue
        for m in INLINE.finditer(CODE_SPAN.sub("", line)):
            yield n, m.group(1)


def known_files(root: Path) -> set[str]:
    out = subprocess.run(["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
                         cwd=root, check=True, capture_output=True).stdout
    files = {p for p in out.decode("utf-8").split("\0") if p and (root / p).exists()}
    folders = {str(parent) for f in files for parent in PurePosixPath(f).parents if str(parent) != "."}
    return files | folders


def resolve(source: str, path: str) -> str | None:
    """The repository path that a link of `source` points to, or None if it leaves the repository."""
    parts = []
    for part in (PurePosixPath(source).parent / path).parts:
        if part == "..":
            if not parts:
                return None
            parts.pop()
        elif part != ".":
            parts.append(part)
    return "/".join(parts)


def check(root: Path, sources: list[str]) -> list[str]:
    known = known_files(root)
    cache: dict[str, set[str]] = {}

    def anchors_of(name: str) -> set[str]:
        if name not in cache:
            cache[name] = anchors((root / name).read_text(encoding="utf-8"))
        return cache[name]

    problems = []
    for source in sources:
        text = (root / source).read_text(encoding="utf-8")
        for n, target in links(text):
            target = target.strip("<>")
            if SCHEME.match(target) or target.startswith("//"):
                continue
            path, _, anchor = target.partition("#")
            if path.startswith("/"):
                name = path.strip("/")
            else:
                name = resolve(source, path) if path else source
            if name is None or (name and name not in known):
                problems.append(f"{source}:{n}: missing file: {target}")
            elif anchor and name.lower().endswith(".md") and anchor.lower() not in anchors_of(name):
                problems.append(f"{source}:{n}: missing anchor: {target}")
    return problems


def main(argv: list[str]) -> int:
    if argv:
        sources = [Path(a).resolve().relative_to(ROOT).as_posix() for a in argv]
    else:
        sources = sorted(f for f in known_files(ROOT) if f.lower().endswith(".md") and (ROOT / f).is_file())
    problems = check(ROOT, sources)
    for problem in problems:
        print(problem)
    print(f"check-links: {len(sources)} file(s), {len(problems)} problem(s)", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
