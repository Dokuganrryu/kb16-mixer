# Install KB16 Mixer on Windows

Read this document if you own the DOIO KB16-01 rev2 and want to use it as a mixer. You do not write code, and a compiler is not necessary.
The document starts with a new pad. At the end, the pad is a mixer that works.
Do the steps in sequence. Each step tells you what to do, what you must see, and what to do if you do not see it.

You flash firmware at your own risk. A flash can void the warranty of the pad.

> **Status:** These steps are for version **0.3.0**, the first published release. It is a **pre-release**.
> The owner used a build of the 0.3.0 changes on one pad. Nobody did these install steps with the 0.3.0 zip.
> If the release is not on the Releases page, build the files yourself. See [build-from-source.md](build-from-source.md).
> The author did the flash, the driver installation and the app steps with earlier builds. The author used one pad with Windows 11, `dfu-util` and `scripts/flash.sh`.
> The end-to-end flash with `flash.ps1` is **not tested** on the pad yet.
> Automated tests run `flash.ps1` and `flash.sh` with a fake `dfu-util` and a stub app (`scripts/test_flash_scripts.py`). They examine each refusal. They do not use a pad. See [testing.md](testing.md#script-checks).
> Windows 10 is **not tested**.

## How to go back

You can stop at each step:

- **Before step 6 (flash):** nothing in the pad changed. Disconnect the pad. Connect it again. The stock firmware starts.
- **After step 6:** write your backup back. See [stock-firmware.md](stock-firmware.md#write-the-backup-back).

## Contents

1. [Get the files](#1-get-the-files)
2. [Examine your pad](#2-examine-your-pad)
3. [Put the pad into the bootloader](#3-put-the-pad-into-the-bootloader)
4. [Install the bootloader driver (one time only)](#4-install-the-bootloader-driver-one-time-only)
5. [Make a backup of the stock firmware](#5-make-a-backup-of-the-stock-firmware)
6. [Flash the firmware](#6-flash-the-firmware)
7. [Restart the pad](#7-restart-the-pad)
8. [Start the app](#8-start-the-app)
9. [Run the check](#9-run-the-check)
10. [First use](#10-first-use)

The backup (step 5) comes after the driver (step 4). Without the [WinUSB](glossary.md#install-build-and-release) driver, `dfu-util` cannot read the pad in the [bootloader](glossary.md#the-parts). For all terms, see [glossary.md](glossary.md).

## What is necessary

| Item | Where it comes from |
|---|---|
| A DOIO KB16-01 **rev2** pad | Step 2 tells you how to examine it. |
| A USB cable that carries data | Some cables carry only power. |
| Windows 11 (Windows 10 is not tested) | |
| The release zip `kb16-mixer-0.3.0-windows-x64.zip` and the file `SHA256SUMS` | The [Releases page](https://github.com/Dokuganrryu/kb16-mixer/releases) of this repository on GitHub. The release has the mark **Pre-release**. |
| `dfu-util` 0.11 | https://dfu-util.sourceforge.net. It is not part of this project. |
| The QMK driver installer (or Zadig) | https://github.com/qmk/qmk_driver_installer/releases. It is not part of this project. |
| Administrator rights | One time only, for the driver in step 4. The app and `flash.ps1` run without them. |

### How to open PowerShell or a command prompt in a folder

Some steps tell you to open PowerShell in a folder. Do this:

1. Open the folder in File Explorer.
2. Click the address bar.
3. Type `powershell`. For a command prompt, type `cmd`.
4. Press Enter. A window opens in this folder.

## 1. Get the files

### 1.1 Download the release

1. Open the [Releases page](https://github.com/Dokuganrryu/kb16-mixer/releases) of this repository.
2. Download `kb16-mixer-0.3.0-windows-x64.zip`.
3. Download `SHA256SUMS`.

The zip holds these files:

| File | Contents |
|---|---|
| `kb16-mixer.exe` | The app |
| `firmware\kb16-mixer-firmware.bin` | The firmware image |
| `flash.ps1` | The flasher |
| `LICENSE`, `THIRD_PARTY_NOTICES.md` | The licences |
| `README.txt` | A short version of this document |

### 1.2 Examine the download

1. Open PowerShell in your Downloads folder (see [How to open PowerShell](#how-to-open-powershell-or-a-command-prompt-in-a-folder)).
2. Run this command:

   ```powershell
   Get-FileHash .\kb16-mixer-0.3.0-windows-x64.zip -Algorithm SHA256
   ```

3. Open `SHA256SUMS` in a text editor. Find the line that ends with the name of the zip.
4. Compare the two values. They must be the same. Upper case and lower case letters do not matter.

`SHA256SUMS` has three lines: the zip, `kb16-mixer-firmware.bin` and `kb16-mixer.exe`.
The release has only the zip and `SHA256SUMS` as separate files. The exe and the firmware image are in the zip.
Thus `sha256sum -c SHA256SUMS` in the download folder cannot find two of the files. This is correct.
To examine only the zip in Git Bash or on Linux, run this command in the download folder:

```sh
sha256sum -c --ignore-missing SHA256SUMS
```

The command must show `kb16-mixer-0.3.0-windows-x64.zip: OK`.

If you use the GitHub CLI, you can also examine the [attestation](glossary.md#install-build-and-release). It tells where GitHub built the file:

```powershell
gh attestation verify .\kb16-mixer-0.3.0-windows-x64.zip --repo Dokuganrryu/kb16-mixer
```

The command must show that the verification succeeded. The attestation shows that GitHub Actions built the file from this repository.

Do not examine the firmware with a build of your own. A build on your computer gives different bytes from the CI build. See [releasing.md](releasing.md#what-a-release-contains).

**If the values are not the same:** do not use the file. Delete it. Download it again. If the values stay different, tell us in an issue.

### 1.3 Unzip the release

1. Make a folder that stays, for example `%LOCALAPPDATA%\Programs\kb16-mixer`.
2. Unzip the release into this folder.
3. Optional: examine the exe and the firmware image. Open PowerShell in this folder.
4. Run these two commands:

   ```powershell
   Get-FileHash .\kb16-mixer.exe -Algorithm SHA256
   Get-FileHash .\firmware\kb16-mixer-firmware.bin -Algorithm SHA256
   ```

5. Compare each value with the line in `SHA256SUMS` that ends with the same file name. The values must be the same.

In Git Bash, copy `SHA256SUMS` into this folder. Then run these two commands in this folder:

```sh
sha256sum -c --ignore-missing SHA256SUMS
(cd firmware && sha256sum -c --ignore-missing ../SHA256SUMS)
```

The first command examines the exe. The second command examines the firmware image. Each command must show `OK`.

`flash.ps1` also shows the SHA-256 value of the image before it writes. Compare this value with the line that ends with `kb16-mixer-firmware.bin`.

**Start with Windows** uses the path of `kb16-mixer.exe`. If you move the folder later, set **Start with Windows** again.

### 1.4 Get dfu-util

1. Download `dfu-util-0.11-binaries.tar.xz` from https://dfu-util.sourceforge.net/releases/.
2. Open PowerShell in your Downloads folder (see [How to open PowerShell](#how-to-open-powershell-or-a-command-prompt-in-a-folder)).
3. Unpack the archive:

   ```powershell
   tar -xf .\dfu-util-0.11-binaries.tar.xz
   ```

4. Make a folder `tools` in the folder of the release.
5. Copy `dfu-util-0.11-binaries\win64\dfu-util-static.exe` into the `tools` folder.

`flash.ps1` looks for `dfu-util.exe`, then `dfu-util-static.exe`, in this order:

1. The path that you give with `-DfuUtil <path>`.
2. The folders of the [`PATH`](glossary.md#install-build-and-release).
3. The `tools` folder next to `flash.ps1` (the release folder).
4. The `tools` folder one level up (the repository root, for a build from source).

If it does not find one of the two programs, it shows `REFUSED: dfu-util not found` with the full paths of the two `tools` folders.

The archive that the author downloaded (11 November 2026) had the SHA-256 value `6450de30a7dcd8d8c1273f43f0b153f054fd24d85f7f38296b1ad8edbd2ddb25`.
If `tar` cannot open the archive, use 7-Zip.

### Warnings from Windows and from antivirus programs

The app and the script do not have a code signature. Windows can show warnings:

- **"Windows protected your PC"** (SmartScreen) when you start `kb16-mixer.exe`. Click **More info**. Then click **Run anyway**. Do this only after the check in 1.2.
- **"Running scripts is disabled on this system"** when you start `flash.ps1`. Use the command `powershell -ExecutionPolicy Bypass -File .\flash.ps1`. This changes the rule only for that one command.
- An antivirus program can stop `dfu-util` or the driver installer. These tools write to USB devices, so some programs report them. Compare the SHA-256 value of the download with the value from its source. Then let the file run in your antivirus program, or do not continue.

## 2. Examine your pad

Do these checks before you change anything. If one item does not agree, **STOP. Do not flash.** Your pad stays as it is.

| # | Check | Supported | If it does not agree |
|---|---|---|---|
| 1 | The model name on the box, the shop page or the label | `KB16-01`, also sold as "Megalodon Triple Knob Macro Pad" | STOP. `KB16B-02` (wireless) and other DOIO models are not supported. |
| 2 | The pad | 16 keys, 2 small knobs, 1 large knob, a screen, a USB cable, no battery | STOP. |
| 3 | The USB ID in Device Manager (see below) | `VID_D010&PID_1601` | STOP. |
| 4 | `kb16-mixer.exe --check` (see below) | The `pad` line shows `WARN  A pad with other firmware is connected (USB D010:1601, ...)` | See below. |
| 5 | The USB ID in the bootloader (step 3) | `VID_1EAF&PID_0003`, name `Maple 003` | STOP. A rev1 pad shows an Atmel DFU device. |

Items 3 and 4 cannot tell rev1 from rev2: QMK gives both revisions the USB ID `D010:1601`. Only item 5 tells rev2 with certainty.
The chip of rev2 is an APM32F103CBT6 with 128 KB of flash and the stm32duino ("Maple") bootloader at `0x08000000`. See [hardware.md](hardware.md).
The project did its tests on only one rev2 unit. Other production runs of rev2 can be different.

### How to find the USB ID

1. Connect the pad.
2. Right-click the **Start** button.
3. Click **Device Manager**.
4. Open **Keyboards** or **Human Interface Devices**.
5. Double-click a device.
6. Click the **Details** tab.
7. In **Property**, select **Hardware Ids**.
8. Look for `VID_D010&PID_1601`. If this device does not have it, try the next device.

You can also run this command in PowerShell. It shows the devices of the pad:

```powershell
Get-PnpDevice -PresentOnly | Where-Object InstanceId -like 'USB\VID_D010&PID_1601*'
```

### What --check tells you

1. Open the folder of the release.
2. Click the address bar.
3. Type `cmd`.
4. Press Enter. A command prompt opens in this folder.
5. Run this command:

   ```text
   kb16-mixer.exe --check | more
   ```

6. Read the line that starts with `pad`.

| The pad line shows | Meaning | What to do |
|---|---|---|
| `WARN  A pad with other firmware is connected (USB D010:1601, product "...")` | A pad with the USB ID of the KB16 has other firmware, for example the stock firmware. | Continue with step 3. |
| `OK    KB16 Mixer, firmware <version> (USB D010:1601)` | The pad has this project's firmware already. | Go to step 8. |
| `FAIL  No pad found. Check the USB cable and port.` | Windows does not see a pad with this USB ID. | Use a different cable or port. If it still fails, STOP. |
| `FAIL  The pad is in bootloader mode (USB 1EAF:0003). Unplug it and plug it in again.` | The pad is in its bootloader. | Continue with step 4. |

The check only reads. It does not change the pad, the settings or Windows. Its report is safe to paste into an issue.

## 3. Put the pad into the bootloader

The bootloader is a small program in the pad. It stays in the pad, and it can write a new firmware. In the bootloader, the pad does not type and its lights are off.

**Do:**

1. Disconnect the pad.
2. Hold the top-left key.
3. Connect the pad. Continue to hold the key.
4. After 2 seconds, release the key.

**You see:** the screen and the key lights stay dark. Device Manager shows `Maple 003`, or an unknown device with the USB ID `1EAF:0003` (before step 4).

**If you do not see it:** the stock firmware possibly does not have this procedure. For the other procedures, see [stock-firmware.md](stock-firmware.md#put-the-pad-into-the-bootloader-from-the-stock-firmware).
This procedure worked on the author's pad with a community firmware in it. Nobody did a test of it with the DOIO factory firmware.

To go out of the bootloader without a change, disconnect the pad. Then connect it again.

## 4. Install the bootloader driver (one time only)

For usual use, the pad uses no special driver. Windows uses its own HID driver.
Only the bootloader must have the generic WinUSB driver. Without it, `dfu-util` cannot communicate with the bootloader.

> **WARNING:** Install WinUSB only for the USB ID `1EAF:0003`.
> If you change the driver of a different device, that device (for example your keyboard or mouse) can stop.

> **WARNING:** Do not install WinUSB for the USB ID `D010:1601`.
> That is the pad in usual use. It must keep the Windows HID driver, or the keys and the app do not work.

### With the QMK driver installer

Status: **done on the author's PC**. The installer comes from QMK. It uses libwdi. It is not part of this project.

**Do:**

1. Download `qmk_driver_installer.exe` (version 1.01) from https://github.com/qmk/qmk_driver_installer/releases.
2. Put it in a new folder.
3. In the same folder, make a text file with the name `drivers.txt`.
4. Write this one line into the file. Save the file.

   ```text
   winusb,STM32duino Bootloader,1EAF,0003,746915ec-99d8-4a90-a722-3c85ba31e4fe
   ```

   This line is the same as the STM32duino line in the driver list of QMK Toolbox. The line has the format `driver,name,VID,PID,GUID`.
5. Open PowerShell in the folder (see [How to open PowerShell](#how-to-open-powershell-or-a-command-prompt-in-a-folder)).
6. Run this command:

   ```powershell
   Start-Process .\qmk_driver_installer.exe -ArgumentList '--force','--all','drivers.txt' -Verb RunAs -Wait
   ```

7. Windows asks for permission. Click **Yes**.
8. Wait until the installer window closes.

`--all` installs the driver also when the pad is not connected. The author used this command with one line of the same format.

**You see:** in the bootloader (step 3), Device Manager shows `Maple 003` with no warning sign. Its driver is WinUSB.

**If you do not see it:** disconnect the pad, connect it again in the bootloader, and look again. If the warning sign stays, run the installer again. See [troubleshooting.md](troubleshooting.md#maple-003-has-a-warning-sign).

### With Zadig

Status: **not tested**. Zadig (https://zadig.akeo.ie) does the same job.

1. Put the pad into the bootloader (step 3).
2. Start Zadig.
3. In the menu **Options**, select **List All Devices**.
4. In the device list, select `Maple 003`. Make sure that the USB ID shows `1EAF` `0003`.
5. Make sure that the target driver is **WinUSB**.
6. Click **Install Driver** or **Replace Driver**.

You install the driver one time only. Windows keeps it for the next flash.

## 5. Make a backup of the stock firmware

Do this step. Without a backup, you possibly cannot go back to your stock firmware.

**Do:**

1. Put the pad into the bootloader (step 3).
2. Open PowerShell in the folder of the release (see [How to open PowerShell](#how-to-open-powershell-or-a-command-prompt-in-a-folder)).
3. Run this command:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\flash.ps1 -Backup .\stock-backup.bin -DryRun
   ```

**You see:** a line `backup: 122880 bytes, SHA-256 ...`, then `DRY RUN complete: all checks passed. Nothing was written.`

**If you do not see it:**

- A line that starts with `REFUSED`: read the reason. Do not flash. See [troubleshooting.md](troubleshooting.md).
- `REFUSED: -Backup needs the pad in bootloader mode (USB 1EAF:0003), also with -DryRun. ...`: the script made no backup. Put the pad into the bootloader (step 3). Then run the command again.

Write down the SHA-256 value. Copy `stock-backup.bin` to a second place. For the details, see [stock-firmware.md](stock-firmware.md#make-a-backup-of-the-stock-firmware).

## 6. Flash the firmware

> **CAUTION:** Do not disconnect the pad while the script writes. If you disconnect it, the firmware image is incomplete. Nobody did a test of this case. See [Recovery](flashing.md#recovery).

**Do:**

1. Make sure that the pad is in the bootloader (step 3).
2. In PowerShell, in the folder of the release, run this command:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\flash.ps1
   ```

3. Read the output. Make sure that it shows `OK` for the image and a line `bootloader: USB 1EAF:0003, alternate setting 2`.
4. When the script asks, type `YES` and press Enter.
5. Wait until the script shows `Download done.` (approximately 10 seconds).

**You see:** `Download done.`, then `NEXT: unplug the pad and plug it in again.`
After `Download done`, the line `dfu-util exit 74` is usual and correct.

**If you do not see it:**

- `REFUSED: ...`: the script wrote nothing. Read the reason. See [troubleshooting.md](troubleshooting.md).
- `FAILED: dfu-util did not show "Download done"`: do not disconnect the pad. Run the script again.

The script writes only the firmware area (`0x08002000`). It does not write the bootloader. It does not write unless you type `YES`.

If the pad is not in the bootloader when the script starts, the script asks: "Restart it into its bootloader with kb16-mixer.exe? (y/N)".

- If the pad has this firmware, type `y`. The script stops `kb16-mixer.exe` if it runs. After the write, it starts the app again.
  The environment variable `KB16_APP_PROCESS` can give a different process name to stop. The tests use it. Usually you do not set it.
- If the pad has the stock firmware, press Enter. The script then tells you what to do.

The script waits for the bootloader for 120 seconds at most.

## 7. Restart the pad

The bootloader starts the new firmware only after a power cycle.

**Do:**

1. Disconnect the pad.
2. Connect the pad again.

**You see:**

- The screen shows `KB16` in large letters for approximately 2 seconds.
- The key lights show a slow white light that moves across the keys.
- In a text editor, the top-left key types `1`.

**If you do not see it:** see [troubleshooting.md](troubleshooting.md#the-pad-stays-dark-after-the-flash).

## 8. Start the app

**Do:**

1. Double-click `kb16-mixer.exe`.
2. If SmartScreen shows a warning, see [Warnings](#warnings-from-windows-and-from-antivirus-programs).

**You see:** the settings window opens on the **Pad** tab. The window tells you where the tray icon is.
The app does not start with Windows. To start it with Windows, set **Start with Windows** to on, on the **Pad** tab.

**If you do not see it:** look for the tray icon. Windows can hide it under the arrow near the clock. Click the icon to open the settings window.

## 9. Run the check

**Do:**

1. Open a command prompt in the folder of the release (see [What --check tells you](#what---check-tells-you)).
2. Run `kb16-mixer.exe --check | more`.

**You see:** a report like this one. The numbers depend on your PC.

```text
app        OK    kb16-mixer 0.3.0, settings in the default folder
pad        OK    KB16 Mixer, firmware 0.3.0 (USB D010:1601)
settings   OK    config.toml is valid (4 channels).
autostart  OK    Start with Windows is off.
audio      OK    2 output device(s), 3 app session(s), a microphone. Output: the Windows default.
Result: OK
```

**If you do not see it:** find the line with `WARN` or `FAIL` in [troubleshooting.md](troubleshooting.md).

## 10. First use

These are the default settings. Page 1 is the mixer page. Each key column belongs to one channel: **Mic**, **Chat**, **Game**, **Media** (from left to right).

| Try this | What occurs |
|---|---|
| Play a sound, for example music in a browser. | The key lights in the **Media** column show the sound level. |
| Turn knob 2 (the small knob on the right). | The volume of the **Media** channel changes by 2 % for each step. The screen shows the volume. |
| Tap knob 2: push it and release it in less than 350 ms. | The **Media** channel is muted. Tap again to unmute. |
| Hold knob 2 down for 350 ms. | The knob selects one program in the channel. The next turns change only that program. Hold again to select the next program. After 8 seconds without a touch, the knob controls the full channel again. This needs firmware 0.2.0 or later. |
| Push a key in the bottom row. | The channel of that column is muted. Its lights become amber. Push again to unmute. |
| Push the top-left key. | The pad changes to page 2 (a number pad). The screen shows `KEYS`. Push again to go back to `MIXER`. |

Knob 1 (small, left) controls **Chat**. Knob 3 (the large knob) controls **Game**.

> **NOTE:** The knob positions in this step ("right", "left") come from the settings file. At this time, nobody did a test of the physical knob order on a pad. If knob 2 changes a different channel, record it in an issue. See [testing.md](testing.md#verified-on-the-pad).

To change the channels, the programs in them and the keys, click the tray icon. See [configuration.md](configuration.md).
For all the controls, see [How to use the pad](../README.md#how-to-use-the-pad).
