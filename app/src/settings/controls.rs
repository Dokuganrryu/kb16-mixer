//! Making the window's controls and reading and writing them, and the display-scale maths
//! they are laid out with.

use super::ids::*;
use crate::ui::canvas::R;
use crate::ui::widget::{self, Kind};
use crate::ui::{self, Font};
use crate::win::{self, send, send_text};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CBS_DROPDOWNLIST, CreateWindowExW, GWL_STYLE, GetWindowLongPtrW, GetWindowRect, HMENU, SW_HIDE, SW_SHOWNA,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT,
    WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{HSTRING, PCWSTR, w};

thread_local! {
    /// The tab the controls being created belong to (None: always shown).
    pub static BUILDING: Cell<Option<usize>> = const { Cell::new(None) };
    static NEXT_LABEL: Cell<i32> = const { Cell::new(ID_LABELS) };
    /// Every control made, with its id and its tab.
    pub static CONTROLS: RefCell<Vec<(HWND, i32, Option<usize>)>> = const { RefCell::new(Vec::new()) };
    /// Each dropdown's entries as (label, value), so a pick maps to exactly what was shown.
    static CHOICES: RefCell<HashMap<i32, Vec<(String, String)>>> = RefCell::new(HashMap::new());
    /// Set while the window itself writes into its controls, so that is not taken as typing.
    pub static FILLING: Cell<bool> = const { Cell::new(false) };
    /// A scale forced by the hidden renderer.
    static FORCED_DPI: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn force_dpi(dpi: Option<i32>) {
    FORCED_DPI.set(dpi);
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
pub fn dpi_setting() -> Option<i32> {
    FORCED_DPI.get().or_else(|| dpi_from(std::env::var("KB16_MIXER_DPI").ok().as_deref()))
}

fn dpi_from(setting: Option<&str>) -> Option<i32> {
    setting.and_then(|v| v.parse().ok()).filter(|d| (48..=480).contains(d))
}

pub fn scale(v: i32) -> i32 {
    ui::px(v)
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

/// The status line; screen readers read a new one out.
pub fn status(text: &str) {
    if text_of(ID_STATUS) != text {
        set_text(ID_STATUS, text);
        win::live_region_changed(child(ID_STATUS));
    }
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

/// A rectangle of the design (pixels at 100 %) at the window's scale.
pub fn at(x: i32, y: i32, w: i32, h: i32) -> R {
    R::new(scale(x), scale(y), scale(w), scale(h))
}

/// Makes a child control at `r` (pixels). A negative id gives a label an id of its own.
pub fn control_at(parent: HWND, class: PCWSTR, text: &str, style: u32, r: R, id: i32) -> HWND {
    let id = if id < 0 { NEXT_LABEL.replace(NEXT_LABEL.get() + 1) } else { id };
    // SAFETY: the class name is a static string, the text outlives the call, and the "menu" of
    // a child window is its id, which Windows never dereferences.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(style) | WS_CHILD,
            r.x,
            r.y,
            r.w,
            r.h,
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            None,
            None,
        )
        .unwrap_or_default()
    };
    send(hwnd, WM_SETFONT, ui::font(Font::Body).0 as usize, 0);
    CONTROLS.with_borrow_mut(|c| c.push((hwnd, id, BUILDING.get())));
    hwnd
}

/// Makes a child control laid out at 96 dpi and scaled.
#[allow(clippy::too_many_arguments)]
pub fn control(parent: HWND, class: PCWSTR, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
    control_at(parent, class, text, style, at(x, y, w, h), id)
}

/// A label drawn by the window (a real static under it, for screen readers).
pub fn label(parent: HWND, id: i32, text: &str, r: R, look: ui::Label) {
    let h = control_at(parent, w!("STATIC"), text, SS_OWNERDRAW, r, id);
    widget::add(h, Kind::Label(look), None);
}

pub fn combo(parent: HWND, id: i32, x: i32, y: i32, w: i32) {
    let style = CBS_DROPDOWNLIST as u32 | CBS_OWNERDRAWFIXED | CBS_HASSTRINGS | WS_VSCROLL.0 | WS_TABSTOP.0;
    let h = control(parent, w!("COMBOBOX"), "", style, x, y, w, 28 + 10 * 28 + 8, id);
    send(h, CB_SETMINVISIBLE, 10, 0);
    send(h, CB_SETDROPPEDWIDTH, scale(w.max(200)) as usize, 0);
    // The closed box is as high as its selection field plus the frame: make it 28.
    let mut r = RECT::default();
    // SAFETY: reads the closed box's size.
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    let field = send(h, CB_GETITEMHEIGHT, usize::MAX, 0) as i32;
    send(h, CB_SETITEMHEIGHT, usize::MAX, (field + scale(28) - (r.bottom - r.top)) as isize);
    widget::add(h, Kind::Select, None);
}

pub fn button(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32) {
    let h = control(parent, w!("BUTTON"), text, BS_OWNERDRAW | WS_TABSTOP.0, x, y, w, 28, id);
    widget::add(h, Kind::Button { primary: false }, None);
}

/// A text box: a real edit, without a border, inside a 28 px frame the window paints. Its text
/// starts `inset` in from the frame's left.
pub fn edit(parent: HWND, id: i32, frame: R, inset: i32, style: u32) -> HWND {
    let line = widget::body_line();
    let r = R::new(frame.x + scale(inset), frame.y + (frame.h - line) / 2, frame.w - scale(inset + 10), line);
    let h = control_at(parent, w!("EDIT"), "", WS_TABSTOP.0 | ES_AUTOHSCROLL | style, r, id);
    widget::add(h, Kind::Edit, Some(frame));
    h
}

/// A switch: a real check box, painted as a track and a knob, as wide as its label needs.
pub fn toggle(parent: HWND, text: &str, x: i32, y: i32, id: i32) {
    let width = crate::ui::canvas::measure(text, ui::font(Font::Body), None).cx + scale(56);
    let r = R::new(scale(x), scale(y), width, scale(28));
    let h = control_at(parent, w!("BUTTON"), text, BS_AUTOCHECKBOX | WS_TABSTOP.0, r, id);
    widget::add(h, Kind::Toggle, None);
}

/// One item of a segmented control: a real push-like radio button.
pub fn segment(parent: HWND, text: &str, r: R, id: i32, first: bool) {
    let style = BS_AUTORADIOBUTTON | BS_PUSHLIKE | WS_TABSTOP.0 | if first { WS_GROUP } else { 0 };
    let h = control_at(parent, w!("BUTTON"), text, style, r, id);
    widget::add(h, Kind::Segment, None);
}

/// A list box inside a frame the window paints.
pub fn list(parent: HWND, id: i32, frame: R) {
    // Far enough in that the list's square corners stay inside the frame's rounded ones.
    let inset = ui::line() + (ui::radius(6) * 3 + 9) / 10;
    let style = LBS_OWNERDRAWFIXED | LBS_HASSTRINGS | LBS_NOINTEGRALHEIGHT | WS_VSCROLL.0 | WS_TABSTOP.0;
    let h = control_at(parent, w!("LISTBOX"), "", style, frame.grow(-inset), id);
    widget::add(h, Kind::List, Some(frame));
}

/// Moves a control to `x` (design pixels), keeping its size and row.
pub fn move_x(id: i32, x: i32) {
    let h = child(id);
    let y = widget::rect_in_parent(h).y;
    // SAFETY: SetWindowPos on a child of ours; size and z-order stay.
    unsafe {
        let _ = SetWindowPos(h, None, scale(x), y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Fills a dropdown, touching it only if its entries changed (an open list stays open).
pub fn fill_combo(id: i32, entries: Vec<(String, String)>, at: usize) {
    let h = child(id);
    let same = CHOICES.with_borrow(|c| c.get(&id) == Some(&entries));
    if !same {
        send(h, CB_RESETCONTENT, 0, 0);
        for (i, (label, value)) in entries.iter().enumerate() {
            send_text(h, CB_ADDSTRING, 0, label);
            // Entries that open a dialog are drawn quieter.
            send(h, CB_SETITEMDATA, i, value.starts_with('\u{1}') as isize);
        }
        CHOICES.with_borrow_mut(|c| c.insert(id, entries));
    }
    widget::set_value(h, at as i32);
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
    fn the_developer_scale_setting_is_read_with_care() {
        assert_eq!(dpi_from(None), None, "the display's own scale by default");
        assert_eq!(dpi_from(Some("144")), Some(144), "the developer setting wins");
        assert_eq!(dpi_from(Some("big")), None, "and is ignored when it is not a number");
        assert_eq!(dpi_from(Some("0")), None, "or not a scale Windows has");
    }
}
