# Testing

This document is for contributors and reviewers. It tells you what the tests check, where the tests are, and how to run them.
It also lists which functions somebody tested on a real pad.
For the definition of each term, see [glossary.md](glossary.md).

## Summary

| Test suite | Folder | What it checks | Runs in CI |
|---|---|---|---|
| Protocol tests | `protocol/` | The bytes of each message, encode and decode, the version format | Yes |
| Firmware host tests | `pad-core/` | The firmware logic without hardware: key and knob decoding, the idle rule, the lights, the screens, the flash guard, the name page, the bootloader rule | Yes |
| Firmware image check | `scripts/build-firmware.sh` | The vector table, the interrupts and the size of the firmware image | Yes |
| App tests | `app/` | The config file, the trim rules, the key actions, the pad link, the settings rules | Yes |
| Forbidden API test | `app/tests/forbidden_apis.rs` | The app source, `Cargo.toml` and `Cargo.lock` contain no hook, input injection, process access or network API | Yes |
| End-to-end audio test | `app/tests/audio_e2e.rs` | The app with the real Windows audio system | No |
| Notices check | `scripts/third-party-notices.py` | `THIRD_PARTY_NOTICES.md` agrees with the dependencies | Yes |
| Dependency check | `deny.toml` | The licences, the advisories and the sources of the dependencies of each crate | Yes |
| Script tests | `scripts/test_*.py` | The STE checker, the privacy check and the release packager | Yes |
| Privacy check | `scripts/check-privacy.py` | No e-mail address, user path, account SID or token in the tracked files, and only noreply addresses in the commits | Yes |
| Link check | `scripts/check-links.py` | Each relative link and each anchor in the Markdown files | Yes |
| Release zip | `scripts/package-release.py` | The zip has the release layout, and two runs give the same bytes | Yes |
| Real pad | The pad on your desk | All functions that the automated tests cannot see | No |

