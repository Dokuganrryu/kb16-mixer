//! The real window, made hidden and driven with the messages its controls send: each change is
//! applied, saved to config.toml and confirmed as before the restyle. Nothing is ever shown.
//! Skips (with a note) where Windows cannot make a window, as on a build machine without a
//! desktop.

use super::controls::{CONTROLS, child, text_of};
use super::ids::*;
use super::{PAGE, TAB, create, scene, window};
use crate::win::send;
use std::path::PathBuf;
use windows::Win32::System::Threading::{GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources};
use windows::Win32::UI::WindowsAndMessaging::*;

const CB_SETCURSEL: u32 = 0x014E;
const TBM_SETPOS: u32 = WM_USER + 5;

/// A throwaway settings folder with the default config, named before anything reads it.
fn test_folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kb16-mixer-window-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("config.toml"), crate::config::DEFAULT).expect("a test folder");
    // SAFETY: the only test that reads the settings folder; nothing else in this process reads
    // KB16_MIXER_DIR, and std serialises its own access to the environment.
    unsafe { std::env::set_var(crate::config::DIR_VAR, &dir) };
    assert_eq!(crate::config::custom_dir().as_deref(), Some(dir.as_path()), "never the real settings");
    dir
}

fn command(id: i32, code: u32) {
    let hwnd = window().expect("the window");
    send(hwnd, WM_COMMAND, (code as usize) << 16 | id as usize, child(id).0 as isize);
}

/// Picks entry `at` of a dropdown the way a user does: the selection, then its notification.
fn pick(id: i32, at: usize) {
    send(child(id), CB_SETCURSEL, at, 0);
    command(id, CBN_SELCHANGE);
}

fn config() -> String {
    std::fs::read_to_string(crate::config::path()).unwrap_or_default()
}

fn visible(id: i32) -> bool {
    // SAFETY: reads a child's style.
    unsafe { GetWindowLongPtrW(child(id), GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 }
}

fn gui_objects() -> (u32, u32) {
    // SAFETY: plain queries about this process.
    unsafe {
        (GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS), GetGuiResources(GetCurrentProcess(), GR_USEROBJECTS))
    }
}

fn destroy() {
    if let Some(h) = window() {
        // SAFETY: our own window, on this thread.
        unsafe {
            let _ = DestroyWindow(h);
        }
    }
}

