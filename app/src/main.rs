//! kb16-mixer: the PC half of the KB16 mixer.
//!
//! What it does, in full: reads knob/key events from its own USB device, moves sliders in the
//! Windows volume mixer, and sends levels back for the pad's lights and screen.
//! What it never does: keyboard hooks, input injection, drivers, reading or opening other
//! processes. That is what keeps it out of an anti-cheat's way. (The `windows` crate's keyboard
//! and mouse module is not even enabled; a test below keeps it that way.)
//!
//! Command line (also shown by `--help`):
//!   --settings[=keys|channels|pad]  start with the settings window open, on that tab
//!   --no-pad      never look for the pad (to try the settings window while another copy of
//!                 the app owns the pad); the volume mixer still runs
//!   --selftest    write what the mixer sees into selftest.txt and quit
//!   --bootloader  restart the pad into its bootloader and quit
//! KB16_MIXER_DIR=<folder> replaces the settings folder (%APPDATA%\kb16-mixer): a portable
//! copy, or a throwaway folder for testing. With it, nothing is migrated from an old install
//! and "Start with Windows" is never written.

#![windows_subsystem = "windows"]

mod audio;
mod config;
mod dialog;
mod keys;
mod pad;
mod procs;
mod settings;
mod trim;

use audio::{Audible, Device, Mixer, Moved, Readout};
use config::{ChannelCfg, Config, KnobBinding, Saved};
use keys::Keys;
use pad::{Link, PadEvent, Show};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{
    SEE_MASK_DOENVSUBST, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

const WM_PAD_STATUS: u32 = WM_APP + 1;
/// The mixer moved an app with the Move key (the settings window shows it).
const WM_APPS_MOVED: u32 = WM_APP + 2;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const USAGE: &str = "kb16-mixer [--settings[=keys|channels|pad]] [--no-pad] [--selftest] [--bootloader]\n\n\
    --settings   start with the settings window open, on that tab\n\
    --no-pad     never look for the pad (try the settings window while another copy owns it)\n\
    --selftest   write what the mixer sees into selftest.txt and quit\n\
    --bootloader restart the pad into its bootloader and quit\n\n\
    KB16_MIXER_DIR=<folder> replaces the settings folder (%APPDATA%\\kb16-mixer), for a portable \
    copy or a throwaway test folder. Then nothing is migrated and Start with Windows is never written.";
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("kb16-mixer");
/// The autostart value written before the app was renamed.
const OLD_RUN_VALUE: PCWSTR = w!("kb16-knobs");
/// Meter rate while the pad was touched in the last `STAY_ACTIVE`, programme audio is playing, or
/// a meter is up at `LOUD`; the idle rate otherwise.
const FRAME: Duration = Duration::from_millis(33);
const IDLE_FRAME: Duration = Duration::from_millis(200);
const STAY_ACTIVE: Duration = Duration::from_secs(3);
/// About -30 dBFS on the meters' -48..0 dB scale: a voice into the microphone or an app starting
/// to play. An open microphone in a quiet room stays below it, and must not keep the fast rate.
const LOUD: u8 = 95;
/// Colors and names are sent on connect and on a config change; this is only a safety resync.
const META_RESYNC: Duration = Duration::from_secs(30);
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

fn main() {
    // Should anything ever go badly wrong, leave a note in log.txt instead of vanishing silently.
    std::panic::set_hook(Box::new(|info| log(&format!("crash: {info}"))));
    // An install from before the rename: carry its settings and its autostart across.
    if config::migrate_old_settings() && old_autostart_enabled() {
        set_autostart(true);
    }
    if std::env::args().any(|a| a == "--help" || a == "-h" || a == "/?") {
        unsafe {
            MessageBoxW(None, &HSTRING::from(USAGE), w!("KB16 mixer"), MB_OK | MB_ICONINFORMATION);
        }
        return;
    }
    if std::env::args().any(|a| a == "--selftest") {
        return selftest();
    }
    if std::env::args().any(|a| a == "--bootloader") {
        std::process::exit(if pad::enter_bootloader() { 0 } else { 1 });
    }
    // One copy per settings folder: a copy run against KB16_MIXER_DIR does not stop the real one.
    let instance = match config::custom_dir() {
        Some(dir) => {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            dir.to_string_lossy().to_lowercase().hash(&mut h);
            format!("Local\\kb16-mixer-{:016x}", h.finish())
        }
        None => "Local\\kb16-mixer".into(),
    };
    unsafe {
        let _mutex = CreateMutexW(None, true, &HSTRING::from(instance));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        // The settings window's file dialog wants a single-threaded COM apartment here.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let no_pad = std::env::args().any(|a| a == "--no-pad");

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
    let ui = ui_window().0 as usize;

    if !no_pad {
        let (ev_tx, paused) = (ev_tx.clone(), paused.clone());
        std::thread::spawn(move || pad::listen(ev_tx, paused));
    }
    {
        let (notices, cfg, generation, paused) = (ev_tx.clone(), cfg.clone(), generation.clone(), paused.clone());
        std::thread::spawn(move || worker(ev_rx, notices, cfg, generation, paused, ui));
    }
    if startup_problem.is_some() {
        let _ = ev_tx.send(PadEvent::Notice("Config error".into()));
    }
    APP.set(Some(App { cfg, generation, to_mixer: ev_tx.clone() }));

    let start = Start { first_run, no_pad, problem: startup_problem };
    if let Err(e) = tray(paused, ev_tx, start) {
        log(&format!("tray failed: {e}"));
    }
}

/// The mixer loop: reacts to the pad at once, and ticks to follow Windows and feed the pad.
fn worker(
    events: Receiver<PadEvent>,
    notices: Sender<PadEvent>,
    cfg: Arc<RwLock<Config>>,
    generation: Arc<AtomicU64>,
    paused: Arc<AtomicBool>,
    ui: usize,
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
    let mut playing = false;
    let mut mic_muted = false;
    let mut mic_checked: Option<Instant> = None;
    let mut audible = Audible::default();
    let mut keys = Keys::default();
    let mut was_paused = false;
    // Paused just now: one last full send so the pad is on page 1 too before it goes quiet.
    let mut farewell = false;
    // What each knob is aimed at: its channel, or for a few seconds one app in it.
    let mut aims: [trim::Aim; 3] = Default::default();
    // Channel names of the config in force, to tell a rename from a new channel.
    let mut names: Vec<String> = Vec::new();

    loop {
        let every = if playing || last_active.elapsed() < STAY_ACTIVE { FRAME } else { IDLE_FRAME };
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
            let starting = seen_generation == u64::MAX;
            seen_generation = g;
            // A renamed channel keeps its remembered level and the apps moved into it.
            let renamed = config::renames(&names, channels);
            names = channels.iter().map(|c| c.name.clone()).collect();
            for (old, new) in &renamed {
                if let Some(level) = saved.levels.remove(old) {
                    saved.levels.insert(new.clone(), level);
                }
            }
            mixer.moved = config::load_apps();
            let mut moved_renamed = false;
            for channel in mixer.moved.values_mut() {
                if let Some((_, new)) = renamed.iter().find(|(old, _)| old.eq_ignore_ascii_case(channel)) {
                    *channel = new.clone();
                    moved_renamed = true;
                }
            }
            if moved_renamed {
                config::save_apps(&mixer.moved);
            }
            mixer.set_focus(None, channels);
            mixer.reset(channels, &saved.levels);
            mixer.duck = cfg.duck;
            // After that the mixer keeps its own: Focus has just put back what it turned down.
            if starting {
                mixer.restore = saved.restore.clone();
            }
            mixer.trims = saved.trims.clone();
            aims = Default::default();
            keys.reload();
            meta_sent = None;
        }
        let shown_page = keys.page();
        mixer.refresh(channels, false);

        // Coalesce: a fast spin becomes one change per knob.
        let mut turns = [0i32; 3];
        let mut presses = [0usize; 3];
        let mut closing: Option<Sender<()>> = None;
        let mut popup: Option<Show> = None;
        let is_paused = paused.load(Ordering::Relaxed);
        let touched = batch.iter().any(|e| {
            matches!(e, PadEvent::Turn { .. } | PadEvent::Press { .. } | PadEvent::Key { .. } | PadEvent::KeyUp { .. })
        });
        if touched {
            last_active = Instant::now();
        }
        let now = Instant::now();
        for e in batch {
            let mut desk = Desk { mixer: &mut mixer, channels, popup: &mut popup, notices: &notices, ui };
            match e {
                // Input still on its way when Pause was chosen belongs to no one.
                PadEvent::Turn { .. } | PadEvent::Press { .. } | PadEvent::Key { .. } | PadEvent::KeyUp { .. }
                    if is_paused => {}
                PadEvent::Turn { knob, delta } if knob < 3 => turns[knob as usize] += delta,
                PadEvent::Press { knob } if knob < 3 => presses[knob as usize] += 1,
                PadEvent::Key { row, col } => keys.press(row as usize, col as usize, now, &cfg, &mut desk),
                PadEvent::KeyUp { row, col } => keys.release(row as usize, col as usize, now, &cfg, &mut desk),
                PadEvent::Shutdown(done) => {
                    keys.let_go(&mut desk);
                    closing = Some(done);
                }
                PadEvent::Connected(c) => {
                    pad_present = c;
                    if c {
                        link.open();
                    } else {
                        link.close();
                        keys.let_go(&mut desk);
                    }
                    meta_sent = None;
                    last_frame = [0; 32];
                    post_ui(ui, WM_PAD_STATUS);
                }
                PadEvent::Notice(text) => popup = Some(Show::Notice(text)),
                PadEvent::Apps(reply) => {
                    mixer.refresh(channels, true);
                    let _ = reply.send(mixer.app_names());
                }
                PadEvent::CaughtUp(done) => {
                    let _ = done.send(());
                }
                PadEvent::Unmove { exe, done } => {
                    if mixer.moved.remove(&exe).is_some() {
                        config::save_apps(&mixer.moved);
                        mixer.refresh(channels, true);
                    }
                    let _ = done.send(());
                }
                _ => {}
            }
        }
        // Pause hands the pad to Windows as a plain keyboard, and unpausing hands it back: both
        // ways the app and the pad start over on page 1 with nothing held.
        if is_paused != was_paused {
            was_paused = is_paused;
            keys.let_go(&mut Desk { mixer: &mut mixer, channels, popup: &mut popup, notices: &notices, ui });
            meta_sent = None;
            last_frame = [0; 32];
            farewell = is_paused;
        }

        let show = |r: Readout| Some(level_popup(r));
        let notice = |text: String| Some(Show::Notice(text));
        for knob in 0..3 {
            if turns[knob] == 0 && presses[knob] == 0 {
                continue;
            }
            let binding = cfg.knobs[knob];
            let delta = turns[knob] as f32 * cfg.step;
            let channel = match binding {
                KnobBinding::Selected => Some(mixer.focus().unwrap_or_else(|| cfg.default_selected())),
                KnobBinding::Channel(c) if c < n => Some(c),
                _ => None,
            };
            let Some(c) = channel else {
                // Master, mic or the app in front: a press only shows the level (a zero step
                // changes nothing); mute lives on the keys.
                let mut act = |delta: f32| match binding {
                    KnobBinding::Master => mixer.change_device(Device::Master, Some(delta)),
                    KnobBinding::Mic => mixer.change_device(Device::Mic, Some(delta)),
                    _ => mixer.change_focused(Some(delta)),
                };
                popup = show(act(if turns[knob] != 0 { delta } else { 0.0 }));
                if binding == KnobBinding::Mic {
                    mic_checked = None;
                }
                continue;
            };
            // A press steps the knob from the whole channel through each app playing in it.
            let live = mixer.apps_in(c);
            let now = Instant::now();
            for _ in 0..presses[knob] {
                popup = match aims[knob].step(c, &live, now) {
                    trim::Step::Channel => show(mixer.channel_readout(c, channels)),
                    trim::Step::App(exe) => show(mixer.app_readout(c, &exe)),
                    trim::Step::NoApps => notice(format!("No app in {}", clip(&channels[c].name, 6))),
                };
            }
            if turns[knob] != 0 {
                popup = match aims[knob].app(c, &live, now).map(str::to_string) {
                    Some(exe) => show(mixer.change_app(c, &exe, delta, channels)),
                    None => show(mixer.change_channel(c, Some(delta), channels)),
                };
                aims[knob].touch(now);
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
            trims: mixer
                .trims
                .iter()
                .filter(|(_, t)| **t < 0.999)
                .map(|(exe, t)| (exe.clone(), (t * 1000.0).round() / 1000.0))
                .collect(),
        };
        if now_state != saved {
            let urgent = now_state.restore != saved.restore;
            saved = now_state;
            unsaved_since = Some(if urgent { Instant::now() - SAVE_AFTER } else { Instant::now() });
        }
        if unsaved_since.is_some_and(|t| t.elapsed() >= SAVE_AFTER) || (closing.is_some() && unsaved_since.is_some()) {
            config::save_state(&saved);
            unsaved_since = None;
        }
        if let Some(done) = closing {
            let _ = done.send(());
            return;
        }

        // ---- the pad ----
        if farewell {
            // The listener may already have let go of the pad; one write still reaches it.
            if !link.is_open() {
                link.open();
            }
        } else if is_paused {
            link.close();
        } else if pad_present && !link.is_open() && link_tried.elapsed() >= Duration::from_secs(1) {
            link_tried = Instant::now();
            if link.open() {
                meta_sent = None;
                last_frame = [0; 32];
            }
        }
        let tick = Instant::now() >= deadline || farewell;
        if tick {
            last_tick = Instant::now();
        }
        if !link.is_open() {
            farewell = false;
            continue;
        }
        let page = keys.page();
        if page != shown_page && meta_sent.is_some() {
            for m in pad::key_messages(&cfg.pages[page]) {
                link.send(&m);
            }
            popup = Some(Show::Notice(if page == 0 { "MIXER" } else { "KEYS" }.into()));
        }
        if let Some(show) = popup {
            link.send(&pad::popup_message(&show));
        }
        if meta_sent.is_none_or(|t| t.elapsed() >= META_RESYNC) {
            let colors: Vec<[u8; 3]> = channels.iter().map(|c| c.color).collect();
            link.send(&pad::settings_message(&colors, &cfg));
            link.send(&pad::names_message(channels.iter().map(|c| c.name.as_str())));
            link.send(&pad::logo_name_message(&cfg.name));
            for m in pad::key_messages(&cfg.pages[page]) {
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
            let (levels, heard) = mixer.levels(channels);
            if levels.iter().any(|&l| l >= LOUD) {
                last_active = Instant::now();
            }
            // Computed every tick and part of the frame, so a change is always sent; while it
            // stays the same, the heartbeat tells the pad it still holds.
            playing = audible.update(heard, Instant::now());
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
                playing,
                second_page: page != 0,
            });
            if frame != last_frame {
                link.send(&frame);
                last_frame = frame;
            }
        }
        link.keep_alive();
        if farewell {
            link.close();
            farewell = false;
        }
    }
}

fn level_popup(r: Readout) -> Show {
    let name = if r.live { r.name } else { format!("{} (idle)", r.name) };
    Show::Level { name, volume: r.volume, muted: r.muted }
}

/// What the keys act on in the worker: the mixer, and the popup the pad shows next.
struct Desk<'a> {
    mixer: &'a mut Mixer,
    channels: &'a [ChannelCfg],
    popup: &'a mut Option<Show>,
    notices: &'a Sender<PadEvent>,
    ui: usize,
}

