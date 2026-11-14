//! The mixer loop: reacts to the pad at once, and ticks to follow Windows and feed the pad.
//!
//! One pass of the loop, in order: wait for pad events or the next tick, take in a new config,
//! re-read Windows, handle the events (keys at once, knob turns added up), follow Pause, turn
//! the knobs, save what changed, keep the link to the pad open, then send the pad what it shows.
//! The decisions that need no Windows (the tick rate, what is loud) are plain functions at the
//! bottom, with tests.

use crate::audio::{self, Audible, Device, Mixer, Moved, Readout};
use crate::config::{self, ChannelCfg, Config, KnobBinding, PressAction, Saved};
use crate::gesture::{Button, Gesture};
use crate::keys::{self, Keys};
use crate::pad::{self, Link, PadEvent, Show};
use crate::tray::{WM_APPS_MOVED, WM_PAD_STATUS, post_ui};
use crate::{clip, log, open, trim};
use kb16_mixer_protocol::Report;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, Instant};

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
/// A pad that is present but could not be opened is tried again this often.
const LINK_RETRY: Duration = Duration::from_secs(1);
/// Without a Mic channel, the microphone's mute (for the screen's mark) is read this often.
const MIC_CHECK: Duration = Duration::from_millis(1500);
/// A page change this soon after the last one is a peek (a held page key let go) or a quick
/// return: the pad only nudges its screen, so no notice names the page. As the pad's
/// `PEEK_WINDOW_MS`.
const PEEK_WINDOW: Duration = Duration::from_millis(1500);

/// What the worker shares with the main thread.
pub struct Shared {
    pub cfg: Arc<RwLock<Config>>,
    /// Goes up each time the main thread makes a new config live.
    pub generation: Arc<AtomicU64>,
    pub paused: Arc<AtomicBool>,
    /// The worker's own queue, for the `open:` thread's notices.
    pub notices: Sender<PadEvent>,
    /// The hidden window of the main thread (see `tray::post_ui`).
    pub ui: usize,
}

/// Runs the mixer until the app closes or the event queue goes away.
pub fn run(events: Receiver<PadEvent>, shared: Shared) {
    let mixer = match Mixer::new() {
        Ok(m) => m,
        Err(e) => return log(&format!("audio init failed: {e}")),
    };
    let mut w = Worker::new(mixer, shared.notices.clone(), shared.ui);
    loop {
        let deadline = w.last_tick + frame_interval(w.playing, w.last_active.elapsed());
        // A hold or a waiting tap that falls due wakes the loop too: no timer of its own.
        let gesture = w.gesture_deadline(&shared.cfg.read().unwrap_or_else(PoisonError::into_inner));
        let Some(batch) = next_batch(&events, gesture.map_or(deadline, |g| g.min(deadline))) else { return };
        let cfg = shared.cfg.read().unwrap_or_else(PoisonError::into_inner);
        w.take_config(&cfg, shared.generation.load(Ordering::Relaxed));
        let shown_page = w.keys.page();
        w.mixer.refresh(&cfg.channels, false);
        w.put_back_talk(&cfg.channels);

        let is_paused = shared.paused.load(Ordering::Relaxed);
        let mut popup = None;
        let input = w.handle_events(batch, &cfg, is_paused, &mut popup);
        w.follow_pause(is_paused, &cfg.channels, &mut popup);
        w.turn_knobs(&cfg, &input, &mut popup);
        w.save(&cfg.channels, input.closing.is_some());
        if let Some(done) = input.closing {
            let _ = done.send(());
            return;
        }

        w.keep_link(is_paused);
        let tick = Instant::now() >= deadline || w.farewell;
        if tick {
            w.last_tick = Instant::now();
        }
        w.feed_pad(&cfg, shown_page, popup, tick);
    }
}

/// Waits for the first event or the deadline, then takes every event already queued. None when
/// the queue is gone (the app is closing).
fn next_batch(events: &Receiver<PadEvent>, deadline: Instant) -> Option<Vec<PadEvent>> {
    let mut batch = Vec::new();
    match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(e) => batch.push(e),
        Err(RecvTimeoutError::Timeout) => {}
        Err(RecvTimeoutError::Disconnected) => return None,
    }
    while let Ok(e) = events.try_recv() {
        batch.push(e);
    }
    Some(batch)
}

/// What one batch asks of the knobs: turns added up (a fast spin becomes one change per knob),
/// and the press gestures in the order they fired.
#[derive(Default)]
struct Input {
    turns: [i32; 3],
    gestures: Vec<(usize, Gesture)>,
    /// The app is closing; answered once the state is saved.
    closing: Option<Sender<()>>,
}

