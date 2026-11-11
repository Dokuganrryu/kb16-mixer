# How it works

This document describes the architecture of KB16 Mixer.
It also gives the volume rules, and lists what the app touches in Windows and what it never touches.
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
| Mixer mode | The app sent a message in the last 3 seconds | As set on the current page | Send turns and presses to the app | Level meters of the channels |
| Keyboard mode | No message from the app for 3 seconds | Fixed keyboard layout (see [configuration.md](configuration.md#keyboard-mode)) | Fixed media keys | Slow white light |

The app sends a heartbeat message each second when it has nothing else to send. Thus the pad stays in mixer mode while the app runs.
If you close the app or select **Pause** in the tray menu, the pad goes back to keyboard mode after 3 seconds.
**Pause** first sets the pad back to page 1 and releases all held mixer keys (Focus, Talk, Move, Switch page). When you select **Pause** again to stop it, the app does the same.

In mixer mode, `mixer_keys = false` in the config file gives the keys their keyboard-mode layout. The knobs and lights stay in mixer mode.

## The crates

| Crate | Folder | Target | Purpose |
|---|---|---|---|
| `kb16-mixer-protocol` | `protocol/` | any (`no_std`) | The wire format of the raw HID messages, and the version format. All other crates use it. |
| `kb16-mixer-pad-core` | `pad-core/` | any (`no_std`) | The firmware logic that needs no hardware: key and knob decoding, the idle rule, lights, screens, the flash guard. The PC runs its tests. |
| `kb16-mixer-firmware` | `firmware/` | `thumbv7m-none-eabi` | The firmware of the pad, written with Embassy. It holds the tasks, the drivers and the register code, and uses `pad-core` for each decision. |
| `kb16-mixer` | `app/` | Windows | The tray app |

## Firmware modules

| Module | Contents |
|---|---|
| `main.rs` | Start, the USB device, the tasks `input`, `host_rx`, `usb_task` and `watchdog` |
| `keys.rs` | The 1 kHz scan of the keys and the knobs. The decoding is in `pad_core::input`. |
| `leds.rs` | The output to the WS2812 LEDs. The colors are in `pad_core::leds`. |
| `oled.rs` | The I2C output to the screen. The drawing is in `pad_core::oled`. |
| `state.rs` | The shared state, the reset, and the entry into the bootloader. The crash counter rule is in `pad_core::boot`. |
| `flash.rs` | The flash guard: the only code that erases or writes the flash. It writes only the two reserved pages. See [hardware.md](hardware.md#memory-map). |
| `name.rs` | The logo name in its flash page. The record format is in `pad_core::name`. |
| `diag.rs` | The crash log in its flash page |

`pad-core/` holds the firmware logic that needs no hardware. The PC runs its tests (`pad-core/tests/`). Its modules:

- `input`: the debounce, the knobs, the queue of knob detents, the four-corner combo
- `state`: the reports from the app, and the idle rule
- `leds`, `oled`: what the key lights and the screen show
- `name`: the record of the logo name
- `boot`: the crash counter rule
- `flash`: the pages that the firmware can erase and write
- `assets`, `font`: bitmaps that `firmware/gen_assets.py` makes

## Firmware tasks

| Task | Rate | Work |
|---|---|---|
| `keys::scan` | 1 kHz | Scans the 4 x 5 key matrix (5 ms debounce) and the 3 knobs. Watches the four corner keys. |
| `input` | on each event | Sends a key or knob event as a keyboard report, a media key report, or a raw HID message |
| `host_rx` | on each message | Reads raw HID messages from the app, updates the shared state, and saves a new logo name |
| `usb_task` | continuous | Runs the USB device |
| `leds::run` | 200 Hz | Calculates the key lights and sends the data to the 16 WS2812 LEDs |
| `oled::run` | approximately 30 Hz | Draws the screen. Sends the picture over I2C with DMA, only when the picture changed |
| `watchdog` | 4 Hz | Feeds the watchdog. Marks the firmware as healthy after 10 seconds. |

A key that types (a `Keyboard` or `Media` key) goes from the pad directly to Windows. The app does not see it.
A mixer key (`mute`, `focus`, `talk`, `move`, `page`, `open:`) goes to the app as a raw HID message.

The firmware keeps only the key table of the current page. When the page changes, the app sends the key table of the new page.
The firmware remembers the action of each key at the time of the press. The release then always matches the press, also if the page changed in between.

The firmware never drops a key release:

- If its event queue is full, the key scan tries the change again at the next scan.
- If the PC does not take a release report, the firmware sends it again every 100 ms until the PC takes it.
- When the PC suspends, resets or switches off the USB bus, the firmware releases all typed keys. It sends that state when the bus is back.

Thus a lost release cannot leave a key (for example Ctrl or Shift) held on the PC.
The queue holds at most 8 knob detents for each knob.

When the PC suspends the USB bus (the PC sleeps or is off), the lights and the screen go off.

### Screens

| Screen | When |
|---|---|
| Logo with the name (default `KB16`) | For 1.8 seconds after start, in keyboard mode, and in mixer mode after 45 seconds without activity |
| Mixer screen: one column for each channel (name, live meter, volume bar, value or `MUTE`) | In mixer mode |
| Readout: a name, a value and a bar | For 1.3 seconds after a knob turn, a knob press or a mute |
| Notice: text only (for example `MIXER`, `KEYS`, `Focus Chat`, `Open failed`) | For 1.3 seconds |

On the mixer screen, a line under the names means that the microphone is muted.
A short vertical mark at the right edge means that page 2 is active.

## Idle rule

The screen goes off and the key lights dim after a number of minutes without activity (`screen_off_min`, `lights_dim_min`).
Activity is one of these:

- A key press or a knob turn.
- Audible sound, as the app decides it (frame flag bit 2).

A readout or a notice on the screen is not activity. Thus a notice such as `Config loaded` does not keep the screen on.

The app decides that sound is audible when all of these are true:

- A program plays above approximately −40 dBFS, after its own volume and mute. The microphone does not count.
- The sound continues for at least 1 second. A short notification sound does not count.
- The default output device is not muted.

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
The settings window asks the worker for the list of programs, and asks it to undo a move.

When you click **Quit**, or when Windows ends the session (logoff or shutdown), the main thread tells the worker to stop.
The worker stops Focus and Talk, gives all levels back, and saves the state. The main thread waits for this, for 3 seconds at most.

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
| `settings/` (folder) | The settings window. `layout.rs` makes the controls, and `fill.rs` fills them from the file. `changes.rs` applies a change, `apps.rs` the **Add** and **Remove** buttons, and `file.rs` the edits of `config.toml`. `choices.rs` and `checks.rs` hold the logic. `controls.rs` and `ids.rs` hold the shared Win32 parts. |
| `dialog/` (folder) | The small dialogs: `shortcut.rs` (with the key logic in `keys.rs`) and `text.rs` (a line of text with a **Browse...** button) |

### Frame rate

The worker sends a frame (levels, volumes, mutes, flags) only when the frame changed.
It checks for a change every 33 ms (approximately 30 times per second) when one of these is true:

- A program plays audible sound.
- The pad had activity in the last 3 seconds.

Activity is a touch of a key or a knob, or a meter at approximately −30 dBFS or higher.
Thus the fast rate continues for 3 seconds after the last meter falls below −30 dBFS.
Otherwise the worker checks every 200 ms. An open microphone in a quiet room stays below −30 dBFS, so it does not keep the fast rate.

The worker handles key presses and releases one at a time, in the order in which they arrived (`app/src/keys.rs`).
Thus a quick release and press of Talk cannot leave the microphone in the wrong state.
It reads the volume state of Windows again every 1.5 seconds.
It sends the settings, names, logo name and key table on connect, after a config change, and every 30 seconds.

## What travels over raw HID

| Direction | Contents |
|---|---|
| App to pad | Live frame: level of each channel, volume of each channel, mute bits, the channel that has Focus, and flags (mixer keys, microphone muted, audible sound, page 2) |
| App to pad | Settings: number of channels, channel colors, light brightness, screen offset, idle minutes |
| App to pad | Channel names (6 bytes each), the logo name (10 bytes), popups for the screen, the key table, heartbeat |
| App to pad | The bootloader command. It holds the fixed marker `BOOT`. The marker is not a secret. It only prevents a restart from a random message. |
| Pad to app | Knob turn (one detent), knob press, mixer key press, mixer key release |

Typed keys do not travel over raw HID. The app never sends key presses.
See [protocol.md](protocol.md) for the byte layout.

## Volume rules

A program plays at **channel level x trim**.
The trim is a number from 0 to 1 for each program. The default is 1. The app saves the trims in `state.toml`, by program name.
The source of these rules is the comments at the top of `app/src/audio.rs` and in `app/src/trim.rs`.

### The channel knob

A turn of a knob on the whole channel changes only the channel level. The trims do not change.
Thus the balance between the programs stays, also at 0 % and at 100 %.

### One program

A knob press can select one program in the channel. A turn then changes only that program. The other programs keep their sound:

| New level of the program | Result |
|---|---|
| At or below the channel level | Only the trim of that program changes. |
| Above the channel level | The channel level goes up to the new level. The trim of that program becomes 1. The trims of the other programs go down by the same factor, so that they keep their sound. |
| Any level, on a channel that was off (0 %) | The channel level becomes the new level. The other trims do not change, so the other programs come back at their old balance. |

The list of programs contains each program that has an audio session in the channel, in the order of the program name.
After 8 seconds without a touch on the knob, or when the selected program stops, the knob goes back to the whole channel.
If the channel has no program, the screen shows `No app in <channel>`.

A knob on `master`, `mic` or `focused` has no program list. A press shows only the level.

### Changes in Windows

Windows wins. If you move a slider in the Windows volume mixer or in a program, the app finds the change at the next refresh.
The app then applies the same rule as for a change of one program on the pad. No other program changes its sound.
If a program has more than one session, the other sessions follow the first session that moved.

### Programs that start later

When a program starts to play in a channel, and you set the level of that channel on the pad, the program gets **channel level x its saved trim**.
It also gets the mute of the channel.
In a channel that you never set on the pad, the app follows the level of the program instead.

### Mute and Focus

- A Mute key mutes the whole channel. It never changes a level or a trim.
- A turn up on a muted channel unmutes it. A turn up never unmutes the microphone.
- Focus turns the sessions of the other channels down from their real level. When Focus stops, each session gets exactly that level back.
- The app tracks each audio session separately. A program can have more than one session (for example, one for each output device), and each session keeps its own real level.
- Focus also turns down a session that starts while Focus is on.
- If a program closes while Focus is on, the app keeps its real level. When the program starts again, it gets that level back.
- A session that leaves a turned-down channel gets its real level back.
- While Focus turns a channel down, the app learns nothing from Windows for that channel.
- Focus never turns down the microphone channel.
- The app writes the real levels to `state.toml` at once. If the app stops while Focus is on, it restores those levels at the next start.

## Channels and programs

The app gives each audio session to one channel:

1. A program that you moved with the Move key goes to the channel that you moved it to.
2. Otherwise, the first channel in the file that matches the session gets it. A channel with `apps` matches the programs that it names. A channel with `target = "focused"` matches the foreground program and its child processes.
3. Otherwise, the first channel with `target = "other"` gets the session.
4. No channel controls a session that matches none of these rules.

Thus the order of the channels in the file is important. A `focused` channel above an `apps` channel takes the foreground program, also if the `apps` channel names it.

The settings window shows the programs of each channel with the same rule.

An audio session is what Windows shows as one slider in the volume mixer. Windows makes one session for each process that plays sound.
Thus all tabs of one browser share one slider, and the app cannot change two tabs separately.
The app skips the "System Sounds" session.

## What the app touches in Windows

| Item | API | Read or write |
|---|---|---|
| Volume and mute of each audio session (one slider in the volume mixer) | Core Audio `ISimpleAudioVolume` | Read and write |
| Peak level of each audio session (for the meters and the idle rule) | Core Audio `IAudioMeterInformation` | Read |
| Level and mute of the default communications microphone (if Windows has none, the default microphone) | Core Audio `IAudioEndpointVolume` | Read and write |
| Level and mute of the default output device (write only for a knob set to `master`) | Core Audio `IAudioEndpointVolume` | Read and write |
| Names of running programs, and their parent process | Toolhelp process snapshot (`CreateToolhelp32Snapshot`) | Read |
| The process of the foreground window (for `focused` and Move) | `GetForegroundWindow`, `GetWindowThreadProcessId` | Read |
| The pad, through its raw HID interface, and its USB release number. The app uses only an interface with the product string `KB16 Mixer`, so it does not talk to a pad with the stock firmware. | hidapi | Read and write |
| A message when a HID device is connected. Without the pad, the app searches the HID device list only after such a message, or after 10 seconds. | `RegisterDeviceNotificationW` (`WM_DEVICECHANGE` on the hidden window of the app) | Read |
| The target of an `open:` key | `ShellExecuteExW` with the verb "open" | Starts it, as a double-click in Explorer does. An existing file, given by its full path, starts in its own folder. |
| Files that you select with **Browse...** in a dialog | The standard Windows file dialog | Read the path only |
| `config.toml`, `apps.toml`, `state.toml`, `log.txt`, `selftest.txt` in the settings folder | File system | Read and write |
| The value `kb16-mixer` in `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Run` (Start with Windows) | Registry | Read and write. The app never writes it when `KB16_MIXER_DIR` is set. |
| Notepad and Explorer, when you click **Open config file** or **Open settings folder** | Process start | Starts them |

An `open:` key does not ask for administrator rights (no elevation), and passes no arguments. The app does not keep a handle to the process that it starts.

If the target program needs administrator rights, Windows shows its User Account Control (UAC) prompt. The app does not bypass that prompt. Nobody tested this on a pad yet.

If the target is a file that exists, and you give its full path, the program starts in the folder of that file. Otherwise Windows selects the folder.
If the start fails, the pad shows `Open failed`, and the app writes a line to `log.txt`.

## What the app never touches

- It does not route, capture or record audio.
- It does not create virtual audio devices.
- It does not install or load a driver.
- It does not use keyboard hooks or mouse hooks.
- It does not inject input. It never sends key presses or mouse movements.
- It does not open, read or write other processes. It gets program names from the Toolhelp snapshot only, and opens no process handle.
- It does not connect to the network.

The app uses no API from the keyboard and mouse module of Windows (`Win32_UI_Input_KeyboardAndMouse`).
That feature of the `windows` crate is not enabled. A test in `app/src/lib.rs` makes sure that it stays off.
The test `app/tests/forbidden_apis.rs` reads the source and fails on the name of a hook, an input injection, a process access or a network API.

The shortcut dialog of the settings window reads keys only from its own box, while that box has the keyboard focus.
It reads the normal window messages of that box. It uses no hook and no raw input, and it does not poll the key state.
