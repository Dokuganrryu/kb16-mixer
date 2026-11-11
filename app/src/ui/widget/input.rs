//! What the controls do with the mouse and the keyboard on top of their own behaviour: hover
//! and press feedback, Ctrl+Tab from any control, Home and End in a segmented group, the
//! default button of a dialog, and the look of a select's open list.

use super::*;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass};
use windows::core::w;

/// Hover moves to `hwnd` (row `row` of a list), out of what had it.
fn hover(hwnd: HWND, row: i32) {
    let (old, old_row) = HOT.get();
    if (old, old_row) == (hwnd, row) {
        return;
    }
    HOT.set((hwnd, row));
    if old != hwnd {
        if !old.is_invalid() {
            fade(old, HOVER, 1.0, 0.0, -1);
        }
        if !hwnd.is_invalid() {
            win::track_leave(hwnd);
            fade(hwnd, HOVER, 0.0, 1.0, -1);
        }
    }
    if old_row >= 0 && !old.is_invalid() {
        fade(old, ROW + old_row as u32, 1.0, 0.0, old_row);
    }
    if row >= 0 {
        fade(hwnd, ROW + row as u32, 0.0, 1.0, row);
    }
}

fn fade(hwnd: HWND, part: u32, from: f32, to: f32, row: i32) {
    let paint = if row >= 0 {
        let mut r = RECT::default();
        win::send(hwnd, LB_GETITEMRECT, row as usize, &raw mut r as isize);
        Paint::Area(hwnd, r)
    } else {
        match with(hwnd, |e| (e.kind, e.frame)) {
            Some((Kind::Edit, Some(frame))) => Paint::Area(parent(hwnd), frame.rect()),
            _ => Paint::Own,
        }
    };
    anim::go(hwnd, part, from, to, HOVER_MS, Curve::Ease, paint);
}

/// Home and End in a segmented group: the first or last item of the group.
fn segment_end(hwnd: HWND, last: bool) {
    // SAFETY: walks this group's sibling windows; each call only reads the window tree.
    unsafe {
        let mut at = hwnd;
        let step = if last { GW_HWNDNEXT } else { GW_HWNDPREV };
        loop {
            let group = GetWindowLongPtrW(at, GWL_STYLE) as u32 & WS_GROUP.0 != 0;
            if !last && group {
                break;
            }
            match GetWindow(at, step) {
                Ok(next) if kind(next) == Some(Kind::Segment) => {
                    if last && GetWindowLongPtrW(next, GWL_STYLE) as u32 & WS_GROUP.0 != 0 {
                        break;
                    }
                    at = next;
                }
                _ => break,
            }
        }
        if at != hwnd {
            win::set_focus(at);
            win::send(at, BM_CLICK, 0, 0);
        }
    }
}

