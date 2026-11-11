# Security policy

## Report a vulnerability

Do not report a security problem in a public issue.

Report it privately through GitHub:

1. Go to https://github.com/Dokuganrryu/kb16-mixer/security.
2. Click **Report a vulnerability**.
3. Describe the problem, the affected version (release tag or git commit), and the steps that show it.

The project has no e-mail address for reports.

## Threat model

These facts limit what an attacker can do with KB16 Mixer:

- **The app does not use the network.** It has no network code and no dependency for network access. It does not check for updates.
- **The firmware accepts only the messages in [docs/protocol.md](docs/protocol.md).** It ignores a report that does not start with `0xD1`. For an unknown message ID, it only notes that the app is present. It accepts a logo name only if the name obeys the rules in that document.
- **The firmware writes flash only in two pages:** the name page and the crash log. It never writes the bootloader region.
- **A local program has the same access as the app.** Any program that runs on your PC can open the raw HID interface of the pad and send any message. Such a program can already send keys and change volumes in Windows without the pad. Thus the pad gives it no new power over the PC.
- **The bootloader message is not protected.** Its marker `BOOT` never changes, and anyone can read it in the documentation. It is not a password and not a secret. It only prevents a restart from a random or damaged message.
- **The bootloader accepts any image.** In bootloader mode, a program on the PC can write a different firmware to the pad over DFU. The bootloader does not check a signature. The image check in `scripts/build-firmware.sh` only protects the images that this project builds.

Thus the project does not protect the pad from a program that already runs on your PC.

## Scope

These are examples of security problems:

- The app does something that the documentation says it never does. For example, it opens another process, injects input, loads a driver or connects to the network.
- A message on the raw HID interface makes the firmware write the bootloader region, or write flash outside the name page and the crash log.
- A message from the pad makes the app start a program that no `open:` key in your config file names.
- A file in the settings folder makes the app run code in a way other than an `open:` key that you set.
- An `open:` key starts its target with administrator rights without the User Account Control (UAC) prompt of Windows.
- The output of `kb16-mixer.exe --check` contains personal data, for example a user name or a serial number.

## Supported versions

Only the latest release gets security fixes. This is a small project. There is no promise of a response time.
