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

/// Five user32 functions, declared here with their documented signatures. The `windows` crate
/// keeps all but NotifyWinEvent in its keyboard-and-mouse module, which stays disabled because
/// it also holds the input-injection calls, and NotifyWinEvent in its large accessibility
/// module. None of them sends input or observes another window.
mod user32 {
    use windows::Win32::Foundation::HWND;
    use windows::core::BOOL;

    /// TRACKMOUSEEVENT.
    #[repr(C)]
    pub struct TrackMouse {
        pub size: u32,
        pub flags: u32,
        pub hwnd: HWND,
        pub hover_time: u32,
    }

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        pub fn TrackMouseEvent(event: *mut TrackMouse) -> BOOL;
        pub fn GetKeyState(vk: i32) -> i16;
        pub fn GetFocus() -> HWND;
        pub fn SetFocus(hwnd: HWND) -> HWND;
        pub fn NotifyWinEvent(event: u32, hwnd: HWND, object: i32, child: i32);
    }
}

/// Asks for one WM_MOUSELEAVE when the mouse leaves `h` (TME_LEAVE).
pub fn track_leave(h: HWND) {
    let mut e = user32::TrackMouse { size: size_of::<user32::TrackMouse>() as u32, flags: 2, hwnd: h, hover_time: 0 };
    // SAFETY: the struct carries its own size and lives until the call returns.
    unsafe {
        let _ = user32::TrackMouseEvent(&mut e);
    }
}

/// True while `vk` is down, as this thread's own key messages left it (not the live keyboard).
pub fn key_down(vk: i32) -> bool {
    // SAFETY: GetKeyState only reads this thread's input state.
    unsafe { user32::GetKeyState(vk) < 0 }
}

/// The control of this thread that has the keyboard focus (null for none).
pub fn focus() -> HWND {
    // SAFETY: GetFocus takes no arguments and only reads this thread's state.
    unsafe { user32::GetFocus() }
}

/// Gives `h`, a window of this thread, the keyboard focus.
pub fn set_focus(h: HWND) {
    // SAFETY: SetFocus fails harmlessly on a stale handle or another thread's window.
    unsafe {
        user32::SetFocus(h);
    }
}

/// Virtual-key codes, from WinUser.h.
pub const VK_TAB: u16 = 0x09;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_END: u16 = 0x23;
pub const VK_HOME: u16 = 0x24;

/// Tells screen readers that the text of `h` changed (EVENT_OBJECT_LIVEREGIONCHANGED).
pub fn live_region_changed(h: HWND) {
    const EVENT_OBJECT_LIVEREGIONCHANGED: u32 = 0x8019;
    const OBJID_CLIENT: i32 = -4;
    // SAFETY: NotifyWinEvent only queues an event for accessibility clients; a stale handle is
    // ignored.
    unsafe { user32::NotifyWinEvent(EVENT_OBJECT_LIVEREGIONCHANGED, h, OBJID_CLIENT, 0) }
}
