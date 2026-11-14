# How it works

This document is for contributors, and for users who want to know what the app does in Windows.
It gives the architecture of KB16 Mixer.
It also gives the volume rules, and lists what the app touches in Windows and what it does not touch.
For the definition of each term, see [glossary.md](glossary.md).

## Overview

```text
 +-------------------- pad --------------------+          +----------------------- PC -----------------------+
 |                                             |   USB    |                                                  |
 |  keys, knobs --> firmware --+-- keyboard ---+--------->+--> Windows (built-in HID driver) --> any program |
 |                             +-- media keys -+--------->+--> Windows (built-in HID driver)                 |
 |                             |               |          |                                                  |
 |  lights, screen <-----------+-- raw HID ----+<-------->+--> kb16-mixer.exe --> Windows Core Audio         |
 |                                             |          |                      (sliders and mute switches  |
 +---------------------------------------------+          |                       of the volume mixer)       |
                                                          +--------------------------------------------------+
```

The pad is one USB device with three HID interfaces:

| Interface | Direction | Purpose |
|---|---|---|
| Keyboard (boot protocol, 6 keys) | pad to PC | Typed keys: F13–F24, letters, digits, shortcuts, and all keys in keyboard mode |
| Consumer control (media keys) | pad to PC | Play/Pause, Next, Previous, Stop, and the volume and mute keys of keyboard mode |
| Raw HID (usage page `0xFF60`) | both | Messages between the firmware and the app. See [protocol.md](protocol.md). |

Windows uses its built-in HID driver for all three interfaces. The project has no driver.

## Two modes of the pad