impl Desk<'_> {
    fn notice(&mut self, text: String) {
        *self.popup = Some(Show::Notice(text));
    }
}

impl keys::Hands for Desk<'_> {
    fn focus(&self) -> Option<usize> {
        self.mixer.focus()
    }

    fn set_focus(&mut self, focus: Option<usize>, announce: bool) {
        self.mixer.set_focus(focus, self.channels);
        if announce {
            let text = focus.map_or("Focus off".into(), |c| format!("Focus {}", self.channels[c].name));
            self.notice(text);
        }
    }

    fn mic_muted(&self) -> bool {
        self.mixer.mic_muted()
    }

    fn set_mic_muted(&mut self, muted: bool, announce: bool) {
        self.mixer.set_mic_muted(muted, self.channels);
        if announce {
            self.notice(if muted { "Mic off" } else { "Mic live" }.into());
        }
    }

    fn mute(&mut self, col: usize) {
        *self.popup = Some(level_popup(self.mixer.change_channel(col, None, self.channels)));
    }

    fn move_to(&mut self, col: usize) {
        let (outcome, app) = self.mixer.move_focused(col, self.channels);
        let channel = &self.channels[col].name;
        let text = match outcome {
            Moved::To => format!("{}>{}", clip(&app, 15usize.saturating_sub(channel.len())), channel),
            Moved::BackToAuto => format!("{}>auto", clip(&app, 11)),
            Moved::NothingFocused => "No app in front".into(),
            Moved::NotAnAppChannel => "Apps only".into(),
        };
        if matches!(outcome, Moved::To | Moved::BackToAuto) {
            config::save_apps(&self.mixer.moved);
            post_ui(self.ui, WM_APPS_MOVED);
        }
        self.notice(text);
    }

    fn show_where(&mut self) {
        let text = match self.mixer.where_is_focused(self.channels) {
            Some((app, Some(c))) => {
                let channel = &self.channels[c].name;
                format!("{}: {}", clip(&app, 14usize.saturating_sub(channel.len())), channel)
            }
            Some((app, None)) => clip(&app, 16).to_string(),
            None => "No app in front".into(),
        };
        self.notice(text);
    }

    fn open(&mut self, target: &str) {
        open(target.to_string(), self.notices.clone());
    }
}

