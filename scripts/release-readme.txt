KB16 Mixer
==========

KB16 Mixer is open-source firmware and a Windows tray app for the
DOIO KB16-01 rev2. The pad controls the volume of your programs.
It is also a usual keypad.

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
1. Compare the SHA-256 value of the zip with the line for the zip in SHA256SUMS.
   In PowerShell: Get-FileHash .\<name of the zip> -Algorithm SHA256
   In Git Bash, in the download folder: sha256sum -c --ignore-missing SHA256SUMS
   SHA256SUMS also has lines for kb16-mixer.exe and kb16-mixer-firmware.bin.
   These two files are in this zip, not separate downloads. Examine them in
   this folder with Get-FileHash. flash.ps1 shows the SHA-256 value of the
   image. This value must be the same as the line for kb16-mixer-firmware.bin.
2. Put the pad into the bootloader. The full guide tells you how.
3. Install the bootloader driver one time only.
4. Make a backup of the stock firmware. This command writes nothing to the
   pad. In PowerShell in this folder, run:
   powershell -ExecutionPolicy Bypass -File .\flash.ps1 -Backup .\stock-backup.bin -DryRun
5. Make sure that the output shows "backup: 122880 bytes" and
   "DRY RUN complete". Write down the SHA-256 value of the backup.
   Copy stock-backup.bin to a second place.
6. Flash the firmware. Type YES when the script asks:
   powershell -ExecutionPolicy Bypass -File .\flash.ps1
7. Unplug the pad.
8. Plug the pad in again.
9. Start kb16-mixer.exe.
10. In a command prompt in this folder, run "kb16-mixer.exe --check | more".

Full guide
----------
Read the full guide before you flash:
https://github.com/Dokuganrryu/kb16-mixer/blob/main/docs/install.md
