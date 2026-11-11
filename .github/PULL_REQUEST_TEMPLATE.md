## What this pull request changes

<!-- Describe the change and the reason for it. -->

## Tested on hardware?

<!-- Answer each line. If you did not test on hardware, write "No". -->

- Tested on hardware (yes / no):
- Pad model and revision:
- Firmware and app version (first line of the tray menu):
- Windows version:
- How you tested (what you did on the pad):
- What you did not test:

## Checklist

- [ ] `cargo fmt --check`, `cargo clippy` and `cargo test` pass for each crate that I changed.
- [ ] `sh scripts/build-firmware.sh` accepts the image (if I changed the firmware or the protocol).
- [ ] Protocol changes are only in `protocol/`, with tests, and `docs/protocol.md` is updated.
- [ ] The app still uses no hooks, no input injection, no driver, no process access and no network.
- [ ] I ran `python scripts/third-party-notices.py` (if I changed a dependency).
- [ ] I updated the documentation and `CHANGELOG.md`.
