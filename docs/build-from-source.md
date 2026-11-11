# Build from source on Windows

This document tells you how to build the firmware image and the app yourself, on Windows 10 or 11.
To install a release without a build, see [install.md](install.md).

The steps were written on Windows 11 with Rust 1.99.1, Python 3.14, Visual Studio Build Tools 2026 and Git for Windows.

## What you need

| Item | Why | How to get it |
|---|---|---|
| Git for Windows | The source code, and Git Bash. The build scripts are POSIX shell scripts. | https://git-scm.com |
| Visual Studio Build Tools, workload "Desktop development with C++" | The MSVC linker and the Windows SDK, for the app | https://visualstudio.microsoft.com/downloads/ (Build Tools for Visual Studio). `rustup-init.exe` also offers to install them. |
| Rust (stable), 1.90 or later | All crates. They use Rust edition 2024. The app needs 1.90. The firmware needs 1.88. | https://rustup.rs (`rustup-init.exe`) |
| The firmware target `thumbv7m-none-eabi` | The firmware runs on a Cortex-M3 chip. | `rustup target add thumbv7m-none-eabi` |
| `llvm-tools` | `objcopy` and `nm` for the firmware image | `rustup component add llvm-tools` |
| `rustfmt` and `clippy` | The checks of CI | `rustup component add rustfmt clippy` |
| Python 3 | The image check in `scripts/build-firmware.sh`, and `scripts/third-party-notices.py` | https://www.python.org |
| Pillow (Python) | Only if you change the screen fonts or bitmaps (`firmware/gen_assets.py`) | `python -m pip install pillow` |
| `cargo-about` 0.9 | Only if you change a dependency: it writes `THIRD_PARTY_NOTICES.md` | `cargo install cargo-about --locked --features cli` |

You do not need a pad to build. You need a pad only to flash.

## Set up

1. Install Git for Windows.
2. Install the Visual Studio Build Tools with the workload **Desktop development with C++**.
3. Install Rust with `rustup-init.exe`. Accept the default host `x86_64-pc-windows-msvc`.
4. Open Git Bash.
5. Add the firmware target:

   ```sh
   rustup target add thumbv7m-none-eabi
   ```

6. Add the tools:

   ```sh
   rustup component add llvm-tools rustfmt clippy
   ```

7. Install Python 3. In the installer, select **Add python.exe to PATH**.
8. Get the source code:

   ```sh
   git clone https://github.com/Dokuganrryu/kb16-mixer.git
   cd kb16-mixer
   ```

### Check the setup

Run these commands. Each one must show a version or a list, and no error:

```sh
rustc -V
cargo -V
rustup target list --installed
rustup component list --installed
python --version
```

`rustup target list --installed` must show `thumbv7m-none-eabi` and `x86_64-pc-windows-msvc`.
`rustup component list --installed` must show a line that starts with `llvm-tools`.

`scripts/build-firmware.sh` tries `python3` first, then `python`. It uses the first one that runs a small test. If `python3` is only the Windows "app execution alias", the test fails, and the script uses `python`.

## Build the firmware

1. In Git Bash, in the repository folder, run:

   ```sh
   sh scripts/build-firmware.sh
   ```

2. Wait until the build is complete. The first build downloads the crates and takes some minutes.

**You see:** `<n> bytes, vectors OK` and `wrote .../firmware/target/kb16-mixer-firmware.bin`.

The script checks the image before it writes it. It shows `IMAGE REJECTED` and writes no image if:

- the stack pointer is not in RAM.
- the reset vector is not in the firmware region (above `0x08002000`).
- one of the interrupts for the timer, USB, I2C or DMA goes to `DefaultHandler`. This occurs when the `rt` feature of embassy-stm32 is missing. Such an image stops the pad at the first interrupt.
- the image is larger than 117 KB (`0x1D400` bytes). It then overlaps the name page and the crash log.

`sh scripts/build-firmware.sh --diag` builds an image that also writes start-up markers into the crash log. See [hardware.md](hardware.md#crash-log).

## Build the app

1. In Git Bash, in the repository folder, run:

   ```sh
   sh scripts/build-app.sh
   ```

**You see:** `wrote .../app/target/release/kb16-mixer.exe`.

The script removes the paths of your PC (which contain your user name) from the panic messages in the binary. A plain `cargo build --release` in `app/` also works, but keeps these paths.

## Check the result

1. Check the image with the dry run of the Windows flasher. It writes nothing:

   ```powershell
   powershell -ExecutionPolicy Bypass -File .\scripts\flash.ps1 -DryRun
   ```

   It finds `firmware\target\kb16-mixer-firmware.bin` and shows its size, its first two words and its SHA-256 value. It needs `dfu-util`. See [install.md](install.md#14-get-dfu-util).
2. Run the tests and the checks of each crate that you changed. See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full list. For example, for the app:

   ```sh
   cd app
   cargo fmt --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --locked
   ```

3. Try the app without a change to your real settings. Use a test folder and do not connect to the pad:

   ```powershell
   $env:KB16_MIXER_DIR = "$env:TEMP\kb16-test"
   .\app\target\release\kb16-mixer.exe --no-pad
   ```

   A copy with `KB16_MIXER_DIR` has its own settings and its own single-instance guard. It does not change **Start with Windows**.

## Flash your build

- With PowerShell: `powershell -ExecutionPolicy Bypass -File .\scripts\flash.ps1`. Without `-Firmware`, it takes `firmware\target\kb16-mixer-firmware.bin`.
- With Git Bash: `sh scripts/flash.sh`. See [flashing.md](flashing.md).

Before the first flash, make a backup of the stock firmware. See [stock-firmware.md](stock-firmware.md).

## Write the licence notices

Do this only if you add, remove or update a dependency of the firmware or the app.

1. Install `cargo-about` 0.9 (see [What you need](#what-you-need)).
2. Run:

   ```sh
   python scripts/third-party-notices.py
   ```

The script writes `THIRD_PARTY_NOTICES.md`. It stops with exit code 1 if a dependency has a licence that `scripts/about.toml` does not accept.

## What was run for this document

- The version commands in [Check the setup](#check-the-setup) were run on the author's PC: Rust 1.99.1, Python 3.14.7, `cargo-about` 0.9.3, targets `thumbv7m-none-eabi` and `x86_64-pc-windows-msvc`.
- The dry run of `flash.ps1` was run against the image in `firmware/target/`, with a simulated `dfu-util`.
- The two build scripts and the tests were not run for this document. CI runs them.
- The setup on a new PC (Git, Build Tools, rustup, Python) was not done again for this document.