struct Worker {
    mixer: Mixer,
    link: Link,
    keys: Keys,
    /// What each knob is aimed at: its channel, or for a few seconds one app in it.
    aims: [trim::Aim; 3],
    knob_buttons: KnobButtons,
    saved: Saved,
    /// The mic's mute state from before a Talk key that the last run left held (it crashed),
    /// until the mic is there to take it.
    talk_left: Option<bool>,
    /// When `saved` is to be written next; None when the file holds it.
    save_at: Option<Instant>,
    seen_generation: u64,
    /// Channel names of the config in force, to tell a rename from a new channel.
    names: Vec<String>,
    pad_present: bool,
    link_tried: Instant,
    meta_sent: Option<Instant>,
    /// When the key page last changed.
    page_changed: Option<Instant>,
    last_frame: Report,
    last_tick: Instant,
    last_active: Instant,
    playing: bool,
    audible: Audible,
    mic_muted: bool,
    mic_checked: Option<Instant>,
    was_paused: bool,
    /// Paused just now: one last full send so the pad is on page 1 too before it goes quiet.
    farewell: bool,
    notices: Sender<PadEvent>,
    ui: usize,
}

impl Worker {
    fn new(mixer: Mixer, notices: Sender<PadEvent>, ui: usize) -> Worker {
        Worker {
            mixer,
            link: Link::new(),
            keys: Keys::default(),
            aims: Default::default(),
            knob_buttons: KnobButtons::default(),
            saved: config::load_state(),
            talk_left: None,
            save_at: None,
            seen_generation: u64::MAX,
            names: Vec::new(),
            pad_present: false,
            link_tried: Instant::now(),
            meta_sent: None,
            page_changed: None,
            last_frame: [0; 32],
            last_tick: Instant::now(),
            last_active: Instant::now(),
            playing: false,
            audible: Audible::default(),
            mic_muted: false,
            mic_checked: None,
            was_paused: false,
            farewell: false,
            notices,
            ui,
        }
    }

    /// The pad's screen and the mixer start over on the next send.
    fn resend_all(&mut self) {
        self.meta_sent = None;
        self.last_frame = [0; 32];
    }

    /// Takes in a config made live since the last pass (or the first one).
    fn take_config(&mut self, cfg: &Config, generation: u64) {
        if generation == self.seen_generation {
            return;
        }
        let starting = self.seen_generation == u64::MAX;
        self.seen_generation = generation;
        let channels = &cfg.channels;
        // A renamed channel keeps its remembered level and the apps moved into it.
        let renamed = config::renames(&self.names, channels);
        self.names = channels.iter().map(|c| c.name.clone()).collect();
        for (old, new) in &renamed {
            if let Some(level) = self.saved.levels.remove(old) {
                self.saved.levels.insert(new.clone(), level);
            }
        }
        let mixer = &mut self.mixer;
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
        mixer.output = cfg.output.clone();
        mixer.reset(channels, &self.saved.levels);
        mixer.duck = cfg.duck;
        // After that the mixer keeps its own: Focus has just put back what it turned down.
        if starting {
            mixer.restore = self.saved.restore.clone();
            mixer.restore_ids = self.saved.restore_ids.clone();
            self.talk_left = self.saved.talk;
        }
        mixer.trims = self.saved.trims.clone();
        self.aims = Default::default();
        self.knob_buttons.let_go();
        self.keys.reload();
        self.meta_sent = None;
    }

