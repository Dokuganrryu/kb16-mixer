# Stock firmware: back up, restore, go back

This document tells you how to put a pad with its stock firmware into the bootloader. It also tells you how to make a backup of that firmware and how to write the backup back.
"Stock firmware" here means the firmware in your pad before you flash KB16 Mixer. It can be the factory firmware from DOIO, or a different firmware that a person installed before you.

This project does not include, host or link a copy of any stock firmware. The firmware from the factory is the work of its maker.

You flash firmware at your own risk. A flash can void the warranty of the pad.

## Contents

- [What the project knows and does not know](#what-the-project-knows-and-does-not-know)
- [Put the pad into the bootloader from the stock firmware](#put-the-pad-into-the-bootloader-from-the-stock-firmware)
- [Make a backup of the stock firmware](#make-a-backup-of-the-stock-firmware)
- [Write the backup back (restore)](#write-the-backup-back-restore)
- [Check that the restore worked](#check-that-the-restore-worked)
- [Other sources of a stock image](#other-sources-of-a-stock-image)
- [After you go back to the stock firmware](#after-you-go-back-to-the-stock-firmware)

## What the project knows and does not know

Each step in this document has one of these labels:

| Label | Meaning |
|---|---|
| **Done on the author's pad** | The author did this step on the one tested pad (KB16-01 rev2, Windows 11). |
| **From the seller's documentation, not tested here** | The step comes from the public wiki of KeebMonkey, which says that it is an authorized distributor of this pad. This project did not do the step. |
| **From the QMK documentation, not tested here** | The step comes from the QMK readme for `doio/kb16/rev2`. This project did not do the step. |
| **Not tested** | Nobody in this project did the step. It follows from the facts in [hardware.md](hardware.md). |

These facts are important:

- The author's pad did **not** have the DOIO factory firmware when this project started. It had a community firmware for the Vial configurator. A person installed it before the start of this project. The replies of the firmware to configurator commands showed this.
- The author did **not** make a backup before the first flash. Thus nobody in this project has read or restored a factory firmware.
- The author made backups of the firmware area later, with this project's firmware in the pad. Thus the project knows the backup command and the file size below.

## Put the pad into the bootloader from the stock firmware

The bootloader is a small program at the start of the flash memory. It is not part of the stock firmware and not part of this project. This project never writes it.
The stock firmware decides which ways into the bootloader work. Try the ways in this order.

### Way 1: hold the top-left key while you connect the pad

Status: **done on the author's pad**, with the community Vial firmware in the pad. **Not tested** with the DOIO factory firmware.
The QMK readme gives the same way ("Bootmagic reset").

1. Disconnect the pad.
2. Hold the top-left key.
3. Connect the pad. Continue to hold the key.
4. After 2 seconds, release the key.
5. Look at the pad. The screen and the key lights must stay dark.
6. Open Device Manager. Look for `Maple 003`. See [How to know that the pad is in the bootloader](#how-to-know-that-the-pad-is-in-the-bootloader).

If the pad starts normally (the screen and the lights come on), this way does not work with your stock firmware. Try way 2 or way 3.

### Way 2: the reset button under the bottom cover

Status: **from the seller's documentation, not tested here**. The QMK readme also names "the button on the back of the PCB".

> **CAUTION:** The seller says that you must remove the bottom cover with a hexagon screwdriver. Do not touch the components on the board. Static electricity can damage them.

1. Disconnect the pad.
2. Remove the bottom cover.
3. Connect the pad.
4. Push the reset button one time. Do not touch other parts.
5. Look for `Maple 003` in Device Manager.

This project does not know where the button is on the board. This project has no photo of it.

### Way 3: a RESET key that you set with an old `VIA` version

Status: **from the seller's documentation, not tested here**.

The seller says: install version 1.3.1 of the configurator program `VIA`. Load its file `KB16-01.json`. Give one key the RESET key code. Then push that key.
This way needs the configurator programs from the seller's page. This project did not use them and cannot tell you more.

### How to know that the pad is in the bootloader

Status: **done on the author's pad**.

- The screen and the key lights are dark.
- Device Manager shows a device with the name `Maple 003`.
- The device has the USB ID `1EAF:0003`. Its device instance path starts with `USB\VID_1EAF&PID_0003`.

If Windows shows the USB ID `03EB:2FF4` or the name `ATmega32U4`, **stop**. Your pad is possibly a rev1, which has an Atmel DFU bootloader (QMK documentation). This project does not support rev1. See [Is my pad supported?](install.md#2-check-your-pad).

If no way puts the pad into the bootloader, do not flash. Your pad stays as it is. Tell us in an issue.

## Make a backup of the stock firmware

Make the backup before the first flash. Without a backup, you possibly cannot go back to your stock firmware.

Status of the command: **done on the author's pad**, but with this project's firmware in the pad (to read the crash log). The command gave a file of **122880 bytes** (0x1E000).
Status with a stock firmware in the pad: **not tested**.

The backup reads the firmware area from `0x08002000` to the end of the flash (`0x08020000`). It does not read and does not change the bootloader.
It also holds the last pages of the flash, where a stock firmware can keep its settings.

### Backup with flash.ps1 (recommended)

This command reads the backup, checks it, and writes nothing to the pad. Do it in the folder of the release (see [install.md](install.md)).

1. Put the pad into the bootloader. See [above](#put-the-pad-into-the-bootloader-from-the-stock-firmware).
2. Run this command in PowerShell:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\flash.ps1 -Backup .\stock-backup.bin -DryRun
   ```

3. Read the output. You must see a line that starts with `backup: 122880 bytes, SHA-256`.
4. Write down the SHA-256 value. Keep it with the file.
5. Copy `stock-backup.bin` to a second place, for example a USB stick.

`flash.ps1` refuses a backup that is empty, that holds only `0xFF` bytes (an empty flash area), or that does not have 122880 bytes.
If it refuses, do not flash. Keep the pad as it is and tell us in an issue.

The end-to-end backup with `flash.ps1` was **not tested** on the pad. The project tested its checks with a simulated `dfu-util`.

### Backup with dfu-util only

1. Put the pad into the bootloader.
2. Run this command. Use a new file name:

   ```text
   dfu-util -d 1eaf:0003 -a 2 -U stock-backup.bin
   ```

3. Wait until `dfu-util` stops.
4. Check the size of the file in PowerShell:

   ```powershell
   (Get-Item .\stock-backup.bin).Length
   ```

   The result must be `122880`.
5. Check that the file is not empty flash (all bytes `0xFF`). The result must be `True`:

   ```powershell
   $b = [IO.File]::ReadAllBytes("$PWD\stock-backup.bin"); [Array]::Exists($b, [Predicate[byte]] { $args[0] -ne 0xFF })
   ```

6. Get the SHA-256 value, and write it down:

   ```powershell
   Get-FileHash .\stock-backup.bin -Algorithm SHA256
   ```

7. Disconnect the pad.
8. Connect the pad again. The stock firmware starts again.

> **NOTE:** Do not share your backup. It is the firmware of its maker, and it can hold your own key settings.

## Write the backup back (restore)

Status: **not tested**. Nobody in this project has written a backup of a stock firmware back into a pad.

Use `dfu-util` directly. `flash.ps1` and `flash.sh` refuse the backup, because it is larger than the 117 KB limit for this project's firmware.
That limit protects the name page and the crash log of this project. The backup is a copy of the whole area, so a write of the whole area puts back what was there.

1. Quit the app. Right-click the tray icon. Click **Quit**.
2. Put the pad into the bootloader. With this project's firmware in the pad, use a way in [Recovery](flashing.md#recovery).
3. Check the SHA-256 value of the backup. It must be the value that you wrote down:

   ```powershell
   Get-FileHash .\stock-backup.bin -Algorithm SHA256
   ```

4. Run this command:

   ```text
   dfu-util -d 1eaf:0003 -a 2 -D stock-backup.bin -R
   ```

5. Do not disconnect the pad while `dfu-util` writes.
6. Wait until `dfu-util` shows `Download done`. Exit code 74 after this line is normal.
7. Disconnect the pad.
8. Connect the pad again.

If `dfu-util` shows an error before `Download done`, do not disconnect the pad. Run the command again.
The bootloader stays available, because `dfu-util` writes only the firmware area. See [troubleshooting.md](troubleshooting.md).

## Check that the restore worked

Status: **not tested**.

1. Look at the pad. The screen and the key lights must show what the stock firmware showed before.
2. Open a text editor. Push some keys. The pad must type what it typed before the flash.
3. Run `kb16-mixer.exe --check` in a command prompt. The `pad` line must show other firmware, for example:

   ```text
   pad        WARN  A pad with other firmware is connected (USB D010:1601, product "KB16-01"). This app needs the KB16 Mixer firmware. See docs/install.md.
   ```

   The product name in quotes comes from the stock firmware. `KB16-01` is what the author's pad showed with the community Vial firmware. The factory firmware can show a different name.
4. If you used a configurator program before (`VIA` or `Vial`), open it. Make sure that it finds the pad and shows your old key settings.

## Other sources of a stock image

Use these only if you have no backup. This project did not download or test any of them.

| Source | What it is | Status |
|---|---|---|
| The seller's wiki ("Megalodon Macro Pads Recovery Firmware") | The seller lists a recovery firmware file for "KB16-01 V2". The seller says that VIA finds the pad automatically with it. | From the seller's documentation, not tested here. This project does not know if "V2" is the same board as rev2. |
| QMK, `keyboards/doio/kb16/rev2` in `qmk/qmk_firmware` | Source code that you can build: `qmk compile -kb doio/kb16/rev2 -km default`. See the QMK documentation (https://docs.qmk.fm). | From the QMK documentation, not tested here. The default keymap is possibly not the same as the factory firmware. |
| The Vial project, `vial-kb/vial-qmk` | A firmware for the Vial configurator. A community build of this kind was in the author's pad at the start. | Not tested here as a restore. |

Before you flash any image, check it with the dry run of `flash.ps1`. The dry run checks the size and the first two words of the image, and writes nothing:

```powershell
powershell -ExecutionPolicy Bypass -File .\flash.ps1 -Firmware .\other-image.bin -DryRun
```

> **WARNING:** Do not flash a file that a web browser saved from a web page. A page saved as `.bin` is not firmware. The dry run refuses such a file, because its first two words are not a stack pointer and a reset vector.

## After you go back to the stock firmware

1. If **Start with Windows** is on, clear it on the **Pad** tab of the settings window.
2. Quit the app. Right-click the tray icon. Click **Quit**.
3. Delete the folder of the app.
4. If you want, delete the settings folder `%APPDATA%\kb16-mixer`.

The WinUSB driver for the bootloader can stay. It applies only to the USB ID `1EAF:0003`. See [faq.md](faq.md#how-do-i-uninstall-the-app).
