"""Unit tests for scripts/package-release.py. Run: python -m unittest scripts/test_package_release.py"""

import contextlib
import hashlib
import importlib.util
import io
import struct
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "package_release", Path(__file__).resolve().parent / "package-release.py"
)
pr = importlib.util.module_from_spec(_SPEC)
sys.modules["package_release"] = pr
_SPEC.loader.exec_module(pr)


def image(sp=0x20005000, reset=0x08002131, size=4096) -> bytes:
    return struct.pack("<II", sp, reset) + bytes(size - 8)


EXE = b"MZ" + bytes(1022)


class Workspace:
    """A temporary folder with a fake repository root, an exe and a firmware image."""

    def __init__(self, firmware=None, exe=EXE):
        self.tmp = tempfile.TemporaryDirectory()
        base = Path(self.tmp.name)
        self.root = base / "repo"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / "scripts" / "flash.ps1").write_bytes(b"# flash\n")
        (self.root / "scripts" / "release-readme.txt").write_bytes(b"readme\n")
        (self.root / "LICENSE").write_bytes(b"MIT\n")
        (self.root / "THIRD_PARTY_NOTICES.md").write_bytes(b"# notices\n")
        self.exe = base / "kb16-mixer.exe"
        self.exe.write_bytes(exe)
        self.firmware = base / "kb16-mixer-firmware.bin"
        self.firmware.write_bytes(image() if firmware is None else firmware)
        self.base = base

    def run(self, out="dist", version="0.1.0", epoch=pr.DEFAULT_EPOCH):
        return pr.package(version, self.exe, self.firmware, self.base / out, epoch, root=self.root)

    def close(self):
        self.tmp.cleanup()


class Layout(unittest.TestCase):
    def setUp(self):
        self.ws = Workspace()
        self.addCleanup(self.ws.close)

    def test_zip_has_exactly_the_release_layout(self):
        path = self.ws.run()
        self.assertEqual(path.name, "kb16-mixer-0.1.0-windows-x64.zip")
        with zipfile.ZipFile(path) as z:
            self.assertEqual(z.namelist(), sorted(pr.LAYOUT))
            self.assertEqual(z.read("kb16-mixer.exe"), EXE)
            self.assertEqual(z.read("firmware/kb16-mixer-firmware.bin"), image())
            self.assertEqual(z.read("README.txt"), b"readme\n")
            for info in z.infolist():
                self.assertEqual(info.date_time, (2020, 1, 1, 0, 0, 0))
                self.assertEqual(info.external_attr >> 16, 0o100644)

    def test_sha256sums_lists_zip_firmware_and_exe(self):
        path = self.ws.run()
        out = path.parent
        lines = (out / "SHA256SUMS").read_text(encoding="ascii").splitlines()
        names = [line.split("  ")[1] for line in lines]
        self.assertEqual(names, [path.name, "kb16-mixer-firmware.bin", "kb16-mixer.exe"])
        for line in lines:
            digest, name = line.split("  ")
            self.assertEqual(digest, hashlib.sha256((out / name).read_bytes()).hexdigest())

    def test_two_runs_give_the_same_bytes(self):
        first = self.ws.run(out="a").read_bytes()
        second = self.ws.run(out="b").read_bytes()
        self.assertEqual(first, second)

    def test_source_date_epoch_sets_the_timestamps(self):
        path = self.ws.run(epoch=1700000000)  # 2023-11-14 22:13:20 UTC
        with zipfile.ZipFile(path) as z:
            self.assertEqual(z.infolist()[0].date_time, (2023, 11, 14, 22, 13, 20))

    def test_prerelease_version_is_accepted(self):
        self.assertEqual(self.ws.run(version="0.2.0-rc.1").name, "kb16-mixer-0.2.0-rc.1-windows-x64.zip")


class Refusals(unittest.TestCase):
    def refuses(self, message, firmware=None, exe=EXE, version="0.1.0"):
        ws = Workspace(firmware=firmware, exe=exe)
        self.addCleanup(ws.close)
        with self.assertRaisesRegex(pr.Refused, message):
            ws.run(version=version)
        self.assertFalse((ws.base / "dist").exists(), "nothing is written for a refused input")

    def test_image_too_large(self):
        self.refuses("limit", firmware=image(size=pr.MAX_IMAGE + 4))

    def test_image_at_the_limit_is_accepted(self):
        pr.check_firmware(image(size=pr.MAX_IMAGE))

    def test_image_too_short(self):
        self.refuses("no vector table", firmware=b"\x00\x50\x00\x20")

    def test_stack_pointer_outside_ram(self):
        self.refuses("stack pointer", firmware=image(sp=0x20005004))
        self.refuses("stack pointer", firmware=image(sp=0x20000000))
        self.refuses("stack pointer", firmware=image(sp=0x08004000))

    def test_even_reset_vector(self):
        self.refuses("Thumb", firmware=image(reset=0x08002130))

    def test_reset_vector_in_the_bootloader(self):
        self.refuses("outside", firmware=image(reset=0x08000101))

    def test_reset_vector_after_flash(self):
        self.refuses("outside", firmware=image(reset=0x08020001))

    def test_exe_without_mz(self):
        self.refuses("MZ", exe=b"\x7fELF" + bytes(100))

    def test_bad_version(self):
        self.refuses("version", version="v0.1.0")


class CommandLine(unittest.TestCase):
    def test_refusal_exits_1_and_names_the_reason(self):
        ws = Workspace(firmware=image(sp=0))
        self.addCleanup(ws.close)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = pr.main(["--version", "0.1.0", "--exe", str(ws.exe), "--firmware", str(ws.firmware),
                            "--out", str(ws.base / "dist")])
        self.assertEqual(code, 1)
        self.assertIn("REFUSED: stack pointer", err.getvalue())


if __name__ == "__main__":
    unittest.main()
