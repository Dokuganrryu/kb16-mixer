//! The settings window: three tabs (Keys, Channels, Pad) of real Win32 controls with the look of
//! `crate::ui`, created when opened and destroyed when closed, so it costs nothing the rest of
//! the time.
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
//! The files: `layout` makes the controls, `scene` paints what lies under them, and `fill`
//! fills them from the file; `changes` applies what is changed in them, `apps` the Channels
//! tab's Add and Remove, `file` the edits of config.toml. `choices` and `checks` are the plain
//! logic behind them, `controls` and `ids` the Win32 parts they share. `preview` draws the
//! window hidden into a picture, for the design checks.

mod apps;
mod changes;
mod checks;
mod choices;
mod controls;
mod file;
mod fill;
mod ids;
mod layout;
mod preview;
mod scene;
#[cfg(test)]
mod window_tests;

pub use fill::{apps_moved, pad_changed, reloaded};
pub use preview::{Options, Picture, Preview, render_hidden};

use changes::changed;
use checks::Member;
use controls::{CONTROLS, child, dpi_setting, set_checked, status, system_dpi};
use fill::{catch_up, fill, problem_text};
use ids::*;
use layout::{HEIGHT, WIDTH, build, show_tab};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Controls::{ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
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
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
}

const STYLE: WINDOW_STYLE =
    WINDOW_STYLE(WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_CLIPCHILDREN.0);

/// The open settings window, if any (for the message loop's keyboard navigation).
pub fn window() -> Option<HWND> {
    let h = WINDOW.get();
    (!h.is_invalid()).then_some(h)
}

/// Shows tab `i`; by mouse its indicator slides there.
fn select_tab(i: usize) {
    let old = TAB.replace(i);
    for t in 0..TABS.len() {
        set_checked(ID_TAB + t as i32, t == i);
    }
    scene::slide(scene::PART_TAB, old, i);
    show_tab();
}

/// Ctrl+Tab and Ctrl+Shift+Tab, from any control: the next or previous tab, with the focus on it.
fn ctrl_tab(back: bool) {
    let i = (TAB.get() + if back { TABS.len() - 1 } else { 1 }) % TABS.len();
    select_tab(i);
    crate::win::set_focus(child(ID_TAB + i as i32));
}

