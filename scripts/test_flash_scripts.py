"""Tests for scripts/flash.sh and scripts/flash.ps1 with a fake dfu-util and a stub app.

Run: python -m unittest scripts/test_flash_scripts.py

No test uses the pad, the real dfu-util or the real app. Each test runs a copy of the script in
a temporary folder, so the script cannot find the exe that a source build makes. The flash.ps1
tests need Windows PowerShell and the .NET Framework C# compiler; on other systems they skip.
"""

import os
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
NEXT = "NEXT: unplug the pad and plug it in again."


def image() -> bytes:
    return struct.pack("<II", 0x20005000, 0x08002131) + bytes(4088)


def write_tool(path: Path, body: str) -> None:
    path.write_text("#!/bin/sh\n" + body, encoding="ascii", newline="\n")
    path.chmod(0o755)


@unittest.skipUnless(shutil.which("sh"), "needs sh")
class FlashSh(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        (root / "scripts").mkdir()
        shutil.copy(SCRIPTS / "flash.sh", root / "scripts" / "flash.sh")
        (root / "firmware" / "target").mkdir(parents=True)
        (root / "firmware" / "target" / "kb16-mixer-firmware.bin").write_bytes(image())
        self.root = root
        self.bin = root / "bin"
        self.bin.mkdir()
        # On Windows the script stops the running app with taskkill and lists it with
        # powershell.exe. These fakes take their place, so no real program is stopped.
        write_tool(self.bin / "taskkill", "exit 0\n")
        write_tool(self.bin / "powershell.exe", "exit 0\n")
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"])
        self.env.pop("DFU_UTIL", None)

    def run_script(self, dfu_body: str) -> subprocess.CompletedProcess:
        write_tool(self.bin / "dfu-util", dfu_body)
        found = subprocess.run(["sh", "-c", "command -v taskkill; command -v dfu-util"], env=self.env,
                               capture_output=True, text=True).stdout.split()
        self.assertEqual(len(found), 2)
        for tool in found:
            self.assertEqual(Path(tool).parent.name, "bin", "the fake tools are not first on the PATH")
        return subprocess.run(["sh", str(self.root / "scripts" / "flash.sh")], env=self.env,
                              capture_output=True, text=True, timeout=60)

    def test_next_step_after_download_done(self):
        # The tested bootloader does not answer the reset request: exit 74 after a good download.
        result = self.run_script('echo "Download done."\nexit 74\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(NEXT, result.stdout)
        self.assertIn("dfu-util exit 74", result.stdout)

    def test_no_next_step_and_exit_1_when_the_download_fails(self):
        result = self.run_script('echo "dfu-util: Cannot open DFU device 1eaf:0003" >&2\nexit 74\n')
        self.assertEqual(result.returncode, 1)
        self.assertNotIn(NEXT, result.stdout)
        self.assertIn('FAILED: dfu-util did not show "Download done"', result.stderr)

    def test_exit_0_without_download_done_is_a_failure(self):
        result = self.run_script("exit 0\n")
        self.assertEqual(result.returncode, 1)
        self.assertNotIn(NEXT, result.stdout)


# A program for the Windows GUI subsystem, like kb16-mixer.exe: PowerShell does not wait for it
# when it is called with "&". It waits, records its arguments and exits with the code in STUB_EXIT.
STUB = r"""
class Stub {
    static int Main(string[] args) {
        System.Threading.Thread.Sleep(1500);
        System.IO.File.WriteAllText(System.Environment.GetEnvironmentVariable("STUB_MARKER"),
            string.Join(" ", args));
        return int.Parse(System.Environment.GetEnvironmentVariable("STUB_EXIT"));
    }
}
"""

# Lists the bootloader only after the stub app ran, as the pad does after the request.
FAKE_DFU = (
    "@echo off\r\n"
    'if exist "%STUB_MARKER%" echo Found DFU: [1eaf:0003] ver=0201, devnum=5, cfg=1, intf=0, '
    'path="1-1", alt=2, name="STM32duino bootloader v1.0  Upload to Flash 0x8002000", serial="LLM 003"\r\n'
    "exit /b 0\r\n"
)

POWERSHELL = shutil.which("powershell.exe") or shutil.which("powershell")
CSC = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Microsoft.NET" / "Framework64" / "v4.0.30319" / "csc.exe"


@unittest.skipUnless(sys.platform == "win32" and POWERSHELL and CSC.is_file(),
                     "needs Windows PowerShell and the .NET Framework C# compiler")
class FlashPs1BootloaderRequest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        source = Path(cls.tmp.name) / "stub.cs"
        source.write_text(STUB, encoding="ascii")
        cls.stub = Path(cls.tmp.name) / "stub.exe"
        subprocess.run([str(CSC), "/nologo", "/target:winexe", f"/out:{cls.stub}", str(source)],
                       check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_script(self, app_exit: int) -> tuple[str, Path]:
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        folder = Path(tmp.name)
        text = (SCRIPTS / "flash.ps1").read_text(encoding="utf-8")
        # The script stops every running kb16-mixer process. In the test it must not stop the
        # app of the person who runs the test, so the copy looks for a name that does not exist.
        self.assertEqual(text.count("Get-Process -Name kb16-mixer "), 2)
        text = text.replace("Get-Process -Name kb16-mixer ", "Get-Process -Name kb16-mixer-test-none ")
        (folder / "flash.ps1").write_text(text, encoding="utf-8")
        (folder / "firmware").mkdir()
        (folder / "firmware" / "kb16-mixer-firmware.bin").write_bytes(image())
        shutil.copy(self.stub, folder / "kb16-mixer.exe")
        (folder / "dfu-util.cmd").write_text(FAKE_DFU, encoding="ascii", newline="")
        marker = folder / "marker.txt"
        env = dict(os.environ, STUB_MARKER=str(marker), STUB_EXIT=str(app_exit))
        # "y" asks for the bootloader request; "no" refuses the write, so dfu-util never writes.
        result = subprocess.run(
            [POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(folder / "flash.ps1"), "-DfuUtil", str(folder / "dfu-util.cmd")],
            input="y\nno\n", env=env, capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("REFUSED: You did not type YES.", result.stdout)
        return result.stdout, marker

    def test_app_exit_0_means_the_pad_was_told(self):
        out, marker = self.run_script(0)
        self.assertEqual(marker.read_text(), "--bootloader")
        self.assertIn("The pad was told to enter its bootloader.", out)
        self.assertNotIn("The pad did not answer.", out)

    def test_app_exit_1_means_the_pad_did_not_answer(self):
        out, marker = self.run_script(1)
        self.assertEqual(marker.read_text(), "--bootloader")
        self.assertIn("The pad did not answer.", out)
        self.assertNotIn("The pad was told to enter its bootloader.", out)
        self.assertIn("Put the pad into its bootloader now:", out)


if __name__ == "__main__":
    unittest.main()
