//! The settings window: three tabs (Keys, Channels, Pad) of plain Win32 controls, created when
//! opened and destroyed when closed, so it costs nothing the rest of the time.
//!
//! Every change is written into config.toml with its comments and layout kept, but only once
//! the app's own parser accepts the result: a change that would stop the file from loading is
//! refused with the reason in the status line, and a file that already fails to load is never
//! written. The file is then loaded through the same path as "Reload config". The controls are
//! filled from the file whenever it changes (hand edits included), so they show what is in force.
//!
//! Apps moved with the pad's Move key live in apps.toml, which only the mixer thread writes. The
//! Channels tab shows every app where the mixer puts it (`audio::assign`, the mixer's own rule),
//! marks the moved ones, and asks the mixer to forget a move when one is removed.
//!
//! The files: `layout` makes the controls and `fill` fills them from the file; `changes`
//! applies what is changed in them, `apps` the Channels tab's Add and Remove, `file` the edits
//! of config.toml. `choices` and `checks` are the plain logic behind them, `controls` and `ids`
//! the Win32 parts they share.

mod apps;
mod changes;
mod checks;
mod choices;
mod controls;
mod file;
mod fill;
mod ids;
mod layout;

pub use fill::{apps_moved, pad_changed, reloaded};

use changes::changed;
use checks::Member;
use controls::{CONTROLS, FONT, child, dpi, font_height, scale, status, system_dpi};
use fill::{catch_up, fill, problem_text};
use ids::*;
use layout::{HEIGHT, WIDTH, build, show_tab};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CreateFontIndirectW, DeleteObject, GetSysColor, GetSysColorBrush, HBRUSH, HDC, SetBkColor,
};
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR, TCN_SELCHANGE,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// The tabs, left to right.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Keys,
    Channels,
    Pad,
}

const TABS: [(Tab, &str); 3] = [(Tab::Keys, "Keys"), (Tab::Channels, "Channels"), (Tab::Pad, "Pad")];

const STATUS_IDLE: &str = "Changes apply the moment you make them (a name once you press Enter or leave its box) and are saved to config.toml.";

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static TAB: Cell<usize> = const { Cell::new(0) };
    static PAGE: Cell<usize> = const { Cell::new(0) };
    /// Controls hidden by what they show (a channel that does not exist, a list a kind has not).
    static HIDDEN: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static MEMBERS: RefCell<Vec<Vec<Member>>> = const { RefCell::new(Vec::new()) };
    /// The config text the controls were last filled from.
    static SEEN: RefCell<Option<String>> = const { RefCell::new(None) };
    /// The text box being typed in, if any: Enter applies it.
    static EDITING: Cell<Option<i32>> = const { Cell::new(None) };
    /// The tab pages' colour, for the labels, tick boxes and slider on them.
    static PANE: Cell<Option<COLORREF>> = const { Cell::new(None) };
    static PANE_BRUSH: Cell<HBRUSH> = const { Cell::new(HBRUSH(std::ptr::null_mut())) };
}

/// The open settings window, if any (for the message loop's keyboard navigation).
pub fn window() -> Option<HWND> {
    let h = WINDOW.get();
    (!h.is_invalid()).then_some(h)
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: Windows calls this on the window's own thread with the message's real parameters:
    // WM_NOTIFY's lParam points to an NMHDR, and the WM_CTLCOLOR* wParam is a live DC.
    unsafe {
        match msg {
            WM_COMMAND => {
                changed(hwnd, (wparam.0 & 0xFFFF) as i32, (wparam.0 >> 16) as u32);
                LRESULT(0)
            }
            WM_NOTIFY => {
                let hdr = &*(lparam.0 as *const NMHDR);
                if hdr.idFrom == ID_TABS as usize && hdr.code == TCN_SELCHANGE {
                    TAB.set(crate::win::send(child(ID_TABS), TCM_GETCURSEL, 0, 0).max(0) as usize);
                    show_tab();
                }
                LRESULT(0)
            }
            // The brightness slider, once the mouse or key is released.
            WM_HSCROLL if (wparam.0 & 0xFFFF) as u32 == TB_ENDTRACK => {
                changed(hwnd, ID_BRIGHTNESS, 0);
                LRESULT(0)
            }
            // Labels, tick boxes and the slider take the colour of the tab page under them.
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                let from = HWND(lparam.0 as *mut _);
                let on_page = CONTROLS.with_borrow(|c| c.iter().any(|(h, _, tab)| *h == from && tab.is_some()));
                let (color, brush) = match PANE.get() {
                    Some(c) if on_page => (c, PANE_BRUSH.get()),
                    _ => (COLORREF(GetSysColor(COLOR_WINDOW)), GetSysColorBrush(COLOR_WINDOW)),
                };
                SetBkColor(HDC(wparam.0 as *mut _), color);
                LRESULT(brush.0 as isize)
            }
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE => {
                catch_up();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                if let Some(id) = EDITING.get() {
                    changed(hwnd, id, EN_KILLFOCUS);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                WINDOW.set(HWND::default());
                controls::forget_all();
                MEMBERS.with_borrow_mut(Vec::clear);
                HIDDEN.with_borrow_mut(Vec::clear);
                SEEN.set(None);
                EDITING.set(None);
                if !PANE_BRUSH.get().is_invalid() {
                    let _ = DeleteObject(PANE_BRUSH.replace(HBRUSH::default()).into());
                }
                PANE.set(None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Opens the settings window, or brings it forward if it is already open.
pub fn open() {
    open_at(None, None);
}

/// Opens the window on `tab` (or where it was), with `note` in the status line.
pub fn open_at(tab: Option<Tab>, note: Option<&str>) {
    if let Some(t) = tab {
        TAB.set(TABS.iter().position(|x| x.0 == t).unwrap_or(0));
    }
    // SAFETY: plain Win32 calls on this thread. The structs passed carry their own sizes, the
    // font and class name outlive the calls, and the window procedure is `proc` above.
    unsafe {
        if let Some(existing) = window() {
            crate::win::send(child(ID_TABS), TCM_SETCURSEL, TAB.get(), 0);
            show_tab();
            if let Some(n) = note {
                status(n);
            }
            let _ = SetForegroundWindow(existing);
            return;
        }
        let instance = crate::win::instance();
        if FONT.get().is_invalid() {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES | ICC_TAB_CLASSES,
            });
            let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
            let _ = SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                Some((&raw mut metrics).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let font = &mut metrics.lfMessageFont;
            font.lfHeight = font_height(font.lfHeight, dpi(), system_dpi());
            FONT.set(CreateFontIndirectW(font));
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("kb16-mixer-settings"),
                hbrBackground: GetSysColorBrush(COLOR_WINDOW),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                ..Default::default()
            });
        }
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT { left: 0, top: 0, right: scale(WIDTH), bottom: scale(HEIGHT) };
        let _ = AdjustWindowRect(&mut rect, style, false);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("kb16-mixer-settings"),
            w!("KB16 mixer settings"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return crate::log("could not create the settings window");
        };
        WINDOW.set(hwnd);
        build(hwnd);
        match fill(true) {
            Some(e) => status(&problem_text(&e)),
            None => status(note.unwrap_or(STATUS_IDLE)),
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}
