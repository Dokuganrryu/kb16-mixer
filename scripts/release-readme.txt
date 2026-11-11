KB16 Mixer
==========

KB16 Mixer is open-source firmware and a Windows tray app for the
DOIO KB16-01 rev2 macro pad. The pad controls the volume of your programs.
It is also a normal keypad.

SAFETY
------
You flash firmware at your own risk. A flash can void the warranty of the pad.
Use this firmware only on a DOIO KB16-01 rev2. Other pads are not supported.
Make a backup of the stock firmware before you flash.

Contents of this zip
--------------------
kb16-mixer.exe                     The tray app
firmware\kb16-mixer-firmware.bin   The firmware image for the pad
flash.ps1                          The script that writes the firmware to the pad
LICENSE                            The licence of this project (MIT)
THIRD_PARTY_NOTICES.md             The licences of the third-party code
README.txt                         This file

Quick steps
-----------
1. Compare the SHA-256 hash of the zip with the file SHA256SUMS.
2. Put the pad into the bootloader. The full guide tells you how.
3. Install the bootloader driver one time only.
4. Run flash.ps1 with -Backup to keep the stock firmware and to write the new firmware.
5. Unplug the pad and plug it in again.
6. Start kb16-mixer.exe. Then, in a command prompt in this folder, run
   "kb16-mixer.exe --check | more".

Full guide
----------
Read the full guide before you flash:
https://github.com/Dokuganrryu/kb16-mixer/blob/main/docs/install.md
