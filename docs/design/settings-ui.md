# Settings window: design and drawing

This document is for contributors who change the look of the settings window or its two dialogs.
It gives the colours, the controls, the motion rules, and how the app draws the window.
The code is in `app/src/ui/`, `app/src/settings/` and `app/src/dialog/`.

## Principles

- The window uses the Win32 controls of Windows.
  The app paints them, and Windows keeps their keyboard behaviour, focus, tab order and screen-reader role.
- The app uses only GDI and DWM.
  It uses no GPU, no Direct2D, no GDI+ and no other crate.
- The window costs nothing while it is closed.
  The app makes the fonts, the controls and the timer when the window opens, and deletes them when it closes.
- With nothing on the move, the window uses no timer and no CPU.

## Colours

Each colour has a role (a token).
The code names the tokens in `ui/theme.rs` (`Tok`).
The values come from the approved design.

| Token | Use | Dark | Light |
|---|---|---|---|
| `Bg` | Window, title bar, footer, dialogs | `#111214` | `#f3f4f6` |
| `Surface` | Cards | `#18191c` | `#ffffff` |
| `Control` | Rest fill of selects, buttons, fields, lists | `#212226` | `#ffffff` |
| `Hover` | Fill under the mouse | `#292a2f` | `#f2f3f5` |
| `Press` | Fill of a pressed button, or of a select with its list open | `#1c1d20` | `#e8e9ec` |
| `Seg` | Track of the tab strip and of the page switch | `#212226` | `#e9eaed` |
| `Ind` | Indicator of the selected tab or page | `#2c2e33` | `#ffffff` |
| `Line` | Card borders, footer line (decoration only) | `#26272b` | `#e2e4e8` |
| `Cborder` | Border of each control, slider rail | `#70747d` | `#80858e` |
| `Text` | Main text | `#ecedf0` | `#16171a` |
| `Text2` | Hints, row labels, other tabs | `#a4a8b0` | `#575b63` |
| `Text3` | "(no channel)", the "+" between key chips | `#6c7078` | `#a3a7ae` |
| `Accent` | Focus ring, switch on, slider, primary button, check mark | `#9fcfff` | `#1d63c8` |
| `AccentH` | Primary button under the mouse | `#b8dcff` | `#1a58b2` |
| `AccentFg` | Text on an accent fill | `#0a1622` | `#ffffff` |
| `Tint` | Selected list row, "Saved" chip | `#1d2a38` | `#e6effb` |
| `Amber` | Only problems: a refused change, a config problem | `#f0b75e` | `#9a5800` |

All text has a contrast of 4.5:1 or more against its background.
All control borders and the focus ring have 3:1 or more.
The test `every_pair_meets_the_contrast_of_the_spec` in `ui/theme.rs` calculates each pair.

The app reads the Windows app theme (dark or light) from the registry.
When Windows sends `WM_SETTINGCHANGE`, the app reads the theme again and paints the window again.
With high contrast on, the app uses only the system colours (`GetSysColor`), and no motion.

## Type and sizes

The font is the system message font (Segoe UI, or Segoe UI Variable on Windows 11).
The app makes four fonts: 13 px and 13 px semibold for controls, 12 px and 12 px semibold for hints and titles.

All sizes are pixels at 100 % on a 4 px grid.
The client area is 712 x 424.
Controls are 28 px high.
Corners have a radius of 6 px for controls, 8 px for tracks and dialogs, and 10 px for cards.
The code scales each size with the display scale (`ui::px`).
When the window moves to a display with a different scale (`WM_DPICHANGED`), the app makes all controls again at the new scale.

## Controls

| Control | Win32 control | How the app paints it |
|---|---|---|
| Button | `BUTTON`, `BS_OWNERDRAW` | `WM_DRAWITEM`. OK in a dialog is the primary button. |
| Select | `COMBOBOX`, owner-drawn | Closed box: the subclass paints it on `WM_PAINT`. Rows of the open list: `WM_DRAWITEM`. |
| Text field | `EDIT` without a border | The parent paints the frame. `WM_CTLCOLOREDIT` gives the fill. |
| Switch | `BUTTON`, `BS_AUTOCHECKBOX` | `NM_CUSTOMDRAW`. It stays a check box for screen readers. |
| Tab strip, page switch | `BUTTON`, push-like radio buttons | `NM_CUSTOMDRAW` paints the label. The parent paints the track and the indicator. |
| Slider | `msctls_trackbar32` | `NM_CUSTOMDRAW` paints the rail and the thumb. |
| List | `LISTBOX`, owner-drawn | `WM_DRAWITEM` for each row. The parent paints the frame. |
| Label, status line | `STATIC`, `SS_OWNERDRAW` | `WM_DRAWITEM`. |