    /// Keys act at once, in the order they came; turns and presses are added up per knob.
    fn handle_events(
        &mut self,
        batch: Vec<PadEvent>,
        cfg: &Config,
        is_paused: bool,
        popup: &mut Option<Show>,
    ) -> Input {
        let channels = &cfg.channels;
        let mut input = Input::default();
        if batch.iter().any(is_touch) {
            self.last_active = Instant::now();
        }
        let now = Instant::now();
        for e in batch {
            let mut desk = Desk { mixer: &mut self.mixer, channels, popup, notices: &self.notices, ui: self.ui };
            match e {
                // Input still on its way when Pause was chosen belongs to no one.
                ref e if is_paused && is_touch(e) => {}
                PadEvent::Turn { knob, delta } if knob < 3 => {
                    self.knob_buttons.turn(knob as usize, &mut input.gestures);
                    input.turns[knob as usize] += delta;
                }
                PadEvent::Press { knob } if knob < 3 => {
                    self.knob_buttons.press(knob as usize, now, cfg, &mut input.gestures);
                }
                PadEvent::KnobUp { knob } if knob < 3 => {
                    self.knob_buttons.release(knob as usize, now, cfg, &mut input.gestures);
                }
                PadEvent::Key { row, col } => self.keys.press(row as usize, col as usize, now, cfg, &mut desk),
                PadEvent::KeyUp { row, col } => self.keys.release(row as usize, col as usize, now, cfg, &mut desk),
                PadEvent::Shutdown(done) => {
                    self.keys.let_go(&mut desk);
                    input.closing = Some(done);
                }
                PadEvent::Connected(c) => {
                    self.pad_present = c;
                    if c {
                        self.link.open();
                    } else {
                        self.link.close();
                        self.keys.let_go(&mut desk);
                        self.knob_buttons.let_go();
                    }
                    self.resend_all();
                    post_ui(self.ui, WM_PAD_STATUS);
                }
                PadEvent::Notice(text) => *popup = Some(Show::Notice(text)),
                PadEvent::Apps(reply) => {
                    self.mixer.refresh(channels, true);
                    let _ = reply.send(self.mixer.app_names());
                }
                PadEvent::CaughtUp(done) => {
                    let _ = done.send(());
                }
                PadEvent::Unmove { exe, done } => {
                    if self.mixer.moved.remove(&exe).is_some() {
                        config::save_apps(&self.mixer.moved);
                        self.mixer.refresh(channels, true);
                    }
                    let _ = done.send(());
                }
                _ => {}
            }
        }
        if !is_paused {
            self.knob_buttons.poll(now, cfg, &mut input.gestures);
            let mut desk = Desk { mixer: &mut self.mixer, channels, popup, notices: &self.notices, ui: self.ui };
            self.keys.poll(now, cfg, &mut desk);
        }
        input
    }

    /// When a knob's or a key's gesture falls due, if one is under way.
    fn gesture_deadline(&self, cfg: &Config) -> Option<Instant> {
        self.knob_buttons.deadline(cfg).into_iter().chain(self.keys.deadline(cfg)).min()
    }

    /// Pause hands the pad to Windows as a plain keyboard, and unpausing hands it back: both
    /// ways the app and the pad start over on page 1 with nothing held.
    fn follow_pause(&mut self, is_paused: bool, channels: &[ChannelCfg], popup: &mut Option<Show>) {
        if is_paused == self.was_paused {
            return;
        }
        self.was_paused = is_paused;
        self.keys.let_go(&mut Desk { mixer: &mut self.mixer, channels, popup, notices: &self.notices, ui: self.ui });
        self.knob_buttons.let_go();
        self.resend_all();
        self.farewell = is_paused;
    }

    /// The press gestures in the order they fired, then the turns.
    fn turn_knobs(&mut self, cfg: &Config, input: &Input, popup: &mut Option<Show>) {
        for &(knob, gesture) in &input.gestures {
            let press = &cfg.knob_press[knob];
            let action = match gesture {
                Gesture::Tap => &press.tap,
                Gesture::Hold => &press.hold,
                Gesture::Double => &press.double,
            };
            self.press_knob(cfg, knob, action, popup);
        }
        for knob in 0..3 {
            if input.turns[knob] != 0 {
                self.turn_knob(cfg, knob, input.turns[knob], popup);
            }
        }
    }

    fn press_knob(&mut self, cfg: &Config, knob: usize, action: &PressAction, popup: &mut Option<Show>) {
        let channels = &cfg.channels;
        let mixer = &mut self.mixer;
        let binding = cfg.knobs[knob];
        let channel = knob_channel(binding, mixer.focus(), cfg.default_selected(), channels.len());
        *popup = Some(match (action, channel) {
            (PressAction::Nothing, _) => return,
            (PressAction::Open(target), _) => return open::open(target.clone(), self.notices.clone()),
            (PressAction::Mute, Some(c)) => level_popup(mixer.change_channel(c, None, channels)),
            (PressAction::Pick, Some(c)) => {
                let live = mixer.apps_in(c);
                match self.aims[knob].step(c, &live, Instant::now()) {
                    trim::Step::Channel => level_popup(mixer.channel_readout(c, channels)),
                    trim::Step::App(exe) => level_popup(mixer.app_readout(c, &exe)),
                    trim::Step::NoApps => Show::Notice(format!("No app in {}", clip(&channels[c].name, 6))),
                }
            }
            // Master, mic or the app in front: mute toggles it; with no apps to pick from, a
            // pick only shows the level (a zero step changes nothing).
            (_, None) => {
                let step = if *action == PressAction::Mute { None } else { Some(0.0) };
                let readout = match binding {
                    KnobBinding::Master => mixer.change_device(Device::Master, step),
                    KnobBinding::Mic => {
                        self.mic_checked = None;
                        mixer.change_device(Device::Mic, step)
                    }
                    _ => mixer.change_focused(step),
                };
                level_popup(readout)
            }
        });
    }

