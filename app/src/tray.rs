//! The main thread: the tray icon and its menu, and a hidden window for what other threads and
//! Windows tell it.

use crate::pad::{self, PadEvent};
use crate::{VERSION, clip, config, reload, settings, tell_mixer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, w};

pub const WM_PAD_STATUS: u32 = WM_APP + 1;
/// The mixer moved an app with the Move key (the settings window shows it).
pub const WM_APPS_MOVED: u32 = WM_APP + 2;

/// How this run started, for the tray.
pub struct Start {
    pub first_run: bool,
    pub no_pad: bool,
    pub problem: Option<String>,
}

/// A plain knob: light disc, dark tick. Drawn in code so there are no asset files.
fn icon() -> Result<Icon, tray_icon::BadIcon> {
    let n = 32usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
            let r = (dx * dx + dy * dy).sqrt();
            let tick = dx.abs() < 1.6 && (-11.0..-4.0).contains(&dy);
            let rgba = if r > 15.0 {
                [0, 0, 0, 0]
            } else if r > 12.8 {
                [150, 170, 215, 255]
            } else if tick {
                [34, 38, 48, 255]
            } else {
                [228, 234, 248, 255]
            };
            px[(y * n + x) * 4..][..4].copy_from_slice(&rgba);
        }
    }
    Icon::from_rgba(px, n as u32, n as u32)
}

/// The tray's first line: the app's version and the pad.
fn status_line(no_pad: bool) -> String {
    let pad = match pad::connected_release() {
        _ if no_pad => "pad not used (--no-pad)".to_string(),
        None => "pad not connected".into(),
        Some(kb16_mixer_protocol::UNVERSIONED_RELEASE) => "pad connected".into(),
        Some(r) => {
            let (a, b, c) = kb16_mixer_protocol::release_version(r);
            format!("pad connected, firmware {a}.{b}.{c}")
        }
    };
    format!("KB16 mixer {VERSION}: {pad}")
}

/// The tray menu's items.
struct Items {
    status: MenuItem,
    problem: MenuItem,
    open_settings: MenuItem,
    pause: CheckMenuItem,
    open: MenuItem,
    reload: MenuItem,
    folder: MenuItem,
    autostart: CheckMenuItem,
    quit: MenuItem,
}

impl Items {
    fn new(no_pad: bool) -> Items {
        Items {
            status: MenuItem::new(status_line(no_pad), false, None),
            problem: MenuItem::new("", false, None),
            open_settings: MenuItem::new("Settings...", true, None),
            pause: CheckMenuItem::new("Pause (pad acts as a plain keyboard)", true, false, None),
            open: MenuItem::new("Open config file", true, None),
            reload: MenuItem::new("Reload config file", true, None),
            folder: MenuItem::new("Open settings folder", true, None),
            autostart: match config::custom_dir() {
                Some(_) => CheckMenuItem::new("Start with Windows (off: KB16_MIXER_DIR is set)", false, false, None),
                None => CheckMenuItem::new("Start with Windows", true, crate::autostart_enabled(), None),
            },
            quit: MenuItem::new("Quit", true, None),
        }
    }

    fn menu(&self) -> Result<Menu, tray_icon::menu::Error> {
        let menu = Menu::new();
        menu.append(&self.status)?;
        menu.append(&self.problem)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&self.open_settings)?;
        menu.append(&self.pause)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&self.open)?;
        menu.append(&self.reload)?;
        menu.append(&self.folder)?;
        menu.append(&self.autostart)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&self.quit)?;
        Ok(menu)
    }

    fn show_problem(&self, text: Option<&str>) {
        match text {
            Some(t) => self.problem.set_text(format!("Config problem: {}", clip(t, 90))),
            None => self.problem.set_text("Config OK"),
        }
    }
}

pub fn run(
    paused: Arc<AtomicBool>,
    to_mixer: Sender<PadEvent>,
    start: Start,
) -> Result<(), Box<dyn std::error::Error>> {
    let items = Items::new(start.no_pad);
    items.show_problem(start.problem.as_deref());

    // Left click opens the settings window; right click shows the menu.
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(items.menu()?))
        .with_menu_on_left_click(false)
        .with_tooltip("KB16 mixer")
        .with_icon(icon()?)
        .build()?;

    open_first_window(start.first_run);
    follow_pad_status(&tray, &items, start.no_pad);

    let mut msg = MSG::default();
    // SAFETY: `msg` is a live MSG for every call; the settings window handle comes from the
    // settings module and is only used while that window exists.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // Tab / arrow-key navigation inside the settings window.
            if settings::window().is_some_and(|w| IsDialogMessageW(w, &msg).as_bool()) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);

            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                    settings::open();
                }
            }
            while let Ok(ev) = MenuEvent::receiver().try_recv() {
                on_menu(&ev, &items, &paused, &to_mixer, start.no_pad);
            }
        }
    }
    // It holds the tray icon, which is only taken off the taskbar when the last copy goes.
    PAD_STATUS.set(None);
    Ok(())
}