A control that has the keyboard focus gets a 2 px ring in the accent colour, 2 px outside the control.
The parent paints the ring.
Windows hides the ring until the user presses a key (`UISF_HIDEFOCUS`).

The status line shows up to three lines.
Longer text ends in "...", and a tooltip shows all of it.
When the value of a select does not fit, the select also gets a tooltip.

## Motion

| What moves | Time | Curve | Starts from |
|---|---|---|---|
| Fill under the mouse | 120 ms | ease | The mouse |
| Button press, in and out | 100 ms, 140 ms | strong ease-out | The mouse |
| Indicator of the tab strip or the page switch | 220 ms | strong ease-in-out | A click |
| Knob of a switch | 160 ms | strong ease-out | A click |
| Thumb of the slider, 16 to 18 px | 100 ms | strong ease-out | The mouse |
| "Saved" chip in, then out after 1.6 s | 140 ms, 240 ms | ease-out, ease | A saved change |

Rules:

- An action from the keyboard does not move.
  It changes the state at once.
- With **Animation effects** off in Windows (`SPI_GETCLIENTAREAANIMATION`), all changes are instant.
- A new target starts from the current value, not from the start.
- Nothing moves when the window opens or when the tab changes its contents.

The code is in `ui/anim.rs`.
Each value that moves has a slot in an array of 16 slots.
One thread timer of 16 ms runs while one or more slots move.
The timer stops when the last value arrives.

## How the app draws the window

1. Each paint makes a back buffer for the area to paint (`ui/canvas.rs`).
   The back buffer is a 32-bit DIB section.
2. The parent paints its scene into the buffer: the background, the cards, the tracks, the frames and the focus ring.
   The scene of the settings window is in `settings/scene.rs`.
3. A control paints its own area with the same method.
   First it paints the scene of its parent under it, then itself.
   Thus the corners and the indicator always meet the correct colour, also while the indicator moves.
4. Rounded shapes and lines have smooth edges.
   For each radius, a corner mask holds how much of each pixel the shape covers (8 x 8 samples per pixel).
   The canvas mixes the colour of the shape with the pixel under it.
   Thus no alpha channel is necessary.
5. GDI draws the text into the same buffer.
6. One `BitBlt` copies the buffer to the window.

## Draw the window without showing it

The example `app/examples/render_settings.rs` makes the window and the dialogs hidden.
It draws them into BMP files through `WM_PRINT`.
Nothing shows on the screen.

1. Make an empty folder for a test config.
2. Set `KB16_MIXER_DIR` to that folder.
   The example does not run without it, because the window reads and writes the config.
3. Run `cargo run --example render_settings -- <output folder> all`.
4. If the example writes a manifest file and stops, change the time of the example file (for example with `touch`), and run it again.
   Windows keeps the previous "no manifest" result until the file changes.

The second argument is one of these:

| Argument | Pictures |
|---|---|
| `all` | All tabs and dialogs, dark and light, at 96, 120, 144 and 192 dpi, and the high-contrast colours at 96 dpi |
| `docs` | The three tabs, dark, at 96 dpi (the pictures in `docs/images`) |
| `worst` | The longest texts: a 3-line config problem, a long "Open" label, the focus ring |
| `measure` | No pictures. The time to the first picture, the CPU time and timers when idle, the working set, and the GDI objects after 20 cycles. |

The test `the_window_applies_and_saves_each_change_as_before` in `settings/window_tests.rs` also makes the window hidden.
It sends the messages of clicks and changes, and reads `config.toml` again after each change.
Where Windows cannot make a window, the test stops and writes why.

## Add a control

1. Add the look to `ui/draw.rs`.
   Use only the tokens of `ui/theme.rs`, and the sizes through `ui::px` and `ui::radius`.
2. Add a `Kind` in `ui/widget/mod.rs` if no type fits.
3. Paint the new `Kind` in `ui/widget/paint.rs`.
4. Make the Win32 control in `settings/controls.rs` (or in a dialog template), and give it to `widget::add`.
5. If the control has a state that moves, use `ui::anim::go`.
   Do not start a timer of your own.
6. Draw the window hidden at 96 and 144 dpi, in dark and light, and look at each picture.
