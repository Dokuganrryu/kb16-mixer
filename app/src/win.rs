//! Small safe wrappers for the Win32 calls the settings window and its dialogs repeat. Each
//! holds one `unsafe` block, so the reasoning for that call is written once.

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetDlgItem, GetWindowTextLengthW, GetWindowTextW, SendMessageW, SetWindowTextW,
};
use windows::core::HSTRING;

/// Sends a message whose wParam and lParam are plain numbers or handles, never pointers.
/// Messages that take a string go through `send_text`.
pub fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    // SAFETY: SendMessageW accepts any window handle (a stale one makes it fail and return 0),
    // and with value parameters the receiver has no memory of ours to read or write.
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

/// Sends a message whose lParam is a NUL-terminated UTF-16 string, such as CB_ADDSTRING.
pub fn send_text(h: HWND, msg: u32, w: usize, text: &str) -> isize {
    let text = HSTRING::from(text);
    // SAFETY: the string is NUL-terminated and lives until SendMessageW returns; the call is
    // synchronous, and the controls that take such strings copy them.
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(text.as_ptr() as isize))).0 }
}

/// The control `id` of window `parent`, if it has one.
pub fn dlg_item(parent: Option<HWND>, id: i32) -> Option<HWND> {
    // SAFETY: GetDlgItem only reads the window list; a missing parent or id gives an error.
    unsafe { GetDlgItem(parent, id).ok() }
}

/// A window's text (a control's caption or contents).
pub fn window_text(h: HWND) -> String {
    // SAFETY: the buffer is one longer than the length Windows reports, and GetWindowTextW
    // writes at most the buffer's length, NUL included; only the `n` written are read.
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

pub fn set_window_text(h: HWND, text: &str) {
    // SAFETY: the string outlives the call, which copies it into the window.
    unsafe {
        let _ = SetWindowTextW(h, &HSTRING::from(text));
    }
}

/// This program's module handle, for registering window classes and making windows.
pub fn instance() -> HINSTANCE {
    // SAFETY: with no name, GetModuleHandleW returns the running exe's handle; nothing is freed.
    unsafe { GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default() }
}
