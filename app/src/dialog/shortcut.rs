//! The Shortcut dialog: "press the keys you want" for one pad key.
//!
//! The box reads keys only from its own window messages while it has the focus (WM_KEYDOWN /
//! WM_SYSKEYDOWN and their key-up partners). Nothing here observes the keyboard anywhere else:
//! no hooks, no raw input, no polling of key state.

use super::keys::{Capture, WIN};
use super::{BM_GETCHECK, BM_SETCHECK, Class, ID_CLEAR, ID_EXTRA, ID_FIELD, ID_HINT, Item, TEXT_RESULT};
use super::{end, run, set_item_text, template};
use crate::win::{self, send};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, COLOR_WINDOWTEXT, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawFocusRect, DrawTextW,
    EndPaint, FillRect, FrameRect, GetSysColor, GetSysColorBrush, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

pub enum Shortcut {
    Set(String),
    Clear,
    Cancel,
}

thread_local! {
    static CAPTURE: RefCell<Capture> = RefCell::new(Capture::default());
    static CAPTURE_FONT: Cell<HFONT> = const { Cell::new(HFONT(std::ptr::null_mut())) };
    static CAPTURE_FOCUSED: Cell<bool> = const { Cell::new(false) };
    static WIN_PREFILL: Cell<bool> = const { Cell::new(false) };
}

const CAPTURE_CLASS: PCWSTR = w!("kb16-mixer-keys");

/// The dialog's start: the combination as it is now, with its Win part in the tick box, like one
/// made here.
fn prefill(current: Option<(u8, String)>) -> (Capture, bool) {
    let (mods, key) = current.map_or((0, None), |(m, k)| (m, Some(k)));
    (Capture::with(mods & !WIN, key), mods & WIN != 0)
}

/// What the hint line says the combination still needs, if anything.
fn hint(c: &Capture) -> &'static str {
    if c.refused() {
        "The pad cannot type that key. Letters, digits, F1 to F24, Enter, arrows and the like work."
    } else if c.text(false).is_none() {
        "Press a key, alone or together with Ctrl, Shift, Alt or Win."
    } else {
        ""
    }
}

/// What the box shows.
fn shown(label: String) -> String {
    if label.is_empty() { "(click here, then press the keys)".to_string() } else { label }
}

/// The dialog's end: OK (1) with a combination, Clear (2), or anything else.
fn outcome(code: isize, text: Option<String>) -> Shortcut {
    match code {
        1 => text.map_or(Shortcut::Cancel, Shortcut::Set),
        2 => Shortcut::Clear,
        _ => Shortcut::Cancel,
    }
}

fn win_ticked(dlg: HWND) -> bool {
    win::dlg_item(Some(dlg), ID_EXTRA as i32).is_some_and(|h| send(h, BM_GETCHECK, 0, 0) == 1)
}

/// Repaints the box and says in the hint line what the combination needs, if anything.
fn capture_changed(dlg: HWND) {
    if let Some(h) = win::dlg_item(Some(dlg), ID_FIELD as i32) {
        // SAFETY: InvalidateRect only marks the control for repainting; no rectangle is passed.
        unsafe {
            let _ = InvalidateRect(Some(h), None, true);
        }
    }
    set_item_text(dlg, ID_HINT as i32, CAPTURE.with_borrow(hint));
}

