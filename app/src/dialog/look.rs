//! The dialogs' look: the same colours, fonts and controls as the settings window. The templates
//! give Windows the controls and their order; here they are placed in pixels (the design's, at
//! the window's scale), given their look, and the dialog paints its background.

use crate::ui::canvas::{Canvas, R};
use crate::ui::theme::{self, Tok, theme};
use crate::ui::widget::{self, Kind};
use crate::ui::{self, Align, Font, Label, px};
use crate::win;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The dialogs' prompt, their hints, and a hint that warns.
pub const PROMPT: Label = Label { font: Font::Body, color: Tok::Text, align: Align::Left, wrap: false, quiet: None };
pub const HINT: Label = Label { font: Font::Small, color: Tok::Text2, align: Align::Left, wrap: true, quiet: None };
pub const WARN: Label = Label { color: Tok::Amber, ..HINT };

/// One control of a dialog: its id, where it goes (design pixels) and how it looks. An edit's
/// box is its frame; the edit itself goes inside it.
pub struct Place {
    pub id: u16,
    pub at: R,
    pub kind: Kind,
}

/// The dialog's start: chrome, fonts, looks and places, a client area of `size`, and the
/// dialog centred on the display where its owner is.
pub fn dress(dlg: HWND, size: (i32, i32), places: &[Place]) {
    ui::begin(ui::dpi());
    widget::set_scene(dlg, scene);
    theme::chrome(dlg);
    for p in places {
        let Some(h) = win::dlg_item(Some(dlg), p.id as i32) else { continue };
        let at = R::new(px(p.at.x), px(p.at.y), px(p.at.w), px(p.at.h));
        let (r, frame) = match p.kind {
            Kind::Edit => {
                let line = widget::body_line();
                (R::new(at.x + px(10), at.y + (at.h - line) / 2, at.w - px(20), line), Some(at))
            }
            _ => (at, None),
        };
        let font = match p.kind {
            Kind::Button { primary: true } => Font::BodyStrong,
            _ => Font::Body,
        };
        win::send(h, WM_SETFONT, ui::font(font).0 as usize, 0);
        // SAFETY: a child of our own dialog, moved and sized; nothing else changes.
        unsafe {
            let _ = SetWindowPos(h, None, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        widget::add(h, p.kind, frame);
        if p.kind == (Kind::Button { primary: true }) {
            widget::set_default(h);
        }
    }
    // SAFETY: plain Win32 calls on our own dialog; the structs carry their sizes.
    unsafe {
        let style = WINDOW_STYLE(GetWindowLongPtrW(dlg, GWL_STYLE) as u32);
        let ex = WINDOW_EX_STYLE(GetWindowLongPtrW(dlg, GWL_EXSTYLE) as u32);
        let mut outer = RECT { left: 0, top: 0, right: px(size.0), bottom: px(size.1) };
        let _ = windows::Win32::UI::HiDpi::AdjustWindowRectExForDpi(&mut outer, style, false, ex, ui::dpi() as u32);
        let (w, h) = (outer.right - outer.left, outer.bottom - outer.top);
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let owner = GetWindow(dlg, GW_OWNER).unwrap_or(dlg);
        let _ = GetMonitorInfoW(MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST), &mut info);
        let work = info.rcWork;
        let (x, y) = (work.left + (work.right - work.left - w) / 2, work.top + (work.bottom - work.top - h) / 2);
        let _ = SetWindowPos(dlg, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// What a dialog paints under its controls.
fn scene(cv: &mut Canvas, dlg: HWND) {
    cv.fill(cv.area(), theme().get(Tok::Bg));
    widget::paint_frames(cv, dlg);
    widget::paint_ring(cv, dlg);
}

/// A dialog procedure's share of the look; Some(result) for the messages it answered.
pub fn dialog_msg(dlg: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<isize> {
    match msg {
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
            if theme::refresh() {
                widget::restyle(dlg);
            }
            return None;
        }
        WM_NCDESTROY => {
            widget::drop_scene(dlg);
            ui::end();
            return None;
        }
        _ => {}
    }
    let r = widget::parent_msg(dlg, msg, wparam, lparam)?.0;
    Some(match msg {
        // These hand back a brush as the dialog procedure's own result.
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORDLG => r,
        _ => {
            // SAFETY: DWLP_MSGRESULT is where a dialog procedure leaves a message's result.
            unsafe {
                SetWindowLongPtrW(dlg, WINDOW_LONG_PTR_INDEX(DWLP_MSGRESULT as i32), r);
            }
            1
        }
    })
}
