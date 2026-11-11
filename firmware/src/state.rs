//! State shared between tasks, the handling of host reports (wire format in the
//! `kb16-mixer-protocol` crate), and the reset / bootloader logic.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use embassy_stm32::pac;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::{Duration, Instant};
use kb16_mixer_protocol::{HostMsg, Key, LOGO_NAME_LEN, NEVER, NOTICE};

pub const NCH: usize = kb16_mixer_protocol::CHANNELS;
const HOST_TIMEOUT: Duration = Duration::from_millis(3000);
pub const DEFAULT_BRIGHTNESS: u8 = 170;
/// The case hides the panel's top rows; screens are drawn this many pixels lower.
pub const DEFAULT_OLED_TOP: u8 = 3;
pub const RIPPLES: usize = 4;
/// Minutes of quiet before the screen turns off / the lights dim, unless the PC app says otherwise.
pub const DEFAULT_IDLE_MIN: u8 = 10;

/// Set while the USB bus is suspended (PC asleep or off): lights and screen go dark.
pub static SUSPENDED: AtomicBool = AtomicBool::new(false);

pub fn suspended() -> bool {
    SUSPENDED.load(Ordering::Relaxed)
}

#[derive(Clone, Copy)]
pub struct Popup {
    pub at: Instant,
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
pub struct Ripple {
    pub at: Instant,
    pub row: u8,
    pub col: u8,
}

pub struct State {
    pub host_seen: Option<Instant>,
    pub mixer: Mixer,
    pub brightness: u8,
    pub oled_top: u8,
    /// Minutes of quiet before the screen turns off, and before the lights dim (`NEVER` = stay on).
    pub screen_off_min: u8,
    pub lights_dim_min: u8,
    pub popup: Option<Popup>,
    pub activity: Instant,
    pub keys: [Key; 16],
    pub ripples: [Option<Ripple>; RIPPLES],
    ripple_next: usize,
    /// What the logo screen shows (loaded from flash at boot).
    pub logo_name: [u8; LOGO_NAME_LEN],
}

impl State {
    const fn new() -> State {
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
            popup: None,
            activity: Instant::from_ticks(0),
            keys: [Key::Host; 16],
            ripples: [None; RIPPLES],
            ripple_next: 0,
            logo_name: crate::name::DEFAULT,
        }
    }

    pub fn host_alive(&self, now: Instant) -> bool {
        self.host_seen.is_some_and(|t| now.saturating_duration_since(t) < HOST_TIMEOUT)
    }

    pub fn add_ripple(&mut self, now: Instant, row: u8, col: u8) {
        self.ripples[self.ripple_next] = Some(Ripple { at: now, row, col });
        self.ripple_next = (self.ripple_next + 1) % RIPPLES;
    }
}

static STATE: Mutex<CriticalSectionRawMutex, RefCell<State>> = Mutex::new(RefCell::new(State::new()));

/// Meter levels after ballistics (8.8 fixed point), written by the LED task, read by the OLED task.
pub static METER: [AtomicU16; NCH] = [const { AtomicU16::new(0) }; NCH];

pub fn meter(i: usize) -> u8 {
    (METER[i].load(Ordering::Relaxed) >> 8) as u8
}

/// True once `idle_ms` of quiet has passed a limit given in minutes.
pub fn past(idle_ms: u64, minutes: u8) -> bool {
    minutes != NEVER && idle_ms > minutes as u64 * 60_000
}

pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.lock(|s| f(&mut s.borrow_mut()))
}

/// What the report task has to do after a report, outside the state lock.
pub enum After {
    Nothing,
    Bootloader,
    /// Keep this logo name in flash (it may already be there).
    SaveName([u8; LOGO_NAME_LEN]),
}

