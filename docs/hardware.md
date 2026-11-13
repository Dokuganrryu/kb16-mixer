# Hardware: DOIO KB16-01 rev2

This document gives the facts about the board for persons who want to write their own firmware for this pad.
The facts come from the firmware source (`firmware/src`, `firmware/memory.x`) and from the public QMK board definition (`keyboards/doio/kb16/rev2` in `qmk/qmk_firmware`).

The project examined only one unit, and did its tests on that unit. Other production runs can be different.

## Parts

| Part | Details |
|---|---|
| Microcontroller | APM32F103CBT6, compatible with the STM32F103CB |
| Flash memory | 128 KB, in pages of 1 KB |
| RAM | 20 KB |
| Clock | 8 MHz crystal (HSE). The PLL multiplies it by 9 to 72 MHz. |
| USB | Full speed device. The board has a fixed pull-up resistor on D+. |
| USB ID (firmware) | `D010:1601` (the stock DOIO ID, also used by this firmware) |
| USB ID (bootloader) | `1EAF:0003` (stm32duino bootloader). Windows shows it as `Maple 003`. |
| Keys | 16 keys in a 4 x 4 matrix |
| Knobs | 3 rotary encoders. Each knob is also a push button. |
| Key lights | 16 WS2812 LEDs, one under each key |
| Screen | SSD1306 OLED, 128 x 32 pixels, on I2C, address `0x3C` |

### rev1 and rev2

The QMK board definition gives both revisions the same USB ID, `D010:1601`. Thus the USB ID of the firmware cannot tell them apart.

| Revision | Chip | Bootloader (QMK board definition) | Supported |
|---|---|---|---|
| rev1 | ATmega32U4 | Atmel DFU | No |
| rev2 | APM32F103CBT6 | stm32duino, USB ID `1EAF:0003` | Yes |

