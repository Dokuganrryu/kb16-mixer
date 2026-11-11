"""Writes THIRD_PARTY_NOTICES.md: the copyright notices and full licence texts of every
third-party component in the firmware image and in the app.

Run from anywhere:  python scripts/third-party-notices.py
Needs Python 3, cargo and cargo-about 0.9 (cargo install cargo-about --locked --features cli).

cargo-about (configured by scripts/about.toml) lists the crates of each binary for its target,
picks the licence of each crate and finds the licence files in the crate. For a crate that ships
no licence file, cargo-about gives the standard SPDX text, and this script adds a copyright line
with the authors from the crate's Cargo.toml. The script then adds the Rust standard library,
the HIDAPI C library, and the fonts. It exits 1 if cargo-about refuses a licence.
"""
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "THIRD_PARTY_NOTICES.md"
CONFIG = ROOT / "scripts" / "about.toml"
BINARIES = [
    ("firmware", "thumbv7m-none-eabi", "Firmware image (`kb16-mixer-firmware.bin`)"),
    ("app", "x86_64-pc-windows-msvc", "App (`kb16-mixer.exe`)"),
]
PLACEHOLDER = re.compile(r"^.*<(year|copyright holders?|owner|name of copyright owner)>.*\n?", re.M)


def read_text(path):
    raw = Path(path).read_bytes()
    try:
        text = raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        text = raw.decode("cp1252")
    return text.replace("\r\n", "\n").strip("\n") + "\n"


def about(crate, target):
    subprocess.run(["cargo", "fetch", "--locked"], cwd=ROOT / crate, check=True)
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / "about.json"
        done = subprocess.run(
            ["cargo", "about", "generate", "--format", "json", "--frozen", "-c", str(CONFIG),
             "-m", str(ROOT / crate / "Cargo.toml"), "--target", target, "-o", str(out)])
        if done.returncode != 0 or not out.exists():
            sys.exit(f"cargo about failed for {crate}")
        return json.loads(out.read_text(encoding="utf-8"))


def linked(crate, target):
    """(name, version) of each package whose code is in the binary: the normal dependencies for the
    target, without procedural-macro crates and what only they use (that code runs in the build)."""
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--frozen", "--filter-platform", target], cwd=ROOT / crate))
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    macro = {pid for pid, p in packages.items() if any("proc-macro" in t["kind"] for t in p["targets"])}
    seen, todo = set(), [meta["resolve"]["root"]]
    while todo:
        for dep in nodes[todo.pop()]["deps"]:
            normal = any(k["kind"] is None for k in dep["dep_kinds"])
            if normal and dep["pkg"] not in macro and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                todo.append(dep["pkg"])
    return {(packages[p]["name"], packages[p]["version"]) for p in seen}


def authors(package):
    names = [re.sub(r"\s*<[^>]*>", "", a).strip() for a in package.get("authors") or []]
    names = [n for n in names if n]
    line = f"Copyright (c) the authors of {package['name']} {package['version']}"
    if names:
        line += f": {', '.join(names)}"
    if package.get("repository"):
        line += f" ({package['repository']})"
    return line


