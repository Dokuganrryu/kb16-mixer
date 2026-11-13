# Changelog

This file lists all important changes to this project.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The version numbers follow [SemVer 2.0.0](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A CI job `msrv` checks each crate with the oldest Rust version that its `Cargo.toml` gives in `rust-version`.
- A weekly workflow runs `cargo deny` for the four crates on `main`, so a new advisory shows without a push.

### Fixed

- The firmware sends a knob release again when the USB bus did not accept it. Before, a lost release could make a tap into a hold.
- The flash wait limit comes from the longest page erase (40 ms) with a margin of 2: 1,440,000 polls. It was 1,000,000.
- After a crash while Focus is on, each live session gets its own level back. Before, all sessions of one program got one level.
- After a crash while you hold **Talk**, the app sets the mute state of the microphone back at the next start.

### Changed

- The documents say that a local build of the firmware does not give the bytes of the CI build. To examine a release, use `SHA256SUMS` and the attestation.

## [0.2.0] - 2026-11-13

This is the first public version. It is a pre-release.

Version 0.1.0 was an internal version. Nobody published it. The firmware of 0.2.0 sends the version 0.2.0. Thus `--check` and the settings window tell it from the firmware of 0.1.0.

Nobody did a test of version 0.2.0 on a pad. The hardware tests in the section "Tests" used earlier builds.

The sections "Added", "Changed", "Removed" and "Fixed" list the work of the quality rounds after the first two independent reviews. The other sections list the parts of the first version.

### Added

- `kb16-mixer.exe --check` prints a short report about the app, the pad, the settings, **Start with Windows** and audio. The report contains no personal data, so you can paste it into a bug report.
- The `pad-core` crate holds the firmware logic that touches no hardware register. Its tests run on the PC and in CI. It holds the route of each key and knob, the keyboard report, and the bootloader marks and their order. It also holds the order of the flash steps (behind a `Controller` trait) and the write limit of the name page.
- A flash guard in the firmware. The firmware writes flash only in the name page and in the crash log. It also reads the error flags of the flash controller after each write (PGERR and WRPRTERR).
- An end-to-end audio test for the app: `cargo test --test audio_e2e -- --ignored`. A Windows PC with an audio output device is necessary. CI does not run it.
- A release workflow. For a version tag, it checks the versions and the changelog, and builds one zip file, `SHA256SUMS` and a build attestation. It makes only a draft pre-release. It does not publish a release. A manual dry run and a run for the internal tag `v0.1.0` passed.
- `scripts/package-release.py` makes the release zip. It rejects a bad firmware image or exe. The zip is reproducible.
- `scripts/check-privacy.py` finds e-mail addresses, user paths, account SIDs and tokens in the repository, and other addresses than GitHub noreply addresses in the commits. The check accepts the bot form that Dependabot uses.
- `scripts/check-links.py` checks the relative links and the anchors in the Markdown files.
- `app/tests/forbidden_apis.rs` gives an error if the app names a hook, an input injection, a process access or a network API.
- `deny.toml` for `cargo deny`: licences, advisories, sources (crates.io only) and the network crates.
- Dependabot opens one grouped update pull request each week for the GitHub Actions. It does not update the crates: `cargo deny` reports advisories, and a maintainer updates the crates by hand.
- `scripts/flash.ps1`, a flash script for Windows PowerShell. It writes only to the bootloader (USB `1EAF:0003`) and only a valid image. It can make a backup first.
- `scripts/flash.sh` checks the image as `flash.ps1` does.
- `scripts/test_flash_scripts.py` runs the two flashers with a fake `dfu-util` and a stub app. Each stop with `REFUSED` has a test. CI runs the `flash.ps1` tests on Windows. No test runs a flasher on a pad.
- `scripts/ste-lint.py`, a checker for the mechanical rules of Simplified Technical English in the documents. Its `vocabulary` rule finds words from a short table of words that the STE dictionary does not approve.
- New documents in `docs/`:
  - `install.md`, `troubleshooting.md` and `faq.md` for users.
  - `stock-firmware.md` for the firmware in the pad before KB16 Mixer (the stock firmware).
  - `build-from-source.md` with the build steps for Windows.
  - `glossary.md` with one definition for each term.
  - `testing.md` with the test suites and the list of hardware tests.
  - `roadmap.md` and `releasing.md`.
- `CODE_OF_CONDUCT.md` (Contributor Covenant 2.1).
- `rust-toolchain.toml` pins Rust 1.99.1.
- Pictures of the five pad screens in `docs/images/`. The code that draws the firmware screens makes them (`pad-core/examples/render_oled.rs`).
- Tests: no 32-byte report makes a protocol decoder panic, and the option tables of the settings window do not change by accident.
- `docs/flashing.md` gives a last-resort recovery with an SWD debug probe. Nobody did a test of it.
- An issue form for a pad that does not work. It asks for the output of `--check`.
- A knob has a tap, a hold (350 ms) and a double tap. By default, a tap mutes or unmutes the knob target, and a hold selects the next program in the channel. The new `[knob_press]` table changes the actions.
- Mute keys, `open:` keys and `none` keys can have a hold and a double tap: `[keys_hold]`, `[keys_double]`, `[keys2_hold]` and `[keys2_double]`.
- Protocol: the pad sends `KNOB_UP` (`0x05`) when you release a knob, while the app is present. The app does not wait for it, because the firmware of the internal version 0.1.0 does not send it. An earlier app ignores the message.
- Protocol: byte 19 of `SETTINGS` (`idle_led_percent`) sets the light of the dimmed key lights. The new setting is `[lights] idle_percent`. The firmware of the internal version 0.1.0 ignores the byte.
- `[audio] output`: `"default"` follows the Windows default output device when it changes. A device ID selects one device. While that device is disconnected, the app uses the Windows default.
- The `audio` line of `--check` tells which output device the app uses.
- New controls in the settings window: **Output device** and **Dim lights to** (**Pad** tab), and the knob tap, hold and double tap (**Keys** tab).
- Screen motion: the meters glide, a readout pushes in and out, and the page slides. A channel name flashes when its mute changes. The screen fades before it goes off.

### Changed

- CI runs for pull requests and for pushes to `main`, not for each branch push. A new push to a pull request stops the previous run.
- CI pins each third-party action to a full commit SHA, and gives the workflow only read access.
- CI runs Clippy on the firmware with warnings as errors. It runs the `pad-core` tests also in the release profile.
- CI has new jobs: a smoke test of the app with `--check`, the script checks, `cargo deny`, and a dry run of the release zip.
- The app is a library and a small program, so that tests can use the library. The worker, the tray, `--check`, `open:` and **Start with Windows** each have their own module.
- The settings window code is in the folders `app/src/settings/` and `app/src/dialog/`.
- The settings window has a new look. It has dark and light themes that follow Windows, smooth rounded controls and short animations that respect the Windows animation setting. It also has per-monitor scaling, and the same controls and keyboard use as before.
- Each `unsafe` block in the app and in the firmware has a `SAFETY` comment. Clippy rejects a block without one.
- The documents follow the rules of Simplified Technical English more strictly.
- `SECURITY.md` gives the threat model. It says that the bootloader marker `BOOT` does not change and is not a secret.
- The documents tell that an `open:` target with administrator rights shows the User Account Control (UAC) prompt of Windows. Nobody did a test of this.
- `deny.toml` and `scripts/about.toml` accept only the licences that the dependencies use: MIT, Apache-2.0 and Unicode-3.0.
- The `protocol` crate forbids unsafe code.
- The settings window test shows `SKIPPED` when Windows cannot make a window. When `CI` has a value, it gives an error in that case.
- A knob tap mutes. Before, a knob press selected the next program. The hold does that at this time.
- The key lights glide to each new level, with gamma 2.4 and a dither in quarter steps. Before, the light changed in visible steps.
- Idle: the key lights dim to 6 % of their light in 0.7 seconds. After three times `lights_dim_min` (at most 240 minutes), they go off. Before, they dimmed to approximately 14 % and stayed on.
- New default channel tints, all ice white with a different warmth: Mic `#e6dcd7`, Chat `#c8d7ff`, Game `#afcdff`, Media `#d7def5`. A `config.toml` with a `color` for each channel keeps its colours.
- The rule for audible sound uses the output device of each program. Before, it used only the Windows default device.

### Removed

- The app does not copy settings from the folder of an earlier private name of the project. It does not remove an autostart value of that name. No public version used that name.

### Fixed

- `flash.ps1` waits for `kb16-mixer.exe --bootloader` and reads its exit code. Before, the message about the bootloader request could be incorrect.
- `flash.ps1 -Backup <file> -DryRun` without a pad in the bootloader stops with `REFUSED`. Before, it showed "DRY RUN complete" and made no backup.
- `flash.ps1` also looks for `dfu-util` in the `tools` folder of the repository root, as `flash.sh` does.
- The keyboard: a key release frees only what that key pressed. Before, key A had `ctrl+c` and key B had `c`. A tap of B while you held A released Ctrl, and `c` repeated. A key with modifiers only holds its modifiers. The app does not send such a key, thus this change shows only with a different host program.
- A `focused` channel or knob: when the desktop or the taskbar is in front, no program is in front. Before, the channel got all programs that Explorer started, and a knob turn set all of them to one level.
- When the desktop is in front, a focused channel or knob treats it as no program in front. It does the same when the front program is not in the program list.
- If the app cannot read the level of a session when Focus starts, it does not turn that session down. Before, it kept 100 % as the level and later set the program to 100 %.
- The app writes `state.toml` to the disk before it renames the new file over the previous file.
- `THIRD_PARTY_NOTICES.md` names `pad-core/src/assets.rs` and `pad-core/src/font.rs`, the files that moved there.
- `flash.sh` shows the last step (unplug the pad) only after `dfu-util` shows `Download done`. Otherwise it stops with exit code 1.
- `nan`, `inf` and `-inf` for `step`, `brightness` or `duck` in `config.toml` are errors. Before, `nan` went through the range check.
- `--settings` accepts only the full tab names `keys`, `channels` and `pad`, in upper case or lower case. Before, `--settingsx` also opened the window.

### Firmware

- Embassy firmware for the DOIO KB16-01 rev2 (APM32F103CB). The firmware starts at `0x08002000`. It does not write the stm32duino bootloader.
- USB HID keyboard, media keys and a raw HID interface (usage page `0xFF60`). The pad uses no special driver.
- Mixer mode with the app: level meters on the 16 key lights, and channel names, levels and volumes on the screen, with readouts and notices.
- Keyboard mode without the app: number keys and arrows. The knobs send media keys and page keys.
- Two key pages. The screen shows `MIXER` or `KEYS`, and a mark while page 2 is active.
- A logo name in its own flash page at `0x0801F400`. Thus the pad shows the name at power-up. The firmware writes the page at most one time each minute, and at most 8 times for each power-up.
- Idle timers that count only keys, knobs and audible sound as activity. The app makes the decision about what is audible.
- The firmware does not lose a key release. On USB suspend or reset, the pad releases all typed keys.
- The firmware version in the release number of the USB device.
- Three procedures that put the pad into the bootloader: the top-left key at start, the four corner keys for 2 seconds, and an app message.
- A watchdog and a restart after a crash. The design stops the firmware in the bootloader after three crashed starts in a row. Nobody did a test of this on the pad.
- A crash log in the flash page at `0x0801F800`.

### App

- A Windows tray app that controls the volume and mute of programs in the Windows volume mixer, and the microphone.
- Up to 4 channels, with names of 1 to 5 characters. A channel is a list of programs, the microphone, all other programs, or the foreground program.
- A knob hold selects one program in a channel. Each program keeps its balance in the channel (trim), also at 0 % and 100 %.
- The app tracks each audio session separately. Focus gives each session its previous level back, also for programs that closed while Focus was on.
- The app handles key presses and releases in the order of arrival.
- Key actions: **Mute**, **Focus**, **Talk**, **Move**, **Switch page** and `open:<target>`. The pad types all other keys.
- A settings window with the tabs Keys, Channels and Pad, with a shortcut dialog and an open dialog. A text box uses its value when you press Enter or go to a different control.
- `config.toml`. If the file has an error, the app keeps the last correct settings.
- The versions of the app and the firmware in the tray menu and on the Pad tab.
- The first start opens the settings window. The app does not start with Windows until you select that option.
- At quit, logoff and shutdown, the app gives all levels back. It waits 3 seconds at most.
- The options `--settings[=keys|channels|pad]`, `--no-pad`, `--bootloader`, `--selftest`, `--help`, and the environment variable `KB16_MIXER_DIR`.
- No keyboard hooks, no input injection, no driver, and no access to other processes. A test makes sure that the keyboard and mouse module of the `windows` crate stays off.

### Protocol

- `kb16-mixer-protocol`: the shared raw HID wire format and the version format, with tests.

### Build

- CI for all crates, with an image check for the firmware.

### Tests

For the full list of hardware tests, see [docs/testing.md](https://github.com/Dokuganrryu/kb16-mixer/blob/v0.2.0/docs/testing.md#verified-on-the-pad).

[Unreleased]: https://github.com/Dokuganrryu/kb16-mixer/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/Dokuganrryu/kb16-mixer/releases/tag/v0.2.0
