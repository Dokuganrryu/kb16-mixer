//! The `open:` key: starts a program, file, folder or URL as Explorer would.

use crate::log;
use crate::pad::PadEvent;
use std::sync::mpsc::Sender;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::UI::Shell::{
    SEE_MASK_DOENVSUBST, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR, w};

/// Opens a program, file, folder or URL the way a double-click in Explorer would: the shell
/// picks the handler, nothing is elevated, no arguments are passed, no other process is touched,
/// and a file named by its path starts in its own folder (`start_folder`). It runs on a
/// thread of its own, so a slow target (a network path, a sleeping drive) never holds up the
/// mixer; a failure is logged and shown on the pad.
pub fn open(target: String, notices: Sender<PadEvent>) {
    std::thread::spawn(move || {
        let file = HSTRING::from(target.as_str());
        let folder = start_folder(&target).map(|d| HSTRING::from(d.as_os_str()));
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            // Our own notice replaces the shell's error dialogs; NOASYNC because this thread
            // ends right after the call.
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_DOENVSUBST,
            lpVerb: w!("open"),
            lpFile: PCWSTR(file.as_ptr()),
            lpDirectory: folder.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        // SAFETY: `info` and the strings it points to (`file`, `folder`) live until the call
        // returns; NOASYNC makes the shell finish with them before that.
        let opened = unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
            ShellExecuteExW(&mut info)
        };
        if let Err(e) = opened {
            log(&format!("could not open {target:?}: {e}"));
            let _ = notices.send(PadEvent::Notice("Open failed".into()));
        }
    });
}

/// The folder a program or file starts in: its own, when the target is a file named by its full
/// path. Without one it would inherit the app's working folder, System32 when Windows starts it.
fn start_folder(target: &str) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(expand_env(target));
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    path.parent().map(std::path::Path::to_path_buf)
}

/// `%NAME%` replaced by that environment variable, as the shell does for the target itself; an
/// unknown name stays as written.
fn expand_env(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else { break };
        out.push_str(&rest[..start]);
        match std::env::var(&after[..end]) {
            Ok(value) if end > 0 => {
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            // The closing % may open the next name.
            _ => {
                out.push('%');
                out.push_str(&after[..end]);
                rest = &after[end..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_starts_in_its_own_folder() {
        let dir = std::env::temp_dir().join(format!("kb16-open-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("tool.exe");
        std::fs::write(&tool, "").unwrap();
        assert_eq!(start_folder(tool.to_str().unwrap()), Some(dir.clone()));
        assert_eq!(start_folder(dir.to_str().unwrap()), None, "a folder opens as itself");
        assert_eq!(start_folder("https://example.com/a"), None);
        assert_eq!(start_folder("notepad.exe"), None, "found by the shell, not here");
        assert_eq!(start_folder("C:\\no\\such\\thing.exe"), None);
        let cmd = start_folder("%SystemRoot%\\System32\\cmd.exe").expect("variables are expanded");
        assert!(cmd.ends_with("System32"), "{cmd:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn variables_expand_as_the_shell_does() {
        let root = std::env::var("SystemRoot").unwrap();
        assert_eq!(expand_env("%SystemRoot%\\x"), format!("{root}\\x"));
        assert_eq!(expand_env("%KB16_NO_SUCH_VAR%\\x"), "%KB16_NO_SUCH_VAR%\\x", "unknown stays as written");
        assert_eq!(expand_env("100% sure"), "100% sure");
        assert_eq!(expand_env("a%%b"), "a%%b");
    }
}