The automated tests cannot see the hardware. They cannot press a key, turn a knob, look at the screen or measure the lights.
Thus the table [Verified on the real pad](#verified-on-the-real-pad) is the only record of hardware tests.

## Protocol tests

```sh
cd protocol
cargo test
```

The test `wire_layout_is_fixed` holds the exact bytes of each message.
If this test fails, you changed the wire format. Change the test only when you intend to change the wire format.

## Firmware host tests

The `pad-core` crate holds the part of the firmware that touches no register.
Its tests run on the PC. They do not need a pad.

```sh
cd pad-core
cargo test
```

The firmware crate itself has no tests, because its code needs the chip.
CI builds the image, and the image check examines it:

```sh
sh scripts/build-firmware.sh
```

## App tests

The app tests run on Windows only.

```sh
cd app
cargo test --locked
```

These tests do not need a pad.

## End-to-end audio test

This test uses the real Windows audio system. Thus it needs a Windows PC with an audio output device.
CI does not run it. Run it on your PC:

```sh
cd app
cargo test --test audio_e2e -- --ignored
```

## Script checks

The STE checker examines the Markdown documents. See [CONTRIBUTING.md](../CONTRIBUTING.md#documentation).
The other scripts check the repository. They need only Python 3.

```sh
python scripts/ste-lint.py
python scripts/check-privacy.py --history HEAD
python scripts/check-links.py
python -m unittest scripts/test_ste_lint.py scripts/test_check_privacy.py scripts/test_package_release.py scripts/test_flash_scripts.py
```

`test_flash_scripts.py` runs copies of `flash.sh` and `flash.ps1` with a fake `dfu-util` and a stub app. It never uses the pad or the real app. The `flash.ps1` tests run only on Windows.

For each problem, `check-privacy.py` shows the file, the line and the rule. It never shows the value that it found.

## Dependency check

`cargo deny` checks the dependencies of each crate with the settings in `deny.toml`:

```sh
cargo install cargo-deny --locked   # one time only
cargo deny --manifest-path app/Cargo.toml --config deny.toml check
```

Do the same for `protocol`, `pad-core` and `firmware`.
The check accepts only the licences that `scripts/about.toml` accepts, and only crates from crates.io. It refuses the crates for network access.
`deny.toml` ignores one advisory, RUSTSEC-2026-0110. It tells that the `bare-metal` crate has no maintainer. The firmware gets that crate through `cortex-m` 0.7.10, and no newer release exists.

Dependabot updates only the GitHub Actions (see `.github/dependabot.yml`). It does not update the crates, for two reasons:

- A crate update changes `THIRD_PARTY_NOTICES.md`. The `notices` job fails until somebody runs `python scripts/third-party-notices.py`.
- A firmware crate update can need a newer Rust than the version in `rust-toolchain.toml`.

To update the crates of one crate folder, run `cargo update` in that folder. Then run `python scripts/third-party-notices.py`, the tests and `cargo deny`. Commit the lock file and the notices together.

## CI

`.github/workflows/ci.yml` runs for each pull request, for each push to `main`, and when somebody starts it by hand.
A new push to a pull request stops the older run of that pull request.
The workflow pins each third-party action to a full commit SHA.

| Job | Runner | Steps |
|---|---|---|
| `protocol` | Linux | `cargo fmt --check`, `cargo clippy` (warnings are errors), `cargo test` |
| `pad-core` | Linux | `cargo fmt --check`, `cargo clippy` (warnings are errors), `cargo test` in the debug profile and in the release profile |
| `firmware` | Linux | `cargo fmt --check`, `cargo clippy` in both profiles (warnings are errors), the image build and the image check. It keeps the image as an artifact. |
| `app` | Windows | `cargo fmt --check`, `cargo clippy` (warnings are errors), `cargo test`, the release build, and a smoke test with `--check`. It keeps the exe as an artifact. |
| `notices` | Linux | Makes `THIRD_PARTY_NOTICES.md` again and compares it with the file in the repository |
| `docs` | Linux | The script tests, the STE checker, the privacy check, the link check, and a schema check of the workflows, the Dependabot file and the issue forms |
| `supply-chain` | Linux | `cargo deny` for each crate |
| `package` | Windows | Makes the release zip from the image and the exe of the run. It compares the zip with the release layout and with a second zip. |

The smoke test runs `kb16-mixer.exe --check`. The runner has no pad and no audio device, so the report has `FAIL` lines for the pad and for audio. The step accepts this, but it fails on a crash or on a report with the wrong form.
For the exact steps, read the workflow file. The workflow file is correct if this table and the file disagree.

## Release build

`.github/workflows/release.yml` runs when somebody pushes a version tag (for example `v0.1.0`).
It checks the versions, `CHANGELOG.md` and the tag. It then builds the firmware image and the app.
It makes a draft pre-release with one zip file, `SHA256SUMS` and a build attestation. It never publishes the release.
See [releasing.md](releasing.md).

## Verified on the real pad

This table records the tests on a real pad. The only test pad is one KB16-01 rev2 with Windows 11.
"VERIFIED" means that a person did the action on the pad and saw the result.
"NOT YET VERIFIED" means that nobody did the action on a real pad. Automated tests can still cover the logic.

| Feature | How it was verified | Date | Firmware version | Status |
|---|---|---|---|---|
| Knob volume for each channel | Manual use on the pad. There is no written test record. | Before 2026-11-11 (no exact date in the record) | An earlier build, before 0.1.0 | VERIFIED |
| Mute row | Manual use on the pad. There is no written test record. | Before 2026-11-11 (no exact date in the record) | An earlier build, before 0.1.0 | VERIFIED |
| Key light and screen output | Manual use on the pad. There is no written test record. | Before 2026-11-11 (no exact date in the record) | An earlier build, before 0.1.0 | VERIFIED |
| Recovery with the top-left key at plug-in | Manual use on the pad. There is no written test record. | Before 2026-11-11 (no exact date in the record) | An earlier build, before 0.1.0 | VERIFIED |
| Key pages | | | | NOT YET VERIFIED |
| Knob press to select one program | | | | NOT YET VERIFIED |
| The idle fix and its thresholds | | | | NOT YET VERIFIED |
| The logo name in flash | | | | NOT YET VERIFIED |
| `open:` | | | | NOT YET VERIFIED |
| Live clicks in the settings window (**Browse...**, the **Add...** menu) | | | | NOT YET VERIFIED |
| The match on the product string, and the wake-up when a device arrives | | | | NOT YET VERIFIED |
| `flash.sh` and `flash.ps1` from start to end | | | | NOT YET VERIFIED |
| Bootloader after three crashed starts in a row | | | | NOT YET VERIFIED |
| Bootloader entry from the stock firmware, and restore of a stock backup | | | | NOT YET VERIFIED |
| Settings window at 125 % and 150 % display scale | | | | NOT YET VERIFIED |
| No flicker of the key lights | | | | NOT YET VERIFIED |
| The physical order of the knobs | | | | NOT YET VERIFIED |
| The feel of Focus, Talk and Move | | | | NOT YET VERIFIED |

Nobody verified the functions that are not in this table on a real pad.
If you test a function on your pad, report the result in an issue. Give the firmware version and the app version.
