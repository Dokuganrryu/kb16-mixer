# Glossary

This glossary gives one definition for each term in the KB16 Mixer documents.
The documents use each term only with this definition.
A term in `code font` is a name in the code or in the config file.
The documents follow ASD-STE100 Simplified Technical English. STE lets a document use technical names and technical verbs that its dictionary does not have. This glossary gives the definition of each such term that the documents use.

## The parts

| Term | Definition |
|---|---|
| pad | The DOIO KB16-01 rev2 macro pad: 16 keys, 3 knobs, one screen and 16 key lights. "USB device" means the pad as Windows sees it on the USB bus. |
| rev2 | The second hardware revision of the KB16-01. It has the APM32F103CB chip. KB16 Mixer supports only rev2. See [hardware.md](hardware.md). |
| app | `kb16-mixer.exe`, the Windows tray app of this project. The documents do not use "app" for other software. |
| program | A different Windows program, for example a game or a music player. In the settings window, the labels use the word "app" for a program (for example **These apps**), because the labels are short. |
| firmware | The software of this project that runs on the chip of the pad. It starts at address `0x08002000`. |
| stock firmware | The firmware in the pad before you flash KB16 Mixer. It can be the factory firmware, or a different firmware that a person installed before you. See [stock-firmware.md](stock-firmware.md). |
| factory firmware | The firmware that DOIO installs in the pad at the factory. The pad of the author did not have it: it had a community firmware. |
| community firmware | A firmware that a person other than DOIO made from public source code, for example a build of QMK or Vial. |
| bootloader | The small program in the first 8 KB of the flash memory (`0x08000000` to `0x08001FFF`). It starts before the firmware and can write a new firmware image. The firmware does not write the bootloader. |
| bootloader mode | The state of the pad when the bootloader runs and waits for an image. The pad then has the USB IDs `1EAF:0003`. |
| DFU | Device Firmware Upgrade, the USB standard that the bootloader uses to receive an image. `dfu-util` is a program that sends an image over DFU. |
| image | The firmware file that the build makes: `kb16-mixer-firmware.bin`. |
| image check | The checks in `scripts/build-firmware.sh` that reject a bad image. See [CONTRIBUTING.md](../CONTRIBUTING.md#the-image-check-and-why-it-exists). |
| flash | (noun) The flash memory of the chip. (verb) To write an image to the pad. See [flashing.md](flashing.md). |
| crate | One Rust package of this project: `protocol`, `pad-core`, `firmware` or `app`. Also a Rust package from crates.io that a crate uses. |
| build | (noun) The result of a compilation: an image or an exe. (verb) To compile the code into an image or an exe. |

## Sound and volume

| Term | Definition |
|---|---|
| audio session | One slider in the Windows volume mixer. Windows makes one session for each process that plays sound. The short form is "session". |
| program audio | The sound that programs play, not the microphone. Some texts use the British spelling "programme audio". |
| channel | A group of audio sessions that one knob and one key column control. A channel has a name of 1 to 5 characters. There are 1 to 4 channels. |
| channel level | The volume (0 % to 100 %) of a channel. |
| trim | A number from 0 to 1 for each program. A program plays at channel level x trim. |
| knob target | What a knob controls: a channel, one program in a channel, the master volume, the microphone, the foreground program, or the channel that has Focus. |
| master volume | The volume of the output device: the default output device of Windows, or the device in `[audio] output`. |
| output device | A speaker or a headset that Windows plays sound on. By default, the app uses the default output device of Windows and follows it when it changes. |
| meter | The live level that the pad shows for a channel, on the screen and on the key lights. |
| audible | A program plays sound above approximately −40 dBFS for at least 1 second. See [how-it-works.md](how-it-works.md#idle-rule). |

## Keys and actions

| Term | Definition |
|---|---|
| key | One of the 16 keys of the pad. The documents do not use "button" for a key. |
| button | A control in a window of the app on the PC. |
| knob | One of the 3 knobs of the pad: the small left knob, the small right knob and the large knob. The settings window calls them **Left knob**, **Right knob** and **Big knob**. |
| detent | One click of a knob. One detent is one `TURN` message. |
| tap | A push and release of a knob or a key in less than 350 ms. |
| hold | A push of a knob or a key for 350 ms or more. The hold action starts while you hold. |
| double tap | A tap, then a second push in less than 280 ms after the release. The app looks for a double tap only on a knob or a key that has a double-tap action. |
| pick | The knob action that selects the next program in the channel of the knob. By default, a knob hold does it. |
| page | One of the two key layouts. Page 1 is the mixer page (`[keys]`). Page 2 is the keypad page (`[keys2]`). |
| key table | The actions of the 16 keys of the current page. The app sends it to the pad. |
| typed key | A key that the pad sends to Windows as usual keyboard input or as a media key. The app does not see it. |
| page keys | The keys Page Up, Page Down and Home. In keyboard mode, knob 2 sends them. |
| mixer key | A key with an action of the app: Mute, Focus, Talk, Move, Switch page or `open:`. The pad sends it to the app. |
| Mute | The action that mutes or unmutes a channel. By default, a knob tap mutes or unmutes the knob target. |
| Focus | The action that turns all other channels down to a part of their level. "The channel that has Focus" is the channel of the Focus key that is on. |
| ducking | What Focus does to the other channels. The `duck` setting gives the part of the level that stays. |
| Talk | The action that changes the microphone mute while you hold the key. |
| Move | The action that moves the foreground program into the channel of a key column. |
| Switch page | The action that changes the page. Its config value is `page`. |
| `open:` | The action that opens a program, a file, a folder or a web address. |
| foreground program | The program of the window that has the keyboard focus. |

## The pad and the app together

| Term | Definition |
|---|---|
| mixer mode | The mode of the pad while the app sends messages. The knobs and the mixer keys go to the app. |
| keyboard mode | The mode of the pad when no message from the app came for 3 seconds. The pad then has a fixed layout. |
| raw HID | The third USB interface of the pad (usage page `0xFF60`). The firmware and the app send their messages on it. |
| message | One 32-byte report on the raw HID interface. See [protocol.md](protocol.md). |
| heartbeat | The message that the app sends each second when it has nothing else to send. It keeps the pad in mixer mode. |
| frame | The message with the live state: levels, volumes, mutes, Focus and flags. |
| popup | A message that shows text on the screen for 1.3 seconds. A popup is a readout or a notice. |
| readout | A popup with a name, a value and a bar, for example after a knob turn. |
| notice | A popup with text only, for example `Open failed`. |
| activity | A key press, a knob turn, or audible sound. A popup is not activity. |
| idle | The state after some minutes without activity. The screen goes off and the key lights dim. After a longer time without activity, the key lights go off. |
| logo name | The name on the logo screen, 1 to 10 characters. The default is `KB16`. The pad keeps it in the name page. |
| name page | The flash page at `0x0801F400` that holds the logo name. |
| crash log | The flash page at `0x0801F800`. The firmware writes the details of each panic and hard fault into it. See [hardware.md](hardware.md#crash-log). |
| watchdog | The timer of the chip that restarts the firmware if the firmware stops. |
| BOOT marker | The 4 fixed bytes `BOOT` in the bootloader message. The marker is not a secret. |

## Files and windows

| Term | Definition |
|---|---|
| settings folder | `%APPDATA%\kb16-mixer`, or the folder in `KB16_MIXER_DIR`. |
| config file | `config.toml` in the settings folder. |
| settings window | The window of the app with the tabs **Keys**, **Channels** and **Pad**. |
| tray icon | The icon of the app in the notification area of the Windows taskbar. |
| tray menu | The menu that opens when you right-click the tray icon. |
| `--check` | The option that prints a short report about the app, the pad and the settings. You can paste the report into a bug report. |
| release | A version of the app and the firmware on the GitHub releases page, as one zip file. |

## Install, build and release

| Term | Definition |
|---|---|
| WinUSB | The generic USB driver of Windows. `dfu-util` uses it to send an image to the bootloader. You install it one time, for the USB ID `1EAF:0003` only. |
| DFU alternate setting | One numbered part of the DFU interface of the bootloader. Each alternate setting writes to a different memory area. This project uses only alternate setting 2 (`-a 2`), the firmware area at `0x08002000`. |
| `PATH` | The environment variable of Windows that lists the folders where Windows finds programs. |
| flasher | One of the two scripts that write an image to the pad: `flash.ps1` (Windows PowerShell) and `flash.sh` (POSIX shell). |
| dry run | A run of a flasher with `-DryRun`. It examines the image and `dfu-util`, and it writes nothing. |
| SHA-256 value | A 64-character checksum of a file. Two files with the same SHA-256 value have the same contents. |
| `SHA256SUMS` | The file of a release that lists the SHA-256 value of each release file. |
| attestation | A signed record from GitHub that says which workflow run made a file. `gh attestation verify` examines it. |
| not tested | Nobody did this action on a pad, and the documents make no promise about its result. |
| VERIFIED, NOT YET VERIFIED | The status of a function in the table of [testing.md](testing.md#verified-on-the-pad). VERIFIED: a person did the action on a pad and saw the result. |
| SWD | Serial Wire Debug, the debug port of the chip. It has two signals: `SWDIO` (pin PA13) and `SWCLK` (pin PA14). |
| debug probe | A device that connects a PC to the SWD port. It can write the flash directly, also the bootloader region. An ST-Link is one example. |
