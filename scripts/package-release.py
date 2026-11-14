"""Builds the release zip of KB16 Mixer and its SHA256SUMS file.

    python scripts/package-release.py --version 0.3.0 \
        --exe app/target/release/kb16-mixer.exe \
        --firmware firmware/target/kb16-mixer-firmware.bin --out dist

The zip kb16-mixer-<version>-windows-x64.zip holds exactly:
    kb16-mixer.exe, firmware/kb16-mixer-firmware.bin, flash.ps1, LICENSE,
    THIRD_PARTY_NOTICES.md, README.txt
SHA256SUMS lists the zip and the loose copies of the exe and the firmware image, which the
script also writes to --out. The release uploads only the zip and SHA256SUMS, so a user checks
with `sha256sum -c --ignore-missing SHA256SUMS`: in the download folder for the zip, in the
unzipped folder for the exe, and in its firmware folder for the image. docs/install.md and
README.txt give these steps and the Get-FileHash steps.

The script refuses a firmware image that flash.ps1 would refuse, and an exe without the MZ
header, so that a broken build never becomes a release. The zip is reproducible: the same
inputs and the same Python build give the same bytes. Entries are sorted, every timestamp is
SOURCE_DATE_EPOCH (default 2020-01-01 00:00:00 UTC), and every entry has mode 0644.
Standard library only, so that it runs on every CI runner without a setup step.
"""
import argparse
import hashlib
import os
import re
import shutil
import struct
import sys
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The same limits as scripts/build-firmware.sh and scripts/flash.ps1: above 0x1D400 bytes the
# image overlaps the name page at 0x0801F400 and the crash log after it.
MAX_IMAGE = 0x1D400
RAM = (0x20000000, 0x20005000)
APP_REGION = (0x08002000, 0x08020000)
DEFAULT_EPOCH = 1577836800  # 2020-01-01 00:00:00 UTC
VERSION = re.compile(r"^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$")

EXE_NAME = "kb16-mixer.exe"
FIRMWARE_NAME = "kb16-mixer-firmware.bin"
LAYOUT = (
    EXE_NAME,
    "firmware/" + FIRMWARE_NAME,
    "flash.ps1",
    "LICENSE",
    "THIRD_PARTY_NOTICES.md",
    "README.txt",
)


class Refused(Exception):
    """An input that must not go into a release."""


def check_firmware(data: bytes) -> None:
    if len(data) < 8:
        raise Refused(f"firmware image has {len(data)} bytes; it has no vector table")
    if len(data) > MAX_IMAGE:
        raise Refused(f"firmware image has {len(data)} bytes; the limit is {MAX_IMAGE:#x}")
    sp, reset = struct.unpack_from("<II", data)
    if not RAM[0] < sp <= RAM[1]:
        raise Refused(f"stack pointer {sp:#010x} is not in RAM")
    if not reset & 1:
        raise Refused(f"reset vector {reset:#010x} is not a Thumb address (bit 0 is clear)")
    if not APP_REGION[0] <= reset < APP_REGION[1]:
        raise Refused(f"reset vector {reset:#010x} is outside the firmware region")


def check_exe(data: bytes) -> None:
    if data[:2] != b"MZ":
        raise Refused("the exe does not start with MZ; it is not a Windows program")


def zip_time(epoch: int) -> tuple:
    # The zip format cannot store a time before 1980.
    return time.gmtime(max(epoch, 315532800))[:6]


def write_zip(path: Path, files: dict[str, bytes], epoch: int) -> None:
    with zipfile.ZipFile(path, "w") as z:
        for name in sorted(files):
            info = zipfile.ZipInfo(name, zip_time(epoch))
            info.compress_type = zipfile.ZIP_DEFLATED
            # create_system defaults to the host OS; fix it so Windows and Linux agree.
            info.create_system = 3
            info.external_attr = 0o100644 << 16
            z.writestr(info, files[name], compresslevel=9)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def package(version: str, exe: Path, firmware: Path, out: Path, epoch: int, root: Path = ROOT) -> Path:
    if not VERSION.match(version):
        raise Refused(f"version {version!r} is not <major>.<minor>.<patch>[-suffix]")
    exe_bytes = exe.read_bytes()
    firmware_bytes = firmware.read_bytes()
    check_exe(exe_bytes)
    check_firmware(firmware_bytes)
    files = {
        EXE_NAME: exe_bytes,
        "firmware/" + FIRMWARE_NAME: firmware_bytes,
        "flash.ps1": (root / "scripts" / "flash.ps1").read_bytes(),
        "LICENSE": (root / "LICENSE").read_bytes(),
        "THIRD_PARTY_NOTICES.md": (root / "THIRD_PARTY_NOTICES.md").read_bytes(),
        "README.txt": (root / "scripts" / "release-readme.txt").read_bytes(),
    }

    out.mkdir(parents=True, exist_ok=True)
    zip_path = out / f"kb16-mixer-{version}-windows-x64.zip"
    write_zip(zip_path, files, epoch)
    loose_exe = out / EXE_NAME
    loose_firmware = out / FIRMWARE_NAME
    if exe.resolve() != loose_exe.resolve():
        shutil.copyfile(exe, loose_exe)
    if firmware.resolve() != loose_firmware.resolve():
        shutil.copyfile(firmware, loose_firmware)
    sums = "".join(f"{sha256(p)}  {p.name}\n" for p in (zip_path, loose_firmware, loose_exe))
    (out / "SHA256SUMS").write_bytes(sums.encode("ascii"))
    return zip_path


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--version", required=True, help="release version, for example 0.3.0")
    p.add_argument("--exe", required=True, type=Path, help="the built kb16-mixer.exe")
    p.add_argument("--firmware", required=True, type=Path, help="the built kb16-mixer-firmware.bin")
    p.add_argument("--out", required=True, type=Path, help="output folder")
    a = p.parse_args(argv)
    epoch_text = os.environ.get("SOURCE_DATE_EPOCH", "")
    if epoch_text and not epoch_text.isdigit():
        print(f"package-release: SOURCE_DATE_EPOCH {epoch_text!r} is not a number", file=sys.stderr)
        return 1
    epoch = int(epoch_text) if epoch_text else DEFAULT_EPOCH
    try:
        zip_path = package(a.version, a.exe, a.firmware, a.out, epoch)
    except (Refused, OSError) as e:
        print(f"package-release: REFUSED: {e}", file=sys.stderr)
        return 1
    print(f"wrote {zip_path}")
    print((a.out / "SHA256SUMS").read_text(encoding="ascii"), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
