# kb16-mixer

kb16-mixer is custom firmware and a small Windows app for the **DOIO KB16-01 rev2** macro pad.
Together they make the pad a per-app volume mixer, in the style of a hardware streaming mixer.
The pad also stays a programmable keypad.

This is an unofficial project. It is not affiliated with DOIO or endorsed by DOIO.

## What it does

- A knob turn changes the volume of a **channel**. A channel is a named list of programs, for example "Chat" (Discord, Teams) or "Media" (Spotify, browsers).
- A knob press selects one program in the channel. The next turns then change only that program.
- A key press mutes a channel, or turns the other channels down (Focus). A held key can unmute your microphone while you hold it (Talk).
- The 16 key lights show the live sound level of each channel. The screen shows the names, levels and volumes.
- Other keys type F13–F24, media keys or key combinations. The pad sends these keys itself, as a normal USB keyboard.
- A second page of keys makes the pad a number pad. A key can also open a program, a file, a folder or a web address.
- Without the app, the pad is a normal keyboard with media knobs.

## What it does not do

- **It does not use a driver.** The pad is a standard USB HID device: a keyboard, media keys and one vendor-defined "raw HID" interface. Windows uses its own built-in HID driver. The only driver step is for flashing. See [Flashing](docs/flashing.md).
- **It does not route audio.** The app does not create a virtual audio device. You do not need a virtual cable.
- **It does not use keyboard hooks or inject input.** The app also does not open or read other processes.

The app changes the things that you see in the Windows volume mixer: the slider and the mute switch of each program.
It also changes the level and the mute of your microphone. If you set a knob to "master", it changes the master volume.

## Is it safe to try?