/// Opens a program, file, folder or URL the way a double-click in Explorer would: the shell
/// picks the handler, nothing is elevated, and no other process is touched. It runs on a
/// thread of its own, so a slow target (a network path, a sleeping drive) never holds up the
/// mixer; a failure is logged and shown on the pad.
fn open(target: String, notices: Sender<PadEvent>) {
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let file = HSTRING::from(target.as_str());
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            // Our own notice replaces the shell's error dialogs; NOASYNC because this thread
            // ends right after the call.
            fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI | SEE_MASK_DOENVSUBST,
            lpVerb: w!("open"),
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        if let Err(e) = ShellExecuteExW(&mut info) {
            log(&format!("could not open {target:?}: {e}"));
            let _ = notices.send(PadEvent::Notice("Open failed".into()));
        }
    });
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
                for (max, l) in peak.iter_mut().zip(mixer.levels(&cfg.channels).0) {
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

/// Always off for a copy run against `KB16_MIXER_DIR`, which never touches the registry.
pub fn autostart_enabled() -> bool {
    config::custom_dir().is_none()
        && unsafe {
            RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS
        }
}

fn old_autostart_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, OLD_RUN_VALUE, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set_autostart(on: bool) {
    if config::custom_dir().is_some() {
        return log("Start with Windows not written: KB16_MIXER_DIR is set");
    }
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

/// How this run started, for the tray.
struct Start {
    first_run: bool,
    no_pad: bool,
    problem: Option<String>,
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

fn tray(paused: Arc<AtomicBool>, to_mixer: Sender<PadEvent>, start: Start) -> Result<(), Box<dyn std::error::Error>> {
    let status = MenuItem::new(status_line(start.no_pad), false, None);
    let problem = MenuItem::new("", false, None);
    let open_settings = MenuItem::new("Settings...", true, None);
    let pause = CheckMenuItem::new("Pause (pad acts as a plain keyboard)", true, false, None);
    let open = MenuItem::new("Open config file", true, None);
    let reload_item = MenuItem::new("Reload config file", true, None);
    let folder = MenuItem::new("Open settings folder", true, None);
    let autostart = match config::custom_dir() {
        Some(_) => CheckMenuItem::new("Start with Windows (off: KB16_MIXER_DIR is set)", false, false, None),
        None => CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None),
    };
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
    show_problem(start.problem.as_deref());

    // Left click opens the settings window; right click shows the menu.
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("KB16 mixer")
        .with_icon(icon())
        .build()?;

    // The first run opens the window where Start with Windows is, and says where the app lives.
    // `--settings[=tab]` starts with the window open too.
    let asked = std::env::args().find_map(|a| a.strip_prefix("--settings").map(str::to_string));
    if start.first_run {
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

    {
        let (tray, status, pause, no_pad) = (tray.clone(), status.clone(), pause.clone(), start.no_pad);
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

    let mut msg = MSG::default();
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
                if ev.id == quit.id() {
                    stop_mixer();
                    PostQuitMessage(0);
                } else if ev.id == open_settings.id() {
                    settings::open();
                } else if ev.id == pause.id() {
                    paused.store(pause.is_checked(), Ordering::Relaxed);
                    status.set_text(if pause.is_checked() {
                        format!("KB16 mixer {VERSION}: paused")
                    } else {
                        status_line(start.no_pad)
                    });
                } else if ev.id == open.id() {
                    let _ = std::process::Command::new("notepad.exe").arg(config::path()).spawn();
                } else if ev.id == folder.id() {
                    let _ = std::process::Command::new("explorer.exe").arg(config::dir()).spawn();
                } else if ev.id == reload_item.id() {
                    let result = reload();
                    settings::reloaded();
                    match result {
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
    // It holds the tray icon, which is only taken off the taskbar when the last copy goes.
    PAD_STATUS.set(None);
    Ok(())
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
fn ui_window() -> HWND {
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
            log(&format!("could not create the hidden window: {e}"));
            HWND::default()
        })
    }
}

fn post_ui(ui: usize, msg: u32) {
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
        WM_QUERYENDSESSION => LRESULT(1),
        // The session really ends (wparam false: it was called off). The process may be
        // stopped any time after this returns, so the apps Focus or Talk changed are put right now.
        WM_ENDSESSION => {
            if wparam.0 != 0 {
                stop_mixer();
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    /// The app never observes the keyboard: the module that could is not even compiled in.
    #[test]
    fn the_keyboard_module_stays_out() {
        let manifest = include_str!("../Cargo.toml");
        assert!(!manifest.contains("KeyboardAndMouse"), "Win32_UI_Input_KeyboardAndMouse must stay disabled");
    }
}
