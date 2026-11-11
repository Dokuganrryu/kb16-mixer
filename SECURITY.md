# Security policy

## Report a vulnerability

Do not report a security problem in a public issue.

Report it privately through GitHub:

1. Go to https://github.com/Dokuganrryu/kb16-mixer/security.
2. Click **Report a vulnerability**.
3. Describe the problem, the affected version (release tag or git commit), and the steps to show it.

The project has no e-mail address for reports.

## Scope

These are examples of security problems:

- The app does something that the documentation says it never does (for example, it opens another process, injects input, or loads a driver).
- A message on the raw HID interface makes the firmware write the bootloader region.
- A message from the pad makes the app start a program that no `open:` key in your config file names.
- A file in the settings folder makes the app run code in a way other than an `open:` key that you set.
- An `open:` key starts its target with administrator rights.

## Supported versions

Only the latest release gets security fixes. No release exists yet. This is a small project. There is no promise of a response time.
