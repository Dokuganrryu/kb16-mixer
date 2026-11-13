"""Tests for scripts/flash.sh and scripts/flash.ps1 with a fake dfu-util and a stub app.

Run: python -m unittest scripts/test_flash_scripts.py

No test uses the pad, the real dfu-util or the real app. Each test runs a copy of the script in
a temporary folder, so the script cannot find the exe that a source build makes. The flash.ps1
tests need Windows PowerShell and the .NET Framework C# compiler; on other systems they skip.
Set KB16_REQUIRE_PS1_TESTS=1 (CI does) to make that a failure instead of a skip.

There is a test for each refusal of each flasher. Each refusal test also checks that dfu-util
got no write command.

The flashers stop the running app. Each test gives them a made-up process name in
KB16_APP_PROCESS, so that no test can stop the app of the person who runs the tests. Every start
of a flasher goes through launch(), which refuses an environment without such a name.
"""

import ast
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import uuid
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent
NEXT = "NEXT: unplug the pad and plug it in again."


MAX_IMAGE = 0x1D400  # the image limit of both flashers: the name page starts at 0x0801F400
FULL_AREA = 122880  # the size of a backup of alternate setting 2 on the tested pad


def image(size: int = 4096, sp: int = 0x20005000, reset: int = 0x08002131) -> bytes:
    vectors = struct.pack("<II", sp, reset)
    return vectors[:size] + bytes(max(size - len(vectors), 0))


APP_PROCESS = "KB16_APP_PROCESS"
TEST_NAME = "kb16-test-"


def made_up_name() -> str:
    return TEST_NAME + uuid.uuid4().hex[:12]


def launch(command: list[str], env: dict, **kwargs) -> subprocess.CompletedProcess:
    """Runs a flasher, or a tool that a flasher test needs, but only with a made-up app name."""
    name = env.get(APP_PROCESS, "")
    if not name.startswith(TEST_NAME):
        raise AssertionError(f"{APP_PROCESS} must start with {TEST_NAME!r}, not {name!r}")
    return subprocess.run(command, env=env, capture_output=True, text=True, timeout=120, **kwargs)


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
        self.image = root / "firmware" / "target" / "kb16-mixer-firmware.bin"
        self.image.write_bytes(image())
        self.root = root
        self.bin = root / "bin"
        self.bin.mkdir()
        # On Windows the script stops the running app with taskkill and lists it with
        # powershell.exe. These fakes record their arguments. The made-up name is the main guard;
        # the fakes are a second one.
        self.tools_log = root / "tools.log"
        record = f'echo "$(basename "$0") $*" >> "{self.tools_log.as_posix()}"\nexit 0\n'
        write_tool(self.bin / "taskkill", record)
        write_tool(self.bin / "powershell.exe", record)
        self.name = made_up_name()
        self.env = dict(os.environ, PATH=str(self.bin) + os.pathsep + os.environ["PATH"])
        self.env[APP_PROCESS] = self.name
        self.env.pop("DFU_UTIL", None)

    def run_script(self, dfu_body: str) -> subprocess.CompletedProcess:
        write_tool(self.bin / "dfu-util", dfu_body)
        found = launch(["sh", "-c", "command -v taskkill; command -v dfu-util"], self.env).stdout.split()
        self.assertEqual(len(found), 2)
        for tool in found:
            self.assertEqual(Path(tool).parent.name, "bin", "the fake tools are not first on the PATH")
        return launch(["sh", str(self.root / "scripts" / "flash.sh")], self.env)

    def test_the_stop_uses_the_name_in_the_variable(self):
        result = self.run_script('echo "Download done."\nexit 74\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = self.tools_log.read_text().splitlines()
        self.assertIn(f"taskkill //IM {self.name}.exe //F", lines)
        self.assertTrue([line for line in lines if line.startswith("powershell.exe ")
                         and f"Get-Process -Name {self.name} " in line], lines)
        self.assertFalse([line for line in lines if "kb16-mixer" in line], lines)

    def test_a_bad_name_is_refused_before_anything_runs(self):
        log = self.root / "dfu.log"
        for bad in ("*", "?", " x", "\nkb16-mixer", "'", "/x"):
            with self.subTest(repr(bad)):
                self.env[APP_PROCESS] = self.name + bad
                result = self.run_script(f'echo "$@" >> "{log.as_posix()}"\necho "Download done."\n')
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("REFUSED: KB16_APP_PROCESS may hold only", result.stderr)
                self.assertFalse(log.exists(), "dfu-util ran after a refusal")
                self.assertFalse(self.tools_log.exists(), "taskkill or powershell.exe ran after a refusal")

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

    def test_image_refusals(self):
        log = self.root / "dfu.log"
        for name, data in (
            ("empty", b""),
            ("shorter than the vector table", image(size=7)),
            ("larger than the limit", image(size=MAX_IMAGE + 1)),
            ("stack pointer below RAM", image(sp=0x20000000)),
            ("stack pointer above RAM", image(sp=0x20005004)),
            ("reset vector in the bootloader", image(reset=0x08001001)),
            ("reset vector in the name page", image(reset=0x0801F401)),
            ("reset vector without the Thumb bit", image(reset=0x08002130)),
        ):
            with self.subTest(name):
                self.image.write_bytes(data)
                result = self.run_script(f'echo "$@" >> "{log.as_posix()}"\necho "Download done."\n')
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn("REFUSED:", result.stderr)
                self.assertFalse(log.exists(), "dfu-util ran after a refusal")

    def test_image_at_the_limit_is_accepted(self):
        self.image.write_bytes(image(size=MAX_IMAGE))
        result = self.run_script('echo "Download done."\nexit 74\n')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"({MAX_IMAGE} bytes, vectors OK)", result.stdout)

    def test_missing_image_is_refused(self):
        self.image.unlink()
        result = self.run_script('echo "Download done."\n')
        self.assertEqual(result.returncode, 1)
        self.assertIn("no firmware image", result.stderr)

    def test_missing_dfu_util_is_refused(self):
        # No dfu-util on the PATH of the test and no tools/ folder.
        env = dict(self.env, PATH=os.pathsep.join(p for p in self.env["PATH"].split(os.pathsep)
                                                     if not shutil.which("dfu-util", path=p)
                                                     and not shutil.which("dfu-util-static", path=p)))
        result = launch(["sh", str(self.root / "scripts" / "flash.sh")], env)
        self.assertEqual(result.returncode, 1)
        self.assertIn("dfu-util not found", result.stderr)


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

