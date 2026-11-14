# Contributing

Thank you for your help. This document tells you how to make your first contribution.
For the definition of each term, see [docs/glossary.md](docs/glossary.md).
All contributors follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Repository layout

| Folder | Contents |
|---|---|
| `protocol/` | `kb16-mixer-protocol`: the raw HID wire format and the version format, shared by the firmware and the app |
| `pad-core/` | `kb16-mixer-pad-core`: the firmware logic that touches no hardware register, also the route of each key and knob. Its tests run on the PC. |
| `firmware/` | `kb16-mixer-firmware`: the Embassy firmware for the pad |
| `app/` | `kb16-mixer`: the Windows tray app |
| `scripts/` | Build, flash, notice and STE scripts |
| `docs/` | Documentation |
| `.github/` | CI, releases, and the templates for issues and pull requests |

Each crate has its own `Cargo.toml` and `Cargo.lock`. There is no Cargo workspace.

## Your first contribution

1. Find an issue, or open one that tells about your change.
2. Fork the repository on GitHub.
3. Make a branch for your change.
4. Do the setup for your type of change. See the next sections.
5. Make the change.
6. Run the checks for each part that you changed.
7. Open a pull request. Complete the template.

### Setup for documentation

Python 3 and a text editor are necessary. Rust and a pad are not necessary.
Write in ASD-STE100 Simplified Technical English. See [Documentation](#documentation).

### Setup for the app

Windows 10 or 11 and the Rust toolchain are necessary. For the steps, see [docs/build-from-source.md](docs/build-from-source.md).
A pad is not necessary. To try the app without a change to your settings, use a test folder:

```sh
KB16_MIXER_DIR="$TEMP/kb16-test" ./app/target/release/kb16-mixer.exe --no-pad
```

`KB16_MIXER_DIR` replaces the settings folder. With `--no-pad`, the copy does not look for the pad.
Thus it can run next to the copy that has the pad.

### Setup for the firmware

The Rust toolchain with the `thumbv7m-none-eabi` target and `llvm-tools` is necessary. For the steps, see [docs/build-from-source.md](docs/build-from-source.md).
You can change most firmware logic in `pad-core/` and do its tests without a pad.
For a test on hardware, a KB16-01 rev2 is necessary. Read [docs/flashing.md](docs/flashing.md) before you write an image to a pad.

## Checks

For a full list of the test suites, see [docs/testing.md](docs/testing.md).
CI runs the same checks for each pull request and for each push to `main`. Run them before you open a pull request.
CI also runs `cargo check` for each crate with the oldest Rust version that its `Cargo.toml` gives in `rust-version`. Thus do not use a Rust feature that is newer than that version.

| Part | Commands |
|---|---|
| Protocol | `cd protocol`, then `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` |
| Firmware logic | `cd pad-core`, then `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`, `cargo test --locked --release` |
| Firmware image | `cd firmware`, then `cargo fmt --check`, `cargo clippy --release --locked -- -D warnings`, `cargo clippy --locked -- -D warnings`. Then, in the root folder: `sh scripts/build-firmware.sh` |
| App | `cd app`, then `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`. Then, in the root folder: `sh scripts/build-app.sh` |
| End-to-end audio test | `cd app`, then `cargo test --test audio_e2e -- --ignored`. CI does not run it. |
| Documentation | `python scripts/ste-lint.py` and `python scripts/check-links.py` |
| Personal data | `python scripts/check-privacy.py --history HEAD`. See [Personal data](#personal-data). |
| Dependencies | `cargo deny --manifest-path <crate>/Cargo.toml --config deny.toml check` for each crate that you changed. See [docs/testing.md](docs/testing.md#dependency-check). |
| Scripts | `python -m unittest scripts/test_ste_lint.py scripts/test_check_privacy.py scripts/test_package_release.py scripts/test_flash_scripts.py` |

## Documentation

The documents use ASD-STE100 Simplified Technical English (STE). These are the most important rules:

- Write 20 words or fewer in a procedure step. Write 25 words or fewer in other sentences.
- Write one instruction in each sentence.
- Use the active voice and the simple present tense.
- Do not use contractions or semicolons.
- Do not put more than three nouns in a row.
- Use one word for one thing. The definitions are in [docs/glossary.md](docs/glossary.md).
- Do not write that a function works if nobody did a test of it. Write "not tested" in its place.
- Use the words that the ASD-STE100 dictionary approves. For a technical name that the dictionary does not have, add a definition to [docs/glossary.md](docs/glossary.md).

`scripts/ste-lint.py` finds many breaks of these rules:

```sh
python scripts/ste-lint.py               # all Markdown files
python scripts/ste-lint.py docs/faq.md   # one file
python scripts/ste-lint.py --summary     # counts for each rule and each file
```

The exit code is 1 when the checker finds a problem.
The checker is a heuristic helper. It is not a certified STE checker. It does not find all problems, and it can show a problem where there is none.

The `vocabulary` rule knows only a short table of words that the dictionary does not approve. Each message gives the approved alternative. A word that is not in the table can also be incorrect.
A person must read the text.

If the checker shows an incorrect problem, rewrite the sentence. Add an exception to `scripts/ste-lint.allow` only for a quotation, a standard text or a technical name that cannot change. Each exception must have a reason.

## The image check and why it exists

`scripts/build-firmware.sh` checks each firmware image before it writes it. It rejects the image if one of these conditions applies:

- The initial stack pointer is not in RAM.
- The reset vector is not in the firmware region (above `0x08002000`).
- The TIM2, USB, I2C2 or DMA1 channel 4 interrupt goes to `DefaultHandler`.
- The image is larger than 117 KB (`0x1D400` bytes). It then overlaps the name page at `0x0801F400` and the crash log after it.

The interrupt check exists because of a failure that occurred on the pad.
Without the `rt` feature of embassy-stm32, all interrupts go to `DefaultHandler`, also with `default-features = false`.
The build of such an image shows no error. On the pad, the image stops at the first timer interrupt, with no lights.
The check finds this problem before somebody writes the image to a pad.

Do not remove or weaken the check. If you add an interrupt that the firmware uses, add it to the check.
If you move the name page or the crash log, change `firmware/memory.x` and the size check together.

## Code rules

### App

The app must not interfere with other software. Thus these rules are strict:

- Do not add keyboard hooks or mouse hooks.
- Do not inject input (for example `SendInput`). Keys that type come only from the pad, as USB keyboard input.
- Do not enable the `Win32_UI_Input_KeyboardAndMouse` feature of the `windows` crate.
- Do not add or load a driver.
- Do not open, read or write other processes. Get program names only from the Toolhelp snapshot.
- Do not route, capture or record audio. Change only volume and mute through Core Audio.
- Do not add network access.
- Start an `open:` target only through the Windows shell, without elevation.

The test `app/tests/forbidden_apis.rs` examines the app code, `Cargo.toml` and `Cargo.lock`. It gives an error if they name a hook, an input injection, a process access or a network API. It also gives an error for the network crates and for the features `Win32_UI_Input_KeyboardAndMouse` and `Win32_Networking`.
The test checks the names also in comments.

The project does not merge a pull request that breaks one of these rules.

### Protocol

- Change the wire format only in `protocol/`.
- Add or change a test in `protocol/src/lib.rs` for each change.
- The test `wire_layout_is_fixed` holds the correct bytes. Change it only when you intend to change the wire format.
- Update [docs/protocol.md](docs/protocol.md) in the same pull request.

### Firmware

- Do not write the bootloader region (`0x08000000` to `0x08001FFF`).
- Keep the three procedures into the bootloader: the top-left key at start, the four corner keys, and the bootloader message.
- Keep the watchdog and the crash counter.
- Do not write flash more often than necessary. The name page has a limit of one write each minute, and of 8 writes for each power-up.
- Put logic that uses no register into `pad-core/`. Add a test there.
- Give each `unsafe` block a `// SAFETY:` comment.

### All code

- Format the code with `cargo fmt`. The settings are in `rustfmt.toml`.
- Correct all Clippy warnings in each crate that you change.
- Do not use `unwrap()` or `expect()` on a path that can have an error at run time. Tests can use them.
- Write comments that say why, not what.
- Keep build-machine paths out of release binaries. The build scripts do this with `scripts/remap-paths.sh`.

### Screen assets

A script makes `pad-core/src/assets.rs` and `pad-core/src/font.rs`. Do not edit these files.
To make them again, run `firmware/gen_assets.py`. It uses Python 3 and Pillow:

```sh
uv run --with pillow python firmware/gen_assets.py
```

The script also writes `firmware/assets/preview.png`, a preview of the screen at 4 times the size.

## Dependencies and licences

Use only dependencies with a permissive licence: MIT, Apache-2.0, BSD, ISC, 0BSD, Zlib or CC0. Fonts can also use the OFL.
`cargo deny` and `scripts/third-party-notices.py` accept only the licences that the dependencies use at this time: MIT, Apache-2.0 and Unicode-3.0. To add a dependency with a different licence from the list above, add that licence to `deny.toml` and `scripts/about.toml` in the same commit.
Do not copy code from a source with the GPL, the LGPL or an unknown licence.
Give the reason for a new dependency in the commit message.

`THIRD_PARTY_NOTICES.md` is a generated file. Do not edit it.
It holds the copyright notices and the full licence texts of all third-party code in the firmware image and in the app.
When you add, remove or update a dependency, run these commands in the root folder:

```sh
cargo install cargo-about --version 0.9.3 --locked --features cli   # one time only
python scripts/third-party-notices.py
```

The script uses `cargo-about` with the settings in `scripts/about.toml`.
It stops with an error if a dependency has a licence that is not in the `accepted` list of that file.
CI gives an error if `THIRD_PARTY_NOTICES.md` is not the same as the output of the script.

## Personal data

Do not put personal data into the repository. Personal data includes the names of persons, e-mail addresses, user names, machine names, serial numbers, and paths that contain a user name (for example `C:\Users\<name>\...`).
This rule is for files, screenshots, test data, logs and commit messages.
Use neutral examples, for example the pad name `DESK PC`.
Do not attach your own `config.toml`, `log.txt` or `selftest.txt` to a pull request.

`scripts/check-privacy.py` finds many of these problems. CI runs it for each pull request:

```sh
python scripts/check-privacy.py --history HEAD
```

It examines each text file that git tracks. It finds e-mail addresses, paths with a user name, Windows account SIDs and tokens.

With `--history`, it also examines the author and committer addresses of the commits. It accepts only GitHub noreply addresses.
It also accepts the bot form `<digits>+<name>[bot]@users.noreply.github.com`, which Dependabot uses. It accepts no other form.
To keep your address private, set `git config user.email` to the noreply address that GitHub shows in your e-mail settings.

## Commits and pull requests

- Make small commits. Make one commit for each change.
- Write the subject line of a commit in the imperative, for example "Add a test for the trim rule".
- Make one pull request for each change.
- In the pull request, say which tests you did on hardware, and which tests you did not do. For example: "Flashed on a KB16-01 rev2, Windows 11. Tests done: knob turns and mute keys. Not tested: Focus, Move."
- If you did no test on hardware, say so.
- Update the documentation and `CHANGELOG.md` in the same pull request.

## Versions and releases

The app and the firmware each get their version from their `Cargo.toml`.
The firmware sends its version as the USB release number. Thus the major version must be 0 to 99. The minor version and the patch version must be 0 to 9.

The maintainer makes each release. The full checklist is in [docs/releasing.md](docs/releasing.md). In short:

1. Set the same version in the four `Cargo.toml` files.
2. Move the entries of the "Unreleased" section of `CHANGELOG.md` to a new version section with a date.
3. Push a tag of the form `v<major>.<minor>.<patch>`, for example `v0.3.0`. A suffix such as `v0.4.0-rc.1` is also possible.

The tag starts `.github/workflows/release.yml`. The workflow checks the versions, builds the firmware image and the app, and makes a draft pre-release.
The workflow does not publish a release. The maintainer does a test of the draft on a pad. If it is correct, the maintainer publishes it.

## Report a bug

Open an issue with the "Pad does not work" form or the bug report template.
The form asks for the output of `kb16-mixer.exe --check`. That report contains no personal data.

For a firmware crash, the pad keeps a crash log in flash. See [docs/hardware.md](docs/hardware.md#crash-log).

To report a security problem, do not open an issue. See [SECURITY.md](SECURITY.md).
