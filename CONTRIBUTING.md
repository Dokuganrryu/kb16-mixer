# Contributing

Thank you for your help. This document tells you how to build and test the project, which rules the code follows, and how to report a bug.

## Repository layout

| Folder | Contents |
|---|---|
| `protocol/` | `kb16-mixer-protocol`: the raw HID wire format and the version format, shared by the firmware and the app |
| `firmware/` | `kb16-mixer-firmware`: the Embassy firmware for the pad |
| `app/` | `kb16-mixer`: the Windows tray app |
| `scripts/` | Build, flash and notice scripts |
| `docs/` | Documentation |
| `.github/workflows/` | CI (`ci.yml`) and releases (`release.yml`) |

Each crate has its own `Cargo.toml` and `Cargo.lock`. There is no Cargo workspace.

## Build and test

For the tools that you need, see [docs/flashing.md](docs/flashing.md#prerequisites).
CI (`.github/workflows/ci.yml`) runs the same commands as below. Run them before you open a pull request.

### Protocol

```sh
cd protocol
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

### Firmware

```sh
cd firmware
cargo fmt --check
cargo clippy --release --locked
cd ..
sh scripts/build-firmware.sh
```

The firmware has no unit tests. CI builds the image and checks it.

To add start-up stage markers to the crash log page, build with `sh scripts/build-firmware.sh --diag`.
This writes `firmware/target/kb16-mixer-firmware-diag.bin`.

### App (Windows only)

```sh
cd app
cargo fmt --check
cargo clippy --all-targets --locked
cargo test --locked
cd ..
sh scripts/build-app.sh
```

To try the app without a change to your real settings, set `KB16_MIXER_DIR` to an empty test folder.
With `--no-pad`, a second copy can run next to the copy that has the pad.

### Screen assets

`firmware/src/assets.rs` and `firmware/src/font.rs` are generated files. Do not edit them.
To make them again, run `firmware/gen_assets.py`. It needs Python 3 and Pillow:

```sh
uv run --with pillow python firmware/gen_assets.py
```

The script also writes `firmware/assets/preview.png`, a preview of the screen at 4 times the size.
An optional argument sets the name in the preview only. The pad draws the name that the app sends.

### Third-party notices

`THIRD_PARTY_NOTICES.md` is a generated file. Do not edit it.
When you add, remove or update a dependency, run this command from the repository root:

```sh
python scripts/third-party-notices.py
```

The script fails if a dependency has a licence that is not in its permissive list.
Use only dependencies with permissive licences.

## The image check and why it exists

`scripts/build-firmware.sh` checks each firmware image before it writes it. It rejects the image if:

- the initial stack pointer is not in RAM.
- the reset vector is not in the firmware region (above `0x08002000`).
- the TIM2, USB, I2C2 or DMA1 channel 4 interrupt goes to `DefaultHandler`.
- the image is larger than 117 KB (`0x1D400` bytes). It then overlaps the name page at `0x0801F400` and the crash log after it.

The interrupt check exists because of a real failure.
Without the `rt` feature of embassy-stm32, all interrupts go to `DefaultHandler`, also with `default-features = false`.
Such an image builds without an error. On the pad, it stops at the first timer interrupt, with no lights.
The check finds this problem before the image gets to a pad.

Do not remove or weaken the check. If you add an interrupt that the firmware needs, add it to the check.
If you move the name page or the crash log, change `firmware/memory.x` and the size check together.

## Code rules

### App

The app must stay out of the way of other software. Thus these rules are strict:

- Do not add keyboard hooks or mouse hooks.
- Do not inject input (for example `SendInput`). Keys that type come only from the pad, as USB keyboard input.
- Do not enable the `Win32_UI_Input_KeyboardAndMouse` feature of the `windows` crate. A test in `app/src/main.rs` fails if it is enabled.
- Do not add or load a driver.
- Do not open, read or write other processes. Get program names only from the Toolhelp snapshot.
- Do not route, capture or record audio. Change only volume and mute through Core Audio.
- Do not add network access.
- Start an `open:` target only through the Windows shell, without elevation.

A pull request that breaks one of these rules will not be merged.

### Protocol

- Change the wire format only in `protocol/`.
- Add or change a test in `protocol/src/lib.rs` for each change.
- The test `wire_layout_is_fixed` holds the exact bytes. Change it only when you intend to change the wire format.
- Update [docs/protocol.md](docs/protocol.md) in the same pull request.

### Firmware

- Never write the bootloader region (`0x08000000` to `0x08001FFF`).
- Keep the three ways into the bootloader (top-left key at start, four corner keys, the bootloader message).
- Keep the watchdog and the crash counter.
- Do not write flash more often than necessary. The name page has a limit of one write each minute.

### All code

- Format the code with `cargo fmt`. The settings are in `rustfmt.toml`.
- Fix all Clippy warnings.
- Keep build-machine paths out of release binaries. The build scripts do this with `scripts/remap-paths.sh`.

## Versions and releases

The app and the firmware each take their version from their `Cargo.toml`.
The firmware sends its version as the USB device release number. Thus the major version must be 0–99, and the minor and patch versions must be 0–9.

`.github/workflows/release.yml` runs when you push a tag that starts with `v` (for example `v0.1.0`). It does these steps:

1. It makes sure that the tag is the same as the versions in `app/Cargo.toml` and `firmware/Cargo.toml`.
2. It runs the same checks as CI, and builds the firmware image and the app.
3. It makes a GitHub release with the firmware image, the app, `LICENSE`, `THIRD_PARTY_NOTICES.md` and `SHA256SUMS.txt`.

No release exists yet. A first release follows after more tests on hardware.

## Pull requests

In each pull request, say what you tested on hardware and what you did not test.
For example: "Flashed on a KB16-01 rev2, Windows 11. Tested knob turns and mute keys. Not tested: Focus, Move."
If you did not test on hardware, say so. The pull request template asks for this.

## Report a bug

Open an issue with the bug report template. Give this information:

1. The versions. The first line of the tray menu shows them, for example `KB16 mixer 0.1.0: pad connected, firmware 0.1.0`. The **Pad** tab of the settings window shows them too.
2. The release tag or the git commit, if you built from source.
3. The hardware: the pad model and revision (for example KB16-01 rev2).
4. The Windows version.
5. The steps that cause the problem, what you expected, and what occurred.
6. The log file of the app: `log.txt` in the settings folder (`%APPDATA%\kb16-mixer`).
7. If the problem is about channels or levels: run `kb16-mixer.exe --selftest`, then attach `selftest.txt` from the settings folder.
8. If you changed the config file: attach `config.toml` from the settings folder.

> **CAUTION:** `selftest.txt` lists the names of the programs that play sound, with their process IDs. `config.toml` and `log.txt` can contain program names and paths (for example of `open:` targets).
> Read the files before you attach them, and remove what you do not want to share.

For a firmware crash, the pad keeps a crash log in flash. See [docs/hardware.md](docs/hardware.md#crash-log).

To report a security problem, do not open an issue. See [SECURITY.md](SECURITY.md).
