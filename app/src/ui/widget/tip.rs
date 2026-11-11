//! Tooltips with the whole text of a select or the status line, for when it does not fit and
//! ends in "...". One tooltip window per window, made hidden; Windows shows it on hover.

use super::*;
use windows::core::PWSTR;

/// LPSTR_TEXTCALLBACKW: the tooltip asks for its text each time it shows.
const TEXT_CALLBACK: PWSTR = PWSTR(-1isize as *mut u16);

thread_local! {
    /// The text of the tooltip being shown: Windows reads it after the notification returns.
    static SHOWN: RefCell<Vec<u16>> = const { RefCell::new(Vec::new()) };
    /// The tooltip window of each window, replaced when the window makes its controls again.
    static TIPS: RefCell<Vec<(HWND, HWND)>> = const { RefCell::new(Vec::new()) };
}

/// Gives each of `tools` (children of `top`) a tooltip.
pub fn tooltips(top: HWND, tools: &[HWND]) {
    // SAFETY: the tooltip is a hidden popup owned by `top`, so it is destroyed with it; each
    // TTTOOLINFOW carries its own size and is copied by the tooltip.
    unsafe {
        let style = WS_POPUP.0 | TTS_ALWAYSTIP | TTS_NOPREFIX;
        let Ok(tip) = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            None,
            WINDOW_STYLE(style),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            Some(top),
            None,
            Some(win::instance()),
            None,
        ) else {
            return;
        };
        let old = TIPS.with_borrow_mut(|t| {
            t.retain(|(owner, _)| IsWindow(Some(*owner)).as_bool());
            let old = t.iter().position(|(owner, _)| *owner == top).map(|i| t.remove(i).1);
            t.push((top, tip));
            old
        });
        if let Some(old) = old {
            let _ = DestroyWindow(old);
        }
        win::send(tip, TTM_SETMAXTIPWIDTH, 0, px(480) as isize);
        for &tool in tools {
            let info = TTTOOLINFOW {
                cbSize: size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd: top,
                uId: tool.0 as usize,
                lpszText: TEXT_CALLBACK,
                ..Default::default()
            };
            win::send(tip, TTM_ADDTOOLW, 0, &raw const info as isize);
        }
    }
}

/// The whole text of a control, if it is cut short where it shows; else nothing.
fn clipped(tool: HWND) -> Option<String> {
    let r = rect_in_parent(tool);
    match kind(tool)? {
        Kind::Select => {
            let at = win::send(tool, CB_GETCURSEL, 0, 0);
            let text = paint::item_text(tool, CB_GETLBTEXTLEN, CB_GETLBTEXT, usize::try_from(at).ok()?);
            let width = super::super::canvas::measure(&text, font(Font::Body), None).cx;
            (width > r.w - px(36)).then_some(text)
        }
        Kind::Status => {
            let text = win::window_text(tool);
            let lines = super::super::line_of(Font::Small) * 3;
            let height = super::super::canvas::measure(&text, font(Font::Small), Some(r.w - px(16))).cy;
            (height > lines).then_some(text)
        }
        _ => None,
    }
}

/// Answers TTN_GETDISPINFOW: the text, or an empty one (no tooltip) when all of it shows.
pub(super) fn text(d: &mut NMTTDISPINFOW) {
    let text = clipped(HWND(d.hdr.idFrom as *mut _)).unwrap_or_default();
    SHOWN.with_borrow_mut(|s| {
        *s = text.encode_utf16().chain([0]).collect();
        d.lpszText = PWSTR(s.as_mut_ptr());
    });
}
