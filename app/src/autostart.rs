//! "Start with Windows": one value under the current user's Run key.

use crate::{config, log};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("kb16-mixer");
/// The autostart value written before the app was renamed.
const OLD_RUN_VALUE: PCWSTR = w!("kb16-knobs");

fn run_value_exists(name: PCWSTR) -> bool {
    // SAFETY: the key and value names are static wide strings; no output buffers are passed.
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, name, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

/// True if the Run value is there, whatever `KB16_MIXER_DIR` says (for `--check`).
pub fn run_value_present() -> bool {
    run_value_exists(RUN_VALUE)
}

/// Always off for a copy run against `KB16_MIXER_DIR`, which never touches the registry.
pub fn autostart_enabled() -> bool {
    config::custom_dir().is_none() && run_value_present()
}

pub fn old_autostart_enabled() -> bool {
    run_value_exists(OLD_RUN_VALUE)
}

pub fn set_autostart(on: bool) {
    if config::custom_dir().is_some() {
        return log("Start with Windows not written: KB16_MIXER_DIR is set");
    }
    // SAFETY: the key and value names are static wide strings, and `value` is a live,
    // NUL-terminated UTF-16 buffer whose byte length is passed with it.
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let value: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            );
            // The entry from before the rename would start the old exe as well.
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, OLD_RUN_VALUE);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
    }
}