/// The window's size for a client area of the design at `dpi`.
fn outer_size(dpi: i32) -> (i32, i32) {
    let mut rect = RECT { left: 0, top: 0, right: WIDTH * dpi / 96, bottom: HEIGHT * dpi / 96 };
    // SAFETY: AdjustWindowRectExForDpi only computes into `rect`.
    unsafe {
        let _ = AdjustWindowRectExForDpi(&mut rect, STYLE, false, WS_EX_CONTROLPARENT, dpi as u32);
    }
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// Makes every control again at the window's new scale, keeping the tab and the status line.
fn rebuild(hwnd: HWND) {
    let note = controls::text_of(ID_STATUS);
    let kids: Vec<HWND> = CONTROLS.with_borrow(|c| c.iter().map(|k| k.0).collect());
    for k in kids {
        // SAFETY: the children are ours, on this thread.
        unsafe {
            let _ = DestroyWindow(k);
        }
    }
    controls::forget_all();
    MEMBERS.with_borrow_mut(Vec::clear);
    build(hwnd);
    match fill(true) {
        Some(e) => status(&problem_text(&e)),
        None => status(&note),
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if let Some(r) = crate::ui::widget::parent_msg(hwnd, msg, wparam, lparam) {
        return r;
    }
    // SAFETY: Windows calls this on the window's own thread with the message's real parameters:
    // WM_DPICHANGED's lParam points to the suggested window rectangle.
    unsafe {
        match msg {
            WM_COMMAND => {
                let (id, code) = ((wparam.0 & 0xFFFF) as i32, (wparam.0 >> 16) as u32);
                if (ID_TAB..ID_TAB + TABS.len() as i32).contains(&id) {
                    if code == BN_CLICKED {
                        select_tab((id - ID_TAB) as usize);
                    }
                } else {
                    changed(hwnd, id, code);
                }
                LRESULT(0)
            }
            // The brightness slider: its value shows while it moves, and applies once released.
            WM_HSCROLL => {
                fill::show_brightness();
                if (wparam.0 & 0xFFFF) as u32 == TB_ENDTRACK {
                    changed(hwnd, ID_BRIGHTNESS, 0);
                }
                LRESULT(0)
            }
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE => {
                catch_up();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_SETTINGCHANGE | WM_THEMECHANGED | WM_SYSCOLORCHANGE => {
                if crate::ui::theme::refresh() {
                    crate::ui::widget::restyle(hwnd);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_DPICHANGED if dpi_setting().is_none() => {
                crate::ui::rescale((wparam.0 & 0xFFFF) as i32);
                let r = &*(lparam.0 as *const RECT);
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
                rebuild(hwnd);
                LRESULT(0)
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
                scene::stop_chip_timer();
                crate::ui::widget::drop_scene(hwnd);
                crate::ui::widget::on_ctrl_tab(None);
                controls::forget_all();
                MEMBERS.with_borrow_mut(Vec::clear);
                HIDDEN.with_borrow_mut(Vec::clear);
                SEEN.set(None);
                EDITING.set(None);
                LRESULT(0)
            }
            // The children are gone by now: the fonts they used can go too.
            WM_NCDESTROY => {
                crate::ui::end();
                DefWindowProcW(hwnd, msg, wparam, lparam)
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
    if let Some(existing) = window() {
        if let Some(t) = tab {
            select_tab(TABS.iter().position(|x| x.0 == t).unwrap_or(0));
        }
        if let Some(n) = note {
            status(n);
        }
        // SAFETY: the window is ours and on this thread.
        unsafe {
            let _ = SetForegroundWindow(existing);
        }
        return;
    }
    let Some(hwnd) = create(tab, note) else { return };
    // SAFETY: the window was just made on this thread.
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// Makes the window, hidden, with every control filled; None if Windows refused.
fn create(tab: Option<Tab>, note: Option<&str>) -> Option<HWND> {
    if let Some(t) = tab {
        TAB.set(TABS.iter().position(|x| x.0 == t).unwrap_or(0));
    }
    let instance = crate::win::instance();
    // SAFETY: plain Win32 calls on this thread. The structs passed carry their own sizes, the
    // class name outlives the calls, and the window procedure is `proc` above.
    unsafe {
        if !REGISTERED.replace(true) {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            });
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("kb16-mixer-settings"),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                ..Default::default()
            });
        }
        let first = dpi_setting().unwrap_or_else(system_dpi);
        let (w, h) = outer_size(first);
        crate::ui::begin(first);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("kb16-mixer-settings"),
            w!("KB16 mixer settings"),
            STYLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            w,
            h,
            None,
            None,
            Some(instance),
            None,
        ) else {
            crate::ui::end();
            crate::log("could not create the settings window");
            return None;
        };
        // On a display of another scale than the system's, the window takes that one.
        let dpi = dpi_setting().unwrap_or(GetDpiForWindow(hwnd) as i32);
        if dpi != first && dpi > 0 {
            crate::ui::rescale(dpi);
            let (w, h) = outer_size(dpi);
            let _ = SetWindowPos(hwnd, None, 0, 0, w, h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        WINDOW.set(hwnd);
        crate::ui::widget::set_scene(hwnd, scene::paint);
        crate::ui::widget::on_ctrl_tab(Some((hwnd, ctrl_tab)));
        crate::ui::theme::chrome(hwnd);
        build(hwnd);
        match fill(true) {
            Some(e) => status(&problem_text(&e)),
            None => status(note.unwrap_or(STATUS_IDLE)),
        }
        Some(hwnd)
    }
}
