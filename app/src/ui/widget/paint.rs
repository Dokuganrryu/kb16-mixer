//! Painting the controls: each whole control over its parent's scene, and the rows of lists
//! and of a select's open list.

use super::*;

/// Paints a whole control (not a list row) into `dc`, the control's own.
pub(super) fn paint_control(hwnd: HWND, dc: HDC, item_state: Option<ODS_FLAGS>) {
    let Some(kind) = kind(hwnd) else { return };
    if !worth_painting(hwnd) {
        return;
    }
    let Some((mut cv, r)) = backdrop(hwnd) else { return };
    let text = win::window_text(hwnd);
    match kind {
        Kind::Label(l) => draw::label(&mut cv, r, &text, &l),
        Kind::Status => {
            let warn = text.starts_with("Not changed:") || text.starts_with("config.toml has a problem");
            draw::status(&mut cv, r, &text, warn);
        }
        Kind::Button { primary } => {
            let state = item_state.unwrap_or_default();
            let s = ButtonState {
                primary,
                hover: hover_of(hwnd),
                press: anim::value(hwnd, PRESS, has(state, ODS_SELECTED) as i32 as f32),
                enabled: !has(state, ODS_DISABLED),
            };
            draw::button(&mut cv, r, &text, &s);
        }
        Kind::Select => {
            let at = win::send(hwnd, CB_GETCURSEL, 0, 0);
            let value =
                if at >= 0 { item_text(hwnd, CB_GETLBTEXTLEN, CB_GETLBTEXT, at as usize) } else { String::new() };
            let open = win::send(hwnd, CB_GETDROPPEDSTATE, 0, 0) != 0;
            draw::select(&mut cv, r, &value, hover_of(hwnd), open);
        }
        Kind::Toggle => {
            let on = win::send(hwnd, BM_GETCHECK, 0, 0) == 1;
            draw::toggle(&mut cv, r, &text, anim::value(hwnd, ON, on as i32 as f32), hover_of(hwnd));
        }
        Kind::Segment => {
            let on = win::send(hwnd, BM_GETCHECK, 0, 0) == 1;
            draw::segment(&mut cv, r, &text, on, hover_of(hwnd));
        }
        Kind::Slider => {
            let mut channel = RECT::default();
            let mut thumb = RECT::default();
            win::send(hwnd, TBM_GETCHANNELRECT, 0, &raw mut channel as isize);
            win::send(hwnd, TBM_GETTHUMBRECT, 0, &raw mut thumb as isize);
            let rail = R::new(r.x + channel.left, r.y, channel.right - channel.left, r.h);
            let big = anim::value(hwnd, BIG, 0.0);
            draw::slider(&mut cv, rail, r.x + (thumb.left + thumb.right) / 2, big);
        }
        Kind::List | Kind::Edit => {}
    }
    cv.present(dc, (0, 0));
}

/// The text of item `at` of a combobox or list (by its length and text messages).
pub(super) fn item_text(hwnd: HWND, len_msg: u32, text_msg: u32, at: usize) -> String {
    let n = win::send(hwnd, len_msg, at, 0);
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    // SAFETY: the buffer holds the length the control reported plus the NUL it writes.
    let n = unsafe { SendMessageW(hwnd, text_msg, Some(WPARAM(at)), Some(LPARAM(buf.as_mut_ptr() as isize))).0 };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

/// One row of a list box or of a select's open list.
pub(super) fn paint_row(d: &DRAWITEMSTRUCT) {
    if d.itemID == u32::MAX {
        return;
    }
    let mut cv = match Canvas::new(R::of(d.rcItem)) {
        Some(cv) => cv,
        None => return,
    };
    let r = R::of(d.rcItem);
    let selected = has(d.itemState, ODS_SELECTED);
    let at = d.itemID as usize;
    if d.CtlType == ODT_COMBOBOX {
        let text = item_text(d.hwndItem, CB_GETLBTEXTLEN, CB_GETLBTEXT, at);
        let current = with(d.hwndItem, |e| e.value).unwrap_or(-1) == d.itemID as i32;
        let quiet = win::send(d.hwndItem, CB_GETITEMDATA, at, 0) == 1;
        draw::popup_row(&mut cv, r, &text, selected, current, quiet);
    } else {
        let text = item_text(d.hwndItem, LB_GETTEXTLEN, LB_GETTEXT, at);
        let hover = anim::value(d.hwndItem, ROW + d.itemID, (HOT.get() == (d.hwndItem, d.itemID as i32)) as i32 as f32);
        draw::list_row(&mut cv, r, &text, selected, hover);
    }
    cv.present(d.hDC, (r.x, r.y));
}
