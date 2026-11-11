# Flashing (for developers)

This document is for people who build the firmware themselves and flash it with `scripts/flash.sh` or with `dfu-util` directly.
It also tells you how to recover the pad.

- To install a release on Windows, see [install.md](install.md). It uses `flash.ps1`.
- To build the image, see [build-from-source.md](build-from-source.md).
- To make a backup of the stock firmware, or to go back to it, see [stock-firmware.md](stock-firmware.md).

You flash firmware at your own risk. A flash can void the warranty of the pad.
The author tested the procedures on one pad (KB16-01 rev2) with Windows 11.

## Contents

- [Prerequisites](#prerequisites)
- [Put the pad into the bootloader](#put-the-pad-into-the-bootloader)
- [Flash with flash.sh](#flash-with-flashsh)
- [Flash with dfu-util only](#flash-with-dfu-util-only)
- [Recovery](#recovery)

## Prerequisites

| Item | Notes |
|---|---|
| `dfu-util` | Get it from https://dfu-util.sourceforge.net. This project used version 0.11. |
| WinUSB driver for USB ID `1EAF:0003` | For the bootloader only, on Windows. See [install.md](install.md#4-install-the-bootloader-driver-one-time-only). |
| A POSIX shell | Only for `scripts/flash.sh`, for example Git Bash |

`scripts/flash.sh` looks for `dfu-util` in this order:

1. The path in the environment variable `DFU_UTIL`.
2. `dfu-util` or `dfu-util-static` on the `PATH`.
3. Any `dfu-util` program under the `tools/` folder of the repository. Git ignores this folder.

This project does not include `dfu-util`, a driver installer or a driver.

## Put the pad into the bootloader

- From the firmware of this project: use one of the ways in [Recovery](#recovery).
- From the stock firmware: see [stock-firmware.md](stock-firmware.md#put-the-pad-into-the-bootloader-from-the-stock-firmware).
- To know that the pad is in the bootloader: see [stock-firmware.md](stock-firmware.md#how-to-know-that-the-pad-is-in-the-bootloader).

## Flash with flash.sh

> **WARNING:** Flash only an image for the DOIO KB16-01 rev2.
> An image for a different board can prevent the start of the firmware.

> **WARNING:** Use only DFU alternate setting 2 (`-a 2`). This project does not use other alternate settings.

> **CAUTION:** Do not disconnect the pad while `dfu-util` writes.
> If you disconnect it, the firmware image is incomplete, and the pad possibly does not start. Nobody tested this case. See [Recovery](#recovery).

1. Open a POSIX shell in the repository folder.
2. Run the script:

   ```sh
   sh scripts/flash.sh
   ```

   To flash a different image, give its path: `sh scripts/flash.sh path/to/image.bin`.
3. If the script shows `REFUSED`, it did not write. The image is empty, too large, or does not start with a stack pointer in RAM and a reset vector in the firmware area. The limit is `0x1D400` bytes (117 KB). The image of version 0.1.0 is approximately 50 KB.
4. If the script shows "pad told to enter its bootloader", go to step 6.
5. If the script shows "waiting for the bootloader", do these steps:
   1. Disconnect the pad.
   2. Hold the top-left key.
   3. Connect the pad.
   4. Hold the key until the download starts.
6. Wait until `dfu-util` shows "Download done".
7. Disconnect the pad.
8. Connect the pad again. The script shows this step as `NEXT:` at the end.

The script stops `kb16-mixer.exe` before it flashes. Then it runs `kb16-mixer.exe --bootloader` if the built app exists.
After the download, the script starts `kb16-mixer.exe` again from the same file, if the app ran before.
After "Download done", `dfu-util` can show exit code 74. This is normal.

`flash.sh` shows the `NEXT:` line also after a failed write. If `dfu-util` did not show "Download done", do not disconnect the pad. Run the script again.

`flash.sh` asks for no confirmation, and it does not list the devices first. If more than one DFU device has the USB ID `1EAF:0003`, `dfu-util` itself refuses. `flash.ps1` does more checks. See [install.md](install.md#6-flash-the-firmware).

## Flash with dfu-util only

1. Put the pad into the bootloader.
2. Check the image with the dry run of `flash.ps1` (it writes nothing): `powershell -ExecutionPolicy Bypass -File .\scripts\flash.ps1 -Firmware kb16-mixer-firmware.bin -DryRun`.
3. Run this command:

   ```sh
   dfu-util -w -d 1eaf:0003 -a 2 -D kb16-mixer-firmware.bin -R
   ```

4. Wait until `dfu-util` shows "Download done".
5. Disconnect the pad.
6. Connect the pad again.

The bootloader does not start the new firmware after the download. It waits for a power cycle.
Thus each flash ends with one disconnect and one connect.

To check that the new firmware runs, see [install.md](install.md#7-restart-the-pad) and [install.md](install.md#9-run-the-check).

## Recovery

This firmware never writes the bootloader region, so the bootloader should stay available for a new image. A flash guard lets the firmware write only its two reserved pages. See [hardware.md](hardware.md#flash-guard). This is not a guarantee: see [A firmware that does not start at all](#a-firmware-that-does-not-start-at-all) and the [Status table](../README.md#status).

There are three ways into the bootloader while the firmware of this project runs:

| Method | Procedure |
|---|---|
| Top-left key at power-up | Disconnect the pad. Hold the top-left key. Connect the pad. Hold the key until the pad is in the bootloader. |
| Four corner keys | Hold all four corner keys together for 2 seconds. |
| From the PC | Run `kb16-mixer.exe --bootloader`. |

The firmware also recovers by itself:

- A watchdog resets the chip if the firmware stops for approximately 2 seconds. The firmware then starts again.
- After a crash, the firmware starts again.
- After three crashed starts in a row, the firmware keeps the pad in the bootloader (by design). Then flash a correct image. Nobody tested this on the pad.

### A firmware that does not start at all

This case (for example, after an interrupted download) was **not tested**. The project can make no promise about it.
The project knows these facts:

- This firmware never writes the bootloader region.
- The three ways above need this firmware to run. The top-left key check is part of this firmware, not of the bootloader.
- The bootloader possibly stays in DFU mode when the firmware area holds no valid image. Then flash again. See [hardware.md](hardware.md#bootloader).
- The seller and the QMK readme name a reset button on the board, under the bottom cover. This project did not test it. See [stock-firmware.md](stock-firmware.md#way-2-the-reset-button-under-the-bottom-cover).