    fn turn_knob(&mut self, cfg: &Config, knob: usize, turns: i32, popup: &mut Option<Show>) {
        let channels = &cfg.channels;
        let mixer = &mut self.mixer;
        let binding = cfg.knobs[knob];
        let delta = turns as f32 * cfg.step;
        let Some(c) = knob_channel(binding, mixer.focus(), cfg.default_selected(), channels.len()) else {
            let readout = match binding {
                KnobBinding::Master => mixer.change_device(Device::Master, Some(delta)),
                KnobBinding::Mic => mixer.change_device(Device::Mic, Some(delta)),
                _ => mixer.change_focused(Some(delta)),
            };
            *popup = Some(level_popup(readout));
            if binding == KnobBinding::Mic {
                self.mic_checked = None;
            }
            return;
        };
        let live = mixer.apps_in(c);
        let now = Instant::now();
        let aim = &mut self.aims[knob];
        let readout = match aim.app(c, &live, now).map(str::to_string) {
            Some(exe) => mixer.change_app(c, &exe, delta, channels),
            None => mixer.change_channel(c, Some(delta), channels),
        };
        *popup = Some(level_popup(readout));
        aim.touch(now);
    }

    /// Puts the mic back as it was before a Talk key that the last run left held.
    fn put_back_talk(&mut self, channels: &[ChannelCfg]) {
        if let Some(muted) = talk_to_put_back(&mut self.talk_left, self.mixer.has_mic()) {
            self.mixer.set_mic_muted(muted, channels);
        }
    }

    /// Writes state.toml when `save_due` says so.
    fn save(&mut self, channels: &[ChannelCfg], closing: bool) {
        let talk = self.keys.talk_restore().or(self.talk_left);
        let now_state = saved_state(&self.mixer, channels, talk);
        if save_due(&mut self.saved, &mut self.save_at, now_state, closing, Instant::now()) {
            config::save_state(&self.saved);
        }
    }

    /// Opens the write side while the pad is there and the app is not paused; closes it on
    /// Pause, after one last send (the farewell).
    fn keep_link(&mut self, is_paused: bool) {
        if self.farewell {
            // The listener may already have let go of the pad; one write still reaches it.
            if !self.link.is_open() {
                self.link.open();
            }
        } else if is_paused {
            self.link.close();
        } else if self.pad_present && !self.link.is_open() && self.link_tried.elapsed() >= LINK_RETRY {
            self.link_tried = Instant::now();
            if self.link.open() {
                self.resend_all();
            }
        }
    }

    /// Sends the pad what changed: the key page, a popup, the colors and names now and then,
    /// and on a tick the frame.
    fn feed_pad(&mut self, cfg: &Config, shown_page: usize, popup: Option<Show>, tick: bool) {
        if !self.link.is_open() {
            self.farewell = false;
            return;
        }
        let page = self.keys.page();
        let mut popup = popup;
        if page != shown_page && self.meta_sent.is_some() {
            for m in pad::key_messages(cfg, page) {
                self.link.send(&m);
            }
            if let Some(text) = page_notice(page, &mut self.page_changed, Instant::now()) {
                popup = Some(Show::Notice(text.into()));
            }
        }
        if let Some(show) = popup {
            self.link.send(&pad::popup_message(&show));
        }
        if self.meta_sent.is_none_or(|t| t.elapsed() >= META_RESYNC) {
            self.send_meta(cfg, page);
        }
        if tick {
            self.send_frame(cfg, page);
        }
        self.link.keep_alive();
        if self.farewell {
            self.link.close();
            self.farewell = false;
        }
    }

    fn send_meta(&mut self, cfg: &Config, page: usize) {
        let channels = &cfg.channels;
        let colors: Vec<[u8; 3]> = channels.iter().map(|c| c.color).collect();
        self.link.send(&pad::settings_message(&colors, cfg));
        self.link.send(&pad::names_message(channels.iter().map(|c| c.name.as_str())));
        self.link.send(&pad::logo_name_message(&cfg.name));
        for m in pad::key_messages(cfg, page) {
            self.link.send(&m);
        }
        self.meta_sent = Some(Instant::now());
    }

