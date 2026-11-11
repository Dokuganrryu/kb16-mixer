# Changelog

All important changes to this project are in this file.

## Unreleased / 0.1.0: first public version

No release exists yet. Build from source for now.

### Firmware

- Embassy firmware for the DOIO KB16-01 rev2 (APM32F103CB). It is written at `0x08002000`. The stm32duino bootloader is never written.
- USB HID keyboard, media keys and a raw HID interface (usage page `0xFF60`). No driver is necessary.
- Mixer mode with the app: level meters on the 16 key lights, channel names, levels and volumes on the screen, readouts and notices.
- Keyboard mode without the app: number keys, arrows and media knobs.
- Two key pages. The screen shows `MIXER` or `KEYS`, and a mark while page 2 is active.
- A name on the logo screen, kept in its own flash page at `0x0801F400`, so that the pad shows it at power-up. The page is written at most one time each minute and at most 8 times for each power-up.
- Idle timers that count only keys, knobs and audible sound as activity. The app decides what is audible.
- Key releases are never dropped. On USB suspend or reset, the pad releases all typed keys.
- The firmware version in the USB device release number.
- Three ways into the bootloader: the top-left key at start, the four corner keys for 2 seconds, and a command from the app.
- Watchdog, restart after a crash, and a stop in the bootloader after three crashed starts in a row.
- Crash log in the flash page at `0x0801F800`.

### App

- Windows tray app that controls the volume and mute of programs in the Windows volume mixer, and the microphone.
- Up to 4 channels, with names of 1 to 5 characters. A channel is a list of programs, the microphone, all other programs, or the foreground program.
- A knob press selects one program in a channel. Each program keeps its balance in the channel (trim), also at 0 % and 100 %.
- Each audio session is tracked separately. Focus gives each session its real level back, also for programs that closed while Focus was on.
- Key presses and releases are handled in the order in which they arrived.
- Key actions: Mute, Focus, Talk, Move, Switch page and `open:<target>`. Other keys are typed by the pad.
- Settings window with the tabs Keys, Channels and Pad, with a shortcut dialog and an open dialog. Text boxes apply on Enter or when you leave the box.
- `config.toml`. A mistake in the file keeps the last correct settings.
- The versions of the app and the firmware in the tray menu and on the Pad tab.
- The first start opens the settings window. The app does not start with Windows until you select it.
- Quit, logoff and shutdown wait (3 seconds at most) until the app has given all levels back.
- Options `--settings[=keys|channels|pad]`, `--no-pad`, `--bootloader`, `--selftest`, `--help`, and the environment variable `KB16_MIXER_DIR`.
- No keyboard hooks, no input injection, no driver, and no access to other processes. A test keeps the keyboard and mouse module of the `windows` crate disabled.

### Protocol

- `kb16-mixer-protocol`: the shared raw HID wire format and the version format, with tests.

### Build

- CI for the three crates, with an image check for the firmware.
- A release workflow for `v*` tags. It attaches the firmware image, the app, the licence files and SHA-256 sums.

### Tested

- On one KB16-01 rev2 with Windows 11: the mixer, keys, knobs, lights, screen, flashing and recovery (with earlier versions of the firmware).
- Only by automated tests and builds at the time of writing:
  - the key pages
  - the knob press to select one program
  - the idle rule
  - the stored name
  - `open:`
  - the settings window with three tabs
