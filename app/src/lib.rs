//! kb16-mixer: the PC half of the KB16 mixer.
//!
//! What it does, in full: reads knob/key events from its own USB device, moves sliders in the
//! Windows volume mixer, and sends levels back for the pad's lights and screen.
//! What it never does: keyboard hooks, input injection, drivers, reading or opening other
//! processes. That is what keeps it out of an anti-cheat's way. (The `windows` crate's keyboard
//! and mouse module is not even enabled; a test below keeps it that way.)
//!
//! The crate is a library (these modules) and a thin `main.rs`, so that the tests in `tests/`
//! can drive the real modules. The command line is in `USAGE`.

#![deny(clippy::undocumented_unsafe_blocks)]

pub mod audio;
mod autostart;
mod check;
pub mod config;
mod dialog;
mod keys;
mod open;
mod pad;
pub mod procs;
mod settings;
mod tray;
pub mod trim;
mod ui;
mod win;
mod worker;

pub use autostart::{autostart_enabled, set_autostart};
/// Pictures of the settings window and its dialogs, drawn hidden (examples/render_settings.rs).
pub use settings::{Options, Picture, Preview, Tab, render_hidden};
pub use ui::theme::Look;

use config::Config;
use pad::PadEvent;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW};
use windows::core::{HSTRING, w};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "kb16-mixer [--settings[=keys|channels|pad]] [--no-pad] [--check] [--selftest] [--bootloader]\n\n\
    --settings   start with the settings window open, on that tab\n\
    --no-pad     never look for the pad (try the settings window while another copy owns it)\n\
    --check      print a short report (pad, settings, autostart, audio) that is safe to paste into a \
    bug report, and quit. In a command prompt, use: kb16-mixer --check | more\n\
    --selftest   write what the mixer sees into selftest.txt and quit\n\
    --bootloader restart the pad into its bootloader and quit\n\n\
    KB16_MIXER_DIR=<folder> replaces the settings folder (%APPDATA%\\kb16-mixer), for a portable \
    copy or a throwaway test folder. Then nothing is migrated and Start with Windows is never written.";

/// What the tray and the settings window share (both live on the main thread).
struct App {
    cfg: Arc<RwLock<Config>>,
    generation: Arc<AtomicU64>,
    to_mixer: Sender<PadEvent>,
}

thread_local! {
    static APP: std::cell::RefCell<Option<App>> = const { std::cell::RefCell::new(None) };
}

pub fn current_config() -> Config {
    APP.with_borrow(|app| app.as_ref().map(|a| a.cfg.read().unwrap_or_else(PoisonError::into_inner).clone()))
        .unwrap_or_else(config::defaults)
}

/// Loads config.toml and makes it live. On a problem the settings in use stay as they are.
pub fn reload() -> Result<(), String> {
    APP.with_borrow(|app| {
        let Some(app) = app.as_ref() else { return Ok(()) };
        match config::load() {
            Ok(new) => {
                *app.cfg.write().unwrap_or_else(PoisonError::into_inner) = new;
                app.generation.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                log(&format!("config problem: {e}"));
                let _ = app.to_mixer.send(PadEvent::Notice("Config error".into()));
                Err(e)
            }
        }
    })
}

/// Hands the mixer thread a request; false if it is not running (its audio could not start).
pub fn tell_mixer(event: PadEvent) -> bool {
    APP.with_borrow(|app| app.as_ref().is_some_and(|a| a.to_mixer.send(event).is_ok()))
}

/// Waits (briefly) until the mixer has taken in the config made live by `reload`.
pub fn mixer_caught_up() {
    let (done, answer) = mpsc::channel();
    if tell_mixer(PadEvent::CaughtUp(done)) {
        let _ = answer.recv_timeout(Duration::from_secs(1));
    }
}

/// The apps that have a volume slider right now, as the mixer sees them.
pub fn live_apps() -> Option<Vec<String>> {
    let (reply, answer) = mpsc::channel();
    tell_mixer(PadEvent::Apps(reply)).then(|| answer.recv_timeout(Duration::from_secs(1)).ok()).flatten()
}

pub fn log(msg: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = std::fs::create_dir_all(config::dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(config::dir().join("log.txt")) {
        let _ = writeln!(f, "{secs} {msg}");
    }
}

/// The first `max` characters of `s`.
fn clip(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// The whole app: the command line, then the tray, the pad listener and the mixer.
pub fn run() {
    // Should anything ever go badly wrong, leave a note in log.txt instead of vanishing silently.
    std::panic::set_hook(Box::new(|info| log(&format!("crash: {info}"))));
    let arg = |name: &str| std::env::args().any(|a| a == name);
    if arg("--check") {
        std::process::exit(check::run());
    }
    if arg("--help") || arg("-h") || arg("/?") {
        // SAFETY: both strings outlive the call, and no owner window is passed.
        unsafe {
            MessageBoxW(None, &HSTRING::from(USAGE), w!("KB16 mixer"), MB_OK | MB_ICONINFORMATION);
        }
        return;
    }
    if arg("--selftest") {
        return check::selftest();
    }
    if arg("--bootloader") {
        std::process::exit(if pad::enter_bootloader() { 0 } else { 1 });
    }
    if !single_instance() {
        return;
    }
    let no_pad = arg("--no-pad");

    let first_run = config::is_first_run();
    let (cfg, startup_problem) = match config::load() {
        Ok(c) => (c, None),
        Err(e) => {
            log(&format!("config problem at start, using defaults: {e}"));
            (config::defaults(), Some(e))
        }
    };

    let cfg = Arc::new(RwLock::new(cfg));
    let generation = Arc::new(AtomicU64::new(0));
    let paused = Arc::new(AtomicBool::new(false));
    let (ev_tx, ev_rx) = mpsc::channel();
    // Before the worker starts: it posts to this window.
    let ui = tray::ui_window().0 as usize;

    if !no_pad {
        pad::watch_arrivals(HWND(ui as *mut _));
        let (ev_tx, paused) = (ev_tx.clone(), paused.clone());
        std::thread::spawn(move || pad::listen(ev_tx, paused));
    }
    {
        let shared = worker::Shared {
            cfg: cfg.clone(),
            generation: generation.clone(),
            paused: paused.clone(),
            notices: ev_tx.clone(),
            ui,
        };
        std::thread::spawn(move || worker::run(ev_rx, shared));
    }
    if startup_problem.is_some() {
        let _ = ev_tx.send(PadEvent::Notice("Config error".into()));
    }
    APP.set(Some(App { cfg, generation, to_mixer: ev_tx.clone() }));

    let start = tray::Start { first_run, no_pad, problem: startup_problem };
    if let Err(e) = tray::run(paused, ev_tx, start) {
        log(&format!("tray failed: {e}"));
    }
}

/// One copy per settings folder: a copy run against KB16_MIXER_DIR does not stop the real one.
/// False if another copy runs. The guard is held for the life of the process.
fn single_instance() -> bool {
    let instance = match config::custom_dir() {
        Some(dir) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            dir.to_string_lossy().to_lowercase().hash(&mut h);
            format!("Local\\kb16-mixer-{:016x}", h.finish())
        }
        None => "Local\\kb16-mixer".into(),
    };
    // SAFETY: the name outlives the call. The handle is deliberately never closed: the mutex
    // must exist until the process ends.
    unsafe {
        let _mutex = CreateMutexW(None, true, &HSTRING::from(instance));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return false;
        }
        // The settings window's file dialog wants a single-threaded COM apartment here.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    true
}
