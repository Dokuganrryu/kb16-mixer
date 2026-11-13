# Raw HID protocol

This document is for persons who change the firmware or the app. It gives the messages between the firmware and the app.
For the definition of each term, see [glossary.md](glossary.md).

**The source of truth is `protocol/src/lib.rs`.** This document is a copy in table form.
If this document and the code disagree, the code is correct. Report the difference in an issue.

## Transport

| Item | Value |
|---|---|
| USB vendor ID / product ID | `0xD010` / `0x1601` (the stock DOIO KB16-01 IDs) |
| USB product string | `KB16 Mixer` (`PRODUCT`). The stock firmware has the same IDs and usage page. The app uses only an interface with this product string. |
| USB device release (`bcdDevice`) | The firmware version. See [Version](#version). |
| HID usage page | `0xFF60` (the raw HID usage page that QMK and VIA use) |
| HID usage | `0x61` |
| Report length | 32 bytes, in both directions |
| Byte 0 | Always `0xD1` (`MAGIC`) |
| Byte 1 | Message ID |
| Unused bytes | Zero |

On Windows, the app writes 33 bytes: report ID `0`, then the 32 bytes of the message.

## Constants

| Name | Value | Meaning |
|---|---|---|
| `MAGIC` | `0xD1` | First byte of each report |
| `REPORT_LEN` | 32 | Length of each report |
| `CHANNELS` | 4 | Number of channels, one for each key column |
| `KEYS_PER_REPORT` | 8 | Keys in one `KEYS` message |
| `NAME_LEN` | 6 | Bytes of one channel name |
| `NAME_SHOWN` | 5 | Characters of a channel name that the mixer screen shows. The app accepts no longer name. |
| `POPUP_LEN` | 16 | Bytes of popup text |
| `LOGO_NAME_LEN` | 10 | Bytes of the name on the logo screen |
| `NOTICE` | 255 | Popup volume value: show the text only |
| `NEVER` | 255 | Idle minutes value: never |
| `UNVERSIONED_RELEASE` | `0x0300` | Device release of early firmware without a version |

## Message IDs

| ID | Name | Direction |
|---|---|---|
| `0x00` | `HEARTBEAT` | app to pad |
| `0x01` | `TURN` | pad to app |
| `0x02` | `PRESS` | pad to app |
| `0x03` | `KEY_DOWN` | pad to app |
| `0x04` | `KEY_UP` | pad to app |
| `0x05` | `KNOB_UP` | pad to app |
| `0x10` | `POPUP` | app to pad |
| `0x20` | `SETTINGS` | app to pad |
| `0x21` | `FRAME` | app to pad |
| `0x22` | `NAMES` | app to pad |
| `0x23` | `KEYS` | app to pad |
| `0x24` | `LOGO_NAME` | app to pad |
| `0x7F` | `BOOTLOADER` | app to pad |

## App to pad

Each correct message from the app, except `BOOTLOADER`, tells the pad that the app is present. `pad-core/src/state.rs` applies the messages in the firmware.
If the pad gets no message for 3 seconds, it goes to keyboard mode.
The pad accepts an unknown message ID as a sign that the app is present, and ignores its contents.

### `0x00` HEARTBEAT

| Byte | Contents |
|---|---|
| 0 | `0xD1` |
| 1 | `0x00` |

The app sends a heartbeat when it sent no other message for 1 second.

### `0x10` POPUP

A large readout or a notice on the screen.

| Byte | Field | Contents |
|---|---|---|
| 0 | | `0xD1` |
| 1 | | `0x10` |
| 2 | `knob` | Knob number. The current app always sends 0. |
| 3 | `volume` | 0–100: a readout with a value and a bar. 255 (`NOTICE`): a notice, text only. |
| 4 | `muted` | 0 = not muted, other value = muted |
| 5–20 | `name` | Text, 16 bytes, printable ASCII, zero-padded |

`Popup::notice()` in the protocol crate makes a notice: `knob` 0, `volume` 255, `muted` 0.
The app sends all its text messages (for example `Config error`, `MIXER`, `KEYS`, `Open failed`) as notices.
The pad shows a popup for 1.3 seconds. For the idle timers, a popup is not activity.

### `0x20` SETTINGS

| Byte | Field | Contents |
|---|---|---|
| 0 | | `0xD1` |
| 1 | | `0x20` |
| 2 | `channels` | Number of channels in use (the pad uses at most 4) |
| 3–5 | `colors[0]` | Red, green, blue of channel 1 |
| 6–8 | `colors[1]` | Red, green, blue of channel 2 |
| 9–11 | `colors[2]` | Red, green, blue of channel 3 |
| 12–14 | `colors[3]` | Red, green, blue of channel 4 |
| 15 | `brightness` | Light brightness. 0 = pad default (170). |
| 16 | `oled_top_plus_one` | Screen offset in pixel rows, plus one. 0 = pad default (3). The pad uses at most 6. |
| 17 | `screen_off_min` | Minutes without activity before the screen goes off. 0 = pad default (10). 255 = never. |
| 18 | `lights_dim_min` | Minutes without activity before the lights dim. 0 = pad default (10). 255 = never. |
| 19 | `idle_led_percent` | Light that the dimmed lights keep, in percent of the normal light (1–100). 0 = pad default (6). |

The app sends `255` for a setting of `0` (no timer) in the config file. It sends at most `240` minutes, and a brightness of at least `1`.
The pad uses an `idle_led_percent` above 100 as 100.

Byte 19 came after the draft release 0.1.0. Firmware from before it ignores the byte. An app from before it sends `0`, so the pad uses its default.

### `0x21` FRAME

The live state. The app sends a frame only when it changed. It looks for a change at intervals of 33 ms when one of these conditions applies:

- A program plays audible sound (bit 2).
- The pad had activity in the last 3 seconds. Activity is a touch of a key or a knob, or a meter at approximately −30 dBFS or higher.

Otherwise it looks for a change at intervals of 200 ms. See [how-it-works.md](how-it-works.md#frame-rate).

| Byte | Field | Contents |
|---|---|---|
| 0 | | `0xD1` |
| 1 | | `0x21` |
| 2–5 | `levels[0..4]` | Live sound level of each channel, 0–255 (−48 dB to 0 dB), before the volume and mute of the programs |
| 6–9 | `volumes[0..4]` | Volume of each channel, 0–100 |
| 10 | `mutes` | Bit n = channel n+1 is muted |
| 11 | `selected` | Index of the channel that has Focus (0–3), 255 = none |
| 12 | `flags` | See the next table |

| Bit of `flags` | Name | Meaning |
|---|---|---|
| 0 | `mixer_keys` | The keys use the key table. 0 = the keys type their keyboard-mode keys. |
| 1 | `mic_muted` | The microphone is muted. The screen shows a line under the names. |
| 2 | `playing` | A program plays audible sound, as the app decides it. The pad counts this as activity for its idle timers. |
| 3 | `second_page` | Page 2 of the keys is active. The screen shows a small mark at the right edge. |

The raw `levels` are only for the meters. They do not count as activity.
For the rule that sets bit 2, see [how-it-works.md](how-it-works.md#idle-rule).

When the pad receives a message after 3 seconds or more without one, it sets bits 2 and 3 to 0 until the next frame.

### `0x22` NAMES

| Byte | Field | Contents |
|---|---|---|
| 0 | | `0xD1` |
| 1 | | `0x22` |
| 2–7 | `name0` | Name of channel 1, 6 bytes |
| 8–13 | `name1` | Name of channel 2, 6 bytes |
| 14–19 | `name2` | Name of channel 3, 6 bytes |
| 20–25 | `name3` | Name of channel 4, 6 bytes |

The app sends printable ASCII in upper case. A channel name has 1 to 5 characters (`NAME_SHOWN`), because the screen strip of a channel has space for 5. The sixth byte is zero.

### `0x23` KEYS

The action of 8 keys of the current page, from key `first`. The key number is `row * 4 + col`. Row 0 is the top row. Column 0 is the left column.
The app sends two messages: `first = 0` (keys 0–7) and `first = 8` (keys 8–15).
When the page changes, the app sends the two messages again, for the new page.

| Byte | Field | Contents |
|---|---|---|
| 0 | | `0xD1` |
| 1 | | `0x23` |
| 2 | `first` | Number of the first key in this message |
| 3–5 | key `first` | `kind`, `a`, `b` |
| 6–8 | key `first + 1` | `kind`, `a`, `b` |
| … | … | … |
| 24–26 | key `first + 7` | `kind`, `a`, `b` |

Key types:

| `kind` | Name | `a` | `b` | Action of the pad |
|---|---|---|---|---|
| 0 | `Host` | 0 | 0 | Sends `KEY_DOWN` and `KEY_UP` to the app |
| 1 | `Keyboard` | HID keyboard usage | Modifier bits | Types the key |
| 2 | `Media` | Consumer usage for a tap | Consumer usage for a hold, or 0 | Sends the media key |
| 3 | `Nothing` | 0 | 0 | Does nothing |

An unknown `kind` counts as `Host`.
The app sends all its own actions (`mute`, `focus`, `talk`, `move`, `page`, `open:`) as `Host`. The pad does not know the difference.

Modifier bits: bit 0 = Ctrl, bit 1 = Shift, bit 2 = Alt, bit 3 = Win (left-side modifiers).

For a `Media` key with `b = 0`, the pad sends `a` when you press the key.
For a `Media` key with `b` not 0, the pad sends a key when you release the key.
It sends `a` after a press of less than 400 ms. It sends `b` after a press of 400 ms or more.

### `0x24` LOGO_NAME

| Byte | Field | Contents |
|---|---|---|
| 0 | | `0xD1` |
| 1 | | `0x24` |
| 2–11 | `name` | The name on the logo screen, 10 bytes, zero-padded |

The pad accepts a name only if it follows these rules:

- It has only capital letters `A`–`Z`, digits and spaces.
- It has at least one letter or digit.
- Zero bytes occur only at the end.

The pad ignores all other names.
The pad keeps the name in flash, so that it shows the name at power-up. See [hardware.md](hardware.md#name-page).

### `0x7F` BOOTLOADER

| Byte | Contents |
|---|---|
| 0 | `0xD1` |
| 1 | `0x7F` |
| 2–5 | `'B' 'O' 'O' 'T'` (`0x42 0x4F 0x4F 0x54`) |

The pad restarts into its bootloader 20 ms after this message.
The pad ignores a `0x7F` message without the 4 marker bytes.

The marker `BOOT` does not change, and anyone can read it in this document. It is not a password and not a secret.
It only makes sure that a random or damaged message cannot restart the pad.
Each program on the PC that can open the raw HID interface can send this message. See [SECURITY.md](../SECURITY.md).

## Pad to app

| Byte | `0x01` TURN | `0x02` PRESS | `0x03` KEY_DOWN | `0x04` KEY_UP | `0x05` KNOB_UP |
|---|---|---|---|---|---|
| 0 | `0xD1` | `0xD1` | `0xD1` | `0xD1` | `0xD1` |
| 1 | `0x01` | `0x02` | `0x03` | `0x04` | `0x05` |
| 2 | Knob (0–2) | Knob (0–2) | Row (0–3) | Row (0–3) | Knob (0–2) |
| 3 | Delta, signed: +1 clockwise, −1 (`0xFF`) counterclockwise | 0 | Column (0–3) | Column (0–3) | 0 |

Knob 0 is the small left knob, knob 1 is the small right knob, and knob 2 is the large knob.
One `TURN` message is one detent.

The pad sends `PRESS` when you push a knob down. While the app is present, it also sends `KNOB_UP` when you release the knob.
The app uses the time between `PRESS` and `KNOB_UP` to find a hold.
Firmware up to the draft release 0.1.0 does not send `KNOB_UP`. Thus the app must not wait for it. The app then uses each `PRESS` as a press and a release together: a tap or a double tap, but not a hold.

The pad sends `KEY_DOWN` and `KEY_UP` only for keys of the type `Host`.

## Version

The firmware sends its version as the release number of its USB device (`bcdDevice`). Thus the app can read the version from the device list.
`device_release()` in the protocol crate makes the number from the crate version. `release_version()` reads it back.

| Version | Device release |
|---|---|
| JJ.M.N | `0xJJMN` in binary-coded decimal |
| 0.1.0 | `0x0010` |
| 12.3.4 | `0x1234` |

The major version can be 0–99. The minor and patch versions can be 0–9. A version that does not fit stops the build.
`device_release()` ignores a pre-release part (for example `-beta`).

The firmware that sends `KNOB_UP` and reads byte 19 also has the version 0.1.0. Thus the version does not tell it from the draft release 0.1.0.

## Decode rules

| Case | Result |
|---|---|
| A report shorter than 2 bytes, or byte 0 is not `0xD1` | Not a message of this protocol. It is ignored. |
| A message from the app that is shorter than 32 bytes | Read as if zero bytes fill the rest |
| A `BOOTLOADER` message with wrong marker bytes | Ignored |
| An unknown message ID from the app | `Unknown`. It shows that the app is present. |
| A message from the pad shorter than 4 bytes, or with an unknown ID | Ignored |

## Change the protocol

Change the protocol only in `protocol/src/lib.rs`.
Add a test there for each change.
The test `wire_layout_is_fixed` holds the correct bytes. Change it only when you intend to change the wire format.
Then update this document.
