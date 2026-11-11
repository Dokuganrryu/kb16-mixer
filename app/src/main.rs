//! kb16-mixer: the PC half of the KB16 mixer.
//!
//! What it does, in full: reads knob/key events from its own USB device, moves sliders in the
//! Windows volume mixer, and sends levels back for the pad's lights and screen.
//! What it never does: keyboard hooks, input injection, drivers, reading or opening other
//! processes. That is what keeps it out of an anti-cheat's way.

#![windows_subsystem = "windows"]

mod audio;
mod config;
mod pad;
mod procs;
mod settings;

use audio::{Device, Mixer, Moved, Readout};
use config::{Config, KeyAction, KnobBinding, Saved};
use pad::{Link, PadEvent};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

const WM_PAD_STATUS: u32 = WM_APP + 1;
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("kb16-mixer");
/// The autostart value written before the app was renamed.
const OLD_RUN_VALUE: PCWSTR = w!("kb16-knobs");
/// Meter rate while something is playing or being touched, and the idle rate otherwise.
const FRAME: Duration = Duration::from_millis(33);
const IDLE_FRAME: Duration = Duration::from_millis(200);
const STAY_ACTIVE: Duration = Duration::from_secs(3);
/// Colors and names are sent on connect and on a config change; this is only a safety resync.
const META_RESYNC: Duration = Duration::from_secs(30);
/// A Focus key held this long is momentary: it ends on release instead of latching.
const MOMENTARY: Duration = Duration::from_millis(350);
const SAVE_AFTER: Duration = Duration::from_secs(3);

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
    APP.with_borrow(|app| app.as_ref().map(|a| a.cfg.read().unwrap().clone())).unwrap_or_else(config::defaults)
}