| Mode | When | Keys | Knobs | Lights |
|---|---|---|---|---|
| Mixer mode | The app sent a message in the last 3 seconds | As set on the current page | Send turns and presses to the app | Level meters of the channels. After a volume change, the column of that channel shows the volume for 1.3 seconds. |
| Keyboard mode | No message from the app for 3 seconds | Fixed keyboard layout (see [configuration.md](configuration.md#keyboard-mode)) | Fixed media keys | Slow white light |

The app sends a heartbeat message each second when it has nothing else to send. Thus the pad stays in mixer mode while the app runs.
If you close the app or select **Pause** in the tray menu, the pad goes back to keyboard mode after 3 seconds.
**Pause** first sets the pad back to page 1 and releases all held mixer keys (**Focus**, **Talk**, **Move**, **Switch page**). When you select **Pause** again to stop it, the app does the same.

In mixer mode, `mixer_keys = false` in the config file gives the keys their keyboard-mode layout. The knobs and lights stay in mixer mode.

## The crates

| Crate | Folder | Target | Purpose |
|---|---|---|---|
| `kb16-mixer-protocol` | `protocol/` | any (`no_std`) | The wire format of the raw HID messages, and the version format. All other crates use it. |
| `kb16-mixer-pad-core` | `pad-core/` | any (`no_std`) | The firmware logic that touches no hardware register: key and knob decoding, the routing of keys and knobs, the idle rule, lights, screens, the flash guard. The PC runs its tests. |
| `kb16-mixer-firmware` | `firmware/` | `thumbv7m-none-eabi` | The firmware of the pad, written with Embassy. It holds the tasks, the drivers and the register code, and uses `pad-core` for each decision. |
| `kb16-mixer` | `app/` | Windows | The tray app |

## Firmware modules

| Module | Contents |
|---|---|
| `main.rs` | Start, the USB device, the tasks `input`, `host_rx`, `usb_task` and `watchdog`. The routing of keys and knobs is in `pad_core::route`. |
| `keys.rs` | The 1 kHz scan of the keys and the knobs. The decoding is in `pad_core::input`. |
| `leds.rs` | The output to the WS2812 LEDs. The colors are in `pad_core::leds`. |
| `oled.rs` | The I2C output to the screen. The drawing is in `pad_core::oled`. |
| `state.rs` | The shared state, the reset, and the entry into the bootloader. The bootloader marks and the crash counter rule are in `pad_core::boot`. |
| `flash.rs` | The registers of the flash controller: the only code that erases or writes the flash. The steps (address check, unlock, erase or write, read-back, lock) and their order are in `pad_core::flash`. See [hardware.md](hardware.md#flash-guard). |
| `name.rs` | The logo name in its flash page. The record format and the write limit are in `pad_core::name`. |
| `diag.rs` | The crash log in its flash page |

`pad-core/` holds the firmware logic that touches no hardware register. The PC runs its tests (`pad-core/tests/`). Its modules:

- `input`: the debounce, the knobs, the queue of knob detents, the four-corner combo
- `state`: the reports from the app, and the idle rule
- `leds`, `oled`: what the key lights and the screen show
- `route`: where a key press or a knob turn goes (the app, the keyboard report or a media key), and the keyboard report
- `name`: the record of the logo name, and its write limit
- `boot`: the bootloader marks, their order, and the crash counter rule
- `flash`: the pages that the firmware can erase and write, and the steps that do it
- `assets`, `font`: bitmaps that `firmware/gen_assets.py` makes

## Firmware tasks

| Task | Rate | Work |
|---|---|---|
| `keys::scan` | 1 kHz | Scans the 4 x 5 key matrix (5 ms debounce) and the 3 knobs. Watches the four corner keys. |
| `input` | on each event | Sends a key or knob event as a keyboard report, a media key report, or a raw HID message |
| `host_rx` | on each message | Reads raw HID messages from the app, updates the shared state, and saves a new logo name |
| `usb_task` | continuous | Runs the USB device |
| `leds::run` | 200 Hz | Calculates the key lights and sends the data to the 16 WS2812 LEDs |
| `oled::run` | approximately 30 Hz | Draws the screen. A new readout draws at once, at most once in 8 ms. Sends only the changed columns of each 8-row page over I2C with DMA. After start and after the screen comes on again, it sends the full picture. |
| `watchdog` | 4 Hz | Feeds the watchdog. Marks the firmware as healthy after 10 seconds. |

A key that types (a `Keyboard` or `Media` key) goes from the pad directly to Windows. The app does not see it.
A mixer key (`mute`, `focus`, `talk`, `move`, `page`, `open:`) goes to the app as a raw HID message.

The firmware keeps only the key table of the current page. When the page changes, the app sends the key table of the new page.
The firmware keeps the action of each key from the time of the press. The release then always matches the press, also if the page changed in between.

The firmware does not lose a key release:

- If its event queue is full, the key scan tries the change again at the next scan.
- If the PC does not accept a release report, the firmware sends it again at intervals of 100 ms until the PC accepts it.
- A key release frees only the key code and the modifiers of that key. If two held keys send the same key code or modifier, it stays held until the two keys are up.
- The keyboard report holds six key codes. While you hold six keys, a seventh key waits (also its modifiers) until an earlier key is up.
- When the PC suspends, resets or switches off the USB bus, the firmware releases all typed keys. It sends that state when the bus is back.

Thus a lost release cannot keep a key (for example Ctrl or Shift) held on the PC.
The queue holds at most 8 knob detents for each knob.

When the PC suspends the USB bus (the PC sleeps or is off), the lights and the screen go off.

### Screens

| Screen | When |
|---|---|
| Logo with the name (default `KB16`) | For 1.8 seconds after start, in keyboard mode, and in mixer mode after 45 seconds without activity |
| Mixer screen: one column for each channel, in the screen style | In mixer mode |
| Readout: a name, the volume and a bar, in the screen style | For 1.3 seconds after a knob turn, a knob press or a mute |
| Notice: text only (for example `MIXER`, `KEYS`, `Focus Chat`, `Open failed`). A long notice uses a smaller font. | For 1.3 seconds |

The `[screen] style` value in `config.toml` (or the settings window) sets the screen style. The default is `classic`.

| Style | Mixer column | Readout |
|---|---|---|
| `classic` | The name, a meter of 7 segments with a peak mark, a dotted fader and the volume in percent (or `MUTE`) | The name and the volume (or `MUTED`) in large letters, and a bar in a frame |
| `lean` | The name, a solid fader on a thin track, and a meter line of 1 pixel. A muted channel shows `MUTE`. | The name in small letters, the volume in large digits, and a bar without a frame |
| `numbers` | The name in capital letters, the volume as a number (or `MUTE`), and a rail | The name and the volume (or `Muted`) in large letters, and a bar without a frame |

In `classic` and `lean`, a filled pill around a name shows the channel that has Focus. A line under the names means that the microphone is muted. A short mark at the right edge means that page 2 is active.

In `numbers`, an inverted name is the channel that has Focus. Each rail shows the volume of its channel as a line of 1 pixel. The rail is 2 pixels thick as far as the live sound goes.
A short vertical mark at the left edge means that the microphone is muted. A short vertical mark at the right edge means that page 2 is active.
The pad does not know which channel is the microphone, so the microphone mark shows for each mute of the microphone.

For a muted channel, the bar of the readout is hollow.
A knob turn at 0 or 100 cannot change the volume. Then a short end stop shows at that end of the bar for 0.1 seconds.

### Screen changes

All three styles use the same motion. These times are in `pad-core/src/oled.rs`:

- A readout pushes the screen up and comes in from below in 0.1 seconds. The motion starts fast and stops gently. The readout shows on the first frame.
- When the readout time ends, the readout goes down and the screen comes back in 0.07 seconds. A new readout during this motion starts from the current position.
- When the volume changes while the readout shows, the number changes at once. The bar moves to the new value in a few frames.
- The meters go up at once. They come down after a short hold, and they move in small steps.
- A page change moves the screen sideways in 0.14 seconds. Page 2 comes in from the right, and page 1 comes in from the left. The `MIXER` or `KEYS` notice comes in with this motion.
- A page change back in less than 1.5 seconds only moves the screen 4 pixels. This occurs when you release a held page key, or when you push a page key two times quickly. The app then sends no `MIXER` or `KEYS` notice.
- In `classic` and `lean`, a mute change inverts the name of that channel for 0.2 seconds. The channel that has Focus then shows only the outline of its pill. If a readout covers the screen, this change waits until the readout goes.
- After you change the style, the next frame shows the new style without motion.
- After the screen-off limit, the screen fades out in 0.7 seconds, together with the key lights. Then it goes off.
- When the screen comes on again, the firmware sends the full picture first. Then it starts the screen at zero contrast, and the contrast increases to full in 0.15 seconds, together with the key lights. Thus no previous picture or noise shows.
- The logo comes in from the left in 0.9 seconds, after start and when the screensaver starts. Then its streak crosses it one time. One period of its wave is approximately 9 seconds.

The unit tests in `pad-core/src/oled.rs` check these motions frame by frame. The pictures in `docs/images/` show the screens at rest. Nobody did a visual check of these changes on a pad.

### Light motion

The key lights also move smoothly. These values are in `pad-core/src/leds.rs`:

- After a volume change, the three meter lights of that channel show its volume for 1.3 seconds, the same time as the readout. They use the same glides as the meter. The first message from a new app session is not a change.
- On page 2, the meter lights stay dim and even. The mute row does not change.

- Each light glides to its new level. It goes up fast (approximately 35 ms) and down slowly (approximately 250 ms). A mute changes the colour to amber in approximately 60 ms.
- The brightness uses a gamma curve of 2.4, so equal steps of `brightness` look approximately equal.
- The firmware dithers the 8-bit LED values in quarter steps over time. Thus the light can change in steps that are smaller than one LED step.
- The dither pattern repeats at 50 Hz or faster, and it changes the light by 1/16 of a step or less. Nobody did a test for flicker on a pad.

## Idle rule

The screen goes off and the key lights dim after a number of minutes without activity (`screen_off_min`, `lights_dim_min`).

- After `screen_off_min`, the screen fades out in 0.7 seconds. Then it goes off. With the default limits (`screen_off_min` and `lights_dim_min` both 10), the screen and the key lights fade at the same time.
- The key lights fade down in 0.7 seconds to 6 % of their usual light. The setting `[lights] idle_percent` changes this value.
- After three times `lights_dim_min` (at most 240 minutes), the key lights fade off in 2 seconds. When `lights_dim_min` is 0 (no timer), they also do not go off.
- After activity, the lights come back in 0.15 seconds.

Firmware before version 0.2.0 dims the lights only to approximately 14 % of their light. It does not turn them off.

Activity is one of these:

- A key press or a knob turn.
- Audible sound, as the app finds it (frame flag bit 2).

A readout or a notice on the screen is not activity. Thus a notice such as `Config loaded` does not keep the screen on.

The app sets sound as audible when all of these conditions apply:

- A program plays above approximately −40 dBFS, after its own volume and mute. The microphone does not count.
- The sound continues for at least 1 second. A short notification sound does not count.
- The output device of that program is not muted.

The audible state ends after 2.5 seconds of quiet. Thus a short pause between two songs does not end it.
The meters always show the raw levels, before the volume and mute of each program.

## App threads

| Thread | Work |
|---|---|
| Main thread | Tray icon, tray menu, settings window and its dialogs (Win32 message loop) |
| Pad listener | Waits for raw HID messages from the pad and passes them to the worker. It reconnects when the pad is connected again. It also reads the firmware version from the device list. |
| Worker | Handles knob and key events. Changes the volume and mute in Windows. Sends frames, settings, names, the logo name and the key table to the pad. Saves the state. It is the only writer of `apps.toml`. |
| Open thread | One short thread for each `open:` key press. It starts the target, so that a slow target does not stop the worker. |

The listener and the worker each open a handle to the raw HID interface. The listener reads. The worker writes.
The settings window asks the worker for the list of programs, and asks it to cancel a move.

When you click **Quit**, or when Windows ends the session (logoff or shutdown), the main thread tells the worker to stop.
The worker stops **Focus** and **Talk**, gives all levels back, and saves the state. The main thread waits for this, for 3 seconds at most.

### App modules

The app is a library (`app/src/lib.rs`) and a program of 8 lines (`app/src/main.rs`) that calls it. Thus the tests in `app/tests/` can use the library.

| Module | Contents |
|---|---|
| `lib.rs` | Start, command-line options, the single-instance guard |
| `tray.rs` | The main thread: the tray icon, the tray menu, and a hidden window for messages from Windows and from the other threads |
| `worker.rs` | The worker loop |
| `check.rs` | The report of `--check` |
| `open.rs` | The `open:` key |
| `autostart.rs` | **Start with Windows**: one value in the `Run` key of the current user |
| `keys.rs` | The mixer keys (Mute, Focus, Talk, Move, Switch page, `open:`), one event at a time in the order of arrival |
| `audio.rs` | The volume engine: audio sessions, channels, Focus, meters, the audible rule |
| `trim.rs` | The trim rules, and the knob target (whole channel or one program) |
| `pad.rs` | The raw HID link to the pad |
| `config.rs` | `config.toml`, `apps.toml`, `state.toml` |
| `procs.rs` | Program names from the Toolhelp snapshot, and the foreground window |
| `win.rs` | Small safe wrappers for the Win32 calls of the settings window and the dialogs |
| `settings/mod.rs` | The settings window: the three tabs, open and close. The window and its controls exist only while it is open. |
| `settings/layout.rs` | The place of each control, and the controls that each tab shows |
| `settings/scene.rs` | What the window paints below its controls: the background, the tab strip, the pad status, the cards and the footer |
| `settings/controls.rs` | Makes the controls, reads and writes their values, and calculates the display scale |
| `settings/ids.rs` | The control IDs, and Win32 message numbers |
| `settings/fill.rs` | Puts the settings into the controls. Follows the file when another program changes it. |
| `settings/changes.rs` | What each control does when you change it: check, write `config.toml`, and show the result in the status line |
| `settings/apps.rs` | The **Add** and **Remove** buttons of the **Channels** tab |
| `settings/file.rs` | The edits of `config.toml`. The comments and the layout of the file stay. |
| `settings/choices.rs` | What each dropdown list shows. Data and functions only, with no window. |
| `settings/checks.rs` | What the **Channels** tab lists, and the checks that refuse a change before the app writes it |
| `settings/preview.rs` | Draws the window or a dialog into a picture, with the window hidden. For the design checks and the pictures in the docs. |
| `settings/window_tests.rs` | Tests with the real window, hidden: each change applies and goes into `config.toml` |
| `ui/mod.rs` | The look of the settings window and the dialogs: the fonts and the display scale |
| `ui/theme.rs` | The colours, from the Windows app theme (dark or light) or from high contrast. Also the colour of the title bar. |
| `ui/canvas.rs` | A back buffer with smooth rounded shapes and GDI text |
| `ui/anim.rs` | The short animations. A timer runs only while something moves. |
| `ui/draw.rs` | How each kind of control looks in each state (hover, press, focus) |
| `ui/widget/mod.rs` | Real Win32 controls with the look of `draw.rs`. Windows keeps their keyboard use, focus and screen-reader role. |
| `ui/widget/input.rs` | Hover and press feedback, Ctrl+Tab, Home and End in a segmented group, the default button of a dialog |
| `ui/widget/paint.rs` | Paints each control, the rows of the lists, and the open list of a dropdown |
| `ui/widget/tip.rs` | A tooltip with the full text of a dropdown or of the status line when the text does not fit |
| `dialog/` (folder) | The small dialogs: `shortcut.rs` (with the key logic in `keys.rs`), `text.rs` (a line of text with a **Browse...** button) and `look.rs` (the look and the places of their controls) |

[design/settings-ui.md](design/settings-ui.md) gives the colours, the controls and the motion rules of the settings window, and how the app draws it.

### Frame rate

The worker sends a frame (levels, volumes, mutes, flags) only when the frame changed.
It looks for a change at intervals of 33 ms (approximately 30 times per second) when one of these conditions applies:

- A program plays audible sound.
- The pad had activity in the last 3 seconds.

Activity is a touch of a key or a knob, or a meter at approximately −30 dBFS or higher.
Thus the fast rate continues for 3 seconds after the last meter falls below −30 dBFS.
Otherwise the worker looks for a change at intervals of 200 ms. An open microphone in a quiet room stays below −30 dBFS, so it does not keep the fast rate.

The worker handles key presses and releases one at a time, in the order in which they arrived (`app/src/keys.rs`).
Thus a quick release and press of **Talk** cannot put the microphone in an incorrect state.
When you push **Talk**, the app writes the earlier mute state of the microphone to `state.toml` at once. If the app stops while you hold **Talk**, it sets that mute state again at the next start.
A key with a hold or a double tap also gets those actions from this loop.

### Tap, hold and double tap

`app/src/gesture.rs` finds a tap, a hold or a double tap from the press and the release of a knob or a key. These are the rules:

- When a knob or a key has no hold and no double tap, a press is a tap at once.
- A hold occurs while the knob or the key is down, 350 ms after the press. The release then does nothing.
- When a knob or a key has a hold, a tap is a release before 350 ms.
- When a knob or a key has a double tap, a tap waits 280 ms after the release for a second press. The second press is the double tap.
- A knob turn while the knob is down cancels the press.

The worker loop waits with a timeout. It wakes up at the next time limit of a gesture. Thus the gestures do not use a thread or a timer of their own.

For a knob hold, the app must get the knob release (`KNOB_UP`) from the pad. Firmware before version 0.2.0 does not send it. With that firmware, each knob press is a tap, or a double tap, but not a hold. The app finds the new firmware at its first knob release.
It reads the volume state of Windows again at intervals of 1.5 seconds.
It sends the settings, names, logo name and key table on connect, after a config change, and at intervals of 30 seconds.

## What travels over raw HID

| Direction | Contents |
|---|---|
| App to pad | Live frame: level of each channel, volume of each channel, mute bits, the channel that has Focus, and flags (mixer keys, microphone muted, audible sound, page 2) |
| App to pad | Settings: number of channels, channel colors, light brightness, screen offset, idle minutes, idle light percent |
| App to pad | Channel names (6 bytes each), the logo name (10 bytes), popups for the screen, the key table, heartbeat |
| App to pad | The bootloader command. It holds the fixed marker `BOOT`. The marker is not a secret. It only prevents a restart from a random message. |
| Pad to app | Knob turn (one detent), knob press, knob release, mixer key press, mixer key release |

Typed keys do not travel over raw HID. The app does not send key presses.
See [protocol.md](protocol.md) for the byte layout.

## Volume rules

A program plays at **channel level x trim**.
The trim is a number from 0 to 1 for each program. The default is 1. The app saves the trims in `state.toml`, by program name.
The source of these rules is the comments at the top of `app/src/audio.rs` and in `app/src/trim.rs`.

### The channel knob

A turn of a knob on the full channel changes only the channel level. The trims do not change.
Thus the balance between the programs stays, also at 0 % and at 100 %.

### One program

A knob hold (the default `pick` action) can select one program in the channel. A turn then changes only that program. The other programs keep their sound:

| New level of the program | Result |
|---|---|
| At or below the channel level | Only the trim of that program changes. |
| Above the channel level | The channel level goes up to the new level. The trim of that program becomes 1. The trims of the other programs go down by the same factor, so that they keep their sound. |
| Any level, on a channel that was off (0 %) | The channel level becomes the new level. The other trims do not change, so the other programs come back at their old balance. |

The list of programs contains each program that has an audio session in the channel, in the order of the program name.
After 8 seconds without a touch on the knob, or when the selected program stops, the knob goes back to the full channel.
If the channel has no program, the screen shows `No app in <channel>`.

A knob on `master`, `mic` or `focused` has no program list. A `pick` shows only the level. A tap (the default `mute` action) mutes the knob target.

### Changes in Windows

Windows wins. If you move a slider in the Windows volume mixer or in a program, the app finds the change at the next refresh.
The app then applies the same rule as for a change of one program on the pad. No other program changes its sound.
If a program has more than one session, the other sessions follow the first session that moved.

### Programs that start later

When a program starts to play in a channel, and you set the level of that channel on the pad, the program gets **channel level x its saved trim**.
It also gets the mute of the channel.
In a channel that you did not set on the pad, the app follows the level of the program.

### Mute and Focus

- A Mute key mutes the full channel. It does not change a level or a trim.
- A turn up on a muted channel unmutes it. A turn up does not unmute the microphone.
- Focus turns the sessions of the other channels down from their initial level. When Focus stops, each session gets the same level back.
- If the app cannot read the level of a session when Focus starts, it does not turn that session down. It turns the session down at the next refresh that can read the level.
- The app tracks each audio session separately. A program can have more than one session (for example, one for each output device), and each session keeps its own initial level.
- Focus also turns down a session that starts while Focus is on.
- If a program closes while Focus is on, the app keeps its initial level. When the program starts again, it gets that level back.
- A session that goes out of a turned-down channel gets its initial level back.
- While Focus turns a channel down, the app learns nothing from Windows for that channel.
- Focus does not turn down the microphone channel.
- The app writes the initial levels to `state.toml` at once. If the app stops while Focus is on, it sets those levels again at the next start.
- The app keeps these levels for each session. A session that is live again at the next start gets its own level back. A session that stopped gets the level of its program.
- The app writes a side file to the disk (`sync_all`). When the side file is on the disk, the app renames it over `state.toml`. After a crash or a power cut, the disk has the previous file or the new file, not a part of a file. The app does not wait until the rename itself is on the disk. Nobody did a test with a power cut.

## Channels and programs

The app gives each audio session to one channel:

1. A program that you moved with the Move key goes to the channel that you moved it to.
2. Otherwise, the first channel in the file that matches the session gets it. A channel with `apps` matches the programs that it names. A channel with `target = "focused"` matches the foreground program and its child processes. When the desktop, the taskbar or a folder window (`explorer.exe`) is in front, no program is in front, and a `focused` channel gets no session.
3. Otherwise, the first channel with `target = "other"` gets the session.
4. No channel controls a session that matches none of these rules.

Thus the order of the channels in the file is important. A `focused` channel above an `apps` channel gets the foreground program, also if the `apps` channel names it.

The settings window shows the programs of each channel with the same rule.

An audio session is what Windows shows as one slider in the volume mixer. Windows makes one session for each process that plays sound.
Thus all tabs of one browser share one slider, and the app cannot change two tabs separately.
The app ignores the "System Sounds" session.

## What the app touches in Windows

| Item | API | Read or write |
|---|---|---|
| Volume and mute of each audio session (one slider in the volume mixer) | Core Audio `ISimpleAudioVolume` | Read and write |
| Peak level of each audio session (for the meters and the idle rule) | Core Audio `IAudioMeterInformation` | Read |
| Level and mute of the default communications microphone (if Windows has none, the default microphone) | Core Audio `IAudioEndpointVolume` | Read and write |
| Level and mute of the output device: the Windows default, or the device in `[audio] output` (write only for a knob set to `master` or `system`) | Core Audio `IAudioEndpointVolume` | Read and write |
| Names of running programs, and their parent process | Toolhelp process snapshot (`CreateToolhelp32Snapshot`) | Read |
| The process of the foreground window (for `focused` and Move) | `GetForegroundWindow`, `GetWindowThreadProcessId` | Read |
| The pad, through its raw HID interface, and its USB release number. The app uses only an interface with the product string `KB16 Mixer`, so it does not talk to a pad with the stock firmware. | hidapi | Read and write |
| A message when a HID device is connected. Without the pad, the app searches the HID device list only after such a message, or after 10 seconds. | `RegisterDeviceNotificationW` (`WM_DEVICECHANGE` on the hidden window of the app) | Read |
| The target of an `open:` key | `ShellExecuteExW` with the verb "open" | Starts it, as a double-click in Explorer does. An existing file, given by its full path, starts in its own folder. |
| Files that you select with **Browse...** in a dialog | The standard Windows file dialog | Read the path only |
| `config.toml`, `apps.toml`, `state.toml`, `log.txt`, `selftest.txt` in the settings folder | File system | Read and write |
| The value `kb16-mixer` in `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` (Start with Windows) | Registry | Read and write. The app does not write it when `KB16_MIXER_DIR` has a value. |
| Notepad and Explorer, when you click **Open config file** or **Open settings folder** | Process start | Starts them |

An `open:` key does not ask for administrator rights (no elevation), and passes no arguments. The app does not keep a handle to the process that it starts.

If the target program must have administrator rights, Windows shows its User Account Control (UAC) prompt. The app does not bypass that prompt. At this time, nobody did a test of this on a pad.

If the target is a file that exists, and you give its full path, the program starts in the folder of that file. Otherwise Windows selects the folder.
If the program does not start, the pad shows `Open failed`, and the app writes a line to `log.txt`.

## What the app does not touch

- It does not route, capture or record audio.
- It does not make virtual audio devices.
- It does not install or load a driver.
- It does not use keyboard hooks or mouse hooks.
- It does not inject input. It does not send key presses or mouse movements.
- It does not open, read or write other processes. It gets program names from the Toolhelp snapshot only, and opens no process handle.
- It does not connect to the network.

The app uses no API from the keyboard and mouse module of Windows (`Win32_UI_Input_KeyboardAndMouse`).
That feature of the `windows` crate is not enabled. A test in `app/src/lib.rs` makes sure that it stays off.
The test `app/tests/forbidden_apis.rs` reads the source. It gives an error on the name of a hook, an input injection, a process access or a network API.

The shortcut dialog of the settings window reads keys only from its own box, while that box has the keyboard focus.
It reads the usual window messages of that box. It uses no hook and no raw input, and it does not poll the key state.
