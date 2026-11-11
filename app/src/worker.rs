//! The mixer loop: reacts to the pad at once, and ticks to follow Windows and feed the pad.
//!
//! One pass of the loop, in order: wait for pad events or the next tick, take in a new config,
//! re-read Windows, handle the events (keys at once, knob turns added up), follow Pause, turn
//! the knobs, save what changed, keep the link to the pad open, then send the pad what it shows.
//! The decisions that need no Windows (the tick rate, what is loud) are plain functions at the
//! bottom, with tests.

use crate::audio::{self, Audible, Device, Mixer, Moved, Readout};
use crate::config::{self, ChannelCfg, Config, KnobBinding, Saved};
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
        let Some(batch) = next_batch(&events, deadline) else { return };
        let cfg = shared.cfg.read().unwrap_or_else(PoisonError::into_inner);
        w.take_config(&cfg, shared.generation.load(Ordering::Relaxed));
        let shown_page = w.keys.page();
        w.mixer.refresh(&cfg.channels, false);

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

/// What one batch asks of the knobs, added up: a fast spin becomes one change per knob.
#[derive(Default)]
struct Input {
    turns: [i32; 3],
    presses: [usize; 3],
    /// The app is closing; answered once the state is saved.
    closing: Option<Sender<()>>,
}

struct Worker {
    mixer: Mixer,
    link: Link,
    keys: Keys,
    /// What each knob is aimed at: its channel, or for a few seconds one app in it.
    aims: [trim::Aim; 3],
    saved: Saved,
    unsaved_since: Option<Instant>,
    seen_generation: u64,
    /// Channel names of the config in force, to tell a rename from a new channel.
    names: Vec<String>,
    pad_present: bool,
    link_tried: Instant,
    meta_sent: Option<Instant>,
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
            saved: config::load_state(),
            unsaved_since: None,
            seen_generation: u64::MAX,
            names: Vec::new(),
            pad_present: false,
            link_tried: Instant::now(),
            meta_sent: None,
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
        mixer.reset(channels, &self.saved.levels);
        mixer.duck = cfg.duck;
        // After that the mixer keeps its own: Focus has just put back what it turned down.
        if starting {
            mixer.restore = self.saved.restore.clone();
        }
        mixer.trims = self.saved.trims.clone();
        self.aims = Default::default();
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
                PadEvent::Turn { knob, delta } if knob < 3 => input.turns[knob as usize] += delta,
                PadEvent::Press { knob } if knob < 3 => input.presses[knob as usize] += 1,
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
        input
    }

    /// Pause hands the pad to Windows as a plain keyboard, and unpausing hands it back: both
    /// ways the app and the pad start over on page 1 with nothing held.
    fn follow_pause(&mut self, is_paused: bool, channels: &[ChannelCfg], popup: &mut Option<Show>) {
        if is_paused == self.was_paused {
            return;
        }
        self.was_paused = is_paused;
        self.keys.let_go(&mut Desk { mixer: &mut self.mixer, channels, popup, notices: &self.notices, ui: self.ui });
        self.resend_all();
        self.farewell = is_paused;
    }

    fn turn_knobs(&mut self, cfg: &Config, input: &Input, popup: &mut Option<Show>) {
        for knob in 0..3 {
            if input.turns[knob] != 0 || input.presses[knob] != 0 {
                self.turn_knob(cfg, knob, input.turns[knob], input.presses[knob], popup);
            }
        }
    }

    fn turn_knob(&mut self, cfg: &Config, knob: usize, turns: i32, presses: usize, popup: &mut Option<Show>) {
        let channels = &cfg.channels;
        let mixer = &mut self.mixer;
        let binding = cfg.knobs[knob];
        let delta = turns as f32 * cfg.step;
        let channel = match binding {
            KnobBinding::Selected => Some(mixer.focus().unwrap_or_else(|| cfg.default_selected())),
            KnobBinding::Channel(c) if c < channels.len() => Some(c),
            _ => None,
        };
        let Some(c) = channel else {
            // Master, mic or the app in front: a press only shows the level (a zero step
            // changes nothing); mute lives on the keys.
            let delta = if turns != 0 { delta } else { 0.0 };
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
        // A press steps the knob from the whole channel through each app playing in it.
        let live = mixer.apps_in(c);
        let now = Instant::now();
        let aim = &mut self.aims[knob];
        for _ in 0..presses {
            *popup = Some(match aim.step(c, &live, now) {
                trim::Step::Channel => level_popup(mixer.channel_readout(c, channels)),
                trim::Step::App(exe) => level_popup(mixer.app_readout(c, &exe)),
                trim::Step::NoApps => Show::Notice(format!("No app in {}", clip(&channels[c].name, 6))),
            });
        }
        if turns != 0 {
            let readout = match aim.app(c, &live, now).map(str::to_string) {
                Some(exe) => mixer.change_app(c, &exe, delta, channels),
                None => mixer.change_channel(c, Some(delta), channels),
            };
            *popup = Some(level_popup(readout));
            aim.touch(now);
        }
    }

    /// Remembers levels set on the pad (written a moment later). What Focus has turned down is
    /// written at once: it is what puts those apps right after a crash or power cut.
    fn save(&mut self, channels: &[ChannelCfg], closing: bool) {
        let now_state = saved_state(&self.mixer, channels);
        if now_state != self.saved {
            let urgent = now_state.restore != self.saved.restore;
            self.saved = now_state;
            self.unsaved_since = Some(if urgent { Instant::now() - SAVE_AFTER } else { Instant::now() });
        }
        if self.unsaved_since.is_some_and(|t| t.elapsed() >= SAVE_AFTER || closing) {
            config::save_state(&self.saved);
            self.unsaved_since = None;
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
            for m in pad::key_messages(&cfg.pages[page]) {
                self.link.send(&m);
            }
            popup = Some(Show::Notice(if page == 0 { "MIXER" } else { "KEYS" }.into()));
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
        for m in pad::key_messages(&cfg.pages[page]) {
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

/// What is worth remembering across runs: levels set on the pad, trims, and what Focus has
/// turned down.
fn saved_state(mixer: &Mixer, channels: &[ChannelCfg]) -> Saved {
    Saved {
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
    }
}

/// A knob turn or press, or a key going down or up.
fn is_touch(e: &PadEvent) -> bool {
    matches!(e, PadEvent::Turn { .. } | PadEvent::Press { .. } | PadEvent::Key { .. } | PadEvent::KeyUp { .. })
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
        open::open(target.to_string(), self.notices.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
