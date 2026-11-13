//! What the pad knows from the host, how a host report changes it, and the idle rule.

use crate::Time;
use kb16_mixer_protocol::{HostMsg, Key, LOGO_NAME_LEN, NEVER, NOTICE};

pub const NCH: usize = kb16_mixer_protocol::CHANNELS;
/// Without a report for this long, the host counts as gone and the pad is a plain keyboard.
const HOST_TIMEOUT_MS: u64 = 3000;
pub const DEFAULT_BRIGHTNESS: u8 = 170;
/// The case hides the panel's top rows; screens are drawn this many pixels lower.
pub const DEFAULT_OLED_TOP: u8 = 3;
pub const RIPPLES: usize = 4;
/// Minutes of quiet before the screen turns off / the lights dim, unless the PC app says otherwise.
pub const DEFAULT_IDLE_MIN: u8 = 10;

#[derive(Clone, Copy)]
pub struct Popup<T> {
    pub at: T,
    pub volume: u8,
    pub muted: bool,
    pub name: [u8; 16],
}

#[derive(Clone, Copy)]
pub struct Mixer {
    pub n: u8,
    pub colors: [[u8; 3]; NCH],
    pub level: [u8; NCH],
    pub volume: [u8; NCH],
    pub mutes: u8,
    pub selected: u8,
    pub mixer_keys: bool,
    pub mic_muted: bool,
    /// The host hears an app playing; counts as activity while the host is there.
    pub playing: bool,
    pub second_page: bool,
    pub names: [[u8; 6]; NCH],
}

/// A key press, for the light bloom that spreads from it.
#[derive(Clone, Copy)]
pub struct Ripple<T> {
    pub at: T,
    pub row: u8,
    pub col: u8,
}

pub struct State<T> {
    pub host_seen: Option<T>,
    pub mixer: Mixer,
    pub brightness: u8,
    pub oled_top: u8,
    /// Minutes of quiet before the screen turns off, and before the lights dim (`NEVER` = stay on).
    pub screen_off_min: u8,
    pub lights_dim_min: u8,
    /// Light the dimmed LEDs keep, in percent of normal; 0 = the lights' own default.
    pub lights_idle_percent: u8,
    pub popup: Option<Popup<T>>,
    /// The last activity, for the idle rule (see `docs/how-it-works.md`).
    pub activity: T,
    pub keys: [Key; 16],
    pub ripples: [Option<Ripple<T>>; RIPPLES],
    ripple_next: usize,
    /// What the logo screen shows (loaded from flash at boot).
    pub logo_name: [u8; LOGO_NAME_LEN],
}

/// What the report task has to do after a report, outside the state lock.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum After {
    Nothing,
    Bootloader,
    /// Keep this logo name in flash (it may already be there).
    SaveName([u8; LOGO_NAME_LEN]),
}

/// True once `idle_ms` of quiet has passed a limit given in minutes.
pub fn past(idle_ms: u64, minutes: u8) -> bool {
    minutes != NEVER && idle_ms > minutes as u64 * 60_000
}

impl<T: Time> State<T> {
    /// The state at power-up; `boot` is the clock's zero.
    pub const fn new(boot: T) -> State<T> {
        State {
            host_seen: None,
            mixer: Mixer {
                n: 0,
                colors: [[0; 3]; NCH],
                level: [0; NCH],
                volume: [0; NCH],
                mutes: 0,
                selected: 0,
                mixer_keys: true,
                mic_muted: false,
                playing: false,
                second_page: false,
                names: [[b' '; 6]; NCH],
            },
            brightness: DEFAULT_BRIGHTNESS,
            oled_top: DEFAULT_OLED_TOP,
            screen_off_min: DEFAULT_IDLE_MIN,
            lights_dim_min: DEFAULT_IDLE_MIN,
            lights_idle_percent: 0,
            popup: None,
            activity: boot,
            keys: [Key::Host; 16],
            ripples: [None; RIPPLES],
            ripple_next: 0,
            logo_name: crate::name::DEFAULT,
        }
    }

