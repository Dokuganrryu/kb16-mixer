# Flashing

This document tells you how to build the firmware, write it into the pad ("flash" it), and make sure that it works.
It also tells you how to recover the pad and how to go back to the stock firmware.

You flash firmware at your own risk. Flashing can void the warranty of the pad.
The procedures were tested on one pad (KB16-01 rev2) with Windows 11.

## Contents

- [Prerequisites](#prerequisites)
- [Build](#build)
- [Put the pad into the bootloader](#put-the-pad-into-the-bootloader)
- [Install the bootloader driver (one time only)](#install-the-bootloader-driver-one-time-only)
- [Make a backup of the stock firmware](#make-a-backup-of-the-stock-firmware)
- [Flash the firmware](#flash-the-firmware)
- [Verify](#verify)
- [Recovery](#recovery)
- [Return to the stock firmware](#return-to-the-stock-firmware)

## Prerequisites

### To flash

| Item | Notes |
|---|---|
| `dfu-util` | Get it from https://dfu-util.sourceforge.net. This project used version 0.11. |
| WinUSB driver for USB ID `1EAF:0003` | For the bootloader only. See [Install the bootloader driver](#install-the-bootloader-driver-one-time-only). |
| A POSIX shell | Only for `scripts/flash.sh`, for example Git Bash. You can also run `dfu-util` directly. |

`scripts/flash.sh` looks for `dfu-util` in this order:

1. The path in the environment variable `DFU_UTIL`.
2. `dfu-util` or `dfu-util-static` on the `PATH`.
3. Any `dfu-util` program under the `tools/` folder of the repository. Git ignores this folder.

This project does not include `dfu-util`, a driver installer or a driver.

### To build

| Item | Command or notes |
|---|---|
| Rust (stable), 1.85 or later | Install with rustup. The crates use Rust edition 2024. |
| Firmware target | `rustup target add thumbv7m-none-eabi` |
| `objcopy` and `nm` | `rustup component add llvm-tools`, or the `arm-none-eabi` binutils |
| Python 3 | The firmware build script uses it for the image check. |
| MSVC toolchain | For the app. Rust on Windows uses it by default. |
| A POSIX shell | For the scripts, for example Git Bash |

## Build

There is no release yet. Build from source for now. A first release follows after more tests on hardware.

1. Open a POSIX shell (for example Git Bash) in the repository folder.
2. Build the firmware:

   ```sh
   sh scripts/build-firmware.sh
   ```

   The script writes `firmware/target/kb16-mixer-firmware.bin`.
   It also checks the image. If the check fails, the script shows `IMAGE REJECTED` and writes no image.
3. Build the app:

   ```sh
   sh scripts/build-app.sh
   ```

   The script writes `app/target/release/kb16-mixer.exe`.

The image check examines the vector table and the size of the image. It rejects the image if:

- the stack pointer is not in RAM.
- the reset vector is not in the firmware region (above `0x08002000`).
- one of the interrupts for the timer, USB, I2C or DMA goes to `DefaultHandler`. This occurs when the `rt` feature of embassy-stm32 is missing. Such an image stops the pad at the first interrupt.
- the image is larger than 117 KB (`0x1D400` bytes). It then overlaps the name page and the crash log.

## Put the pad into the bootloader

### From the firmware of this project

Use one of the methods in [Recovery](#recovery).

### From the stock firmware

This project did not record which method it used on the stock firmware.
The QMK readme for `doio/kb16/rev2` gives these three ways (quoted from that readme):

- "**Bootmagic reset**: Hold down the key at (0,0) in the matrix (1! key) and plug in the keyboard"
- "**Physical reset button**: Briefly press the button on the back of the PCB"
- "**Keycode in layout**: Press the key mapped to `QK_BOOT` if it is available"

The key at (0,0) is the top-left key. These ways are for QMK firmware. The firmware that DOIO installed at the factory can be different.

### How to know that the pad is in the bootloader

- The screen and the key lights are dark.
- Windows shows a USB device with the name `Maple 003` and the USB ID `1EAF:0003`.

## Install the bootloader driver (one time only)

The pad needs no driver for normal use. Windows uses its built-in HID driver.
Only the bootloader needs a driver, so that `dfu-util` can talk to it.
The bootloader has the USB ID `1EAF:0003`. It needs the generic WinUSB driver.

This project used the QMK driver installer (`qmk_driver_installer`, which uses libwdi) with a driver list of one line.
Zadig (https://zadig.akeo.ie) does the same job. This project did not use Zadig.

> **WARNING:** Install WinUSB only for the USB ID `1EAF:0003`.
> If you replace the driver of a different device, that device (for example your keyboard or mouse) can stop working.

> **WARNING:** Do not install WinUSB for the USB ID `D010:1601`.
> That is the pad in normal use. It must keep the Windows HID driver, or the keys and the app do not work.

### Procedure with the QMK driver installer

1. Download `qmk_driver_installer.exe` from https://github.com/qmk/qmk_driver_installer/releases.
2. Make a text file with the name `drivers.txt` in the same folder.
3. Write this one line into the file, and save it:

   ```text
   winusb,STM32duino Bootloader,1EAF,0003,746915ec-99d8-4a90-a722-3c85ba31e4fe
   ```

   This line is the same as the STM32duino line in the driver list of QMK Toolbox.
4. Put the pad into the bootloader. See [Put the pad into the bootloader](#put-the-pad-into-the-bootloader).
5. Open a command prompt in the folder.
6. Run this command:

   ```text
   qmk_driver_installer.exe drivers.txt
   ```

7. If Windows asks for permission, click **Yes**.
8. Wait until the installer is complete.

The installer installs the driver only for a connected device. Thus the pad must be in the bootloader while it runs.

### Procedure with Zadig

1. Put the pad into the bootloader. See [Put the pad into the bootloader](#put-the-pad-into-the-bootloader).
2. Start Zadig.
3. In the menu **Options**, select **List All Devices**.
4. In the device list, select `Maple 003`. Make sure that the USB ID shows `1EAF` `0003`.
5. Make sure that the target driver is **WinUSB**.
6. Click **Install Driver** or **Replace Driver**.
7. Wait until Zadig shows that the installation is complete.

You install the driver one time only. Windows keeps it for the next flash.

## Make a backup of the stock firmware

Do this before you flash this firmware for the first time.

> **CAUTION:** This project read the firmware area with this method (to read the crash log).
> It did not test to write such a backup back into the pad. Keep a second way back. See [Return to the stock firmware](#return-to-the-stock-firmware).

1. Put the pad into the bootloader. See [Put the pad into the bootloader](#put-the-pad-into-the-bootloader).
2. Run this command. Use a new file name:

   ```sh
   dfu-util -d 1eaf:0003 -a 2 -U backup.bin
   ```

3. Wait until `dfu-util` stops.
4. Make sure that `backup.bin` exists and is not empty.
5. Keep `backup.bin` in a safe place.
6. Disconnect the pad and connect it again.

The command reads the firmware area (from `0x08002000`). It does not read or change the bootloader.

## Flash the firmware

> **WARNING:** Flash only an image for the DOIO KB16-01 rev2.
> An image for a different board can stop the firmware from starting.

> **WARNING:** Use only DFU alternate setting 2 (`-a 2`). This project does not use other alternate settings.

> **CAUTION:** Do not disconnect the pad while `dfu-util` writes.
> If you disconnect it, the firmware image is incomplete, and the pad can fail to start. This case was not tested. See [Recovery](#recovery).

### Procedure with the script

1. Open a POSIX shell in the repository folder.
2. Run the script:

   ```sh
   sh scripts/flash.sh
   ```

   To flash a different image, give its path: `sh scripts/flash.sh path/to/image.bin`.
3. If the script shows "pad told to enter its bootloader", go to step 5.
4. If the script shows "waiting for the bootloader", do these steps:
   1. Disconnect the pad.
   2. Hold the top-left key.
   3. Connect the pad.
   4. Hold the key until the download starts.
5. Wait until `dfu-util` shows "Download done".
6. Disconnect the pad.
7. Connect the pad again.

The script stops `kb16-mixer.exe` before it flashes. Then it runs `kb16-mixer.exe --bootloader` if the built app exists.
After "Download done", `dfu-util` can show exit code 74. This is normal.

### Procedure with dfu-util only

1. Put the pad into the bootloader. See [Put the pad into the bootloader](#put-the-pad-into-the-bootloader).
2. Run this command:

   ```sh
   dfu-util -w -d 1eaf:0003 -a 2 -D kb16-mixer-firmware.bin -R
   ```

3. Wait until `dfu-util` shows "Download done".
4. Disconnect the pad.
5. Connect the pad again.

The bootloader does not start the new firmware after the download. It waits for a power cycle.
Thus each flash ends with one disconnect and one connect.

## Verify

1. Look at the pad after you connect it.
   - The screen shows the logo name (default `KB16`) for approximately 2 seconds.
   - The key lights show a slow white light that moves across the keys.
2. Open a text editor. Press the top-left key. The pad types `1`.
3. Start `kb16-mixer.exe`.
4. Right-click the tray icon. Make sure that the first line of the menu shows "pad connected" and the firmware version.
5. Play a sound. Make sure that the lights in the column of that channel show the level.

If the pad does not start, see [Recovery](#recovery).

## Recovery

The firmware never writes the bootloader. Thus the bootloader stays available to flash a new image.

There are three ways into the bootloader while the firmware of this project runs:

| Method | Procedure |
|---|---|
| Top-left key at power-up | Disconnect the pad. Hold the top-left key. Connect the pad. Hold the key until the pad is in the bootloader. |
| Four corner keys | Hold all four corner keys together for 2 seconds. |
| From the PC | Run `kb16-mixer.exe --bootloader`. |

The firmware also recovers by itself:

- A watchdog resets the chip if the firmware stops for approximately 2 seconds. The firmware then starts again.
- After a crash, the firmware starts again.
- After three crashed starts in a row, the pad stays in the bootloader. Then flash a correct image.

### A firmware that does not start at all

This case (for example, after an interrupted download) was **not tested**. The project can make no promise about it.
These facts are known:

- The bootloader is never overwritten.
- The QMK readme for this board names a reset button on the back of the PCB. This project did not test it.

## Return to the stock firmware

This project does not include the stock firmware.
The project does not know of an official download of the factory image from DOIO.

You have two possible sources of a stock image:

- Your backup. See [Make a backup](#make-a-backup-of-the-stock-firmware). To write a backup back was not tested.
- An image that you build from the QMK source for this board: `keyboards/doio/kb16/rev2` in the repository `qmk/qmk_firmware` on GitHub.

> **CAUTION:** The QMK default keymap is not necessarily the same as the firmware that DOIO installed at the factory.

### Build a stock image from QMK

1. Set up the QMK build environment. See the QMK documentation (https://docs.qmk.fm).
2. Build the image:

   ```sh
   qmk compile -kb doio/kb16/rev2 -km default
   ```

### Flash the stock image

QMK flashes this board with the same `dfu-util` arguments as this project (`-d 1EAF:0003 -a 2`).

1. Close `kb16-mixer.exe`. Right-click the tray icon, then click **Quit**.
2. Put the pad into the bootloader. See [Recovery](#recovery).
3. Run this command. Use the name of your stock image:

   ```sh
   dfu-util -w -d 1eaf:0003 -a 2 -D stock-image.bin -R
   ```

4. Wait until `dfu-util` shows "Download done".
5. Disconnect the pad.
6. Connect the pad again.

You can also use `sh scripts/flash.sh stock-image.bin`.

After you return to the stock firmware, you can remove `kb16-mixer.exe`.
If **Start with Windows** is on, clear it in the tray menu before you remove the app.
