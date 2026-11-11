//! Making the window's controls and reading and writing them, and the display-scale maths
//! they are laid out with.

use super::ids::*;
use crate::win::{self, send, send_text};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HFONT;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CBS_DROPDOWNLIST, CreateWindowExW, GWL_STYLE, GetWindowLongPtrW, HMENU, SW_HIDE, SW_SHOWNA, ShowWindow,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT, WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

thread_local! {
    pub static FONT: Cell<HFONT> = const { Cell::new(HFONT(std::ptr::null_mut())) };
    /// The tab the controls being created belong to (None: always shown).
    pub static BUILDING: Cell<Option<usize>> = const { Cell::new(None) };
    static NEXT_LABEL: Cell<i32> = const { Cell::new(ID_LABELS) };
    /// Every control made, with its id and its tab.
    pub static CONTROLS: RefCell<Vec<(HWND, i32, Option<usize>)>> = const { RefCell::new(Vec::new()) };
    /// Each dropdown's entries as (label, value), so a pick maps to exactly what was shown.
    static CHOICES: RefCell<HashMap<i32, Vec<(String, String)>>> = RefCell::new(HashMap::new());
    /// Set while the window itself writes into its controls, so that is not taken as typing.
    pub static FILLING: Cell<bool> = const { Cell::new(false) };
}

/// The window is gone: so are its controls.
pub fn forget_all() {
    CONTROLS.with_borrow_mut(Vec::clear);
    CHOICES.with_borrow_mut(HashMap::clear);
    NEXT_LABEL.set(ID_LABELS);
}

pub fn system_dpi() -> i32 {
    // SAFETY: GetDpiForSystem takes no arguments and only reads a system setting.
    unsafe { GetDpiForSystem() as i32 }
}

/// KB16_MIXER_DPI (developer setting): lay the window out as at another display scale.
pub fn dpi() -> i32 {
    dpi_from(std::env::var("KB16_MIXER_DPI").ok().as_deref(), system_dpi())
}

fn dpi_from(setting: Option<&str>, system: i32) -> i32 {
    setting.and_then(|v| v.parse().ok()).unwrap_or(system)
}

/// A length at 96 dpi, at `dpi`.
fn scaled(v: i32, dpi: i32) -> i32 {
    v * dpi / 96
}

pub fn scale(v: i32) -> i32 {
    scaled(v, dpi())
}

/// The system's message font is sized for the system dpi; the window may be laid out at another.
pub fn font_height(system_height: i32, dpi: i32, system_dpi: i32) -> i32 {
    system_height * dpi / system_dpi
}

pub fn child(id: i32) -> HWND {
    win::dlg_item(super::window(), id).unwrap_or_default()
}

pub fn text_of(id: i32) -> String {
    win::window_text(child(id))
}

/// Writes a control's text, if it differs, without that being taken as typing.
pub fn set_text(id: i32, text: &str) {
    if text_of(id) != text {
        FILLING.set(true);
        win::set_window_text(child(id), text);
        FILLING.set(false);
    }
}

pub fn status(text: &str) {
    set_text(ID_STATUS, text);
}

pub fn checked(id: i32) -> bool {
    send(child(id), BM_GETCHECK, 0, 0) == 1
}

pub fn set_checked(id: i32, on: bool) {
    send(child(id), BM_SETCHECK, on as usize, 0);
}

/// Shows or hides a control, touching it only if that changes something.
pub fn show(h: HWND, on: bool) {
    // SAFETY: both calls take any window handle and fail harmlessly on a stale one.
    unsafe {
        let visible = GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0;
        if visible != on {
            let _ = ShowWindow(h, if on { SW_SHOWNA } else { SW_HIDE });
        }
    }
}

/// Makes a child control, laid out at 96 dpi and scaled. A negative id gives a label an id of
/// its own.
#[allow(clippy::too_many_arguments)]
pub fn control(parent: HWND, class: PCWSTR, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
    let id = if id < 0 { NEXT_LABEL.replace(NEXT_LABEL.get() + 1) } else { id };
    // SAFETY: the class name is a static string, the text outlives the call, and the "menu" of
    // a child window is its id, which Windows never dereferences.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(style) | WS_CHILD,
            scale(x),
            scale(y),
            scale(w),
            scale(h),
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            None,
            None,
        )
        .unwrap_or_default()
    };
    send(hwnd, WM_SETFONT, FONT.get().0 as usize, 1);
    CONTROLS.with_borrow_mut(|c| c.push((hwnd, id, BUILDING.get())));
    hwnd
}

pub fn label(parent: HWND, text: &str, x: i32, y: i32, w: i32) {
    control(parent, w!("STATIC"), text, 0, x, y + 3, w, 18, -1);
}

pub fn combo(parent: HWND, id: i32, x: i32, y: i32, w: i32) {
    let h =
        control(parent, w!("COMBOBOX"), "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0, x, y, w, 400, id);
    send(h, CB_SETDROPPEDWIDTH, scale(w.max(200)) as usize, 0);
}

pub fn button(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32) {
    control(parent, w!("BUTTON"), text, WS_TABSTOP.0, x, y, w, 26, id);
}

/// Fills a dropdown, touching it only if its entries changed (an open list stays open).
pub fn fill_combo(id: i32, entries: Vec<(String, String)>, at: usize) {
    let h = child(id);
    let same = CHOICES.with_borrow(|c| c.get(&id) == Some(&entries));
    if !same {
        send(h, CB_RESETCONTENT, 0, 0);
        for (label, _) in &entries {
            send_text(h, CB_ADDSTRING, 0, label);
        }
        CHOICES.with_borrow_mut(|c| c.insert(id, entries));
    }
    if send(h, CB_GETCURSEL, 0, 0) != at as isize {
        send(h, CB_SETCURSEL, at, 0);
    }
}

/// The value of the entry picked in a dropdown.
pub fn choice(id: i32) -> Option<String> {
    let at = usize::try_from(send(child(id), CB_GETCURSEL, 0, 0)).ok()?;
    CHOICES.with_borrow(|c| c.get(&id).and_then(|e| e.get(at)).map(|(_, v)| v.clone()))
}

/// Replaces the lines of a list box.
pub fn fill_list(id: i32, lines: &[String]) {
    let list = child(id);
    send(list, LB_RESETCONTENT, 0, 0);
    for line in lines {
        send_text(list, LB_ADDSTRING, 0, line);
    }
}

/// The line picked in a list box.
pub fn list_pick(id: i32) -> Option<usize> {
    usize::try_from(send(child(id), LB_GETCURSEL, 0, 0)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_scales_with_the_display() {
        assert_eq!(dpi_from(None, 120), 120, "the system's scale by default");
        assert_eq!(dpi_from(Some("144"), 96), 144, "the developer setting wins");
        assert_eq!(dpi_from(Some("big"), 96), 96, "and is ignored when it is not a number");
        assert_eq!(scaled(680, 96), 680);
        assert_eq!(scaled(680, 120), 850, "125 %");
        assert_eq!(scaled(532, 144), 798, "150 %");
        assert_eq!(scaled(18, 120), 22, "rounded down");
        assert_eq!(font_height(-12, 144, 96), -18, "the font grows with the layout");
        assert_eq!(font_height(-15, 96, 120), -12);
    }
}