    /// The meters, levels and flags. Computed every tick and sent only when it changed; while
    /// it stays the same, the heartbeat tells the pad it still holds.
    fn send_frame(&mut self, cfg: &Config, page: usize) {
        let channels = &cfg.channels;
        // Without a Mic channel, the screen still flags a muted microphone.
        if audio::mic_channel(channels).is_some() {
            self.mic_muted = false;
        } else if self.mic_checked.is_none_or(|t| t.elapsed() >= MIC_CHECK) {
            self.mic_muted = self.mixer.device_muted(Device::Mic);
            self.mic_checked = Some(Instant::now());
        }
        let (levels, heard) = self.mixer.levels(channels);
        if is_loud(&levels) {
            self.last_active = Instant::now();
        }
        self.playing = self.audible.update(heard, Instant::now());
        let mut volumes = [0u8; 4];
        let mut mutes = 0u8;
        for (i, f) in self.mixer.faders.iter().take(4).enumerate() {
            volumes[i] = if f.known { (f.volume * 100.0).round() as u8 } else { 100 };
            mutes |= (f.muted as u8) << i;
        }
        let frame = pad::frame_message(&pad::Frame {
            levels,
            volumes,
            mutes,
            selected: self.mixer.focus().map_or(255, |f| f as u8),
            mixer_keys: cfg.mixer_keys,
            mic_muted: self.mic_muted,
            playing: self.playing,
            second_page: page != 0,
        });
        if frame != self.last_frame {
            self.link.send(&frame);
            self.last_frame = frame;
        }
    }
}

/// The three knobs' push buttons: tap, hold and double tap (see `gesture.rs`).
#[derive(Default)]
struct KnobButtons {
    buttons: [Button; 3],
    /// The pad reports knob releases. Firmware from before that sends only the press, which
    /// then counts as a press and a release at once: a tap or a double tap, never a hold.
    ups: bool,
}

impl KnobButtons {
    fn press(&mut self, k: usize, now: Instant, cfg: &Config, out: &mut Vec<(usize, Gesture)>) {
        let wants = cfg.knob_press[k].wants();
        let fire = &mut |g| out.push((k, g));
        self.buttons[k].press(now, wants, fire);
        if !self.ups {
            self.buttons[k].release(now, wants, fire);
        }
    }

    fn release(&mut self, k: usize, now: Instant, cfg: &Config, out: &mut Vec<(usize, Gesture)>) {
        self.ups = true;
        self.buttons[k].release(now, cfg.knob_press[k].wants(), &mut |g| out.push((k, g)));
    }

    /// A turn is a turn: it uses up a press that is down.
    fn turn(&mut self, k: usize, out: &mut Vec<(usize, Gesture)>) {
        self.buttons[k].interrupt(&mut |g| out.push((k, g)));
    }

    fn poll(&mut self, now: Instant, cfg: &Config, out: &mut Vec<(usize, Gesture)>) {
        for (k, button) in self.buttons.iter_mut().enumerate() {
            button.poll(now, cfg.knob_press[k].wants(), &mut |g| out.push((k, g)));
        }
    }

    fn deadline(&self, cfg: &Config) -> Option<Instant> {
        self.buttons.iter().zip(&cfg.knob_press).filter_map(|(b, p)| b.deadline(p.wants())).min()
    }

    /// Nothing counts as down or waiting any more; what the pad can send stays known.
    fn let_go(&mut self) {
        self.buttons = Default::default();
    }
}

/// The Talk state the last run left to put back, once the mic is there to take it.
fn talk_to_put_back(left: &mut Option<bool>, has_mic: bool) -> Option<bool> {
    if has_mic { left.take() } else { None }
}

/// What is worth remembering across runs: levels set on the pad, trims, what Focus has turned
/// down, and the mic state from before a held Talk key (`talk`).
fn saved_state(mixer: &Mixer, channels: &[ChannelCfg], talk: Option<bool>) -> Saved {
    Saved {
        restore: mixer.restore.clone(),
        restore_ids: mixer.restore_ids.clone(),
        talk,
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
    }
}

/// Takes in the state as it is `now` and says whether to write it. A level or a trim set on the
/// pad waits `SAVE_AFTER` after the last change, so a knob spin is written once. A change to
/// what Focus has turned down, or a Talk key going down or up, is due at once: it is what puts
/// those apps and the mic right after a crash or a power cut. Closing writes whatever is still
/// waiting.
fn save_due(saved: &mut Saved, save_at: &mut Option<Instant>, now_state: Saved, closing: bool, now: Instant) -> bool {
    if now_state != *saved {
        let urgent = now_state.restore != saved.restore
            || now_state.restore_ids != saved.restore_ids
            || now_state.talk != saved.talk;
        *saved = now_state;
        *save_at = Some(if urgent { now } else { now + SAVE_AFTER });
    }
    let due = save_at.is_some_and(|at| now >= at || closing);
    if due {
        *save_at = None;
    }
    due
}

/// A knob turn or press, or a key going down or up.
fn is_touch(e: &PadEvent) -> bool {
    matches!(
        e,
        PadEvent::Turn { .. }
            | PadEvent::Press { .. }
            | PadEvent::KnobUp { .. }
            | PadEvent::Key { .. }
            | PadEvent::KeyUp { .. }
    )
}

