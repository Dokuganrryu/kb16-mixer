//! Where every control of the window goes, and which of them the open tab shows.

use super::checks::CHANNEL_NAME_LEN;
use super::choices::{BRIGHTNESS_PERCENT, KNOB_NAMES, ROW_NAMES};
use super::controls::{BUILDING, CONTROLS, button, combo, control, label, scale, show};
use super::ids::*;
use super::{HIDDEN, PANE, PANE_BRUSH, STATUS_IDLE, TAB, TABS, Tab};
use crate::win::send;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateSolidBrush, DeleteDC, DeleteObject, GetDC, GetPixel, ReleaseDC,
    SelectObject,
};
use windows::Win32::UI::Controls::{TCIF_TEXT, TCITEMW, WC_TABCONTROLW};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PWSTR, w};

/// Client area at 96 dpi.
pub const WIDTH: i32 = 680;
pub const HEIGHT: i32 = 532;
const COL_X: i32 = 126;
const COL_W: i32 = 136;
const CH_X: i32 = 24;
const CH_W: i32 = 150;
const CH_STEP: i32 = 160;

pub fn build(hwnd: HWND) {
    let on = |tab: Tab| BUILDING.set(TABS.iter().position(|t| t.0 == tab));

    // ---- Keys
    on(Tab::Keys);
    let radio = BS_AUTORADIOBUTTON as u32 | BS_PUSHLIKE as u32 | WS_TABSTOP.0;
    control(hwnd, w!("BUTTON"), "Page 1", radio | WS_GROUP.0, 24, 46, 96, 28, ID_PAGE);
    control(hwnd, w!("BUTTON"), "Page 2", radio, 124, 46, 96, 28, ID_PAGE + 1);
    label(hwnd, "The pad starts on page 1. A Switch page key flips to the other page.", 232, 50, 420);
    for c in 0..4 {
        control(hwnd, w!("STATIC"), "", SS_CENTER, COL_X + c * COL_W, 88, COL_W - 8, 18, ID_COLUMN + c);
    }
    for r in 0..4 {
        label(hwnd, ROW_NAMES[r as usize], 24, 108 + r * 32, 100);
        for c in 0..4 {
            combo(hwnd, ID_KEY + r * 4 + c, COL_X + c * COL_W, 108 + r * 32, COL_W - 8);
        }
    }
    label(
        hwnd,
        "Keys that type (media keys, F13 to F24, shortcuts) are typed by the pad itself as normal keyboard input.",
        24,
        238,
        630,
    );
    for i in 0..3 {
        let x = 24 + i * 214;
        label(hwnd, KNOB_NAMES[i as usize], x, 276, 200);
        combo(hwnd, ID_KNOB + i, x, 298, 200);
    }
    label(
        hwnd,
        "Press a knob to pick one app in its channel. It goes back to the whole channel after 8 seconds.",
        24,
        332,
        630,
    );

    // ---- Channels
    on(Tab::Channels);
    for i in 0..4 {
        let x = CH_X + i * CH_STEP;
        control(hwnd, w!("STATIC"), "Name", 0, x, 46, CH_W, 18, ID_CH_LABEL + i * 2);
        let name = control(
            hwnd,
            w!("EDIT"),
            "",
            WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32,
            x,
            66,
            CH_W,
            24,
            ID_CH_NAME + i,
        );
        send(name, EM_LIMITTEXT, CHANNEL_NAME_LEN, 0);
        control(hwnd, w!("STATIC"), "Plays", 0, x, 98, CH_W, 18, ID_CH_LABEL + i * 2 + 1);
        combo(hwnd, ID_CH_KIND + i, x, 118, CH_W);
        control(hwnd, w!("STATIC"), "", 0, x, 150, CH_W, 34, ID_CH_ABOUT + i);
        let list = WS_BORDER.0 | WS_VSCROLL.0 | WS_TABSTOP.0 | LBS_NOINTEGRALHEIGHT as u32;
        control(hwnd, w!("LISTBOX"), "", list, x, 188, CH_W, 176, ID_CH_LIST + i);
        button(hwnd, "Add...", x, 370, 72, ID_CH_ADD + i);
        button(hwnd, "Remove", x + 78, 370, 72, ID_CH_REMOVE + i);
    }
    label(hwnd, "Windows has one volume slider per program. All tabs of one browser move together.", 24, 404, 630);
    label(
        hwnd,
        "(moved) = put there with the pad's Move key, which wins over the lists. Remove undoes it.",
        24,
        424,
        630,
    );

    // ---- Pad
    on(Tab::Pad);
    let rows = [
        "Name on the screen",
        "Light brightness",
        "Knob step",
        "Focus turns the others down to",
        "Screen shift",
        "Screen off after",
        "Dim lights after",
    ];
    for (i, text) in rows.iter().enumerate() {
        label(hwnd, text, 24, 50 + i as i32 * 36, 200);
    }
    let name = control(
        hwnd,
        w!("EDIT"),
        "",
        WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32 | ES_UPPERCASE as u32,
        232,
        50,
        160,
        24,
        ID_NAME,
    );
    send(name, EM_LIMITTEXT, kb16_mixer_protocol::LOGO_NAME_LEN, 0);
    label(hwnd, "1 to 10 letters, digits or spaces", 404, 50, 250);
    let bar = control(hwnd, w!("msctls_trackbar32"), "", WS_TABSTOP.0, 226, 84, 250, 30, ID_BRIGHTNESS);
    send(bar, TBM_SETRANGE, 1, (BRIGHTNESS_PERCENT.1 << 16) | BRIGHTNESS_PERCENT.0);
    combo(hwnd, ID_STEP, 232, 122, 180);
    combo(hwnd, ID_DUCK, 232, 158, 180);
    combo(hwnd, ID_SHIFT, 232, 194, 180);
    control(hwnd, w!("STATIC"), "Moves the picture down if its top is cut off.", 0, 424, 197, 240, 36, -1);
    combo(hwnd, ID_SCREEN_OFF, 232, 230, 180);
    combo(hwnd, ID_LIGHTS_DIM, 232, 266, 180);
    control(hwnd, w!("STATIC"), "Counted while nothing plays or is touched.", 0, 424, 236, 240, 40, -1);
    control(
        hwnd,
        w!("BUTTON"),
        "Start with Windows",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        24,
        308,
        300,
        24,
        ID_AUTOSTART,
    );
    button(hwnd, "Open config file", 24, 346, 160, ID_OPEN_FILE);
    button(hwnd, "Open settings folder", 192, 346, 180, ID_OPEN_FOLDER);
    control(hwnd, w!("STATIC"), "", 0, 24, 392, 630, 18, ID_VERSION);

    // ---- around the tabs. The tab strip comes last so it sits under the pages.
    BUILDING.set(None);
    control(hwnd, w!("STATIC"), STATUS_IDLE, 0, 16, 468, 548, 54, ID_STATUS);
    button(hwnd, "Close", 572, 470, 92, ID_CLOSE);
    let tabs = control(hwnd, WC_TABCONTROLW, "", WS_CLIPSIBLINGS.0 | WS_TABSTOP.0, 8, 8, WIDTH - 16, 448, ID_TABS);
    for (i, (_, name)) in TABS.iter().enumerate() {
        let mut text: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let item = TCITEMW { mask: TCIF_TEXT, pszText: PWSTR(text.as_mut_ptr()), ..Default::default() };
        // SAFETY: the item and its NUL-terminated text live until the call returns; the tab
        // control copies the text.
        unsafe { SendMessageW(tabs, TCM_INSERTITEMW, Some(WPARAM(i)), Some(LPARAM(&raw const item as isize))) };
    }
    send(tabs, TCM_SETCURSEL, TAB.get(), 0);
    PANE.set(pane_color(tabs));
    if let Some(color) = PANE.get() {
        // SAFETY: a plain GDI call; the window deletes the brush when it is destroyed.
        PANE_BRUSH.set(unsafe { CreateSolidBrush(color) });
    }
}