Only the bootloader shows which revision the pad is. See [install.md](install.md#2-examine-your-pad).

## Layout

The 4 x 4 keys are on the left. On the right, the two small knobs are at the top, the screen is below them, and the large knob is at the bottom.

```text
  +---------------------------------------------------+
  |  +----+----+----+----+    (knob 1)    (knob 2)    |
  |  |    |    |    |    |     small       small      |
  |  +----+----+----+----+                            |
  |  |    |    |    |    |    +----------------+      |
  |  +----+----+----+----+    |     screen     |      |
  |  |    |    |    |    |    +----------------+      |
  |  +----+----+----+----+                            |
  |  |    |    |    |    |        (  knob 3  )        |
  |  +----+----+----+----+            big             |
  +---------------------------------------------------+
```

The seller and the QMK readme name a reset button on the board, under the bottom cover. This project did not do a test of it. See [stock-firmware.md](stock-firmware.md#procedure-2-the-reset-button-under-the-bottom-cover).

## Pin table

| Function | Pin | Notes |
|---|---|---|
| Matrix row 0 (top) | PB3 | PB3 is JTDO after reset. The firmware turns off JTAG to free it. SWD stays on. |
| Matrix row 1 | PB4 | |
| Matrix row 2 | PB9 | |
| Matrix row 3 (bottom) | PB8 | |
| Matrix column 0 (left) | PB14 | |
| Matrix column 1 | PB13 | |
| Matrix column 2 | PB12 | |
| Matrix column 3 (right) | PB0 | |
| Matrix column 4 | PA7 | Knob push buttons: row 0 = knob 1, row 1 = knob 2, row 2 = knob 3 |
| Knob 1 (small, left), A / B | PB5 / PB6 | |
| Knob 2 (small, right), A / B | PA1 / PA2 | |
| Knob 3 (big), A / B | PA3 / PA4 | |
| WS2812 data | PA10 | |
| Screen SCL | PB10 | I2C2 |
| Screen SDA | PB11 | I2C2 |
| USB D− | PA11 | |
| USB D+ | PA12 | |

The matrix uses COL2ROW diodes. The firmware drives one row low at a time and reads the columns with pull-up resistors.

The QMK layout puts the knob push buttons at matrix positions (0,4), (1,4) and (2,4).
In this project, knob 1 is the small left knob, knob 2 is the small right knob, and knob 3 is the large knob.
On the wire, the knobs have the numbers 0, 1 and 2.

In mixer mode, the firmware sends a message to the app for each push and each release of a knob button (`PRESS`, `KNOB_UP`). Firmware before version 0.2.0 sends only the push. See [protocol.md](protocol.md#pad-to-app).

## Memory map

| Address | Size | Contents |
|---|---|---|
| `0x08000000` | 8 KB | stm32duino bootloader. It is not part of this project. This project does not write it. |
| `0x08002000` | 117 KB | Firmware (DFU alternate setting 2) |
| `0x0801F400` | 1 KB | Name for the logo screen (`firmware/src/name.rs`) |
| `0x0801F800` | 1 KB | Crash log (`firmware/src/diag.rs`) |
| `0x0801FC00` | 1 KB | Not used by this firmware |
| `0x20000000` | 20 KB | RAM |

`firmware/memory.x` gives the firmware region as 117 KB from `0x08002000`. The region ends below the name page.
`scripts/build-firmware.sh` rejects an image that is larger than `0x1D400` bytes (117 KB).
Thus the image cannot overlap the name page or the crash log.

The image of version 0.2.0 is approximately 53 KB. A local build and the CI build are not equal in size. The CI build is approximately 64 bytes larger, because the file paths in the panic messages use a different path separator.

### Flash guard

The firmware erases and writes the flash only in two pages: the name page (`0x0801F400`) and the crash log (`0x0801F800`).
All other flash is closed to the firmware: the bootloader, the firmware image, and the last page (`0x0801FC00`).

| Step | Rule | Code |
|---|---|---|
| Address check | An erase must start at the first byte of a reserved page. A write must be on a 2-byte boundary, and stay inside one reserved page. Other addresses get an error, and the firmware does not touch the flash controller. | `pad-core/src/flash.rs` |
| Wait | The firmware waits for the busy flag (`BSY`) for a limited time. After the limit, it stops with an error. | `pad-core/src/flash.rs` |
| Error flags | Before and after each step, the firmware reads and clears the error flags `PGERR` (program error) and `WRPRTERR` (write protection error). A set flag is an error. | `firmware/src/flash.rs` reads and clears the register. `pad-core/src/flash.rs` makes the decision. |
| Read-back | After an erase, each word must read `0xFFFF`. After a write, each word must read back the written value. | `pad-core/src/flash.rs` |
| Lock | The firmware locks the flash controller again after each operation, also after an error. | `pad-core/src/flash.rs` sets the order of the steps. `firmware/src/flash.rs` writes the registers. |

A rejected or unsatisfactory write changes nothing else. The firmware continues. The panic and hard-fault handlers use the same code, so it does not panic and does not wait without a limit.

On the PC, `pad-core/tests/rules.rs` examines the address rules. `pad-core/tests/flash.rs` examines the order of the steps with a fake flash controller. It also makes sure that the controller is locked again after each step, also after each error. No test examines the register writes in `firmware/src/flash.rs`, because they run only on the chip.

### Name page

The name page holds one record of 16-bit words:

| Word | Contents |
|---|---|
| 0 | Magic `0x4E4D` ("NM") |
| 1 | Length of the name: 10 |
| 2–6 | The name, 10 bytes, two bytes in each word, zero-padded |
| 7 | Fletcher-16 checksum of words 0–6 |

The firmware reads the record at start. If the record is not correct (for example, a blank page), the screen shows `KB16`.

The firmware writes the page only when the app sends a different name. It writes at most one time each minute, and at most 8 times for each power-up. This rule is in `pad_core::name`, and `pad-core/tests/rules.rs` examines it.
After the limit, a new name shows from RAM until the next start.
When the firmware erases the page, the CPU stops for approximately 20 ms.

## Bootloader

The bootloader is the stm32duino bootloader (LeafLabs Maple, USB ID `1EAF:0003`).
`dfu-util` writes the firmware with DFU alternate setting 2, at `0x08002000`.
QMK flashes this board with the same `dfu-util` arguments (`-d 1EAF:0003 -a 2`).
In the bootloader, the screen and the key lights are dark.

The bootloader reads the backup register `BKP_DR10` after each reset:

| `BKP_DR10` | Bootloader action |
|---|---|
| `0x424C` | Stay in DFU mode |
| `0x424D` | Start the firmware immediately |

Backup registers keep their values through a reset, but not through a power cycle.
After a power cycle, the bootloader finds no "stay" mark and starts the firmware.

The public source code of the stm32duino bootloader starts the firmware only if its first word is a stack pointer in RAM. Otherwise the bootloader stays in DFU mode. It checks the words at `0x08002000` and `0x08005000`. This project does not know if the bootloader in this pad has the same code.

After a download, the bootloader does not start the new firmware. It waits for a power cycle.
Thus, after each flash, disconnect the pad and connect it again.

## How the firmware uses the bootloader

| Event | Firmware action |
|---|---|
| Top-left key held at start | Writes `0x424C` to `BKP_DR10` and resets. The pad stays in the bootloader. |
| Four corner keys held for 2 s | The same |
| Bootloader message from the app (`D1 7F 'B' 'O' 'O' 'T'`) | The same, 20 ms after the message |
| Normal start | Writes `0x424D` to `BKP_DR10`, so that a reset starts the firmware again |
| Panic or hard fault | Writes a mark to `BKP_DR8` and resets. The bootloader starts the firmware again. |
| Watchdog reset | The firmware starts again. |
| Three crashed starts in a row | The firmware writes `0x424C` and resets. The pad is designed to stay in the bootloader. This was not tested on the pad. |

The independent watchdog has a timeout of approximately 2 seconds. A firmware task feeds it at intervals of 250 ms.

`BKP_DR9` counts crashed starts. A crash is a watchdog reset or a panic mark in `BKP_DR8`.
After 10 seconds of correct operation, the firmware sets the counter to zero.

The top-left key check occurs before the firmware sets the clocks. It reads PB14 while it drives row PB3 low.

## Firmware version

The firmware sends its version as the release number of its USB device (`bcdDevice`). The value comes from `firmware/Cargo.toml`.

The format is `0xJJMN` in binary-coded decimal, for version JJ.M.N. For example, version 0.2.0 is `0x0020`.
Thus the major version can be 0–99, and the minor and patch versions can be 0–9. The build stops for a version that does not fit.
Builds before version 0.1.0 had no version, and sent `0x0300`. The top right of the settings window then shows "Pad firmware: an early build without a version.", and `--check` shows "firmware unknown (an early build)".

The internal version 0.1.0 (`0x0010`) does not send knob releases, and it ignores the idle light byte. Version 0.2.0 (`0x0020`) sends knob releases and reads the idle light byte.

## Crash log

The firmware writes panic and hard-fault details into the flash page at `0x0801F800`.
Each entry is a 16-bit word. An empty word is `0xFFFF`. The page has space for 512 words.

A usual build does not erase the page, so the entries of all crashes stay. When the page is full, the firmware writes no more entries and gives no message.

A flash of a new usual build does not clear the page. `flash.ps1` and `flash.sh` write only below the name page.
A usual build erases the page at start only if the first word is a start-up stage marker (`0x01xx`). Only a build with `--diag` writes these markers.
A build with `--diag` erases the page at each start.

| First word | Next words |
|---|---|
| `0xDEAD` (panic) | Source line, then the last 8 bytes of the source file name, 2 bytes in each word |
| `0xFA17` (hard fault) | PC, LR, CFSR and BFAR. Each is 32 bits, written as two words, low word first. |

To read the crash log:

1. Put the pad into the bootloader. See [Recovery](flashing.md#recovery).
2. Read the firmware area into a new file:

   ```sh
   dfu-util -d 1eaf:0003 -a 2 -U crash-read.bin
   ```

3. Look at the 1 KB at offset `0x1D800` in the file (`0x0801F800` − `0x08002000`).

This project used this method to read the crash log.

A build with `--diag` (`scripts/build-firmware.sh --diag`) also writes start-up stage markers into this page.
The project found the first start-up problems on this board with this method.

## Lessons learned

These facts cost time to learn. They can help you if you write your own firmware for this pad.

- **embassy-stm32 must have the `rt` feature**, also with `default-features = false`. Without it, all interrupts go to `DefaultHandler`. The pad then stops at the first timer interrupt, with no lights. `scripts/build-firmware.sh` rejects such an image.
- **Clear the NVIC at start.** The interrupt state of the bootloader stays after the jump to the firmware. The firmware disables and clears all interrupts before it starts.
- **Force a new USB connection.** The board has a permanent pull-up resistor on D+. The firmware holds D+ low for 10 ms, so that the PC sees a new device.
- **The case hides the top rows of the screen.** On the unit of the author, the case hides approximately 3 pixel rows. All screen layouts use 29 rows and move down by `oled_top` (default 3).
- **Free PB3.** PB3 is a JTAG pin after reset. Disable JTAG (keep SWD) to use it as matrix row 0.
- **Time the WS2812 signal with the cycle counter.** The firmware sends the LED data from RAM with interrupts off, timed by the DWT cycle counter. Flash wait states then cannot change the pulse length. One frame lasts approximately 0.5 ms.
- **Limit the LED current.** The firmware limits the sum of all LED values to 45 % of full white.
- **Knobs:** each detent has four contact steps. If the decoder misses a step (two steps at once), it ignores that step.