/// How long the loop waits for the next tick: the fast rate while programme audio plays or the
/// pad was active (touched, or a meter at `LOUD`) within `STAY_ACTIVE`, the idle rate otherwise.
fn frame_interval(playing: bool, since_active: Duration) -> Duration {
    if playing || since_active < STAY_ACTIVE { FRAME } else { IDLE_FRAME }
}

/// A meter at about -30 dBFS or more.
fn is_loud(levels: &[u8]) -> bool {
    levels.iter().any(|&l| l >= LOUD)
}

/// The channel a knob turns: for "selected", the one Focus stands out, else the default one.
/// None for the master volume, the microphone and the app in front.
fn knob_channel(binding: KnobBinding, focus: Option<usize>, default_selected: usize, channels: usize) -> Option<usize> {
    match binding {
        KnobBinding::Selected => Some(focus.unwrap_or(default_selected)),
        KnobBinding::Channel(c) if c < channels => Some(c),
        _ => None,
    }
}

/// The pad's notice after the Move key, cut to fit the 16 characters of a screen line: the
/// channel name is kept whole and the app name gives way.
fn moved_notice(outcome: &Moved, app: &str, channel: &str) -> String {
    match outcome {
        Moved::To => format!("{}>{}", clip(app, 15usize.saturating_sub(channel.len())), channel),
        Moved::BackToAuto => format!("{}>auto", clip(app, 11)),
        Moved::NothingFocused => "No app in front".into(),
        Moved::NotAnAppChannel => "Apps only".into(),
    }
}

/// The pad's notice for the app in front and its channel, cut to 16 characters as above.
fn where_notice(found: Option<(String, Option<usize>)>, channels: &[ChannelCfg]) -> String {
    match found {
        Some((app, Some(c))) => {
            let channel = channels.get(c).map_or("", |c| c.name.as_str());
            format!("{}: {}", clip(&app, 14usize.saturating_sub(channel.len())), channel)
        }
        Some((app, None)) => clip(&app, 16).to_string(),
        None => "No app in front".into(),
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
        if matches!(outcome, Moved::To | Moved::BackToAuto) {
            config::save_apps(&self.mixer.moved);
            post_ui(self.ui, WM_APPS_MOVED);
        }
        self.notice(moved_notice(&outcome, &app, &self.channels[col].name));
    }

    fn show_where(&mut self) {
        let text = where_notice(self.mixer.where_is_focused(self.channels), self.channels);
        self.notice(text);
    }

    fn open(&mut self, target: &str) {
        open::open(target.to_string(), self.notices.clone());
    }
}