/// The colour the tab control fills its pages with. It differs between Windows versions and
/// themes, so it is read from the control itself, once.
fn pane_color(tabs: HWND) -> Option<COLORREF> {
    const PRF_CLIENT: isize = 4;
    const PRF_ERASEBKGND: isize = 8;
    // SAFETY: GDI calls on a memory DC and bitmap made here, freed here in reverse order with
    // the DC's old bitmap put back first; the screen DC is released once.
    unsafe {
        let mut r = RECT::default();
        GetClientRect(tabs, &mut r).ok()?;
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, r.right, r.bottom);
        let old = SelectObject(dc, bitmap.into());
        send(tabs, WM_PRINTCLIENT, dc.0 as usize, PRF_CLIENT | PRF_ERASEBKGND);
        let color = GetPixel(dc, r.right / 2, r.bottom - scale(24));
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        (color.0 != 0xFFFF_FFFF).then_some(color)
    }
}

/// Shows the controls of the open tab, minus those hidden by what they show.
pub fn show_tab() {
    let tab = TAB.get();
    let hidden = HIDDEN.with_borrow(Clone::clone);
    CONTROLS.with_borrow(|controls| {
        for &(h, id, on) in controls {
            show(h, on.is_none_or(|t| t == tab) && !hidden.contains(&id));
        }
    });
}