- The bootloader of the pad is never written. Thus you can always flash a new firmware image from the bootloader.
- There are three ways to put the pad into the bootloader. One of them uses only the keys. See [Recovery](#recovery).
- The firmware restarts itself after a crash. After three crashes in a row, it stays in the bootloader. Then you can flash again.
- The project was tested on **one** pad only (KB16-01 rev2, Windows 11). Other units can be different.
- A firmware image that does not start at all (for example, after an interrupted download) was not tested.

You flash firmware at your own risk. Flashing can void the warranty of the pad.

## Status

The project has one tested pad. This is what was tested, and how:

| Item | Tested on the real pad | Tested by automated tests and builds only |
|---|---|---|
| Mixer: channels, volume, mute, Focus, Talk, Move | yes | |
| Keys, knob turns, key lights, screen | yes | |
| Flashing and recovery | yes, with earlier versions of the firmware | |
| Two key pages | | yes |
| Knob press to select one program in a channel | | yes |
| Idle rule (screen off and lights dim only after no sound and no touch) | | yes |
| Name on the logo screen, kept in the pad | | yes |
| `open:` key action | | yes |
| Settings window with three tabs | | yes |

The table shows the state at the time of writing. Please report your results, good or bad, in an issue.

## Terms

| Term | Meaning |
|---|---|
| pad | The DOIO KB16-01 rev2 |
| firmware | The program in the pad. "Stock firmware" is the firmware from the factory. |
| bootloader | The small program in the pad that receives a new firmware over USB |
| app | `kb16-mixer.exe`, the Windows tray app of this project |
| program | Any other Windows program, for example Discord. The settings window calls these "apps". |
| channel | A named list of programs, or the microphone. Each channel has one column of keys. |
| page | One set of 16 key actions. Page 1 is the mixer page. Page 2 is the keypad page. |
| trim | The part of the channel level that one program gets, from 0 to 1 |

## What you need

| Item | Notes |
|---|---|
| DOIO KB16-01 **rev2** | rev2 has the APM32F103CBT6 chip. rev1 (ATmega32U4) and other DOIO models are not supported. |
| Windows PC | The app runs only on Windows. It was tested on Windows 11 only. |
| USB cable | A cable that carries data, not only power. |
| `dfu-util` | The program that writes the firmware into the pad. |
| WinUSB driver for the bootloader | You install it one time, for the USB ID `1EAF:0003` only. |
| A copy of the stock firmware | Make it before the first flash. See [Make a backup](docs/flashing.md#make-a-backup-of-the-stock-firmware). |
| Rust, Python 3, a POSIX shell | To build from source. See [Flashing](docs/flashing.md#prerequisites). |

## Quick start

There is no release yet. Build from source for now. A first release follows after more tests on hardware.

1. Build the firmware and the app. See [Build](docs/flashing.md#build).
2. Install the WinUSB driver for the bootloader (one time only). See [Install the bootloader driver](docs/flashing.md#install-the-bootloader-driver-one-time-only).
3. Make a backup of the stock firmware. See [Make a backup](docs/flashing.md#make-a-backup-of-the-stock-firmware).
4. Flash the firmware. See [Flash the firmware](docs/flashing.md#flash-the-firmware).
5. Disconnect the pad and connect it again.
6. Start `kb16-mixer.exe`.

On the first start, the app opens its settings window on the **Pad** tab. The window tells you where the tray icon is.
The app does not start with Windows until you select **Start with Windows** on that tab.

## How to use the pad

This is page 1 (the mixer page) with the default settings. Each key column belongs to the channel above it.

```text
             Mic      Chat     Game     Media
          +--------+--------+--------+--------+
  row 1   | Switch |  Prev  | Play/  |  Next  |     knob 1      knob 2
          |  page  |        | Pause  |        |     (small)     (small)
          +--------+--------+--------+--------+      Chat        Media
  row 2   |  Move  |  Win+  |  F13   |  F14   |
          |        | Shift+S|        |        |     +------------------+
          +--------+--------+--------+--------+     |      screen      |
  row 3   |  Talk  | Focus  | Focus  | Focus  |     +------------------+
          |        |        |        |        |
          +--------+--------+--------+--------+           knob 3
  row 4   |  Mute  |  Mute  |  Mute  |  Mute  |           (big)
          |        |        |        |        |           Game
          +--------+--------+--------+--------+
```

This is page 2 (the keypad page) with the default settings:

```text
          +--------+--------+--------+--------+
  row 1   | Switch |   7    |   8    |   9    |
          |  page  |        |        |        |
          +--------+--------+--------+--------+
  row 2   |   0    |   4    |   5    |   6    |
          +--------+--------+--------+--------+
  row 3   | Enter  |   1    |   2    |   3    |
          +--------+--------+--------+--------+
  row 4   |  Mute  |  Mute  |  Mute  |  Mute  |
          +--------+--------+--------+--------+
```

| Control | Action |
|---|---|
| Knob, turn | Changes the volume of the knob target. One detent is 2 % (setting `step`). |
| Knob, press | Selects the next knob target: the whole channel, then each program that plays in the channel, then the whole channel again. After 8 seconds without a touch, the knob goes back to the whole channel. |
| Mute | Mutes or unmutes the channel of its column. A muted column shows amber lights. |
| Focus, tap | Turns all other channels down (to 25 %, setting `duck`). Tap again to stop. |
| Focus, hold | The same as a tap, but Focus stops when you release the key. |
| Talk, hold | Changes the microphone mute while you hold the key. |
| Switch page, tap | Changes to the other page. The screen shows `MIXER` or `KEYS`. |
| Switch page, hold | Shows the other page only while you hold the key. |
| Move, hold + a mixer key of a column | Moves the foreground program into the channel of that column. Do it again to undo. |
| Move, tap | Shows the channel of the foreground program on the screen. |
| F13–F24, media keys, shortcuts, digits | The pad types these keys itself. |

A knob press does not mute. Use the Mute keys.

When the app does not run, the keys type `1`–`0`, arrows, Enter and Backspace.
The knobs then send media keys. See [Keyboard mode](docs/configuration.md#keyboard-mode).

## Settings

Click the tray icon to open the settings window. It has three tabs: **Keys**, **Channels** and **Pad**.
All changes apply immediately. The app writes them into `%APPDATA%\kb16-mixer\config.toml`.
You can also edit this file by hand. A mistake in the file does not break the app: the last correct settings stay active, and the tray menu shows the problem.

See [Configuration](docs/configuration.md) for the window and for all settings.

## Recovery

1. Disconnect the pad. Hold the top-left key. Connect the pad.
2. Or hold all four corner keys for 2 seconds.
3. Or run `kb16-mixer.exe --bootloader`.

Each of these puts the pad into the bootloader. Then flash again. See [Recovery](docs/flashing.md#recovery).

## FAQ

**Does it need a driver?**
No. The pad uses the HID driver that is part of Windows. Only the bootloader needs the generic WinUSB driver, one time, for flashing.

**Is it safe with anti-cheat?**
The app uses no keyboard hooks, injects no input, loads no driver, and does not open or read other processes.
It gets the names of programs from a system snapshot only.
Keys that type (media keys, F13–F24, shortcuts) come from the pad itself as real USB keyboard input.
An `open:` key starts its target through the Windows shell, as a double-click in Explorer does.
These are facts about the code. The project cannot guarantee how an anti-cheat system will judge it.

**Can I change the volume of one browser tab?**
No. Windows sets volume per process. All tabs of one browser share one slider.

**How do I find the version?**
The first line of the tray menu shows the version of the app and of the firmware. The **Pad** tab of the settings window shows them too.

**Does it work with other pads?**
It was tested on one DOIO KB16-01 rev2 only. rev1 and other DOIO models are not supported. Other pads need their own firmware.

**Does it work on Linux or macOS?**
The app runs only on Windows. Without the app, the pad is a standard USB keyboard. This was not tested on Linux or macOS.

**Can I go back to the stock firmware?**
Yes. This project does not include the stock firmware. Flash a stock image in the same way as this firmware. See [Return to the stock firmware](docs/flashing.md#return-to-the-stock-firmware).

## Documentation

| Document | Contents |
|---|---|
| [docs/flashing.md](docs/flashing.md) | Prerequisites, build, backup, flash, verify, recovery, back to stock |
| [docs/configuration.md](docs/configuration.md) | The settings window and all settings of `config.toml` |
| [docs/how-it-works.md](docs/how-it-works.md) | Architecture, the volume rules, and what the app touches in Windows |
| [docs/protocol.md](docs/protocol.md) | The raw HID messages between the pad and the app |
| [docs/hardware.md](docs/hardware.md) | Chip, pins, memory map and bootloader, for firmware writers |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). To report a security problem, see [SECURITY.md](SECURITY.md).
For the list of changes, see [CHANGELOG.md](CHANGELOG.md).

## Licence and legal notes

- The code is licensed under the MIT licence. See [LICENSE](LICENSE).
- The screen fonts are Open Sans (SIL Open Font License 1.1) and the public-domain X11 5x7 font. The build turns them into bitmaps.
- All dependencies have permissive licences. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- The firmware code, including the knob decoder, is original to this project. The pin assignments come from the public QMK board definition, as facts.
- This is an unofficial project. It is not affiliated with DOIO or endorsed by DOIO.
- All product names and company names are trademarks of their owners. This project uses them only to describe compatibility.
- You flash firmware at your own risk. Flashing can void the warranty of the pad.
- The software comes with no warranty. See the MIT licence.
