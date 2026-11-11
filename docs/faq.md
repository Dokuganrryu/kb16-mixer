# Frequently asked questions

This document answers the questions of pad owners before and after the install: drivers, anti-cheat, rights, stored data, the network and the uninstall.

## Contents

- [Does it need a driver?](#does-it-need-a-driver)
- [Does it work with games that use anti-cheat?](#does-it-work-with-games-that-use-anti-cheat)
- [Does it need administrator rights?](#does-it-need-administrator-rights)
- [What does the app store, and where?](#what-does-the-app-store-and-where)
- [Does the app use the network?](#does-the-app-use-the-network)
- [Why does an `open:` key ask for administrator rights?](#why-does-an-open-key-ask-for-administrator-rights)
- [What is the BOOT message?](#what-is-the-boot-message)
- [How do I uninstall the app?](#how-do-i-uninstall-the-app)
- [How do I go back to the stock firmware?](#how-do-i-go-back-to-the-stock-firmware)
- [Can I change the volume of one browser tab?](#can-i-change-the-volume-of-one-browser-tab)
- [How do I find the version?](#how-do-i-find-the-version)
- [Does it work with other pads?](#does-it-work-with-other-pads)
- [Does it work on Linux or macOS?](#does-it-work-on-linux-or-macos)

## Does it need a driver?

No, not for normal use. The pad is a standard USB HID device: a keyboard, media keys and one vendor-defined "raw HID" interface. Windows uses its own HID driver.

Only the bootloader needs a driver, for a flash. You install the generic WinUSB driver one time, for the USB ID `1EAF:0003` only. See [install.md](install.md#4-install-the-bootloader-driver-one-time-only).

## Does it work with games that use anti-cheat?

This project cannot promise how an anti-cheat system judges it. The owner of each game sets its own rules. Read the rules of your game.

These are the facts about the code:

- The app moves only the volume sliders and the mute switches of the Windows volume mixer. It also changes the level and the mute of your microphone.
- The app has no driver. It uses no keyboard hooks. It does not inject input.
- The app does not open or read other processes. It gets the names of programs from a system snapshot only.
- For the Move key and a `focused` channel, the app asks Windows which program has the foreground window.
- Keys that type (media keys, F13–F24, shortcuts, digits) come from the pad itself, as real USB keyboard input.
- An `open:` key starts its target through the Windows shell, as a double-click in Explorer does.

## Does it need administrator rights?

No. The app runs as your normal user. `flash.ps1` and `dfu-util` also run without administrator rights.

Only the driver installation for the bootloader needs administrator rights, one time. See [install.md](install.md#4-install-the-bootloader-driver-one-time-only).

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

If the environment variable `KB16_MIXER_DIR` names a folder, the app uses that folder instead, and it does not change **Start with Windows**.

## Does the app use the network?

No. The app has no network code, and it does not connect to the internet. It does not look for updates.

There is one exception that you control: an `open:` key with a web address (for example `open:https://example.com`). The app gives the address to Windows, and Windows opens it in your web browser.

## Why does an `open:` key ask for administrator rights?

The app starts the target of an `open:` key as your normal user. Some programs need administrator rights. For such a program, Windows shows a prompt of User Account Control (UAC).

This is correct. The app cannot answer the prompt for you, and it does not try. If you do not want the prompt, select a target that does not need administrator rights.

## What is the BOOT message?

The BOOT message is one fixed raw HID packet: the bytes `D1 7F`, followed by the letters `BOOT`. When the firmware receives it, the pad restarts into its bootloader. `kb16-mixer.exe --bootloader` and `flash.ps1` send it, so that you do not have to hold a key.

- It is a marker, not a password. All copies of this firmware use the same bytes.
- It gives no access to the PC, to your files or to other devices.
- A program on your PC can send it. A program can then write a new firmware into the pad. You can always write this firmware back.
- The bootloader itself stays the same. This firmware never writes it.

[protocol.md](protocol.md) describes the message.

## How do I uninstall the app?

1. Open the settings window. On the **Pad** tab, clear **Start with Windows**.
2. Right-click the tray icon. Click **Quit**.
3. Delete the folder of the app, for example `%LOCALAPPDATA%\Programs\kb16-mixer`.
4. If you want, delete the settings folder `%APPDATA%\kb16-mixer`.

The firmware stays in the pad. Without the app, the pad is a normal keyboard with media knobs.
To go back to the stock firmware also, see the next question.

The WinUSB driver for the bootloader can stay. It applies only to the USB ID `1EAF:0003`. To remove it, open Device Manager with the pad in the bootloader, right-click `Maple 003`, and click **Uninstall device**. Select the option to delete the driver, if Windows shows it. Nobody tested this removal.

## How do I go back to the stock firmware?

Write your backup back into the pad. See [stock-firmware.md](stock-firmware.md).
This project does not include the stock firmware. Make the backup before the first flash.

## Can I change the volume of one browser tab?

No. Windows sets the volume for each process. All tabs of one browser share one slider.

## Why can `sha256sum -c SHA256SUMS` not find two files?

`SHA256SUMS` has a line for the zip, for `kb16-mixer.exe` and for `kb16-mixer-firmware.bin`.
The release has only the zip and `SHA256SUMS` as separate files. The exe and the firmware image are in the zip.

Use `sha256sum -c --ignore-missing SHA256SUMS` in the download folder. It checks the zip only.
To check the exe and the image after you unzip, see [install.md](install.md#13-unzip-the-release).
In PowerShell, use `Get-FileHash`. `flash.ps1` also shows the SHA-256 value of the image.

## How do I find the version?

The first line of the tray menu shows the version of the app and of the firmware. The **Pad** tab of the settings window shows them too.
`kb16-mixer.exe --check` also shows them.

## Does it work with other pads?

The project tested it on one DOIO KB16-01 rev2 only. rev1, the wireless KB16B-02 and other DOIO models are not supported. Other pads need their own firmware.
To check your pad, see [install.md](install.md#2-check-your-pad).

## Does it work on Linux or macOS?

The app runs only on Windows. Without the app, the pad is a standard USB keyboard. Nobody tested this on Linux or macOS.
`scripts/flash.sh` is also for Linux. Nobody tested it there.
