# Troubleshooting

Read this document if a step of [install.md](install.md) or the pad does not work as the documents say.
Find your symptom. Each entry gives the cause and the repair.
For the terms, for example [bootloader](glossary.md#the-parts), [WinUSB](glossary.md#install-build-and-release) and [`PATH`](glossary.md#install-build-and-release), see [glossary.md](glossary.md).
Many entries start from a line of `kb16-mixer.exe --check`. To run it, see [install.md](install.md#what---check-tells-you).

If your problem is not here, open an issue. Paste the output of `kb16-mixer.exe --check` into it. The report has no names, user names, paths or serial numbers.

## Contents

- [Lines of --check](#lines-of---check)
- [Messages of flash.ps1](#messages-of-flashps1)
- [The bootloader and dfu-util](#the-bootloader-and-dfu-util)
- [The pad after the flash](#the-pad-after-the-flash)
- [Windows warnings](#windows-warnings)

## Lines of --check

### --check shows nothing

| Cause | Fix |
|---|---|
| The app is a Windows program without its own console. In some shells, its output goes nowhere. | Run `kb16-mixer.exe --check \| more` in a command prompt (`cmd`). |

### pad: `FAIL  No pad found. Check the USB cable and port.`

| Cause | Fix |
|---|---|
| The cable carries only power. | Use a cable that carries data. |
| The USB port or hub does not work. | Connect the pad directly to a different USB port of the PC. |
| The pad has firmware with a different USB ID. | Look in Device Manager for the USB ID. See [install.md](install.md#how-to-find-the-usb-id). If it is not `D010:1601` or `1EAF:0003`, this project does not support the pad. |

### pad: `FAIL  The pad is in bootloader mode (USB 1EAF:0003). Unplug it and plug it in again.`

| Cause | Fix |
|---|---|
| The pad is still in the bootloader after a flash or a backup. | Disconnect the pad. Connect it again. |
| The pad goes back to the bootloader after each connection. The firmware possibly did not start. | Flash the firmware again. See [install.md](install.md#6-flash-the-firmware). |

### pad: `WARN  A pad with other firmware is connected (USB D010:1601, product "...")`

| Cause | Fix |
|---|---|
| The pad has its stock firmware or a different firmware. | Flash this project's firmware. See [install.md](install.md). |
| You wrote your stock backup back. | This is correct. The app does not work with the stock firmware. |

### pad: `WARN  <n> pads are connected (...). The app uses only one. Connect only one pad.`

| Cause | Fix |
|---|---|
| More than one pad, or one pad and one bootloader, is connected. | Disconnect all pads except one. |

### pad: `OK    KB16 Mixer, firmware unknown (an early build)`

| Cause | Fix |
|---|---|
| The pad has an early build of this firmware, without a version number. | Flash the current firmware. |

### settings: `FAIL  config.toml, line <n>, column <m>: ...`

| Cause | Fix |
|---|---|
| A mistake in `config.toml`, for example after an edit by hand. The app keeps the last correct settings. | Open `%APPDATA%\kb16-mixer\config.toml`. Go to the line and the column in the message. Correct the mistake. Then click **Reload config file** in the tray menu. See [configuration.md](configuration.md). |

### settings: `FAIL  config.toml cannot be read (...)`

| Cause | Fix |
|---|---|
| A different program has the file open, or the file has no read permission. | Close the other program. Make sure that your user can read the folder `%APPDATA%\kb16-mixer`. |

### audio: `FAIL  No audio output device is active.` or `WARN  No default output device is set.`

| Cause | Fix |
|---|---|
| Windows has no active speaker or headset, or no default device. | Open **Settings > System > Sound**. Select an output device. |

### audio: `FAIL  Core Audio does not answer (error 0x...).`

| Cause | Fix |
|---|---|
| The Windows audio service does not run. | Restart the PC. If the line stays, paste the report into an issue. |

## Messages of flash.ps1

Each `REFUSED` message means that the script wrote nothing to the pad.

| Message starts with | Cause | Fix |
|---|---|---|
| `REFUSED: dfu-util not found` | The script did not find `dfu-util.exe` or `dfu-util-static.exe`. It looks in this order: the `-DfuUtil` path, the `PATH`, the `tools` folder next to `flash.ps1`, and the `tools` folder one level up (the repository root). | Copy `dfu-util-static.exe` into the `tools` folder next to `flash.ps1`. See [install.md](install.md#14-get-dfu-util). Or give `-DfuUtil <path>`. |
| `REFUSED: No firmware image found` | The firmware file is not in the `firmware` folder next to the script. | Unzip the release again, with its folders. Or give `-Firmware <path>`. |
| `REFUSED: The image ... is empty`, `... is too small` | The file is not firmware. | Download the release again. Check it. See [install.md](install.md#12-examine-the-download). |
| `REFUSED: The image is <n> bytes. The limit is 119808 bytes` | The image is too large for the firmware area of this project. A full stock backup (122880 bytes) is larger than this limit. | To write a stock backup back, see [stock-firmware.md](stock-firmware.md#write-the-backup-back). |
| `REFUSED: The first word ... is not a stack pointer in RAM` or `The second word ... is not a reset vector` | The file is not an image for this pad. For example, it is a web page saved as `.bin`. | Do not flash the file. |
| `REFUSED: No pad in bootloader mode (USB 1EAF:0003) was found` | The pad is not in the bootloader, or the bootloader has no driver. | Put the pad into the bootloader. See [install.md](install.md#3-put-the-pad-into-the-bootloader). Look at Device Manager. See [Maple 003 has a warning sign](#maple-003-has-a-warning-sign). |
| `REFUSED: <n> devices with the USB ID 1EAF:0003 are connected` | More than one device is in the bootloader. | Disconnect all devices except the pad. |
| `REFUSED: The bootloader has no alternate setting 2` or `Alternate setting 2 writes to a different address` | The device is not the bootloader of a KB16-01 rev2. | Do not flash. Tell us in an issue. |
| `REFUSED: The backup file ... exists` | The script does not overwrite a file. | Give a new file name. |
| `REFUSED: The backup file is empty`, `holds only 0xFF bytes`, `has <n> bytes` | The backup is not a full copy of the firmware area. | Do not flash. Keep the pad as it is. Tell us in an issue. |
| `REFUSED: You did not type YES.` | You did not confirm the write. | Run the script again. Type `YES` in capital letters. |
| `WARNING: alternate setting 2 does not name its address` | The bootloader has a different name for its alternate setting 2. | The script continues. If you are not sure, type something other than `YES`, and tell us in an issue. |
| `FAILED: dfu-util did not show "Download done"` | The write did not finish. | Do not disconnect the pad. Run the script again. |

## The bootloader and dfu-util

### Maple 003 has a warning sign

| Cause | Fix |
|---|---|
| The WinUSB driver for the bootloader is not installed. | Install it. See [install.md](install.md#4-install-the-bootloader-driver-one-time-only). Then disconnect the pad and put it into the bootloader again. |

### The pad does not go into the bootloader

| Cause | Fix |
|---|---|
| You released the top-left key too early. | Hold the key while you connect the pad. Release it after 2 seconds. |
| The stock firmware does not have this way. | Use a different way. See [stock-firmware.md](stock-firmware.md#put-the-pad-into-the-bootloader-from-the-stock-firmware). |
| This project's firmware runs. | Use a way in [Recovery](flashing.md#recovery), for example the four corner keys for 2 seconds. |

### dfu-util exits with code 74 after "Download done"

| Cause | Fix |
|---|---|
| The bootloader does not answer the reset request of `dfu-util`. | Nothing. This is usual and correct. Disconnect the pad and connect it again. |

### dfu-util shows LIBUSB_ERROR_PIPE or LIBUSB_ERROR_IO

Other users of this pad report these errors. This project did not see them.

| Cause | Fix |
|---|---|
| The connection to the bootloader stopped. | Disconnect the pad. Put it into the bootloader again. Run the command again. |

## The pad after the flash

### The pad stays dark after the flash

| Cause | Fix |
|---|---|
| You did not disconnect the pad after the flash. The bootloader starts the new firmware only after a power cycle. | Disconnect the pad. Connect it again. |
| The pad is still in the bootloader (Device Manager shows `Maple 003`). | Flash again. See [install.md](install.md#6-flash-the-firmware). |
| The write did not finish, and the firmware does not start. | Put the pad into the bootloader and flash again. If the top-left key does not work, see [A firmware that does not start at all](flashing.md#a-firmware-that-does-not-start-at-all). This case was not tested. |

### The top of the screen is cut off

| Cause | Fix |
|---|---|
| The case hides some pixel rows of the screen. | On the **Pad** tab, increase **Screen shift** (`oled_top` in `config.toml`). See [configuration.md](configuration.md#general-settings). |

### A key of the `open:` type shows a "User Account Control" prompt

| Cause | Fix |
|---|---|
| The target program needs administrator rights. | This is correct. See [faq.md](faq.md#why-does-an-open-key-ask-for-administrator-rights). |

## Windows warnings

### "Windows protected your PC"

| Cause | Fix |
|---|---|
| `kb16-mixer.exe` has no code signature. SmartScreen does not know it. | Check the download first. See [install.md](install.md#12-examine-the-download). Then click **More info**, then **Run anyway**. |

### "Running scripts is disabled on this system"

| Cause | Fix |
|---|---|
| The PowerShell execution policy blocks scripts. | Start the script with `powershell -ExecutionPolicy Bypass -File .\flash.ps1`. This changes the rule for that one command only. |

### The antivirus program stops dfu-util or the driver installer

| Cause | Fix |
|---|---|
| These tools write to USB devices. Some antivirus programs report such tools. | Compare the SHA-256 value of the download with the value from its source. Then allow the file, or do not continue. |