/// Loads config.toml and makes it live. On a problem the settings in use stay as they are.
pub fn reload() -> Result<(), String> {
    APP.with_borrow(|app| {
        let Some(app) = app.as_ref() else { return Ok(()) };
        match config::load() {
            Ok(new) => {
                *app.cfg.write().unwrap() = new;
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

pub fn log(msg: &str) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = std::fs::create_dir_all(config::dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(config::dir().join("log.txt")) {
        let _ = writeln!(f, "{secs} {msg}");
    }
}

fn main() {
    // Should anything ever go badly wrong, leave a note in log.txt instead of vanishing silently.
    std::panic::set_hook(Box::new(|info| log(&format!("crash: {info}"))));
    // An install from before the rename: carry its settings and its autostart across.
    if config::migrate_old_settings() && old_autostart_enabled() {
        set_autostart(true);
    }
    if std::env::args().any(|a| a == "--selftest") {
        return selftest();
    }
    if std::env::args().any(|a| a == "--bootloader") {
        std::process::exit(if pad::enter_bootloader() { 0 } else { 1 });
    }
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\kb16-mixer"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }

    let first_run = config::is_first_run();
    let (cfg, startup_problem) = match config::load() {
        Ok(c) => (c, None),
        Err(e) => {
            log(&format!("config problem at start, using defaults: {e}"));
            (config::defaults(), Some(e))
        }
    };
    if first_run {
        set_autostart(true);
    }

    let cfg = Arc::new(RwLock::new(cfg));
    let generation = Arc::new(AtomicU64::new(0));
    let paused = Arc::new(AtomicBool::new(false));
    let (ev_tx, ev_rx) = mpsc::channel();
    let main_thread = unsafe { GetCurrentThreadId() };

    {
        let (ev_tx, paused) = (ev_tx.clone(), paused.clone());
        std::thread::spawn(move || pad::listen(ev_tx, paused));
    }
    {
        let (cfg, generation, paused) = (cfg.clone(), generation.clone(), paused.clone());
        std::thread::spawn(move || worker(ev_rx, cfg, generation, paused, main_thread));
    }
    if startup_problem.is_some() {
        let _ = ev_tx.send(PadEvent::Notice("Config error".into()));
    }
    APP.set(Some(App { cfg, generation, to_mixer: ev_tx.clone() }));

    if let Err(e) = tray(paused, ev_tx, startup_problem) {
        log(&format!("tray failed: {e}"));
    }
}

/// The mixer loop: reacts to the pad at once, and ticks to follow Windows and feed the pad.
fn worker(
    events: Receiver<PadEvent>,
    cfg: Arc<RwLock<Config>>,
    generation: Arc<AtomicU64>,
    paused: Arc<AtomicBool>,
    main_thread: u32,
) {
    let mut mixer = match Mixer::new() {
        Ok(m) => m,
        Err(e) => return log(&format!("audio init failed: {e}")),
    };
    let mut link = Link::new();
    let mut saved = config::load_state();
    let mut unsaved_since: Option<Instant> = None;
    let mut seen_generation = u64::MAX;
    let mut pad_present = false;
    let mut link_tried = Instant::now();
    let mut meta_sent: Option<Instant> = None;
    let mut last_frame = [0u8; 32];
    let mut last_tick = Instant::now();
    let mut last_active = Instant::now();
    let mut mic_muted = false;
    let mut mic_checked: Option<Instant> = None;
    // Focus key that is down: channel, since when, and whether it was already latched.
    let mut focus_key: Option<(usize, Instant, bool)> = None;
    // Talk key that is down: the mic mute state to go back to on release.
    let mut talk_restore: Option<bool> = None;
    let mut move_armed = false;

    loop {
        let every = if last_active.elapsed() < STAY_ACTIVE { FRAME } else { IDLE_FRAME };
        let deadline = last_tick + every;
        let mut batch = Vec::new();
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(e) => batch.push(e),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        while let Ok(e) = events.try_recv() {
            batch.push(e);
        }

        let cfg = cfg.read().unwrap();
        let channels = &cfg.channels;
        let n = channels.len();
        let g = generation.load(Ordering::Relaxed);
        if g != seen_generation {
            seen_generation = g;
            mixer.set_focus(None, channels);
            mixer.reset(channels, &saved.levels);
            mixer.moved = config::load_apps();
            mixer.duck = cfg.duck;
            mixer.restore = saved.restore.clone();
            focus_key = None;
            meta_sent = None;
        }
        mixer.refresh(channels, false);

        // Coalesce: a fast spin becomes one change per knob.
        let mut turns = [0i32; 3];
        let mut presses = [false; 3];
        let mut keys = Vec::new();
        let mut released = Vec::new();
        let mut let_go = false; // undo everything temporary (pad gone, app closing)
        let mut closing = false;
        let mut popup: Option<(String, u8, bool)> = None;
        if !batch.is_empty() {
            last_active = Instant::now();
        }
        for e in batch {
            match e {
                PadEvent::Turn { knob, delta } if knob < 3 => turns[knob as usize] += delta,
                PadEvent::Press { knob } if knob < 3 => presses[knob as usize] ^= true,
                PadEvent::Key { row, col } => keys.push((row, col as usize)),
                PadEvent::KeyUp { row, col } => released.push((row as usize, col as usize)),
                PadEvent::Shutdown => {
                    let_go = true;
                    closing = true;
                }
                PadEvent::Connected(c) => {
                    pad_present = c;
                    if c {
                        link.open();
                    } else {
                        link.close();
                        let_go = true;
                    }
                    meta_sent = None;
                    last_frame = [0; 32];
                    unsafe {
                        let _ = PostThreadMessageW(main_thread, WM_PAD_STATUS, WPARAM(c as usize), LPARAM(0));
                    }
                }
                PadEvent::Notice(text) => popup = Some((text, 255, false)),
                _ => {}
            }
        }

        let show = |r: Readout| {
            let label = if r.live { r.name } else { format!("{} (idle)", r.name) };
            Some((label, r.volume, r.muted))
        };
        let notice = |text: String| Some((text, 255u8, false));
        for (row, col) in keys {
            if row as usize >= 4 || col >= 4 {
                continue;
            }
            let action = cfg.keys[row as usize][col];
            // Move is held like a shift key: the next key picks the column.
            if move_armed && action != KeyAction::Move && col < n {
                let (outcome, app) = mixer.move_focused(col, channels);
                let channel = &channels[col].name;
                popup = match outcome {
                    Moved::To => {
                        config::save_apps(&mixer.moved);
                        notice(format!("{}>{}", clip(&app, 15usize.saturating_sub(channel.len())), channel))
                    }
                    Moved::BackToAuto => {
                        config::save_apps(&mixer.moved);
                        notice(format!("{}>auto", clip(&app, 11)))
                    }
                    Moved::NothingFocused => notice("No app in front".into()),
                    Moved::NotAnAppChannel => notice("Apps only".into()),
                };
                continue;
            }
            match action {
                KeyAction::Mute if col < n => popup = show(mixer.change_channel(col, None, channels)),
                // Talk, or Focus on the microphone's column, which means the same thing.
                _ if is_talk(action, col, channels) => {
                    if talk_restore.is_none() {
                        let muted = mixer.mic_muted();
                        talk_restore = Some(muted);
                        mixer.set_mic_muted(!muted, channels);
                        popup = notice(if muted { "Mic live".into() } else { "Mic off".into() });
                    }
                }
                KeyAction::Focus if col < n => {
                    let latched = mixer.focus() == Some(col);
                    mixer.set_focus(Some(col), channels);
                    focus_key = Some((col, Instant::now(), latched));
                    if !latched {
                        popup = notice(format!("Focus {}", channels[col].name));
                    }
                }
                KeyAction::Move => {
                    move_armed = true;
                    popup = match mixer.where_is_focused(channels) {
                        Some((app, Some(c))) => notice(format!(
                            "{}: {}",
                            clip(&app, 14usize.saturating_sub(channels[c].name.len())),
                            channels[c].name
                        )),
                        Some((app, None)) => notice(clip(&app, 16).to_string()),
                        None => notice("No app in front".into()),
                    };
                }
                _ => {}
            }
        }
        for (row, col) in released {
            if row >= 4 || col >= 4 {
                continue;
            }
            let action = cfg.keys[row][col];
            match action {
                KeyAction::Move => move_armed = false,
                _ if is_talk(action, col, channels) => {
                    if let Some(muted) = talk_restore.take() {
                        mixer.set_mic_muted(muted, channels);
                        popup = notice(if muted { "Mic off".into() } else { "Mic live".into() });
                    }
                }
                KeyAction::Focus => {
                    if let Some((_, since, latched)) = focus_key.filter(|(c, ..)| *c == col) {
                        focus_key = None;
                        // A tap latches (or unlatches); a hold lasts only while held.
                        if latched || since.elapsed() >= MOMENTARY {
                            mixer.set_focus(None, channels);
                            popup = notice("Focus off".into());
                        }
                    }
                }
                _ => {}
            }
        }
        if let_go {
            mixer.set_focus(None, channels);
            focus_key = None;
            move_armed = false;
            if let Some(muted) = talk_restore.take() {
                mixer.set_mic_muted(muted, channels);
            }
        }
        for knob in 0..3 {
            if turns[knob] == 0 && !presses[knob] {
                continue;
            }
            let binding = cfg.knobs[knob];
            let mut act = |delta: Option<f32>| match binding {
                KnobBinding::Selected => {
                    mixer.change_channel(mixer.focus().unwrap_or_else(|| cfg.default_selected()), delta, channels)
                }
                KnobBinding::Channel(c) if c < n => mixer.change_channel(c, delta, channels),
                KnobBinding::Master => mixer.change_device(Device::Master, delta),
                KnobBinding::Mic => mixer.change_device(Device::Mic, delta),
                _ => mixer.change_focused(delta),
            };
            if presses[knob] {
                popup = show(act(None));
            }
            if turns[knob] != 0 {
                popup = show(act(Some(turns[knob] as f32 * cfg.step)));
            }
            if binding == KnobBinding::Mic {
                mic_checked = None;
            }
        }

        // Remember levels set on the pad (written a moment later). What Focus has turned down
        // is written at once: it is what puts those apps right after a crash or power cut.
        let now_state = Saved {
            restore: mixer.restore.clone(),
            levels: channels
                .iter()
                .zip(&mixer.faders)
                .filter(|(_, f)| f.pinned)
                .map(|(c, f)| (c.name.clone(), (f.volume * 100.0).round() / 100.0))
                .collect(),
        };
        if now_state != saved {
            let urgent = now_state.restore != saved.restore;
            saved = now_state;
            unsaved_since = Some(if urgent { Instant::now() - SAVE_AFTER } else { Instant::now() });
        }
        if unsaved_since.is_some_and(|t| t.elapsed() >= SAVE_AFTER) || (closing && unsaved_since.is_some()) {
            config::save_state(&saved);
            unsaved_since = None;
        }
        if closing {
            return;
        }

        // ---- the pad ----
        if paused.load(Ordering::Relaxed) {
            link.close();
        } else if pad_present && !link.is_open() && link_tried.elapsed() >= Duration::from_secs(1) {
            link_tried = Instant::now();
            if link.open() {
                meta_sent = None;
                last_frame = [0; 32];
            }
        }
        let tick = Instant::now() >= deadline;
        if tick {
            last_tick = Instant::now();
        }
        if !link.is_open() {
            continue;
        }
        if let Some((label, volume, muted)) = popup {
            link.send(&pad::popup_message(volume, muted, &label));
        }
        if meta_sent.is_none_or(|t| t.elapsed() >= META_RESYNC) {
            let colors: Vec<[u8; 3]> = channels.iter().map(|c| c.color).collect();
            link.send(&pad::settings_message(&colors, &cfg));
            link.send(&pad::names_message(channels.iter().map(|c| c.name.as_str())));
            for m in pad::key_messages(&cfg.keys) {
                link.send(&m);
            }
            meta_sent = Some(Instant::now());
        }
        if tick {
            // Without a Mic channel, the screen still flags a muted microphone.
            if audio::mic_channel(channels).is_some() {
                mic_muted = false;
            } else if mic_checked.is_none_or(|t| t.elapsed() >= Duration::from_millis(1500)) {
                mic_muted = mixer.device_muted(Device::Mic);
                mic_checked = Some(Instant::now());
            }
            let levels = mixer.levels(channels);
            if levels.iter().any(|&l| l > 0) {
                last_active = Instant::now();
            }
            let mut volumes = [0u8; 4];
            let mut mutes = 0u8;
            for (i, f) in mixer.faders.iter().take(4).enumerate() {
                volumes[i] = if f.known { (f.volume * 100.0).round() as u8 } else { 100 };
                mutes |= (f.muted as u8) << i;
            }
            let frame = pad::frame_message(&pad::Frame {
                levels,
                volumes,
                mutes,
                selected: mixer.focus().map_or(255, |f| f as u8),
                mixer_keys: cfg.mixer_keys,
                mic_muted,
            });
            if frame != last_frame {
                link.send(&frame);
                last_frame = frame;
            }
        }
        link.keep_alive();
    }
}

/// Talk, or a Focus key sitting in the microphone's column.
fn is_talk(action: KeyAction, col: usize, channels: &[config::ChannelCfg]) -> bool {
    action == KeyAction::Talk || (action == KeyAction::Focus && audio::mic_channel(channels) == Some(col))
}

fn clip(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Writes what the engine sees into selftest.txt (sessions, channels, live levels).
fn selftest() {
    let mut out = Vec::new();
    match (Mixer::new(), config::load()) {
        (Ok(mut mixer), Ok(cfg)) => {
            mixer.moved = config::load_apps();
            mixer.reset(&cfg.channels, &config::load_state().levels);
            mixer.refresh(&cfg.channels, true);
            out.push("sessions:".to_string());
            out.extend(mixer.describe());
            let mut peak = [0u8; 4];
            for _ in 0..30 {
                for (max, l) in peak.iter_mut().zip(mixer.levels(&cfg.channels)) {
                    *max = (*max).max(l);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            for (i, c) in cfg.channels.iter().enumerate() {
                let r = mixer.channel_readout(i, &cfg.channels);
                out.push(format!(
                    "channel {} {:<6} live {} vol {} muted {} peak {}",
                    i + 1,
                    c.name,
                    r.live,
                    r.volume,
                    r.muted,
                    peak[i]
                ));
            }
        }
        (Err(e), _) => out.push(format!("audio init failed: {e}")),
        (_, Err(e)) => out.push(format!("config problem: {e}")),
    }
    let _ = std::fs::write(config::dir().join("selftest.txt"), out.join("\n"));
}

pub fn autostart_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

fn old_autostart_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, OLD_RUN_VALUE, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let Ok(exe) = std::env::current_exe() else { return };
            let value: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(value.as_ptr().cast()),
                (value.len() * 2) as u32,
            );
            // The entry from before the rename would start the old exe as well.
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, OLD_RUN_VALUE);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE);
        }
    }
}

/// A plain knob: light disc, dark tick. Drawn in code so there are no asset files.
fn icon() -> Icon {
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
    Icon::from_rgba(px, n as u32, n as u32).expect("icon")
}

fn tray(
    paused: Arc<AtomicBool>,
    to_mixer: Sender<PadEvent>,
    startup_problem: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = MenuItem::new("Looking for the pad...", false, None);
    let problem = MenuItem::new("", false, None);
    let open_settings = MenuItem::new("Settings...", true, None);
    let pause = CheckMenuItem::new("Pause (pad acts as a plain keyboard)", true, false, None);
    let open = MenuItem::new("Open config file", true, None);
    let reload_item = MenuItem::new("Reload config file", true, None);
    let folder = MenuItem::new("Open settings folder", true, None);
    let autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::new();
    menu.append(&status)?;
    menu.append(&problem)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&open_settings)?;
    menu.append(&pause)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&open)?;
    menu.append(&reload_item)?;
    menu.append(&folder)?;
    menu.append(&autostart)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    let show_problem = |text: Option<&str>| match text {
        Some(t) => problem.set_text(format!("Config problem: {}", clip(t, 90))),
        None => problem.set_text("Config OK"),
    };
    show_problem(startup_problem.as_deref());

    // Left click opens the settings window; right click shows the menu.
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("KB16 mixer")
        .with_icon(icon())
        .build()?;

    // `kb16-mixer.exe --settings` starts with the window open.
    if std::env::args().any(|a| a == "--settings") {
        settings::open();
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_PAD_STATUS && !pause.is_checked() {
                let text = if msg.wParam.0 != 0 { "Pad connected" } else { "Pad not connected" };
                status.set_text(text);
                let _ = tray.set_tooltip(Some(format!("KB16 mixer: {}", text.to_lowercase())));
            }
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
                if ev.id == quit.id() {
                    // Let the mixer undo Focus / Talk and save before the process ends.
                    let _ = to_mixer.send(PadEvent::Shutdown);
                    std::thread::sleep(Duration::from_millis(250));
                    PostQuitMessage(0);
                } else if ev.id == open_settings.id() {
                    settings::open();
                } else if ev.id == pause.id() {
                    paused.store(pause.is_checked(), Ordering::Relaxed);
                    if pause.is_checked() {
                        status.set_text("Paused");
                    }
                } else if ev.id == open.id() {
                    let _ = std::process::Command::new("notepad.exe").arg(config::path()).spawn();
                } else if ev.id == folder.id() {
                    let _ = std::process::Command::new("explorer.exe").arg(config::dir()).spawn();
                } else if ev.id == reload_item.id() {
                    match reload() {
                        Ok(()) => {
                            show_problem(None);
                            let _ = to_mixer.send(PadEvent::Notice("Config loaded".into()));
                        }
                        Err(e) => {
                            // The settings that work stay active; say exactly what is wrong.
                            show_problem(Some(&e));
                            let text = HSTRING::from(format!(
                                "config.toml has a problem, so the previous settings stay active.\n\n{e}"
                            ));
                            MessageBoxW(None, &text, w!("KB16 mixer"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND);
                        }
                    }
                } else if ev.id == autostart.id() {
                    set_autostart(autostart.is_checked());
                }
            }
        }
    }
    Ok(())
}