unsafe extern "system" fn capture_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Windows calls this on the box's own thread with the message's real parameters:
    // WM_GETDLGCODE's lParam is null or points to a MSG, and WM_PAINT is answered between
    // BeginPaint and EndPaint with the font the dialog set, put back before EndPaint.
    unsafe {
        let dlg = GetParent(hwnd).unwrap_or_default();
        match msg {
            WM_GETDLGCODE => {
                // A plain Tab still moves on to the buttons, so the dialog works without a mouse.
                let m = lparam.0 as *const MSG;
                let plain_tab = !m.is_null()
                    && (*m).message == WM_KEYDOWN
                    && (*m).wParam.0 == 0x09
                    && !CAPTURE.with_borrow(Capture::holding);
                LRESULT(if plain_tab { 0 } else { DLGC_WANTALLKEYS as isize })
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                CAPTURE.with_borrow_mut(|c| c.down(wparam.0 as u32));
                capture_changed(dlg);
                LRESULT(0)
            }
            WM_KEYUP | WM_SYSKEYUP => {
                CAPTURE.with_borrow_mut(|c| c.up(wparam.0 as u32));
                capture_changed(dlg);
                LRESULT(0)
            }
            // Swallowed so Alt+letter does not beep and F10 / Alt do not open the window menu.
            WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => LRESULT(0),
            WM_SETFOCUS | WM_KILLFOCUS => {
                CAPTURE_FOCUSED.set(msg == WM_SETFOCUS);
                if msg == WM_KILLFOCUS {
                    CAPTURE.with_borrow_mut(Capture::let_go);
                }
                capture_changed(dlg);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                send(dlg, WM_NEXTDLGCTL, hwnd.0 as usize, 1);
                LRESULT(0)
            }
            WM_SETFONT => {
                CAPTURE_FONT.set(HFONT(wparam.0 as *mut _));
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                FillRect(dc, &rect, GetSysColorBrush(COLOR_WINDOW));
                FrameRect(dc, &rect, GetSysColorBrush(COLOR_WINDOWTEXT));
                let old = SelectObject(dc, CAPTURE_FONT.get().into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
                let label = CAPTURE.with_borrow(|c| c.label(win_ticked(dlg)));
                let mut text: Vec<u16> = shown(label).encode_utf16().collect();
                DrawTextW(dc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                if CAPTURE_FOCUSED.get() {
                    let inner =
                        RECT { left: rect.left + 3, top: rect.top + 3, right: rect.right - 3, bottom: rect.bottom - 3 };
                    let _ = DrawFocusRect(dc, &inner);
                }
                SelectObject(dc, old);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn shortcut_proc(dlg: HWND, msg: u32, wparam: WPARAM, _: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            if WIN_PREFILL.get() {
                send(win::dlg_item(Some(dlg), ID_EXTRA as i32).unwrap_or_default(), BM_SETCHECK, 1, 0);
            }
            capture_changed(dlg);
            1 // focus goes to the first control: the box
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as i32;
            match id {
                1 => {
                    // OK: only once there is something to keep.
                    if let Some(text) = CAPTURE.with_borrow(|c| c.text(win_ticked(dlg))) {
                        TEXT_RESULT.set(Some(text));
                        end(dlg, 1);
                    } else {
                        capture_changed(dlg);
                    }
                }
                2 => end(dlg, 0),
                _ if id == ID_CLEAR as i32 => end(dlg, 2),
                _ if id == ID_EXTRA as i32 => capture_changed(dlg),
                _ => {}
            }
            1
        }
        _ => 0,
    }
}

/// "Press the keys you want" for one pad key. `current` prefills it (mods, key name).
pub fn shortcut(owner: HWND, current: Option<(u8, String)>) -> Shortcut {
    // SAFETY: the class name is a static string and the procedure is `capture_proc` above. A
    // second registration fails harmlessly: the class from the first one is used.
    unsafe {
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(capture_proc),
            hInstance: win::instance(),
            lpszClassName: CAPTURE_CLASS,
            hCursor: LoadCursorW(None, IDC_IBEAM).unwrap_or_default(),
            ..Default::default()
        });
    }
    let (capture, win) = prefill(current);
    CAPTURE.set(capture);
    TEXT_RESULT.set(None);
    let tick = BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0;
    let items = [
        Item {
            class: Class::Static,
            text: "Press the keys you want, for example Ctrl+Shift+M.",
            id: 0xFFFF,
            style: 0,
            rect: [7, 7, 250, 10],
        },
        Item {
            class: Class::Named(CAPTURE_CLASS),
            text: "",
            id: ID_FIELD,
            style: WS_TABSTOP.0,
            rect: [7, 20, 250, 20],
        },
        Item { class: Class::Static, text: "", id: ID_HINT, style: 0, rect: [7, 44, 250, 18] },
        Item { class: Class::Button, text: "Add the Win key", id: ID_EXTRA, style: tick, rect: [7, 64, 250, 12] },
        Item {
            class: Class::Static,
            text: "Windows keeps some combinations for itself (Win+L, Ctrl+Alt+Del, often the Win key). \
                   Those can be typed into the config file instead.",
            id: 0xFFFF,
            style: 0,
            rect: [7, 80, 250, 20],
        },
        Item { class: Class::Button, text: "Clear", id: ID_CLEAR, style: WS_TABSTOP.0, rect: [7, 108, 56, 15] },
        Item {
            class: Class::Button,
            text: "OK",
            id: 1,
            style: BS_DEFPUSHBUTTON as u32 | WS_TABSTOP.0,
            rect: [141, 108, 56, 15],
        },
        Item { class: Class::Button, text: "Cancel", id: 2, style: WS_TABSTOP.0, rect: [201, 108, 56, 15] },
    ];
    let t = template("Shortcut", [264, 130], &items);
    WIN_PREFILL.set(win);
    let code = run(owner, &t, Some(shortcut_proc));
    outcome(code, TEXT_RESULT.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_starts_from_the_current_shortcut_and_ends_with_a_choice() {
        let (c, win) = prefill(Some((WIN | 0x01, "m".into())));
        assert!(win, "the Win part goes to the tick box");
        assert_eq!(c.text(false).as_deref(), Some("ctrl+m"));
        assert_eq!(c.text(win).as_deref(), Some("win+ctrl+m"), "and comes back from it");
        let (c, win) = prefill(None);
        assert!(!win);
        assert_eq!(c.text(false), None);
        assert_eq!(shown(c.label(false)), "(click here, then press the keys)");
        assert!(hint(&c).starts_with("Press a key"));
        let mut c = Capture::default();
        c.down(0x60); // number pad 0
        assert!(hint(&c).starts_with("The pad cannot type"));
        c.down(0x41);
        assert_eq!(hint(&c), "", "nothing to say once there is a key");

        assert!(matches!(outcome(1, Some("ctrl+m".into())), Shortcut::Set(t) if t == "ctrl+m"));
        assert!(matches!(outcome(1, None), Shortcut::Cancel));
        assert!(matches!(outcome(2, None), Shortcut::Clear));
        assert!(matches!(outcome(0, Some("x".into())), Shortcut::Cancel), "Cancel keeps nothing");
        assert!(matches!(outcome(-1, None), Shortcut::Cancel), "nor does a dialog that failed to open");
    }
}