pub(super) unsafe extern "system" fn sub(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    // SAFETY: Windows calls this on the control's thread with the message's real parameters;
    // WM_GETDLGCODE's lParam is null or points to a MSG. Everything else is passed on.
    unsafe {
        let Some(kind) = kind(hwnd) else { return DefSubclassProc(hwnd, msg, wparam, lparam) };
        match msg {
            WM_MOUSEMOVE => {
                let row = if kind == Kind::List {
                    let at = win::send(hwnd, LB_ITEMFROMPOINT, 0, lparam.0);
                    if at >> 16 & 0xFFFF == 0 { (at & 0xFFFF) as i32 } else { -1 }
                } else {
                    -1
                };
                hover(hwnd, row);
            }
            WM_MOUSELEAVE => {
                if HOT.get().0 == hwnd {
                    hover(HWND::default(), -1);
                }
            }
            WM_LBUTTONDOWN => {
                KEYBOARD.set(false);
                if kind == Kind::Slider {
                    anim::go(hwnd, BIG, 0.0, 1.0, 100, Curve::Out, Paint::Own);
                }
            }
            WM_LBUTTONUP => {
                if kind == Kind::Slider {
                    anim::go(hwnd, BIG, 1.0, 0.0, 100, Curve::Out, Paint::Own);
                }
                CLICKING.set(hwnd);
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                CLICKING.set(HWND::default());
                return r;
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                KEYBOARD.set(true);
                let vk = wparam.0 as u16;
                if vk == VK_TAB
                    && win::key_down(VK_CONTROL as i32)
                    && let Some(f) = CTRL_TAB.get()
                {
                    f(win::key_down(VK_SHIFT as i32));
                    return LRESULT(0);
                }
                if kind == Kind::Segment && (vk == VK_HOME || vk == VK_END) {
                    segment_end(hwnd, vk == VK_END);
                    return LRESULT(0);
                }
            }
            WM_CHAR if wparam.0 == 9 && win::key_down(VK_CONTROL as i32) => return LRESULT(0),
            WM_GETDLGCODE => {
                let m = lparam.0 as *const MSG;
                let key = !m.is_null() && (*m).message == WM_KEYDOWN;
                if key {
                    KEYBOARD.set(true);
                }
                let mut code = match kind {
                    Kind::Button { .. } => {
                        let default = with(hwnd, |e| e.default).unwrap_or(false);
                        (DLGC_BUTTON | if default { DLGC_DEFPUSHBUTTON } else { DLGC_UNDEFPUSHBUTTON }) as isize
                    }
                    _ => DefSubclassProc(hwnd, msg, wparam, lparam).0,
                };
                let ctrl_tab = key && (*m).wParam.0 == VK_TAB as usize && win::key_down(VK_CONTROL as i32);
                if ctrl_tab && CTRL_TAB.get().is_some() {
                    code |= DLGC_WANTMESSAGE as isize;
                }
                return LRESULT(code);
            }
            // Dialogs make the focused button the default by restyling it; it stays owner-drawn.
            BM_SETSTYLE if matches!(kind, Kind::Button { .. }) => {
                with(hwnd, |e| e.default = wparam.0 as u32 & 0xF == BS_DEFPUSHBUTTON as u32);
                let style = (wparam.0 & !0xF) | BS_OWNERDRAW as usize;
                return DefSubclassProc(hwnd, msg, WPARAM(style), lparam);
            }
            BM_SETSTATE if matches!(kind, Kind::Button { .. }) => {
                let down = wparam.0 != 0;
                let ms = if KEYBOARD.get() {
                    0
                } else if down {
                    100
                } else {
                    140
                };
                anim::go(hwnd, PRESS, (!down) as i32 as f32, down as i32 as f32, ms, Curve::Out, Paint::Own);
            }
            BM_SETCHECK if kind == Kind::Toggle => {
                let was = win::send(hwnd, BM_GETCHECK, 0, 0) == 1;
                let now = wparam.0 == 1;
                if was != now && CLICKING.get() == hwnd && !KEYBOARD.get() {
                    anim::go(hwnd, ON, was as i32 as f32, now as i32 as f32, 160, Curve::Out, Paint::Own);
                }
            }
            WM_SETFOCUS | WM_KILLFOCUS => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                redraw_frame(hwnd);
                return r;
            }
            WM_PAINT if kind == Kind::Select => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                paint_control(hwnd, dc, None);
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            WM_PRINTCLIENT if kind == Kind::Select => {
                paint_control(hwnd, HDC(wparam.0 as *mut _), None);
                return LRESULT(0);
            }
            WM_ERASEBKGND if kind == Kind::Select => return LRESULT(1),
            WM_NCDESTROY => {
                ENTRIES.with_borrow_mut(|e| e.retain(|e| e.hwnd != hwnd));
                anim::forget(hwnd);
                if HOT.get().0 == hwnd {
                    HOT.set((HWND::default(), -1));
                }
                let _ = RemoveWindowSubclass(hwnd, Some(sub), SUBCLASS);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }
}

/// A select's open list: rounded with a border on Windows 11, a dark scrollbar in dark mode.
pub(super) fn dress_dropdown(combo: HWND) {
    let mut info = COMBOBOXINFO { cbSize: size_of::<COMBOBOXINFO>() as u32, ..Default::default() };
    // SAFETY: the struct carries its own size; the list belongs to the combobox and lives as
    // long as it does. Its subclass is removed when it is destroyed.
    unsafe {
        if GetComboBoxInfo(combo, &mut info).is_err() || info.hwndList.is_invalid() {
            return;
        }
        let rounded = crate::ui::theme::popup_chrome(info.hwndList);
        let _ = SetWindowSubclass(info.hwndList, Some(dropdown), SUBCLASS, rounded as usize);
    }
    scrollbar_theme(info.hwndList);
}

/// Dark scrollbars: the "DarkMode_Explorer" theme is not documented, but widely used; where it
/// is missing the scrollbar stays light, which is harmless.
pub(super) fn scrollbar_theme(hwnd: HWND) {
    let dark = theme().look == crate::ui::theme::Look::Dark;
    // SAFETY: SetWindowTheme only changes which visual style the window draws with.
    unsafe {
        let _ = SetWindowTheme(hwnd, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, None);
    }
}

/// A select's open list draws its 1 px frame in the control colours (Windows 11 draws the
/// rounded border itself, so the list's own frame takes the fill colour there).
unsafe extern "system" fn dropdown(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    rounded: usize,
) -> LRESULT {
    // SAFETY: as in `sub`. The window DC is released before returning.
    unsafe {
        match msg {
            WM_NCPAINT => {
                let r = DefSubclassProc(hwnd, msg, wparam, lparam);
                let dc = GetWindowDC(Some(hwnd));
                let mut wr = RECT::default();
                let _ = GetWindowRect(hwnd, &mut wr);
                let frame = RECT { left: 0, top: 0, right: wr.right - wr.left, bottom: wr.bottom - wr.top };
                let t = theme();
                let edge = if rounded != 0 { t.get(Tok::Control) } else { t.get(Tok::Cborder) };
                SetDCBrushColor(dc, edge.cr());
                let _ = FrameRect(dc, &frame, HBRUSH(GetStockObject(DC_BRUSH).0));
                ReleaseDC(Some(hwnd), dc);
                r
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(dropdown), SUBCLASS);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// The theme changed: the dropdowns and scrollbars follow, and everything repaints.
pub fn restyle(top: HWND) {
    let mine: Vec<(HWND, Kind)> =
        ENTRIES.with_borrow(|e| e.iter().filter(|e| parent(e.hwnd) == top).map(|e| (e.hwnd, e.kind)).collect());
    for (h, k) in mine {
        match k {
            Kind::Select => {
                let mut info = COMBOBOXINFO { cbSize: size_of::<COMBOBOXINFO>() as u32, ..Default::default() };
                // SAFETY: the struct carries its own size.
                if unsafe { GetComboBoxInfo(h, &mut info) }.is_ok() {
                    crate::ui::theme::popup_chrome(info.hwndList);
                    scrollbar_theme(info.hwndList);
                }
            }
            Kind::List => scrollbar_theme(h),
            _ => {}
        }
    }
    crate::ui::theme::chrome(top);
    anim::redraw(top, Paint::Own);
}
