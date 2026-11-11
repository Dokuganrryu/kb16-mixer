# Configuration

This document describes the settings window and the config file of the app, `config.toml`.
The source of the default file is `DEFAULT` in `app/src/config.rs`.

## Contents

- [Where the files are](#where-the-files-are)
- [The first start](#the-first-start)
- [The settings window](#the-settings-window)
- [Edit the file by hand](#edit-the-file-by-hand)
- [General settings](#general-settings)
- [Knobs](#knobs)
- [Keys and pages](#keys-and-pages)
- [Channels](#channels)
- [Examples](#examples)
- [Error messages](#error-messages)
- [Keyboard mode](#keyboard-mode)
- [Command-line options and environment variables](#command-line-options-and-environment-variables)

## Where the files are

All files are in the settings folder, `%APPDATA%\kb16-mixer`.
The environment variable `KB16_MIXER_DIR` can replace this folder. See [Environment variables](#environment-variables).

| File | Contents | Who writes it |
|---|---|---|
| `config.toml` | All settings | You, or the settings window. The app writes the defaults on the first start. |
| `apps.toml` | Programs that you moved to a channel with the Move key | Only the app. To undo a move, use **Remove** in the settings window, or delete the line. |
| `state.toml` | Channel levels that you set on the pad, the trim of each program, and levels to restore after Focus | The app |
| `log.txt` | Problems and errors | The app |
| `selftest.txt` | Output of `kb16-mixer.exe --selftest` | The app |

To open the folder, right-click the tray icon, then click **Open settings folder**.

## The first start

On the first start (when `config.toml` does not exist), the app does these steps:

1. It writes `config.toml` with the default settings.
2. It opens the settings window on the **Pad** tab.
3. The status line tells you that the app is in the tray, near the clock (possibly behind the `^` arrow).

The app does **not** start with Windows until you select **Start with Windows** on the **Pad** tab.

## The settings window

To open the window, click the tray icon. You can also right-click the tray icon, then click **Settings...**.
The window has three tabs: **Keys**, **Channels** and **Pad**.

A change in a list, a slider, a tick box or a button applies immediately and goes into `config.toml`.
The text boxes are the channel names and the name on the screen. A text box applies when you press Enter, leave the box, or close the window.
While you type, the status line tells you if the name is not correct.
The app keeps the comments and the layout of the file.
The status line at the bottom says what changed, or why the app did not accept a change.

- The window does not accept a change that makes the file incorrect. The status line then gives the reason.
- If `config.toml` already has a mistake, the window writes nothing until you correct the file.
- If you change the file in a text editor, the window shows the new contents when you come back to it.

In the window, "app" means a program, for example "These apps" or "App in front".

### Keys tab

![The Keys tab of the settings window](images/settings-keys.png)

| Control | Use |
|---|---|
| **Page 1**, **Page 2** | Select the page that the grid below shows. The pad starts on page 1. |
| Column names | The name of the channel of each column, or "(no channel)" |
| **Row 1 (top)** to **Row 4 (bottom)** | One list for each key. Select the action of the key. |
| **Small knob, left**, **Small knob, right**, **Big knob** | Select the target of each knob. |

The key lists contain these entries:

| Entry | Action |
|---|---|
| **Mute**, **Focus**, **Talk (hold)**, **Move app**, **Switch page** | The mixer actions. See [Mixer actions](#mixer-actions). |
| **Play / pause**, **Next track**, **Previous track**, **Next (hold: prev)**, **Stop** | Media keys |
| **F13** to **F24** | Function keys |
| **Nothing** | The key does nothing. |
| **Shortcut...** | Opens the [Shortcut dialog](#shortcut-dialog). |
| **Open...** | Opens the [Open dialog](#open-dialog). |

A key can have an action from the config file that is not in the list, for example `ctrl+shift+m`. The list then shows it as an extra entry.

The knob lists contain the channel names, **Master volume**, **Microphone**, **App in front** and **Focused channel**.

The window does not accept a key change that leaves a **Switch page** key on page 1 and none on page 2.
Put the **Switch page** key on page 2 first.

### Shortcut dialog

The dialog sets a key to a combination, for example Ctrl+Shift+M.

1. Click the box.
2. Press the keys. The box shows them, for example `Ctrl+Shift+M`.
3. To add the Win key, select **Add the Win key**.
4. Click **OK**.

To set the key to nothing, click **Clear**. To keep the old setting, click **Cancel**.

- The dialog reads keys only from its own box, while that box has the keyboard focus. It uses no hook.
- Windows keeps some combinations for itself (for example Win+L and Ctrl+Alt+Del, and often the Win key). The dialog cannot see these. Type them into the config file instead, for example `"win+l"`.
- The dialog refuses keys that the pad cannot type, for example the number-pad keys and the Pause key. It then shows: "The pad cannot type that key."

### Open dialog

The dialog sets a key to `open:<target>`.

1. Type the target, or click **Browse...** to select a file.
2. Click **OK**.

The target can be a program, a folder, a file or a web address. Examples: `C:\Tools\obs64.exe`, `%USERPROFILE%\Downloads`, `https://example.com`.

### Channels tab

![The Channels tab of the settings window](images/settings-channels.png)

The tab has one column for each channel.

| Control | Use |
|---|---|
| **Name** | The name of the channel: 1 to 5 letters, digits or spaces. Press Enter or leave the box to apply it. |
| **Plays** | The kind of channel: **These apps**, **Everything else**, **Microphone** or **App in front** |
| List | The programs of the channel |
| **Add...** | Adds a program to the channel. Only for **These apps**. |
| **Remove** | Removes the selected program from the channel |

| Kind | Config value | The channel controls |
|---|---|---|
| **These apps** | `apps = [...]` | The programs in its list |
| **Everything else** | `target = "other"` | All programs that no other channel takes |
| **Microphone** | `target = "mic"` | The microphone. The tab shows no list for it. |
| **App in front** | `target = "focused"` | The foreground program |

The list shows each program where the app really puts it. A program that you moved with the Move key shows "(moved)".
A moved program wins over the lists. **Remove** on a moved program undoes the move.
The lists of **Everything else** and **App in front** show only the moved programs.

**Add...** opens a menu:

- The programs that have a volume slider now, with their current channel, for example "(now in Media)".
- **Type a program name...**: a dialog for a program name, as Task Manager shows it on the **Details** tab (for example `Spotify.exe`).

A program that you add to one channel goes out of the list of every other channel.

The window does not accept these changes:

- A name that another channel has.
- A name that the knobs read as a different target: `master`, `system`, `mic`, `focused` or `selected`. The microphone channel can have the name `Mic`.
- A second channel of the kind **Everything else**, **Microphone** or **App in front**. Only one channel can have each of these kinds.
- A channel with the name `Mic` that is not of the kind **Microphone**.

To add or remove a channel, edit the config file. See [Channels](#channels).

### Pad tab

![The Pad tab of the settings window](images/settings-pad.png)

| Control | Values | Config setting |
|---|---|---|
| **Name on the screen** | 1 to 10 letters, digits or spaces. Press Enter or leave the box to apply it. | `name` |
| **Light brightness** | A slider, 10 % to 100 % | `brightness` |
| **Knob step** | 1, 2, 3, 5 or 10 % per click | `step` |
| **Focus turns the others down to** | Almost silent (10 %), A quarter (25 %), Half (50 %), Three quarters (75 %) | `duck` |
| **Screen shift** | 0 to 6 px down | `oled_top` |
| **Screen off after** | 1, 2, 5, 10 or 30 minutes, 1 hour, or Never | `screen_off_min` |
| **Dim lights after** | 1, 2, 5, 10 or 30 minutes, 1 hour, or Never | `lights_dim_min` |
| **Start with Windows** | On or off | The registry, not the config file |
| **Open config file** | Opens `config.toml` in Notepad | |
| **Open settings folder** | Opens the settings folder in Explorer | |

The line below the buttons shows the version of the app and of the firmware, for example `KB16 mixer 0.1.0.   Pad firmware 0.1.0.`
If no pad is connected, it shows `Pad not connected.`

## Edit the file by hand

1. Right-click the tray icon, then click **Open config file**.
2. Change the file.
3. Save the file.
4. Right-click the tray icon, then click **Reload config file**.

If the file has a mistake, the app keeps the last correct settings.
The tray menu then shows "Config problem" and the cause. The screen of the pad shows `Config error`.
If the file has a mistake when the app starts, the app uses the default settings and writes the cause to `log.txt`.

The file uses the TOML format. Text values are in double quotes.
A reload sets the pad back to page 1.

## General settings

| Setting | Default | Range | Meaning |
|---|---|---|---|
| `name` | `"KB16"` | 1 to 10 letters, digits or spaces | The name on the logo screen. The pad shows it in capitals. |
| `step` | `2` | 0.5–20 | Percent of volume for one knob detent |
| `brightness` | `0.65` | 0.1–1.0 | Brightness of the key lights |
| `oled_top` | `3` | 0–6 | Pixel rows that the screen content moves down. Increase it if the top of the screen looks cut off. |
| `duck` | `0.25` | 0.0–1.0 | Focus sets the other channels to this part of their level. 0.25 = one quarter. |
| `screen_off_min` | `10` | 0–240 | Minutes without activity before the screen goes off. 0 = never. |
| `lights_dim_min` | `10` | 0–240 | Minutes without activity before the key lights dim. 0 = never. |
| `mixer_keys` | `true` | `true`, `false` | `false` = the keys type their keyboard-mode keys. The knobs and the lights stay in mixer mode. |

A number outside its range is set to the nearest limit. For example, `step = 900` becomes `20`.
A `name` outside its rule is a mistake: the app does not load the file.

Activity is a key press, a knob turn, or audible sound from a program. A readout or a notice on the screen is not activity.
Sound is audible when a program plays above approximately −40 dBFS, after its own volume and mute, for at least 1 second.
A muted output device and the microphone do not count. For the full rule, see [how-it-works.md](how-it-works.md#idle-rule).

The pad keeps the `name` in its flash memory. Thus it shows the name also at power-up, before the app runs.
A name that is too wide for the logo letters uses a smaller font.

## Knobs

The `[knobs]` table gives the target of each knob.

| Setting | Knob | Default in the file |
|---|---|---|
| `knob1` | Small knob, left | `"Chat"` |
| `knob2` | Small knob, right | `"Media"` |
| `knob3` | Big knob | `"Game"` |

Values:

| Value | Target | Knob press |
|---|---|---|
| The name of a channel | That channel (case is not important) | Selects the next program in the channel |
| `"master"` or `"system"` | The volume of the default output device | Shows the level |
| `"mic"` | The level of the microphone | Shows the level |
| `"focused"` | The foreground program, in any channel | Shows the level |
| `"selected"` | The channel that has Focus. If no channel has Focus: the `other` channel, or else the first channel that is not the microphone. | Selects the next program in that channel |

If you remove a knob line, `knob1` and `knob2` become `"master"`, and `knob3` becomes `"selected"`.

A knob press does not mute. Use a Mute key.

### One program in a channel

A knob on a channel can control the whole channel or one program in it.

1. Press the knob. It selects the first program that has an audio session in the channel (in the order of the program name).
2. Turn the knob. Only that program changes. The other programs keep their sound.
3. Press the knob again. It selects the next program.
4. After the last program, a press selects the whole channel again.

The screen shows the name and the level of the knob target after each press.
After 8 seconds without a touch on that knob, the knob goes back to the whole channel. It also goes back when the selected program stops.
If the channel has no program, the screen shows `No app in <channel>`.

A turn on the whole channel keeps the balance between its programs. For the rules, see [how-it-works.md](how-it-works.md#volume-rules).

## Keys and pages

The keys have two pages:

| Table | Page | Default use |
|---|---|---|
| `[keys]` | Page 1, the mixer page | Mixer keys and media keys |
| `[keys2]` | Page 2, the keypad page | Digits |

The pad starts on page 1. These events set it back to page 1:

- A disconnect of the pad.
- A stop of the app.
- **Pause** in the tray menu.
- A reload of the config file.


Each table has four rows, `row1` (top) to `row4` (bottom). Each row has exactly four entries, from left to right.
Each column belongs to the channel with the same position in the list of channels.

```toml
[keys]
row1 = ["page", "prev",        "play",  "next"]
row2 = ["move", "win+shift+s", "F13",   "F14"]
row3 = ["talk", "focus",       "focus", "focus"]
row4 = ["mute", "mute",        "mute",  "mute"]

[keys2]
row1 = ["page",  "7", "8", "9"]
row2 = ["0",     "4", "5", "6"]
row3 = ["enter", "1", "2", "3"]
row4 = ["mute",  "mute", "mute", "mute"]
```

If you remove a row, the app uses the default row of that page.

> **CAUTION:** If `[keys]` has a `"page"` key, `[keys2]` must also have one.
> Otherwise there is no way back to page 1. The app does not load such a file.

If neither page has a `"page"` key, the file is correct. Page 2 is then not used.

### Mixer actions

The app does these actions. They work only while the app runs.

| Value | Action |
|---|---|
| `"mute"` | Mutes or unmutes the channel of this column. The lights of a muted column are amber. |
| `"focus"` | Tap: turns all other channels down (see `duck`). Tap again to stop. Hold for 350 ms or more: Focus stops when you release the key. In the microphone column, `focus` does the same as `talk`. |
| `"talk"` | Hold: changes the microphone mute while you hold the key. If the microphone is muted, it is live while you hold. If it is live, it is muted while you hold. |
| `"move"` | Hold `move` and tap another mixer key in a column: the foreground program moves into the channel of that column. Do it again to move it back to its automatic channel. Tap `move` alone: the screen shows the channel of the foreground program. |
| `"page"` | Tap: changes to the other page. Tap again to come back. Hold for 350 ms or more: the other page stays only while you hold the key. The screen shows `MIXER` or `KEYS`. |
| `"open:<target>"` | Opens a program, a file, a folder or a web address. See [Open a target](#open-a-target). |
| `"none"` or `""` | Does nothing |

A mixer key is a key with one of these actions: `mute`, `focus`, `talk`, `move`, `page` or `open:`.
You cannot move a program into the microphone channel.

### Open a target

An `open:` key opens its target as a double-click in Explorer does. Windows selects the program that opens it.
If the target is an existing file, given by its full path, the program starts in the folder of that file.

```toml
row1 = ["page", "open:C:\\Tools\\obs64.exe", "open:%USERPROFILE%\\Downloads", "open:https://example.com"]
```

- In the TOML file, write each `\` as `\\`.
- The app replaces environment variables such as `%USERPROFILE%`.
- The target keeps its upper and lower case.
- The app does not ask for administrator rights.
- If the target does not open, the pad shows `Open failed`, and the app writes a line to `log.txt`.

### Keys that the pad types

The pad sends these keys itself, as a USB keyboard. Windows receives them as normal keyboard input, so most programs and games see them.

| Value | Key |
|---|---|
| `"play"`, `"pause"`, `"playpause"` | Media key Play/Pause |
| `"next"` | Media key Next track |
| `"prev"`, `"previous"` | Media key Previous track |
| `"stop"` | Media key Stop |
| `"next\|prev"` | Media keys: tap for the first, hold (400 ms or more) for the second. Both must be media keys. |
| `"F1"` to `"F24"` | Function keys. Programs do not use F13–F24 by default, so you can bind them in a program (for example Discord, OBS or a game). |
| `"a"` to `"z"`, `"0"` to `"9"` | Letters and digits (the digits of the main keyboard, not of the number pad) |
| `"enter"` (`"return"`), `"esc"` (`"escape"`), `"backspace"`, `"tab"`, `"space"` | |
| `"up"`, `"down"`, `"left"`, `"right"` | Arrow keys |
| `"home"`, `"end"`, `"pageup"` (`"pgup"`), `"pagedown"` (`"pgdn"`), `"insert"`, `"delete"` (`"del"`) | |
| `"printscreen"` | |

In a table cell, `\|` is the character `|`. In the file, write `"next|prev"`.

To add modifiers, join them to one key with `+`, for example `"ctrl+shift+m"` or `"win+shift+s"`.

| Modifier | Values |
|---|---|
| Ctrl | `ctrl`, `control` |
| Shift | `shift` |
| Alt | `alt` |
| Win | `win`, `gui`, `cmd` |

Rules:

- Case is not important: `"F13"` and `"f13"` are the same.
- One entry has one key and zero or more modifiers. `"a+b"` is not correct.
- `"pause"` alone is the media key Play/Pause. With a modifier (for example `"ctrl+pause"`), it is the Pause key of the keyboard.

## Channels

Each `[[channel]]` section is one channel. You can have 1 to 4 channels.
The first channel belongs to the left key column, the second to the next column, and so on.

| Setting | Required | Meaning |
|---|---|---|
| `name` | yes | Name of the channel: 1 to 5 characters. The mixer screen shows it in upper case. The settings window accepts only letters, digits and spaces. |
| `color` | no | Tint of the key lights, as `"#rrggbb"`. Without it, the app uses a soft white tint. |
| `apps` | see below | A list of program file names, for example `["Discord.exe", "Teams.exe"]`. Case is not important. |
| `target` | see below | `"apps"`, `"mic"`, `"other"` or `"focused"` |

Each channel needs `apps` (not empty) or `target`.

| `target` | The channel controls |
|---|---|
| (none, with `apps`) | The programs in `apps` |
| `"apps"` | The programs in `apps`. With `target = "apps"`, the list can be empty. |
| `"mic"` | The microphone. The meter shows your voice. |
| `"other"` | All programs that no other channel names. Games usually land here. |
| `"focused"` | The foreground program and its child processes |

If two channels name the same program, the first channel gets it.
A program that you moved with the Move key stays in that channel, whatever `apps` says.

A level that you set on the pad stays with the channel. A program that starts later in that channel gets that level, multiplied by its trim.
The app keeps the levels and the trims in `state.toml` between runs.
If you rename a channel and keep its position, its saved level and its moved programs stay with it.

## Examples

### The default channels

```toml
[[channel]]
name = "Mic"
color = "#dce6ff"
target = "mic"

[[channel]]
name = "Chat"
color = "#c8d2ff"
apps = ["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe", "Teams.exe", "ms-teams.exe", "Zoom.exe", "TeamSpeak.exe", "ts3client_win64.exe", "Mumble.exe"]

[[channel]]
name = "Game"
color = "#e6ecff"
target = "other"

[[channel]]
name = "Media"
color = "#d2f0ff"
apps = ["Spotify.exe", "AppleMusic.exe", "TIDAL.exe", "foobar2000.exe", "MusicBee.exe", "vlc.exe", "mpv.exe",
        "chrome.exe", "firefox.exe", "msedge.exe", "brave.exe", "opera.exe", "opera_gx.exe", "vivaldi.exe", "arc.exe"]
```

### Three channels, a master knob and one page

```toml
name = "STUDIO"
step = 5

[knobs]
knob1 = "master"
knob2 = "Music"
knob3 = "Game"

[keys]
row1 = ["F13",   "F14",   "F15",   "open:C:\\Tools\\obs64.exe"]
row2 = ["F16",   "F17",   "F18",   "none"]
row3 = ["focus", "focus", "focus", "none"]
row4 = ["mute",  "mute",  "mute",  "none"]

[[channel]]
name = "Voice"
apps = ["Discord.exe"]

[[channel]]
name = "Music"
apps = ["Spotify.exe"]

[[channel]]
name = "Game"
target = "other"
```

This file has no `"page"` key on page 1. Thus page 2 is not used, and `[keys2]` can stay at its defaults.
The fourth column has no channel. Its `mute` and `focus` keys would do nothing, so they are set to `"none"`.

## Error messages

| Message | Cause |
|---|---|
| `no [[channel]] sections` | The file has no channel. |
| `5 channels, the pad has 4 columns` | The file has more than 4 channels. |
| `channel X: color "..." is not #rrggbb` | The color is not 6 hexadecimal digits. |
| `channel X: needs apps = [...] or a target` | The channel has no `apps` and no `target`. |
| `channel X: unknown target "..."` | `target` is not `apps`, `mic`, `other` or `focused`. |
| `knob1 = "...": no channel with that name` | The knob names a channel that does not exist. |
| `name = "...": 1 to 10 letters, digits or spaces` | The logo name is empty, too long, or has other characters. |
| `channel name "...": 1 to 5 characters, what the pad's screen shows` | A channel name is empty or longer than 5 characters. |
| `[keys] row1: 3 entries, a row has 4 keys` | A row does not have exactly 4 entries. The same for `[keys2]`. |
| `[keys2] has no "page" key, so there would be no way back to [keys]` | Page 1 has a `page` key, and page 2 has none. |
| `key "...": not an action, media key or key name` | The key name is not known. |
| `key "...": one key plus modifiers, please` | The entry has more than one key. |
| `key "...": tap\|hold works with media keys (play, next, prev, stop)` | A tap/hold entry has a value that is not a media key. |
| `key "...": open: needs a program, file, folder or web address after it` | An `open:` entry has no target. |

A TOML syntax error shows the line number.

## Keyboard mode

When the app does not run (or after **Pause** in the tray menu), the pad uses this fixed layout:

```text
          +-----------+-----------+-----------+-----------+
  row 1   |     1     |     2     |     3     |     4     |      knob 1           knob 2
          +-----------+-----------+-----------+-----------+      turn: Next /     turn: Page Up /
  row 2   |     5     |     6     |     7     |     8     |            Previous         Page Down
          +-----------+-----------+-----------+-----------+      press: Play/     press: Home
  row 3   |     9     |     0     |    Up     |   Enter   |             Pause
          +-----------+-----------+-----------+-----------+
  row 4   | Backspace |   Left    |   Down    |   Right   |      knob 3: turn: Volume up / down
          +-----------+-----------+-----------+-----------+              press: Mute
```

A clockwise turn sends the first key of each pair (Next, Page Up, Volume up).
The key lights show a slow white light. The screen shows the logo name.

You cannot change this layout in the config file.

## Command-line options and environment variables

### Command-line options

| Option | Action |
|---|---|
| `--settings` | Starts the app with the settings window open |
| `--settings=keys`, `--settings=channels`, `--settings=pad` | Starts the app with the settings window open on that tab |
| `--no-pad` | The app never looks for the pad. The volume control still runs. Use it to try the settings window while another copy of the app has the pad. |
| `--bootloader` | Tells the pad to restart into its bootloader, then stops. Exit code 0 = the app sent the command. Exit code 1 = the app found no pad, or the write failed. |
| `--selftest` | Writes the audio sessions, their channels and their levels to `selftest.txt`, then stops |
| `--help` (also `-h` or `/?`) | Shows a window with these options, then stops |

Only one copy of the app runs for each settings folder. A second start for the same folder stops at once.

### Environment variables

| Variable | Effect |
|---|---|
| `KB16_MIXER_DIR=<folder>` | Replaces the settings folder, for example for a portable copy or a test. With it set, the app never writes **Start with Windows**, and the menu item is not available. |
| `KB16_MIXER_DPI=<number>` | Only for developers: lays the settings window out for a different display scale. |