# A fake dfu-util. It is a console program, so that the tools-folder search can find an .exe.
#   -l  prints the file FAKE_DFU_LIST, but only when FAKE_DFU_LIST_AFTER is unset or names a
#       file that exists (the pad lists only after the bootloader request).
#   -U  copies FAKE_DFU_UPLOAD (if set) to the given path: the backup.
#   -D  prints FAKE_DFU_WRITE_OUTPUT and exits with FAKE_DFU_WRITE_EXIT.
# Each call appends its arguments to FAKE_DFU_LOG.
FAKE_DFU = r"""
using System;
using System.IO;
class FakeDfu {
    static string Env(string name) { return Environment.GetEnvironmentVariable(name); }
    static int Main(string[] args) {
        string log = Env("FAKE_DFU_LOG");
        if (log != null) File.AppendAllText(log, string.Join(" ", args) + "\n");
        int i = Array.IndexOf(args, "-U");
        if (i >= 0) {
            if (Env("FAKE_DFU_UPLOAD") != null) File.Copy(Env("FAKE_DFU_UPLOAD"), args[i + 1]);
            return 0;
        }
        if (Array.IndexOf(args, "-D") >= 0) {
            Console.WriteLine(Env("FAKE_DFU_WRITE_OUTPUT") ?? "");
            return int.Parse(Env("FAKE_DFU_WRITE_EXIT") ?? "0");
        }
        string after = Env("FAKE_DFU_LIST_AFTER");
        string list = Env("FAKE_DFU_LIST");
        if (list != null && File.Exists(list) && (after == null || File.Exists(after))) {
            Console.Write(File.ReadAllText(list));
        }
        return 0;
    }
}
"""


def dfu_line(path: str = "1-1", alt: int = 2, address: str = "0x8002000") -> str:
    return (f'Found DFU: [1eaf:0003] ver=0201, devnum=5, cfg=1, intf=0, path="{path}", alt={alt}, '
            f'name="STM32duino bootloader v1.0  Upload to Flash {address}", serial="LLM 003"\n')


