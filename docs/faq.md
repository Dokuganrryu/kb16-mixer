# Frequently asked questions

This document answers the questions of pad owners before and after the install: drivers, anti-cheat, rights, stored data, the network and the uninstall.

## Contents

- [Is a driver necessary?](#is-a-driver-necessary)
- [Does it work with games that use anti-cheat?](#does-it-work-with-games-that-use-anti-cheat)
- [Are administrator rights necessary?](#are-administrator-rights-necessary)
- [What does the app store, and where?](#what-does-the-app-store-and-where)
- [Does the app use the network?](#does-the-app-use-the-network)
- [Why does an `open:` key ask for administrator rights?](#why-does-an-open-key-ask-for-administrator-rights)
- [What is the BOOT message?](#what-is-the-boot-message)
- [How do I uninstall the app?](#how-do-i-uninstall-the-app)
- [How do I go back to the stock firmware?](#how-do-i-go-back-to-the-stock-firmware)
- [Can I change the volume of one browser tab?](#can-i-change-the-volume-of-one-browser-tab)
- [Which output device does the app control?](#which-output-device-does-the-app-control)
- [Why does a knob press mute?](#why-does-a-knob-press-mute)
- [Why can `sha256sum -c SHA256SUMS` not find two files?](#why-can-sha256sum--c-sha256sums-not-find-two-files)
- [How do I find the version?](#how-do-i-find-the-version)
- [Does it work with other pads?](#does-it-work-with-other-pads)
- [Does it work on Linux or macOS?](#does-it-work-on-linux-or-macos)

## Is a driver necessary?

No, not for usual use. The pad is a standard USB HID device: a keyboard, media keys and one vendor-defined [raw HID](glossary.md#the-pad-and-the-app-together) interface. Windows uses its own HID driver.

Only the [bootloader](glossary.md#the-parts) uses a special driver, for a flash. You install the generic [WinUSB](glossary.md#install-build-and-release) driver one time, for the USB ID `1EAF:0003` only. See [install.md](install.md#4-install-the-bootloader-driver-one-time-only).

## Does it work with games that use anti-cheat?

This project cannot promise how an anti-cheat system judges it. The owner of each game sets its own rules. Read the rules of your game.

These are the facts about the code:

- The app moves only the volume sliders and the mute switches of the Windows volume mixer. It also changes the level and the mute of your microphone.
- The app has no driver. It uses no keyboard hooks. It does not inject input.
- The app does not open or read other processes. It gets the names of programs from a system snapshot only.
- For the Move key and a `focused` channel, the app asks Windows which program has the foreground window.
- Keys that type (media keys, F13–F24, shortcuts, digits) come from the pad itself, as usual USB keyboard input.
- An `open:` key starts its target through the Windows shell, as a double-click in Explorer does.
- One key can send a key combination, for example `Win+Shift+S` (a default key on page 1). The rules of some games forbid one key that sends a combination. This project has no macros (a series of keys). But the owner of the game makes the rules.

## Are administrator rights necessary?

No. The app runs as your usual user. `flash.ps1` and `dfu-util` also run without administrator rights.

Only the driver installation for the bootloader must have administrator rights, one time. See [install.md](install.md#4-install-the-bootloader-driver-one-time-only).

## What does the app store, and where?

On the PC, the app writes only in the folder `%APPDATA%\kb16-mixer`:

| File | Contents |
|---|---|
| `config.toml` | Your settings. The settings window writes it. You can also edit it. See [configuration.md](configuration.md). |
| `apps.toml` | The programs that you moved to a channel with the Move key |
| `state.toml` | The levels of the channels, the trims of the programs, and what Focus turned down |
| `log.txt` | Notes about problems, for example a settings error or a crash |
| `selftest.txt` | Only after `kb16-mixer.exe --selftest`: what the mixer sees |

When **Start with Windows** is on, the app also writes the value `kb16-mixer` in the registry key `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run`. The value holds the path of `kb16-mixer.exe`.

In the pad, the firmware keeps the name for the logo screen in one flash page. It also keeps a crash log in a second flash page. See [hardware.md](hardware.md#memory-map).

If the environment variable `KB16_MIXER_DIR` names a folder, the app uses that folder in place of the settings folder. It then does not change **Start with Windows**.

## Does the app use the network?

No. The app has no network code, and it does not connect to the internet. It does not look for updates.

There is one exception that you control: an `open:` key with a web address (for example `open:https://example.com`). The app gives the address to Windows, and Windows opens it in your web browser.

## Why does an `open:` key ask for administrator rights?

The app starts the target of an `open:` key as your usual user. Some programs must have administrator rights. For such a program, Windows shows a prompt of User Account Control (UAC).

This is correct. The app cannot answer the prompt for you, and it does not try. If you do not want the prompt, select a target that does not ask for administrator rights.

## What is the BOOT message?

The BOOT message is one raw HID packet that does not change: the bytes `D1 7F`, followed by the letters `BOOT`. When the firmware receives it, the pad restarts into its bootloader. `kb16-mixer.exe --bootloader` and `flash.ps1` send it, so that you do not have to hold a key.

- It is a marker, not a password. All copies of this firmware use the same bytes.
- It gives no access to the PC, to your files or to other devices.
- A program on your PC can send it. A program can then write a new firmware into the pad. You can always write this firmware back.
- The bootloader itself stays the same. This firmware does not write it.

[protocol.md](protocol.md) gives the message.

## How do I uninstall the app?

1. Open the settings window. On the **Pad** tab, set **Start with Windows** to off.
2. Right-click the tray icon. Click **Quit**.
3. Delete the folder of the app, for example `%LOCALAPPDATA%\Programs\kb16-mixer`.
4. If you want, delete the settings folder `%APPDATA%\kb16-mixer`.

The firmware stays in the pad. Without the app, the pad is a usual keyboard. Its knobs send media keys and page keys.
If you also want to go back to the stock firmware, see [How do I go back to the stock firmware?](#how-do-i-go-back-to-the-stock-firmware).

The WinUSB driver for the bootloader can stay. It applies only to the USB ID `1EAF:0003`.
If you want to remove it, do these steps. Nobody did a test of this removal.

1. Put the pad into the bootloader.
2. Open Device Manager.
3. Right-click `Maple 003`.
4. Click **Uninstall device**.
5. If Windows shows the option to delete the driver, select it.

## How do I go back to the stock firmware?

Write your backup back into the pad. See [stock-firmware.md](stock-firmware.md).
This project does not include the stock firmware. Make the backup before the first flash.

## Can I change the volume of one browser tab?

No. Windows sets the volume for each process. All tabs of one browser share one slider.

## Which output device does the app control?

By default, the app uses the Windows default output device. When you change the default device in Windows, the app follows it at once.
The `master` knob and the mute rule use that device. The programs on all active output devices stay on their knobs.

You can also select one device in `[audio] output`. The app then uses only that device. When the device is disconnected, the app uses the Windows default until the device comes back.
See [Audio output](configuration.md#audio-output). Nobody did a test of this on a pad.

## Why does a knob press mute?

By default, a knob tap mutes or unmutes the knob target. A knob hold (350 ms) selects one program in the channel.
You can change the two actions and add a double tap in `[knob_press]`. See [Knob presses](configuration.md#knob-presses).

For the hold, the pad must have firmware 0.2.0 or later. With earlier firmware, each knob press is a tap.

## Why can `sha256sum -c SHA256SUMS` not find two files?

`SHA256SUMS` has a line for the zip, for `kb16-mixer.exe` and for `kb16-mixer-firmware.bin`.
The release has only the zip and `SHA256SUMS` as separate files. The exe and the firmware image are in the zip.

Use `sha256sum -c --ignore-missing SHA256SUMS` in the download folder. It examines only the zip.
To examine the exe and the image after you unzip, see [install.md](install.md#13-unzip-the-release).
In PowerShell, use `Get-FileHash`. `flash.ps1` also shows the SHA-256 value of the image.

## How do I find the version?

The first line of the tray menu shows the version of the app and of the firmware. The settings window shows them too: the app version on the **Pad** tab, and the firmware version at the top right.
`kb16-mixer.exe --check` also shows them.

## Does it work with other pads?

The project did its tests on one DOIO KB16-01 rev2 only. rev1, the wireless KB16B-02 and other DOIO models are not supported. Other pads must have their own firmware.
To examine your pad, see [install.md](install.md#2-examine-your-pad).

## Does it work on Linux or macOS?

The app runs only on Windows. Without the app, the pad is a standard USB keyboard. Nobody did a test of this on Linux or macOS.
`scripts/flash.sh` is also for Linux. Nobody did a test of it there.