/// Applies one report from the host.
pub fn handle_report(r: &[u8]) -> After {
    let Some(msg) = HostMsg::decode(r) else { return After::Nothing };
    if msg == HostMsg::Bootloader {
        return After::Bootloader;
    }
    let now = Instant::now();
    let mut after = After::Nothing;
    with(|s| {
        // A host that comes back starts from silence, not from what it said last time.
        if !s.host_alive(now) {
            s.mixer.playing = false;
            s.mixer.second_page = false;
        }
        s.host_seen = Some(now);
        let m = &mut s.mixer;
        match msg {
            HostMsg::Popup(p) => {
                let volume = if p.volume == NOTICE { NOTICE } else { p.volume.min(100) };
                // Not activity: a readout of the user's own knob or key comes right after that
                // input, which already counted, and a notice nobody asked for ("Config loaded")
                // must not keep the pad awake.
                s.popup = Some(Popup { at: now, volume, muted: p.muted, name: p.name });
            }
            HostMsg::Settings(c) => {
                m.n = c.channels.min(NCH as u8);
                m.colors = c.colors;
                s.brightness = if c.brightness == 0 { DEFAULT_BRIGHTNESS } else { c.brightness };
                s.oled_top = if c.oled_top_plus_one == 0 { DEFAULT_OLED_TOP } else { (c.oled_top_plus_one - 1).min(6) };
                s.screen_off_min = if c.screen_off_min == 0 { DEFAULT_IDLE_MIN } else { c.screen_off_min };
                s.lights_dim_min = if c.lights_dim_min == 0 { DEFAULT_IDLE_MIN } else { c.lights_dim_min };
            }
            HostMsg::Frame(f) => {
                m.level = f.levels;
                for i in 0..NCH {
                    m.volume[i] = f.volumes[i].min(100);
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
                    if let Some(slot) = s.keys.get_mut(first + i) {
                        *slot = key;
                    }
                }
            }
            HostMsg::LogoName(name) if crate::name::valid(&name) => {
                s.logo_name = name;
                after = After::SaveName(name);
            }
            HostMsg::LogoName(_) | HostMsg::Heartbeat | HostMsg::Bootloader | HostMsg::Unknown(_) => {}
        }
        // The host only sends a frame when it changes, but some report (a heartbeat at least)
        // comes every second, so the flag keeps the pad awake exactly as long as it holds.
        if s.mixer.playing {
            s.activity = now;
        }
    });
    after
}

// ---------------------------------------------------------------------------
// Resets and the bootloader.
//
// The stm32duino bootloader reads backup register DR10 after every reset:
//   0x424C  stay in DFU          0x424D  start the app at once
// Backup registers survive resets but not a power cycle. We keep DR10 at "start the app" so a
// watchdog reset or panic restarts the firmware by itself, and count those restarts in DR9:
// three in a row without ~10 s of healthy running means the firmware is broken, and the pad
// parks itself in DFU where the PC can always reflash it.

const BKP_DR8: *mut u32 = 0x4000_6C20 as *mut u32;
const BKP_DR9: *mut u32 = 0x4000_6C24 as *mut u32;
const BKP_DR10: *mut u32 = 0x4000_6C28 as *mut u32;
const STAY_IN_DFU: u32 = 0x424C;
const START_APP: u32 = 0x424D;
const PANIC_MARK: u32 = 0x5041;
const MAX_CRASHES: u32 = 3;

fn backup_access() {
    pac::RCC.apb1enr().modify(|w| {
        w.set_pwren(true);
        w.set_bkpen(true);
    });
    pac::PWR.cr().modify(|w| w.set_dbp(true));
}

/// Call first thing at boot. Parks in DFU after repeated crashes; otherwise arms self-restart.
pub fn boot_guard() {
    backup_access();
    let csr = pac::RCC.csr().read();
    let crashed = csr.iwdgrstf() || unsafe { BKP_DR8.read_volatile() } == PANIC_MARK;
    pac::RCC.csr().modify(|w| w.set_rmvf(true));
    unsafe {
        BKP_DR8.write_volatile(0);
        let count = if crashed { BKP_DR9.read_volatile() + 1 } else { 0 };
        if count >= MAX_CRASHES {
            BKP_DR9.write_volatile(0);
            enter_bootloader();
        }
        BKP_DR9.write_volatile(count);
        BKP_DR10.write_volatile(START_APP);
    }
}

/// The firmware has run long enough to count as healthy.
pub fn mark_healthy() {
    unsafe { BKP_DR9.write_volatile(0) };
}

/// Restart the firmware after a panic or fault (counted by `boot_guard`).
pub fn restart_after_crash() -> ! {
    cortex_m::interrupt::disable();
    backup_access();
    unsafe {
        BKP_DR8.write_volatile(PANIC_MARK);
        BKP_DR10.write_volatile(START_APP);
    }
    cortex_m::peripheral::SCB::sys_reset()
}

/// Restart into the bootloader's DFU mode: write the stay-in-bootloader mark that the
/// stm32duino bootloader reads from BKP_DR10, then reset.
pub fn enter_bootloader() -> ! {
    cortex_m::interrupt::disable();
    backup_access();
    unsafe { BKP_DR10.write_volatile(STAY_IN_DFU) };
    cortex_m::peripheral::SCB::sys_reset()
}