# The listing of the tested pad: alternate settings 0, 1 and 2 of one device.
PAD = (dfu_line(alt=0, address="0x8005000") + dfu_line(alt=1, address="0x8005000")
       + dfu_line(alt=2, address="0x8002000"))

POWERSHELL = shutil.which("powershell.exe") or shutil.which("powershell")
CSC = Path(os.environ.get("WINDIR", r"C:\Windows")) / "Microsoft.NET" / "Framework64" / "v4.0.30319" / "csc.exe"
HAVE_PS1 = sys.platform == "win32" and bool(POWERSHELL) and CSC.is_file()
REQUIRE_PS1 = os.environ.get("KB16_REQUIRE_PS1_TESTS") == "1"


def compile_cs(source: str, out: Path, target: str) -> None:
    cs = out.with_suffix(".cs")
    cs.write_text(source, encoding="ascii")
    subprocess.run([str(CSC), "/nologo", f"/target:{target}", f"/out:{out}", str(cs)],
                   check=True, capture_output=True)


class RequirePs1(unittest.TestCase):
    @unittest.skipUnless(REQUIRE_PS1, "KB16_REQUIRE_PS1_TESTS is not set")
    def test_flash_ps1_tests_can_run(self):
        self.assertTrue(HAVE_PS1, "the flash.ps1 tests would skip: no Windows PowerShell or C# compiler")


