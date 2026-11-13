# Flashing (for developers)

This document is for persons who build the firmware themselves and flash it with `scripts/flash.sh` or with `dfu-util` directly.
It also tells you how to recover the pad.

- To install a release on Windows, see [install.md](install.md). It uses `flash.ps1`.
- To build the image, see [build-from-source.md](build-from-source.md).
- To make a backup of the stock firmware, or to go back to it, see [stock-firmware.md](stock-firmware.md).

You flash firmware at your own risk. A flash can void the warranty of the pad.
The author did tests of the procedures on one pad (KB16-01 rev2) with Windows 11.

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
3. A `dfu-util` program under the `tools/` folder of the repository. Git ignores this folder.

This project does not include `dfu-util`, a driver installer or a driver.

## Put the pad into the bootloader

- From the firmware of this project: use one of the procedures in [Recovery](#recovery).
- From the stock firmware: see [stock-firmware.md](stock-firmware.md#put-the-pad-into-the-bootloader-from-the-stock-firmware).
- To know that the pad is in the bootloader: see [stock-firmware.md](stock-firmware.md#how-to-know-that-the-pad-is-in-the-bootloader).

## Flash with flash.sh

> **WARNING:** Flash only an image for the DOIO KB16-01 rev2.
> An image for a different board can prevent the start of the firmware.

> **WARNING:** Use only DFU alternate setting 2 (`-a 2`). This project does not use other alternate settings.

> **CAUTION:** Do not disconnect the pad while `dfu-util` writes.
> If you disconnect it, the firmware image is incomplete, and the pad possibly does not start. Nobody did a test of this case. See [Recovery](#recovery).

1. Open a POSIX shell in the repository folder.
2. Run the script:

   ```sh
   sh scripts/flash.sh
   ```

   To flash a different image, give its path: `sh scripts/flash.sh path/to/image.bin`.
3. If the script shows `REFUSED`, it did not write. The image is empty, too large, or does not start with a stack pointer in RAM and a reset vector in the firmware area. The limit is `0x1D400` bytes (117 KB). The image of version 0.1.0 is approximately 51 KB.
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
After "Download done", `dfu-util` can show exit code 74. This is usual and correct.

The environment variable `KB16_APP_PROCESS` can give a different process name to stop, without `.exe`. The tests use it. The script rejects a name with characters other than letters, digits, `-`, `_` and `.`.

`flash.sh` shows the `NEXT:` line also after an unsatisfactory write. If `dfu-util` did not show "Download done", do not disconnect the pad. Run the script again.

`flash.sh` asks for no confirmation, and it does not list the devices first. If more than one DFU device has the USB ID `1EAF:0003`, `dfu-util` itself stops. `flash.ps1` does more checks. See [install.md](install.md#6-flash-the-firmware).

## Flash with dfu-util only

1. Put the pad into the bootloader.
2. Examine the image with the dry run of `flash.ps1` (it writes nothing): `powershell -ExecutionPolicy Bypass -File .\scripts\flash.ps1 -Firmware kb16-mixer-firmware.bin -DryRun`.
3. Run this command:

   ```sh
   dfu-util -w -d 1eaf:0003 -a 2 -D kb16-mixer-firmware.bin -R
   ```

4. Wait until `dfu-util` shows "Download done".
5. Disconnect the pad.
6. Connect the pad again.

The bootloader does not start the new firmware after the download. It waits for a power cycle.
Thus each flash ends with one disconnect and one connect.

To make sure that the new firmware runs, see [install.md](install.md#7-restart-the-pad) and [install.md](install.md#9-run-the-check).

## Recovery

This firmware does not write the bootloader region. Thus the bootloader stays available for a new image, if nothing else causes damage to it. A flash guard lets the firmware write only its two reserved pages. See [hardware.md](hardware.md#flash-guard). This is not a guarantee: see [A firmware that does not start at all](#a-firmware-that-does-not-start-at-all) and the [Status table](../README.md#status).

There are three procedures that put the pad into the bootloader while the firmware of this project runs:

| Method | Procedure |
|---|---|
| Top-left key at power-up | Disconnect the pad. Hold the top-left key. Connect the pad. Hold the key until the pad is in the bootloader. |
| Four corner keys | Hold all four corner keys together for 2 seconds. |
| From the PC | Run `kb16-mixer.exe --bootloader`. |

The firmware also recovers by itself:

- A watchdog resets the chip if the firmware stops for approximately 2 seconds. The firmware then starts again.
- After a crash, the firmware starts again.
- After three crashed starts in a row, the firmware keeps the pad in the bootloader (by design). Then flash a correct image. Nobody did a test of this on the pad.

### A firmware that does not start at all

This case (for example, after an interrupted download) was **not tested**. The project can make no promise about it.
The project knows these facts:

- This firmware does not write the bootloader region.
- The three procedures above work only while this firmware runs. The top-left key check is part of this firmware, not of the bootloader.
- The bootloader possibly stays in DFU mode when the firmware area holds no valid image. Then flash again. See [hardware.md](hardware.md#bootloader).
- The seller and the QMK readme name a reset button on the board, under the bottom cover. This project did not do a test of it. See [stock-firmware.md](stock-firmware.md#procedure-2-the-reset-button-under-the-bottom-cover).

### If the pad does not start and the reset button does not help

> **CAUTION:** This procedure is **not tested**. The author did not do it on a pad, and did not find the SWD pads on the board of the KB16-01 rev2. Use it only as a last resort.

A debug probe can write the flash directly, without the bootloader. The chip has a Serial Wire Debug (SWD) port. An ST-Link is one example of such a probe.

These facts come from public sources:

- The chip is an APM32F103CBT6 (LQFP48). The Geehy datasheet "APM32F103xB" (version 1.3) names pin PA13 (pin 34) as `SWDIO` and pin PA14 (pin 37) as `SWCLK`.
- The probe connects to four signals: `SWDIO`, `SWCLK`, ground and 3.3 V. The board must have test pads or a header for these signals. This project does not know where they are.
- The bootloader region is `0x08000000` to `0x08002000` (8 KB). The pad uses the stm32duino bootloader. See [hardware.md](hardware.md#bootloader).

If something damaged the bootloader region, the probe must write a bootloader at `0x08000000` first.

This project does not ship a bootloader image. The stm32duino project publishes its source code and its images on GitHub (`rogerclarkmelbourne/STM32duino-bootloader`). This project does not know which of its images matches the pad. Before you use one, read its licence and its notes yourself. On the date of this document, GitHub shows no licence file for that repository.

When a bootloader is in place, flash the firmware of this project as usual (see [Flash with dfu-util only](#flash-with-dfu-util-only)). A probe can also write the firmware image at `0x08002000` directly. This project did not do a test of the programs for SWD with this chip.