    pub fn host_alive(&self, now: T) -> bool {
        self.host_seen.is_some_and(|t| now.ms_since(t) < HOST_TIMEOUT_MS)
    }

    pub fn add_ripple(&mut self, now: T, row: u8, col: u8) {
        self.ripples[self.ripple_next] = Some(Ripple { at: now, row, col });
        self.ripple_next = (self.ripple_next + 1) % RIPPLES;
    }

    /// A key press or a knob turn: it counts as activity, and a press on one of the 16 keys
    /// starts a light bloom.
    pub fn touched(&mut self, now: T, key: Option<(u8, u8)>) {
        self.activity = now;
        if let Some((row, col)) = key
            && col < 4
        {
            self.add_ripple(now, row, col);
        }
    }

    /// Applies one report from the host.
    pub fn handle(&mut self, msg: HostMsg, now: T) -> After {
        if msg == HostMsg::Bootloader {
            return After::Bootloader;
        }
        let mut after = After::Nothing;
        // A host that comes back starts from silence, not from what it said last time.
        if !self.host_alive(now) {
            self.mixer.playing = false;
            self.mixer.second_page = false;
        }
        self.host_seen = Some(now);
        let m = &mut self.mixer;
        match msg {
            HostMsg::Popup(p) => {
                let volume = if p.volume == NOTICE { NOTICE } else { p.volume.min(100) };
                // Not activity: a readout of the user's own knob or key comes right after that
                // input, which already counted, and a notice nobody asked for ("Config loaded")
                // must not keep the pad awake.
                self.popup = Some(Popup { at: now, volume, muted: p.muted, name: p.name });
            }
            HostMsg::Settings(c) => {
                m.n = c.channels.min(NCH as u8);
                m.colors = c.colors;
                self.brightness = if c.brightness == 0 { DEFAULT_BRIGHTNESS } else { c.brightness };
                self.oled_top =
                    if c.oled_top_plus_one == 0 { DEFAULT_OLED_TOP } else { (c.oled_top_plus_one - 1).min(6) };
                self.screen_off_min = if c.screen_off_min == 0 { DEFAULT_IDLE_MIN } else { c.screen_off_min };
                self.lights_dim_min = if c.lights_dim_min == 0 { DEFAULT_IDLE_MIN } else { c.lights_dim_min };
                self.lights_idle_percent = c.idle_led_percent.min(100);
            }
            HostMsg::Frame(f) => {
                m.level = f.levels;
                for (v, &new) in m.volume.iter_mut().zip(f.volumes.iter()) {
                    *v = new.min(100);
                }
                m.mutes = f.mutes;
                m.selected = f.selected;
                m.mixer_keys = f.mixer_keys;
                m.mic_muted = f.mic_muted;
                // Raw levels are not activity: they come before the app's own volume and mute,
                // and include the microphone. Whether something is heard is the host's call.
                m.playing = f.playing;
                m.second_page = f.second_page;
            }
            HostMsg::Names(names) => m.names = names,
            HostMsg::Keys(k) => {
                let first = k.first as usize;
                for (i, key) in k.keys.into_iter().enumerate() {
                    if let Some(slot) = self.keys.get_mut(first + i) {
                        *slot = key;
                    }
                }
            }
            HostMsg::LogoName(name) if crate::name::valid(&name) => {
                self.logo_name = name;
                after = After::SaveName(name);
            }
            HostMsg::LogoName(_) | HostMsg::Heartbeat | HostMsg::Bootloader | HostMsg::Unknown(_) => {}
        }
        // The host only sends a frame when it changes, but some report (a heartbeat at least)
        // comes every second, so the flag keeps the pad awake exactly as long as it holds.
        if self.mixer.playing {
            self.activity = now;
        }
        after
    }
}