@unittest.skipUnless(HAVE_PS1, "needs Windows PowerShell and the .NET Framework C# compiler")
class FlashPs1(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.stub = Path(cls.tmp.name) / "stub.exe"
        compile_cs(STUB, cls.stub, "winexe")
        cls.fake_dfu = Path(cls.tmp.name) / "fake-dfu.exe"
        compile_cs(FAKE_DFU, cls.fake_dfu, "exe")

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        # "release" is the folder of the script, as in the release zip. Its parent stands for the
        # repository root, for the ..\tools search.
        # resolve(): the temp folder can be a short 8.3 path (as on the GitHub Windows runner),
        # and PowerShell prints the long one. The tests compare paths as text.
        self.root = Path(tmp.name).resolve()
        self.folder = self.root / "release"
        self.folder.mkdir()
        text = (SCRIPTS / "flash.ps1").read_text(encoding="utf-8")
        # A real run waits 120 seconds for the bootloader; the copy waits 2.
        self.assertEqual(text.count("$WaitSeconds = 120\n"), 1)
        text = text.replace("$WaitSeconds = 120\n", "$WaitSeconds = 2\n")
        (self.folder / "flash.ps1").write_text(text, encoding="utf-8")
        (self.folder / "firmware").mkdir()
        self.image = self.folder / "firmware" / "kb16-mixer-firmware.bin"
        self.image.write_bytes(image())
        self.list = self.root / "list.txt"
        self.log = self.root / "dfu.log"
        # Only the Windows folders on the PATH, so that no dfu-util of this PC is found.
        windir = Path(os.environ.get("WINDIR", r"C:\Windows"))
        self.env = dict(os.environ, FAKE_DFU_LIST=str(self.list), FAKE_DFU_LOG=str(self.log),
                        PATH=os.pathsep.join([str(windir / "System32"),
                                              str(windir / "System32" / "WindowsPowerShell" / "v1.0")]))
        self.name = made_up_name()
        self.env[APP_PROCESS] = self.name
        for name in ("FAKE_DFU_LIST_AFTER", "FAKE_DFU_UPLOAD", "FAKE_DFU_WRITE_OUTPUT", "FAKE_DFU_WRITE_EXIT"):
            self.env.pop(name, None)

    def listing(self, text: str) -> None:
        self.list.write_text(text, encoding="ascii")

    def run_script(self, *args: str, answers: str = "", dfu: bool = True) -> subprocess.CompletedProcess:
        command = [POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(self.folder / "flash.ps1")]
        if dfu:
            command += ["-DfuUtil", str(self.fake_dfu)]
        return launch(command + list(args), self.env, input=answers)

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines() if self.log.exists() else []

    def assert_refused(self, result: subprocess.CompletedProcess, message: str):
        out = " ".join(result.stdout.split())  # PowerShell can wrap long lines
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("REFUSED: " + message, out)
        self.assertIn("Nothing was written to the pad.", out)
        self.assertFalse([c for c in self.calls() if "-D" in c.split()], "dfu-util got a write command")

    # --- The image -------------------------------------------------------------------------

    def test_image_refusals(self):
        self.listing(PAD)
        for name, data, message in (
            ("empty", b"", "The image"),
            ("shorter than the vector table", image(size=7), "The image"),
            ("larger than the limit", image(size=MAX_IMAGE + 1), f"The image is {MAX_IMAGE + 1} bytes. The limit is"),
            ("stack pointer below RAM", image(sp=0x20000000), "The first word (0x20000000) is not a stack pointer"),
            ("stack pointer above RAM", image(sp=0x20005004), "The first word (0x20005004) is not a stack pointer"),
            ("reset vector in the bootloader", image(reset=0x08001001), "The second word (0x08001001) is not a reset"),
            ("reset vector in the name page", image(reset=0x0801F401), "The second word (0x0801F401) is not a reset"),
            ("reset vector without the Thumb bit", image(reset=0x08002130), "The second word (0x08002130) is not a reset"),
        ):
            with self.subTest(name):
                self.image.write_bytes(data)
                self.assert_refused(self.run_script("-DryRun"), message)

    def test_empty_image_names_the_reason(self):
        self.image.write_bytes(b"")
        self.assertIn("is empty.", self.run_script("-DryRun").stdout)

    def test_image_at_the_limit_is_accepted(self):
        self.listing(PAD)
        self.image.write_bytes(image(size=MAX_IMAGE))
        result = self.run_script("-DryRun")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"{MAX_IMAGE} bytes, stack pointer 0x20005000, reset vector 0x08002131: OK", result.stdout)
        self.assertIn("DRY RUN complete: all checks passed.", result.stdout)

    def test_missing_image_is_refused(self):
        self.assert_refused(self.run_script("-DryRun", "-Firmware", str(self.root / "none.bin")),
                            "No firmware image at")

    # --- The bootloader --------------------------------------------------------------------

    def test_wrong_address_of_alternate_setting_2_is_refused(self):
        self.listing(dfu_line(alt=0) + dfu_line(alt=1) + dfu_line(alt=2, address="0x8005000"))
        self.assert_refused(self.run_script("-DryRun"), "Alternate setting 2 writes to a different address")

    def test_two_bootloaders_are_refused(self):
        self.listing(PAD + dfu_line(path="1-2", alt=2))
        self.assert_refused(self.run_script("-DryRun"), "2 devices with the USB ID 1EAF:0003 are connected.")

    def test_no_alternate_setting_2_is_refused(self):
        self.listing(dfu_line(alt=0) + dfu_line(alt=1))
        self.assert_refused(self.run_script("-DryRun"), "The bootloader has no alternate setting 2.")

    def test_no_bootloader_is_refused_after_the_wait(self):
        self.listing("")
        result = self.run_script()
        self.assert_refused(result, "No pad in bootloader mode (USB 1EAF:0003) was found.")
        self.assertIn("Put the pad into its bootloader now:", result.stdout)

    def test_dry_run_without_a_bootloader_checks_the_image_only(self):
        self.listing("")
        result = self.run_script("-DryRun")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("DRY RUN complete: the image and dfu-util are OK.", result.stdout)

    # --- The backup ------------------------------------------------------------------------

    def run_backup(self, data, *extra: str) -> tuple[subprocess.CompletedProcess, Path]:
        self.listing(PAD)
        if data is not None:
            upload = self.root / "upload.bin"
            upload.write_bytes(data)
            self.env["FAKE_DFU_UPLOAD"] = str(upload)
        backup = self.root / "backup.bin"
        return self.run_script("-Backup", str(backup), *extra), backup

    def test_backup_refusals(self):
        for name, data, message in (
            ("no file", None, "dfu-util wrote no backup file."),
            ("empty", b"", "The backup file is empty."),
            ("all 0xFF", b"\xff" * FULL_AREA, "The backup holds only 0xFF bytes"),
            ("too short", b"\x01" * (FULL_AREA - 1024), f"The backup has {FULL_AREA - 1024} bytes"),
            ("too long", b"\x01" * (FULL_AREA + 1024), f"The backup has {FULL_AREA + 1024} bytes"),
        ):
            for mode in ((), ("-DryRun",)):
                with self.subTest(name, mode=mode):
                    self.log.unlink(missing_ok=True)
                    (self.root / "backup.bin").unlink(missing_ok=True)
                    self.env.pop("FAKE_DFU_UPLOAD", None)
                    result, _ = self.run_backup(data, *mode)
                    self.assert_refused(result, message)

    def test_good_backup_in_a_dry_run(self):
        result, backup = self.run_backup(b"\x01" * FULL_AREA, "-DryRun")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(backup.stat().st_size, FULL_AREA)
        self.assertIn(f"backup: {FULL_AREA} bytes, SHA-256", result.stdout)
        self.assertIn("DRY RUN complete: all checks passed.", result.stdout)

    def test_backup_in_a_dry_run_without_a_bootloader_is_refused(self):
        self.listing("")
        result = self.run_script("-DryRun", "-Backup", str(self.root / "backup.bin"))
        self.assert_refused(result, "-Backup needs the pad in bootloader mode")
        self.assertNotIn("DRY RUN complete", result.stdout)
        self.assertFalse((self.root / "backup.bin").exists())

    def test_existing_backup_file_is_refused(self):
        backup = self.root / "backup.bin"
        backup.write_bytes(b"old")
        self.listing(PAD)
        self.assert_refused(self.run_script("-DryRun", "-Backup", str(backup)), "The backup file")
        self.assertEqual(backup.read_bytes(), b"old")

    # --- The write -------------------------------------------------------------------------

    def test_no_yes_is_refused(self):
        self.listing(PAD)
        self.assert_refused(self.run_script(answers="yes\n"), "You did not type YES.")

    def test_write_with_download_done(self):
        self.listing(PAD)
        self.env.update(FAKE_DFU_WRITE_OUTPUT="Download done.", FAKE_DFU_WRITE_EXIT="74")
        result = self.run_script(answers="YES\n")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(NEXT, result.stdout)
        self.assertIn(f"-d 1eaf:0003 -a 2 -D {self.image} -R", self.calls())

    def test_write_without_download_done_fails(self):
        self.listing(PAD)
        self.env.update(FAKE_DFU_WRITE_OUTPUT="dfu-util: Cannot open DFU device 1eaf:0003", FAKE_DFU_WRITE_EXIT="0")
        result = self.run_script(answers="YES\n")
        self.assertEqual(result.returncode, 1)
        self.assertIn('FAILED: dfu-util did not show "Download done"', result.stdout)
        self.assertNotIn(NEXT, result.stdout)

    # --- Where the script looks for dfu-util ----------------------------------------------

    def test_dfu_util_in_tools_next_to_the_script(self):
        self.listing("")
        (self.folder / "tools").mkdir()
        shutil.copy(self.fake_dfu, self.folder / "tools" / "dfu-util-static.exe")
        result = self.run_script("-DryRun", dfu=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("dfu-util: " + str(self.folder / "tools" / "dfu-util-static.exe"), result.stdout)

    def test_dfu_util_in_tools_of_the_repository_root(self):
        self.listing("")
        (self.root / "tools").mkdir()
        shutil.copy(self.fake_dfu, self.root / "tools" / "dfu-util.exe")
        result = self.run_script("-DryRun", dfu=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("dfu-util: " + str(self.root / "tools" / "dfu-util.exe"), result.stdout)

    def test_no_dfu_util_names_every_place(self):
        result = self.run_script("-DryRun", dfu=False)
        self.assert_refused(result, "dfu-util not found.")
        out = " ".join(result.stdout.split())
        self.assertIn("on the PATH", out)
        self.assertIn(str(self.folder / "tools"), out)
        self.assertIn(str(self.root / "tools"), out)

    def test_missing_dfu_util_path_is_refused(self):
        self.assert_refused(self.run_script("-DfuUtil", str(self.root / "none.exe"), dfu=False), "dfu-util not found at")

    # --- The bootloader request through the app --------------------------------------------

    def run_request(self, app_exit: int) -> tuple[str, Path]:
        shutil.copy(self.stub, self.folder / "kb16-mixer.exe")
        marker = self.root / "marker.txt"
        self.listing(PAD)
        self.env.update(STUB_MARKER=str(marker), STUB_EXIT=str(app_exit), FAKE_DFU_LIST_AFTER=str(marker))
        # "y" asks for the bootloader request; "no" refuses the write, so dfu-util never writes.
        result = self.run_script(answers="y\nno\n")
        self.assert_refused(result, "You did not type YES.")
        return result.stdout, marker

    def test_app_exit_0_means_the_pad_was_told(self):
        out, marker = self.run_request(0)
        self.assertEqual(marker.read_text(), "--bootloader")
        self.assertIn("The pad was told to enter its bootloader.", out)
        self.assertNotIn("The pad did not answer.", out)

    def test_app_exit_1_means_the_pad_did_not_answer(self):
        out, marker = self.run_request(1)
        self.assertEqual(marker.read_text(), "--bootloader")
        self.assertIn("The pad did not answer.", out)
        self.assertNotIn("The pad was told to enter its bootloader.", out)
        self.assertIn("Put the pad into its bootloader now:", out)

    def test_the_stop_uses_the_name_in_the_variable(self):
        # A long-running program with the made-up name stands for a running app.
        decoy = start_decoy(self.root / f"{self.name}.exe")
        self.addCleanup(decoy.kill)
        out, _ = self.run_request(0)
        self.assertIn("The pad was told to enter its bootloader.", out)
        self.assertIsNotNone(decoy.poll(), "the script did not stop the process with the name in the variable")

    def test_a_bad_name_is_refused_before_anything_runs(self):
        self.listing(PAD)
        for bad in ("*", "?", " x", "\n", "'", "/x"):
            with self.subTest(repr(bad)):
                self.env[APP_PROCESS] = self.name + bad
                self.assert_refused(self.run_script("-DryRun"), "KB16_APP_PROCESS may hold only")
                self.assertEqual(self.calls(), [], "dfu-util ran after a refusal")


def start_decoy(exe: Path) -> subprocess.Popen:
    """Starts a copy of ping.exe under the given name. It runs for about a minute."""
    shutil.copy(Path(os.environ.get("WINDIR", r"C:\Windows")) / "System32" / "PING.EXE", exe)
    return subprocess.Popen([str(exe), "-n", "60", "127.0.0.1"], stdout=subprocess.DEVNULL)


class Guard(unittest.TestCase):
    """Checks of the tests themselves: no flasher starts without a made-up app name."""

    def test_launch_refuses_an_environment_without_a_made_up_name(self):
        for env in ({}, {APP_PROCESS: ""}, {APP_PROCESS: "kb16-mixer"}):
            with self.subTest(env=env), self.assertRaises(AssertionError):
                launch([sys.executable, "-c", "pass"], env)

    def test_every_process_start_goes_through_launch(self):
        # compile_cs runs the C# compiler and start_decoy runs a copy of ping.exe: no flasher.
        tree = ast.parse(Path(__file__).read_text(encoding="utf-8"))
        starts = set()
        for function in ast.walk(tree):
            if isinstance(function, ast.FunctionDef):
                for node in ast.walk(function):
                    if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                            and isinstance(node.func.value, ast.Name)
                            and node.func.value.id in ("subprocess", "os")):
                        starts.add((function.name, f"{node.func.value.id}.{node.func.attr}"))
        self.assertEqual(starts, {("launch", "subprocess.run"), ("compile_cs", "subprocess.run"),
                                  ("start_decoy", "subprocess.Popen")})

    def test_the_default_name_is_the_name_of_the_app(self):
        # No test runs a flasher without the variable, so the default is read from the text.
        cargo = (SCRIPTS.parent / "app" / "Cargo.toml").read_text(encoding="utf-8")
        app = re.search(r'^name = "([^"]+)"', cargo, re.M).group(1)
        self.assertEqual(app, "kb16-mixer")
        sh = (SCRIPTS / "flash.sh").read_text(encoding="utf-8")
        self.assertIn('APP_PROCESS="${KB16_APP_PROCESS:-' + app + '}"\n', sh)
        self.assertIn('taskkill //IM "$APP_PROCESS.exe" //F', sh)
        ps1 = (SCRIPTS / "flash.ps1").read_text(encoding="utf-8")
        self.assertIn("$AppProcess = if ($env:KB16_APP_PROCESS) { $env:KB16_APP_PROCESS } else { '"
                      + app + "' }", ps1)
        self.assertEqual(ps1.count("Get-Process -Name $AppProcess -ErrorAction"), 2)
        for text in (sh, ps1):
            self.assertNotRegex(text, r"(Get-Process|taskkill|Stop-Process)[^\n]*kb16-mixer")


if __name__ == "__main__":
    unittest.main()
