# Roadmap

This list shows the work that the project plans next. It is not a promise.
The order and the content can change at any time.

## Planned

1. **Tests on the real pad.** Test each function that has the status NOT YET VERIFIED in [testing.md](testing.md#verified-on-the-real-pad). Record the result in that table.
2. **A status message for the idle state.** The pad sends its idle state to the app. Thus an automatic test can examine the idle rule without a person at the pad.
3. **A new look for the settings window.** The window stays a plain Win32 window, with a new style.
   The owner measured a GPU user-interface framework for the window and rejected it.
   With that framework, the app file was approximately 15 MB. It used approximately 95 MB of memory. The window took approximately 10 seconds to open.
4. **Spring motion on the screen of the pad.** The meters and the popups move with spring physics, not in fixed steps.
5. **A code signature for the release.** Windows can then show the publisher of `kb16-mixer.exe`.

## Not planned

- Other macro pads, or other revisions of the KB16-01.
- macOS.
- Linux. The flash script `scripts/flash.sh` is also for Linux, but the app is for Windows only.

To suggest a change, open an issue with the feature request template.