/// The notice that names the new key page, unless the last page change was a moment ago.
fn page_notice(page: usize, last: &mut Option<Instant>, now: Instant) -> Option<&'static str> {
    let quick = last.is_some_and(|t| now.saturating_duration_since(t) < PEEK_WINDOW);
    *last = Some(now);
    (!quick).then_some(if page == 0 { "MIXER" } else { "KEYS" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peek_or_a_quick_return_shows_no_page_notice() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut last = None;
        assert_eq!(page_notice(1, &mut last, t0), Some("KEYS"), "the first change");
        assert_eq!(page_notice(0, &mut last, t0 + ms(400)), None, "a held page key let go");
        assert_eq!(page_notice(1, &mut last, t0 + ms(400) + ms(1499)), None, "still soon after the last change");
        let later = t0 + ms(400) + ms(1499) + PEEK_WINDOW;
        assert_eq!(page_notice(0, &mut last, later), Some("MIXER"), "a page change after a pause");
        assert_eq!(PEEK_WINDOW, ms(1500));
    }

    #[test]
    fn the_fast_rate_lasts_three_seconds_after_activity_or_while_audio_plays() {
        let ms = Duration::from_millis;
        assert_eq!(frame_interval(false, ms(0)), FRAME);
        assert_eq!(frame_interval(false, ms(2999)), FRAME, "touched 3 s ago, nearly");
        assert_eq!(frame_interval(false, ms(3000)), IDLE_FRAME, "idle after 3 s");
        assert_eq!(frame_interval(true, ms(60_000)), FRAME, "programme audio keeps it fast");
        assert_eq!((FRAME, IDLE_FRAME), (ms(33), ms(200)));
    }

    #[test]
    fn only_a_meter_at_about_minus_30_dbfs_counts_as_loud() {
        // The meters map -48..0 dBFS onto 0..255, so -30 dBFS is (48 - 30) / 48 * 255 = 95.6.
        let db = |l: u8| f32::from(l) / 255.0 * 48.0 - 48.0;
        assert!((db(LOUD) - -30.0).abs() < 0.5, "{}", db(LOUD));
        assert!(!is_loud(&[0, 94, 0, 0]), "an open microphone in a quiet room");
        assert!(is_loud(&[0, 0, 0, 95]));
        assert!(!is_loud(&[]));
    }

    #[test]
    fn what_focus_turned_down_is_written_at_once_and_levels_a_moment_later() {
        let t0 = Instant::now();
        let (mut saved, mut save_at) = (Saved::default(), None);
        let focus_on = Saved { restore: [("spotify.exe".to_string(), 0.6)].into(), ..Saved::default() };
        assert!(save_due(&mut saved, &mut save_at, focus_on.clone(), false, t0), "in the same pass");
        assert!(saved == focus_on && save_at.is_none());
        assert!(!save_due(&mut saved, &mut save_at, focus_on.clone(), false, t0 + SAVE_AFTER), "nothing new");

        let mut turned = focus_on.clone();
        turned.levels.insert("Game".into(), 0.4);
        assert!(!save_due(&mut saved, &mut save_at, turned.clone(), false, t0), "a knob turn waits");
        turned.levels.insert("Game".into(), 0.5);
        let t1 = t0 + Duration::from_secs(2);
        assert!(!save_due(&mut saved, &mut save_at, turned.clone(), false, t1), "still turning: the wait starts over");
        assert!(!save_due(&mut saved, &mut save_at, turned.clone(), false, t1 + SAVE_AFTER - Duration::from_millis(1)));
        assert!(save_due(&mut saved, &mut save_at, turned.clone(), false, t1 + SAVE_AFTER), "once, after the wait");

        let mut moved = turned.clone();
        moved.trims.insert("game.exe".into(), 0.5);
        assert!(!save_due(&mut saved, &mut save_at, moved.clone(), false, t1));
        assert!(save_due(&mut saved, &mut save_at, moved, true, t1), "closing writes what is waiting");
        let same = saved.clone();
        assert!(!save_due(&mut saved, &mut save_at, same, true, t1), "and nothing twice");
    }

    #[test]
    fn session_levels_and_a_held_talk_key_are_written_at_once() {
        let t0 = Instant::now();
        let (mut saved, mut save_at) = (Saved::default(), None);
        let session = Saved { restore_ids: [("a".to_string(), 0.6)].into(), ..Saved::default() };
        assert!(save_due(&mut saved, &mut save_at, session, false, t0), "a session Focus turned down");
        let talk = Saved { talk: Some(false), ..saved.clone() };
        assert!(save_due(&mut saved, &mut save_at, talk, false, t0), "Talk went down");
        let let_go = Saved { talk: None, ..saved.clone() };
        assert!(save_due(&mut saved, &mut save_at, let_go, false, t0), "Talk went up");
    }

    #[test]
    fn talk_left_held_by_the_last_run_is_put_back_once_the_mic_is_there() {
        let mut left = Some(true);
        assert_eq!(talk_to_put_back(&mut left, false), None, "no mic yet: wait");
        assert_eq!(left, Some(true), "and keep it on disk");
        assert_eq!(talk_to_put_back(&mut left, true), Some(true));
        assert_eq!(talk_to_put_back(&mut left, true), None, "once");
    }

    #[test]
    fn a_knob_set_to_selected_follows_focus() {
        assert_eq!(knob_channel(KnobBinding::Selected, None, 2, 4), Some(2), "the default channel");
        assert_eq!(knob_channel(KnobBinding::Selected, Some(1), 2, 4), Some(1), "the one Focus stands out");
        assert_eq!(knob_channel(KnobBinding::Channel(3), Some(1), 2, 4), Some(3), "its own channel, whatever Focus");
        assert_eq!(knob_channel(KnobBinding::Channel(4), None, 2, 4), None, "no such channel");
        for other in [KnobBinding::Master, KnobBinding::Mic, KnobBinding::Focused] {
            assert_eq!(knob_channel(other, Some(1), 2, 4), None);
        }
    }

    #[test]
    fn notices_about_the_app_in_front_fit_one_screen_line() {
        let channels = config::defaults().channels; // Mic, Chat, Game, Media
        let long = "Averyverylongappname";
        let cases = [
            (moved_notice(&Moved::To, "Spotify", "Media"), "Spotify>Media"),
            (moved_notice(&Moved::To, long, "Media"), "Averyveryl>Media"),
            (moved_notice(&Moved::BackToAuto, long, "Media"), "Averyverylo>auto"),
            (moved_notice(&Moved::NothingFocused, long, "Media"), "No app in front"),
            (moved_notice(&Moved::NotAnAppChannel, long, "Mic"), "Apps only"),
            (where_notice(Some(("Spotify".into(), Some(3))), &channels), "Spotify: Media"),
            (where_notice(Some((long.into(), Some(1))), &channels), "Averyveryl: Chat"),
            (where_notice(Some((long.into(), None)), &channels), "Averyverylongapp"),
            (where_notice(None, &channels), "No app in front"),
        ];
        for (got, want) in cases {
            assert_eq!(got, want);
            assert!(got.chars().count() <= 16, "{got}");
        }
        assert_eq!(
            moved_notice(&Moved::To, "Ünïcödéappnamelong", "Game"),
            "Ünïcödéappn>Game",
            "app: characters, not bytes"
        );
    }

    #[test]
    fn the_saved_state_keeps_pad_levels_and_real_trims_rounded() {
        let channels = config::defaults().channels; // Mic, Chat, Game, Media
        let mut mixer = Mixer::new().expect("COM");
        mixer.faders = vec![audio::Fader::default(); 4];
        mixer.faders[2].volume = 0.4567;
        mixer.faders[2].pinned = true;
        mixer.faders[3].volume = 0.9; // read from an app, never set on the pad
        mixer.trims = [("a.exe".to_string(), 0.12345), ("b.exe".to_string(), 0.9995)].into();
        mixer.restore = [("c.exe".to_string(), 0.7)].into();
        mixer.restore_ids = [("c1".to_string(), 0.7)].into();
        let state = saved_state(&mixer, &channels, Some(true));
        assert_eq!(state.levels, [("Game".to_string(), 0.46)].into(), "only what was set on the pad");
        assert_eq!(state.trims, [("a.exe".to_string(), 0.123)].into(), "a trim of about 1 is no trim");
        assert_eq!(state.restore, mixer.restore);
        assert_eq!(state.restore_ids, mixer.restore_ids);
        assert_eq!(state.talk, Some(true), "the mic state from before a held Talk key");
    }

    #[test]
    fn a_knob_hold_needs_firmware_that_reports_the_release() {
        let (cfg, t0) = (config::defaults(), Instant::now()); // tap mute, hold pick, no double
        let at = |ms| t0 + Duration::from_millis(ms);
        let (mut knobs, mut out) = (KnobButtons::default(), Vec::new());
        knobs.press(2, at(0), &cfg, &mut out);
        assert_eq!(out, [(2, Gesture::Tap)], "older firmware: every press is a tap, at once");
        assert_eq!(knobs.deadline(&cfg), None, "and nothing to wait for");

        out.clear();
        knobs.press(2, at(1000), &cfg, &mut out);
        knobs.release(2, at(1100), &cfg, &mut out);
        assert_eq!(out, [(2, Gesture::Tap)], "the first release seen comes too late for that press");
        out.clear();
        knobs.press(2, at(2000), &cfg, &mut out);
        assert_eq!(knobs.deadline(&cfg), Some(at(2000) + crate::gesture::HOLD));
        knobs.poll(at(2350), &cfg, &mut out);
        assert_eq!(out, [(2, Gesture::Hold)], "from then on a hold is a hold");
        knobs.release(2, at(2600), &cfg, &mut out);
        knobs.press(1, at(3000), &cfg, &mut out);
        knobs.release(1, at(3100), &cfg, &mut out);
        assert_eq!(out[1..], [(1, Gesture::Tap)], "a short press on another knob is a tap");
        knobs.let_go();
        knobs.press(0, at(4000), &cfg, &mut out);
        assert_eq!(out.len(), 2, "after a let-go the pad is still known to send releases");
    }

    /// A knob let go just after Pause was chosen must not finish a tap: the pause drops it,
    /// like the press before it.
    #[test]
    fn a_knob_release_is_a_touch_like_its_press() {
        assert!(is_touch(&PadEvent::Press { knob: 0 }));
        assert!(is_touch(&PadEvent::KnobUp { knob: 0 }));
    }

    #[test]
    fn turning_a_held_knob_is_only_a_turn() {
        let (cfg, t0) = (config::defaults(), Instant::now());
        let at = |ms| t0 + Duration::from_millis(ms);
        let (mut knobs, mut out) = (KnobButtons { ups: true, ..KnobButtons::default() }, Vec::new());
        knobs.press(0, at(0), &cfg, &mut out);
        knobs.turn(0, &mut out);
        knobs.poll(at(500), &cfg, &mut out);
        knobs.release(0, at(600), &cfg, &mut out);
        assert!(out.is_empty(), "no mute and no pick");
    }

    #[test]
    fn a_readout_of_an_idle_channel_says_so() {
        let r = |live| Readout { name: "Game".into(), volume: 40, muted: true, live };
        let name = |s: Show| match s {
            Show::Level { name, .. } => name,
            Show::Notice(_) => unreachable!(),
        };
        assert_eq!(name(level_popup(r(true))), "Game");
        assert_eq!(name(level_popup(r(false))), "Game (idle)");
    }
}
