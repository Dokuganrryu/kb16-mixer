# Testing

This document tells you what the tests check, where the tests are, and how to run them.
It also lists which functions somebody tested on a real pad.
For the definition of each term, see [glossary.md](glossary.md).

## Summary

| Test suite | Folder | What it checks | Runs in CI |
|---|---|---|---|
| Protocol tests | `protocol/` | The bytes of each message, encode and decode, the version format | Yes |
| Firmware host tests | `pad-core/` | The firmware logic without hardware: key and knob decoding, the idle rule, the lights, the screens, the flash guard, the name page, the bootloader rule | Yes |
| Firmware image check | `scripts/build-firmware.sh` | The vector table, the interrupts and the size of the firmware image | Yes |
| App tests | `app/` | The config file, the trim rules, the key actions, the pad link, the settings rules | Yes |
| End-to-end audio test | `app/tests/audio_e2e.rs` | The app with the real Windows audio system | No |
| Notices check | `scripts/third-party-notices.py` | `THIRD_PARTY_NOTICES.md` agrees with the dependencies | Yes |
| STE checker tests | `scripts/test_ste_lint.py` | The rules of the STE checker | No |
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

## STE checker

The STE checker examines the Markdown documents. See [CONTRIBUTING.md](../CONTRIBUTING.md#documentation).

```sh
python scripts/ste-lint.py
python -m unittest scripts/test_ste_lint.py
```

## CI

`.github/workflows/ci.yml` runs these jobs:

| Job | Runner | Steps |
|---|---|---|
| `protocol` | Linux | `cargo fmt --check`, `cargo clippy`, `cargo test` |
| `firmware` | Linux | `cargo fmt --check`, `cargo clippy`, the `pad-core` tests, the image build and the image check |
| `app` | Windows | `cargo fmt --check`, `cargo clippy`, `cargo test`, the release build |
| `notices` | Linux | Makes `THIRD_PARTY_NOTICES.md` again and compares it with the file in the repository |

For the exact steps, read the workflow file. The workflow file is correct if this table and the file disagree.

## Release build

`.github/workflows/release.yml` runs when somebody pushes a version tag (for example `v0.1.0`).
It runs the same checks as CI, and builds the firmware image and the app.
It then makes a draft release with one zip file, `SHA256SUMS` and a build attestation.
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