/// The first run opens the window where Start with Windows is, and says where the app lives.
/// `--settings[=tab]` starts with the window open too.
fn open_first_window(first_run: bool) {
    let asked = std::env::args().find_map(|a| a.strip_prefix("--settings").map(str::to_string));
    if first_run {
        let welcome = "KB16 mixer is running. It lives in the tray by the clock (maybe behind the ^ arrow): \
                       click its icon to come back here. It does not start with Windows unless you tick that below.";
        settings::open_at(Some(settings::Tab::Pad), Some(welcome));
    } else if let Some(tab) = asked {
        let tab = match tab.trim_start_matches('=') {
            "channels" => Some(settings::Tab::Channels),
            "pad" => Some(settings::Tab::Pad),
            "keys" => Some(settings::Tab::Keys),
            _ => None,
        };
        settings::open_at(tab, None);
    }
}

/// What the hidden window does when the pad comes or goes: the tooltip, the status line and the
/// settings window follow.
fn follow_pad_status(tray: &TrayIcon, items: &Items, no_pad: bool) {
    let (tray, status, pause) = (tray.clone(), items.status.clone(), items.pause.clone());
    let on_pad_status = move || {
        if !pause.is_checked() {
            let text = status_line(no_pad);
            let _ = tray.set_tooltip(Some(&text));
            status.set_text(text);
            settings::pad_changed();
        }
    };
    PAD_STATUS.set(Some(Box::new(on_pad_status)));
}

fn on_menu(ev: &MenuEvent, items: &Items, paused: &AtomicBool, to_mixer: &Sender<PadEvent>, no_pad: bool) {
    if ev.id == items.quit.id() {
        stop_mixer();
        // SAFETY: no arguments that point anywhere; it only posts WM_QUIT to this thread.
        unsafe { PostQuitMessage(0) };
    } else if ev.id == items.open_settings.id() {
        settings::open();
    } else if ev.id == items.pause.id() {
        let on = items.pause.is_checked();
        paused.store(on, Ordering::Relaxed);
        items.status.set_text(if on { format!("KB16 mixer {VERSION}: paused") } else { status_line(no_pad) });
    } else if ev.id == items.open.id() {
        let _ = std::process::Command::new("notepad.exe").arg(config::path()).spawn();
    } else if ev.id == items.folder.id() {
        let _ = std::process::Command::new("explorer.exe").arg(config::dir()).spawn();
    } else if ev.id == items.reload.id() {
        let result = reload();
        settings::reloaded();
        match result {
            Ok(()) => {
                items.show_problem(None);
                let _ = to_mixer.send(PadEvent::Notice("Config loaded".into()));
            }
            Err(e) => {
                // The settings that work stay active; say exactly what is wrong.
                items.show_problem(Some(&e));
                let text =
                    HSTRING::from(format!("config.toml has a problem, so the previous settings stay active.\n\n{e}"));
                // SAFETY: both strings outlive the call, and no owner window is passed.
                unsafe {
                    MessageBoxW(None, &text, w!("KB16 mixer"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND);
                }
            }
        }
    } else if ev.id == items.autostart.id() {
        crate::set_autostart(items.autostart.is_checked());
    }
}

thread_local! {
    /// What the hidden window does when the pad comes or goes (it updates the tray).
    static PAD_STATUS: std::cell::RefCell<Option<Box<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

/// Lets the mixer undo Focus and Talk and save, and waits until it has. The wait is bounded,
/// so a mixer stuck in a Core Audio call cannot hold up Quit or the end of a Windows session.
fn stop_mixer() {
    let (done, answer) = mpsc::channel();
    if tell_mixer(PadEvent::Shutdown(done)) {
        let _ = answer.recv_timeout(Duration::from_secs(3));
    }
}

/// A window nobody sees, for what the worker tells the main thread and for the end of a Windows
/// session. A message to a window still arrives while a menu or a dialog runs its own message
/// loop, where a thread message would be dropped. It is a top-level window, not a message-only
/// one, because only top-level windows are told that the session ends.
pub fn ui_window() -> HWND {
    // SAFETY: the class and window names are static wide strings, and `ui_proc` has the
    // signature a window procedure needs. A failure returns a null window, which is logged.
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ui_proc),
            hInstance: instance,
            lpszClassName: w!("kb16-mixer-ui"),
            ..Default::default()
        });
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("kb16-mixer-ui"),
            w!("KB16 mixer"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
        .unwrap_or_else(|e| {
            crate::log(&format!("could not create the hidden window: {e}"));
            HWND::default()
        })
    }
}

/// Tells the main thread something, through the hidden window (`ui` is its handle).
pub fn post_ui(ui: usize, msg: u32) {
    // SAFETY: PostMessageW only queues the message; an invalid handle makes it fail, harmlessly.
    unsafe {
        let _ = PostMessageW(Some(HWND(ui as *mut _)), msg, WPARAM(0), LPARAM(0));
    }
}

unsafe extern "system" fn ui_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAD_STATUS => {
            PAD_STATUS.with_borrow(|f| f.as_ref().map(|f| f()));
            LRESULT(0)
        }
        WM_APPS_MOVED => {
            settings::apps_moved();
            LRESULT(0)
        }
        WM_DEVICECHANGE => {
            if wparam.0 == DBT_DEVICEARRIVAL as usize {
                pad::device_arrived();
            }
            LRESULT(1)
        }
        WM_QUERYENDSESSION => LRESULT(1),
        // The session really ends (wparam false: it was called off). The process may be
        // stopped any time after this returns, so the apps Focus or Talk changed are put right now.
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                stop_mixer();
            }
            LRESULT(0)
        }
        // SAFETY: the arguments are the ones Windows passed to this window procedure.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
