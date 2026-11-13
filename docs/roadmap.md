# Roadmap

This list shows the work that the project plans next. It is not a promise.
The order and the content can change.

## Planned

1. **Tests on the pad.** Do a test of each function that has the status NOT YET VERIFIED in [testing.md](testing.md#verified-on-the-pad). Record the result in that table.
2. **A status message for the idle state.** The pad sends its idle state to the app. Thus an automatic test can examine the idle rule without a person at the pad.
3. **A code signature for the release.** Windows can then show the publisher of `kb16-mixer.exe`.

## Done

- **A release with the new firmware.** Version 0.2.0 has the firmware that sends knob releases. Its version lets `--check` tell it from the internal version 0.1.0.
- **Motion on the pad, and knob gestures.** The screen and the key lights move smoothly along set curves, not with spring physics. A knob has a tap, a hold and a double tap. The app follows the Windows default output device. See [how-it-works.md](how-it-works.md#screen-motion).
  Nobody did a test of these functions on a pad. For the knob hold, the pad must have firmware 0.2.0 or later.
- **A new look for the settings window.** The window stays a plain Win32 window, with a new style: dark and light themes, smooth rounded controls and short animations. See [design/settings-ui.md](design/settings-ui.md).
  The owner measured a GPU user-interface framework for the window and rejected it.
  With that framework, the app file was approximately 15 MB. It used approximately 95 MB of memory. The window opened after approximately 10 seconds.
  At this time, nobody examined these parts visually: the open dropdown lists in the dark theme, the **Add** menu, and Windows 10. The **Add** menu stays the light menu of Windows.

## Not planned

- Other macro pads, or other revisions of the KB16-01.
- macOS.
- Linux. The flash script `scripts/flash.sh` is also for Linux, but the app is for Windows only.

To suggest a change, open an issue with the feature request template.