def main():
    # One entry per distinct licence text: (licence id, text) -> set of (crate, version).
    texts = {}
    # Licence texts that cargo-about filled in from SPDX: licence id -> {(crate, version): copyright}.
    fallbacks = {}
    tables = []
    build_only = {}
    hidapi_dir = None
    unicode_text = None
    for crate, target, title in BINARIES:
        data = about(crate, target)
        in_binary = linked(crate, target)
        used = {}
        for lic in data["licenses"]:
            for use in lic["used_by"]:
                pkg = use["crate"]
                if pkg["source"] is None:
                    continue  # this repository's own crates: see LICENSE
                key = (pkg["name"], pkg["version"])
                if lic["id"] == "Unicode-3.0" and lic.get("source_path"):
                    unicode_text = read_text(lic["source_path"])
                if key not in in_binary:
                    build_only[key] = pkg["license"]
                    continue
                used.setdefault(key, (pkg, set()))[1].add(lic["id"])
                if pkg["name"] == "hidapi":
                    hidapi_dir = Path(pkg["manifest_path"]).parent
                if lic.get("source_path"):
                    texts.setdefault((lic["id"], read_text(lic["source_path"])), set()).add(key)
                else:
                    fallbacks.setdefault((lic["id"], lic["text"]), {})[key] = authors(pkg)
        rows = []
        for (name, version), (pkg, ids) in sorted(used.items()):
            rows.append(f"| {name} | {version} | {pkg['license']} | {' AND '.join(sorted(ids))} |")
        tables.append((title, target, rows))

    lines = [
        "<!-- Generated by scripts/third-party-notices.py (cargo-about, scripts/about.toml). Do not edit.",
        "     Regenerate with: python scripts/third-party-notices.py -->",
        "",
        "# Third-party notices",
        "",
        "kb16-mixer itself is MIT licensed. See [LICENSE](LICENSE).",
        "The firmware image and the app contain code of the components below.",
        "This file gives the copyright notices and the full licence text of each component, as their licences require for a binary.",
        "",
        "If a crate offers a choice of licences (for example `MIT OR Apache-2.0`), the column \"Used under\" names the licence that kb16-mixer uses. The text of that licence is in [Licence texts](#licence-texts).",
        "Procedural-macro crates, and the crates that only they use, run only during the build and put no code into the binaries. They are listed at the end, without licence texts. Build scripts and their dependencies are not listed.",
        "",
    ]
    for title, target, rows in tables:
        lines += [f"## {title}", "", f"Target `{target}`, {len(rows)} crates.", "",
                  "| Crate | Version | Licence (declared) | Used under |", "|---|---|---|---|", *rows, ""]
    in_a_binary = set().union(*texts.values(), *fallbacks.values())
    build_rows = [f"| {n} | {v} | {lic} |" for (n, v), lic in sorted(build_only.items()) if (n, v) not in in_a_binary]

    if hidapi_dir is None:
        sys.exit("the app no longer uses hidapi: update the HIDAPI notice in this script")
    hidapi_version = read_text(hidapi_dir / "etc" / "hidapi" / "VERSION").strip()
    fonts = ROOT / "firmware" / "assets" / "fonts"
    lines += [
        "## Other components",
        "",
        "### Rust standard library (firmware and app)",
        "",
        "Each Rust program contains parts of the Rust standard library (`core`, `alloc`, `std`, `compiler_builtins` and their dependencies).",
        "Copyright (c) The Rust Project Developers (see https://thanks.rust-lang.org).",
        "The Rust standard library is licensed under `MIT OR Apache-2.0`. kb16-mixer uses it under the MIT licence. The text is in [Licence texts](#licence-texts).",
        "Some parts of it have other permissive licences, for example the Unicode tables of `core` (Unicode-3.0, text below).",
        "The file `share/doc/rust/COPYRIGHT-library.html` of each Rust toolchain lists all of them.",
        "",
        f"### HIDAPI C library {hidapi_version} (app)",
        "",
        "The `hidapi` crate compiles the HIDAPI C library (`windows/hid.c`) into the app.",
        "HIDAPI, Copyright Alan Ott / Signal 11 Software and contributors, used under the BSD-3-Clause licence option.",
        "",
        "```text",
        read_text(hidapi_dir / "etc" / "hidapi" / "LICENSE-bsd.txt").rstrip("\n"),
        "```",
        "",
        "### Open Sans (firmware)",
        "",
        "The logo and big font bitmaps in `firmware/src/assets.rs` are rendered from `firmware/assets/fonts/OpenSans-ExtraBold.ttf` and `OpenSans-SemiBold.ttf`.",
        "",
        "```text",
        read_text(fonts / "OFL.txt").rstrip("\n"),
        "```",
        "",
        "### X11 misc-fixed 5x7 (firmware)",
        "",
        "`firmware/src/font.rs` is generated from `firmware/assets/fonts/5x7.bdf`, the X11 misc-fixed 5x7 font (via https://github.com/embedded-graphics/embedded-graphics).",
        "It is in the public domain: its COPYRIGHT property says \"Public domain font. Share and enjoy.\"",
        "",
        "## Licence texts",
        "",
    ]

    rust_std = ("Rust standard library", "")
    if unicode_text is None:
        sys.exit("no Unicode-3.0 licence file found: update the Rust standard library notice in this script")
    texts.setdefault(("Unicode-3.0", unicode_text), set()).add(("Rust standard library (Unicode tables of core)", ""))
    blocks = []
    for (lic_id, text), crates in texts.items():
        blocks.append((lic_id, sorted(crates), False, text))
    for (lic_id, text), crates in fallbacks.items():
        copyrights = [crates[k] for k in sorted(crates)]
        if lic_id == "MIT":
            crates = {**crates, rust_std: "Copyright (c) The Rust Project Developers"}
            copyrights.append(crates[rust_std])
        notice = "\n".join(copyrights) + "\n"
        body, found = PLACEHOLDER.subn(lambda _: notice, text, count=1)
        blocks.append((lic_id, sorted(crates), True, body if found else notice + "\n" + text))
    for lic_id, crates, standard, text in sorted(blocks, key=lambda b: (b[0], b[1])):
        names = ", ".join(f"{n} {v}".strip() for n, v in crates)
        lines += [f"### {lic_id}: {names}", ""]
        if standard:
            lines += ["These components ship no licence file. This is the standard SPDX text, with a copyright line for each component. "
                      "For a crate, the line names the authors from its Cargo.toml.", ""]
        lines += ["```text", text.rstrip("\n"), "```", ""]
    lines += ["## Crates used only during the build", "",
              "| Crate | Version | Licence (declared) |", "|---|---|---|", *build_rows, ""]

    OUT.write_text("\n".join(lines), encoding="utf-8", newline="\n")
    fallback_crates = sum(len(c) for c in fallbacks.values())
    print(f"wrote {OUT.name}: {sum(len(r) for _, _, r in tables)} crate rows, {len(blocks)} licence texts, "
          f"{fallback_crates} crates with a standard text")


main()