#[test]
fn the_window_applies_and_saves_each_change_as_before() {
    let dir = test_folder();
    let Some(hwnd) = create(None, None) else {
        eprintln!("skipped: Windows could not make a window here (no desktop)");
        return;
    };
    // SAFETY: reads the style of our own window.
    let shown = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32 & WS_VISIBLE.0 != 0;
    assert!(!shown, "the window is made hidden; only open() shows it");
    assert!(!crate::ui::anim::timer_running(), "nothing moves at rest: no timer");

    // Keys: row 2, column 3 to F13 (entry 10 of the key list).
    pick(ID_KEY + 6, 10);
    let saved = crate::config::parse(&config()).expect("a valid config");
    assert_eq!(Ok(saved.pages[0][1][2].clone()), crate::config::parse_key("F13"), "saved where it was set");
    assert_eq!(text_of(ID_STATUS), "Page 1, row 2, Game column: F13. Saved and applied.");
    assert!(scene::chip_shown(), "the Saved chip confirms it");

    // A knob: the big knob to "Master volume" (after the four channels).
    pick(ID_KNOB + 2, 4);
    assert!(config().contains("master"), "{}", config());
    assert_eq!(text_of(ID_STATUS), "Saved and applied.");

    // Page 2 shows the second page's keys, and the page switch follows.
    command(ID_PAGE + 1, BN_CLICKED);
    assert_eq!(PAGE.get(), 1);

    // Tabs: a click on the second tab shows the Channels controls and hides the Keys ones.
    command(ID_TAB + 1, BN_CLICKED);
    assert_eq!(TAB.get(), 1);
    assert!(visible(ID_CH_KIND) && !visible(ID_KEY), "the Channels tab shows");
    super::ctrl_tab(false);
    assert_eq!(TAB.get(), 2, "Ctrl+Tab goes on to the Pad tab");
    super::ctrl_tab(true);
    assert_eq!(TAB.get(), 1, "and Ctrl+Shift+Tab back");

    // A refused change: two channels cannot both be "Everything else".
    pick(ID_CH_KIND + 3, 1);
    assert!(text_of(ID_STATUS).starts_with("Not changed:"), "{}", text_of(ID_STATUS));
    assert!(!scene::chip_shown(), "a refusal takes the chip away");

    // A channel name: typed, then applied on leaving the box, and refused when invalid.
    command(ID_CH_NAME + 1, EN_SETFOCUS);
    crate::win::set_window_text(child(ID_CH_NAME + 1), "Talk");
    command(ID_CH_NAME + 1, EN_KILLFOCUS);
    assert!(config().contains("name = \"Talk\""), "{}", config());
    assert_eq!(text_of(ID_STATUS), "Channel Chat is now called Talk. Saved and applied.");
    command(ID_CH_NAME + 1, EN_SETFOCUS);
    crate::win::set_window_text(child(ID_CH_NAME + 1), "Way too long");
    command(ID_CH_NAME + 1, EN_KILLFOCUS);
    assert!(text_of(ID_STATUS).starts_with("Not changed:"), "{}", text_of(ID_STATUS));

    // Pad: the knob step, the screen name with Enter, and the slider once released.
    command(ID_TAB + 2, BN_CLICKED);
    pick(ID_STEP, 3);
    assert!(config().contains("step = 5"), "{}", config());
    command(ID_NAME, EN_SETFOCUS);
    crate::win::set_window_text(child(ID_NAME), "DESK PC");
    command(IDOK.0, BN_CLICKED);
    assert!(config().contains("name = \"DESK PC\""), "Enter applies the name: {}", config());
    send(child(ID_BRIGHTNESS), TBM_SETPOS, 1, 40);
    send(hwnd, WM_HSCROLL, super::ids::TB_ENDTRACK as usize, child(ID_BRIGHTNESS).0 as isize);
    assert!(config().contains("brightness = 0.4"), "{}", config());
    assert_eq!(text_of(ID_BRIGHTNESS_VALUE), "40%");

    // Start with Windows stays off for a test copy, as before.
    command(ID_AUTOSTART, BN_CLICKED);
    assert!(text_of(ID_STATUS).contains("stays off"), "{}", text_of(ID_STATUS));

    // Owner-drawn buttons stay owner-drawn when a dialog makes one the default.
    let close = child(ID_CLOSE);
    send(close, 0x00F4, BS_DEFPUSHBUTTON as usize, 1); // BM_SETSTYLE
    // SAFETY: reads the style of our own control.
    let style = unsafe { GetWindowLongPtrW(close, GWL_STYLE) } as u32 & 0xF;
    assert_eq!(style, super::ids::BS_OWNERDRAW, "it keeps painting as ours");
    let code = send(close, WM_GETDLGCODE, 0, 0) as u32;
    assert_ne!(code & DLGC_DEFPUSHBUTTON, 0, "and Enter presses it");

    // Close is the same as the title bar's close.
    command(ID_CLOSE, BN_CLICKED);
    assert!(window().is_none(), "closed");
    assert!(CONTROLS.with_borrow(Vec::is_empty), "and its controls forgotten");
    assert!(!crate::ui::anim::timer_running(), "no timer is left running");

    // Opening and closing again and again leaves nothing behind.
    let before = gui_objects();
    for i in 0..20 {
        let Some(_) =
            create([None, Some(super::Tab::Channels), Some(super::Tab::Pad)].get(i % 3).copied().flatten(), None)
        else {
            break;
        };
        destroy();
    }
    let after = gui_objects();
    assert!(after.0 <= before.0 && after.1 <= before.1, "GDI and USER objects: {before:?} then {after:?}");
    let _ = std::fs::remove_dir_all(dir);
}
