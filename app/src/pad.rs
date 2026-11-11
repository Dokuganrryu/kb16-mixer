//! The link to the pad: its vendor raw-HID interface (usage page 0xFF60), 32-byte reports
//! that start with 0xD1. The wire format lives in the `kb16-mixer-protocol` crate.
//!
//! Two handles are opened on the same interface. `listen` blocks on one for knob and key
//! events, so nothing polls; `Link` is the other, used by the mixer to write.

use hidapi::{HidApi, HidDevice};
use kb16_mixer_protocol::{self as protocol, HostMsg, Key, PadMsg, Report};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    DBT_DEVTYP_DEVICEINTERFACE, DEV_BROADCAST_DEVICEINTERFACE_W, DEVICE_NOTIFY_WINDOW_HANDLE,
    RegisterDeviceNotificationW,
};
use windows::core::GUID;

const HEARTBEAT: Duration = Duration::from_millis(1000);

pub enum PadEvent {
    Turn {
        knob: u8,
        delta: i32,
    },
    Press {
        knob: u8,
    },
    Key {
        row: u8,
        col: u8,
    },
    KeyUp {
        row: u8,
        col: u8,
    },
    Connected(bool),
    /// Not from the pad: a short message for its screen (config problems and the like).
    Notice(String),
    /// Not from the pad: the app is closing; undo anything temporary first. Answered once that
    /// is done and the state is saved.
    Shutdown(Sender<()>),
    /// Not from the pad: the settings window asks which apps have a volume slider right now
    /// (exe names, lowercase, sorted).
    Apps(Sender<Vec<String>>),
    /// Not from the pad: the settings window takes an app out of the Move key's list (the
    /// mixer is the only writer of apps.toml). `done` answers once the file is written.
    Unmove {
        exe: String,
        done: Sender<()>,
    },
    /// Not from the pad: answered once the mixer has taken in the config that is in force (a
    /// renamed channel's moved apps are rewritten then).
    CaughtUp(Sender<()>),
}

/// USB release number of the connected pad plus 1 << 16, or 0 while none is connected.
static PAD: AtomicU32 = AtomicU32::new(0);

/// The connected pad's firmware release number (bcdDevice), or None while no pad is connected.
pub fn connected_release() -> Option<u16> {
    let v = PAD.load(Ordering::Relaxed);
    (v != 0).then_some(v as u16)
}

/// True for the raw-HID interface of a pad that runs this project's firmware. The stock firmware
/// has the same VID, PID and usage page; only the product string tells the two apart.
fn is_ours(vendor_id: u16, product_id: u16, usage_page: u16, product: Option<&str>) -> bool {
    vendor_id == protocol::VID
        && product_id == protocol::PID
        && usage_page == protocol::USAGE_PAGE
        && product == Some(protocol::PRODUCT)
}

fn pad_info(api: &HidApi) -> Option<&hidapi::DeviceInfo> {
    api.device_list().find(|d| is_ours(d.vendor_id(), d.product_id(), d.usage_page(), d.product_string()))
}

fn find(api: &mut HidApi) -> Option<CString> {
    let _ = api.refresh_devices();
    pad_info(api).map(|d| d.path().to_owned())
}

/// Without a pad, the next search waits for Windows to report a new HID interface, or this long.
/// A search reads every HID device of the PC (a few ms of CPU), so it must not run every second.
const SEARCH_AGAIN: Duration = Duration::from_secs(10);

/// Set when Windows reports a new HID interface (WM_DEVICECHANGE on the hidden window).
static ARRIVED: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// Called by the hidden window when a HID interface arrives: the search runs at once.
pub fn device_arrived() {
    *ARRIVED.0.lock().unwrap() = true;
    ARRIVED.1.notify_all();
}

/// Waits until a HID interface arrives or `timeout` ends; true if one arrived.
fn wait_for_arrival(timeout: Duration) -> bool {
    let guard = ARRIVED.0.lock().unwrap();
    let (mut arrived, _) = ARRIVED.1.wait_timeout_while(guard, timeout, |a| !*a).unwrap();
    std::mem::take(&mut *arrived)
}

/// Asks Windows to send WM_DEVICECHANGE to `hwnd` when a HID interface arrives.
pub fn watch_arrivals(hwnd: HWND) {
    // GUID_DEVINTERFACE_HID
    const HID: GUID = GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);
    let filter = DEV_BROADCAST_DEVICEINTERFACE_W {
        dbcc_size: size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
        dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
        dbcc_classguid: HID,
        ..Default::default()
    };
    // SAFETY: `filter` is a filled-in DEV_BROADCAST_DEVICEINTERFACE_W that lives until the call
    // returns; `hwnd` is the hidden window of this process.
    let handle = unsafe {
        RegisterDeviceNotificationW(HANDLE(hwnd.0), &filter as *const _ as *const _, DEVICE_NOTIFY_WINDOW_HANDLE)
    };
    if let Err(e) = handle {
        crate::log(&format!("no device notifications, the pad search runs every {}s: {e}", SEARCH_AGAIN.as_secs()));
    }
}

/// Blocks on the pad's input for the life of the app, reconnecting as needed.
pub fn listen(events: Sender<PadEvent>, paused: Arc<AtomicBool>) {
    let mut api = match HidApi::new() {
        Ok(a) => a,
        Err(e) => return crate::log(&format!("hidapi init failed: {e}")),
    };
    loop {
        if paused.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }
        let Some(path) = find(&mut api) else {
            wait_for_arrival(SEARCH_AGAIN);
            continue;
        };
        let Ok(dev) = api.open_path(&path) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let release = pad_info(&api).map_or(0, |d| d.release_number());
        PAD.store(1 << 16 | release as u32, Ordering::Relaxed);
        let _ = events.send(PadEvent::Connected(true));
        let mut buf = [0u8; protocol::REPORT_LEN];
        // The timeout only exists so a pause is noticed; input wakes the read at once.
        while !paused.load(Ordering::Relaxed) {
            match dev.read_timeout(&mut buf, 300) {
                Ok(n) => {
                    let event = match PadMsg::decode(&buf[..n]) {
                        Some(PadMsg::Turn { knob, delta }) => PadEvent::Turn { knob, delta: delta as i32 },
                        Some(PadMsg::Press { knob }) => PadEvent::Press { knob },
                        Some(PadMsg::KeyDown { row, col }) => PadEvent::Key { row, col },
                        Some(PadMsg::KeyUp { row, col }) => PadEvent::KeyUp { row, col },
                        None => continue,
                    };
                    let _ = events.send(event);
                }
                Err(_) => break,
            }
        }
        drop(dev);
        PAD.store(0, Ordering::Relaxed);
        let _ = events.send(PadEvent::Connected(false));
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// The mixer's write side.
pub struct Link {
    api: Option<HidApi>,
    dev: Option<HidDevice>,
    last_write: Instant,
}

impl Link {
    /// Touches nothing yet: the device list is only read on the first `open`.
    pub fn new() -> Link {
        Link { api: None, dev: None, last_write: Instant::now() }
    }

    pub fn is_open(&self) -> bool {
        self.dev.is_some()
    }

    pub fn open(&mut self) -> bool {
        if self.api.is_none() {
            self.api = HidApi::new().ok();
        }
        let Some(api) = self.api.as_mut() else { return false };
        self.dev = find(api).and_then(|p| api.open_path(&p).ok());
        self.dev.is_some()
    }

    pub fn close(&mut self) {
        self.dev = None;
    }

    /// Sends one report; a failed write drops the handle (the listener will report the unplug).
    pub fn send(&mut self, msg: &Report) {
        let Some(dev) = &self.dev else { return };
        let mut report = [0u8; protocol::REPORT_LEN + 1]; // report id 0 + payload
        report[1..].copy_from_slice(msg);
        if dev.write(&report).is_err() {
            self.dev = None;
        }
        self.last_write = Instant::now();
    }

    /// The pad drops back to plain-keyboard mode after 3 s of silence; any report counts.
    pub fn keep_alive(&mut self) {
        if self.last_write.elapsed() >= HEARTBEAT {
            self.send(&HostMsg::Heartbeat.encode());
        }
    }
}

/// Asks the pad to restart into its bootloader. Returns false if no pad answered.
pub fn enter_bootloader() -> bool {
    let mut link = Link::new();
    if !link.open() {
        return false;
    }
    link.send(&HostMsg::Bootloader.encode());
    link.is_open()
}

/// The first `N` bytes of `text` the pad's fonts can draw (printable ASCII), zero-padded.
fn printable<const N: usize>(text: &str, upper: bool) -> [u8; N] {
    let mut out = [0u8; N];
    let bytes = text.bytes().filter(|b| b.is_ascii_graphic() || *b == b' ');
    for (dst, b) in out.iter_mut().zip(bytes) {
        *dst = if upper { b.to_ascii_uppercase() } else { b };
    }
    out
}

/// What the pad's big readout shows for a moment.
pub enum Show {
    /// A name, a value (0..=100) and a bar.
    Level { name: String, volume: u8, muted: bool },
    /// Text on its own.
    Notice(String),
}

pub fn popup_message(show: &Show) -> Report {
    let popup = match show {
        Show::Level { name, volume, muted } => {
            protocol::Popup { knob: 0, volume: (*volume).min(100), muted: *muted, name: printable(name, false) }
        }
        Show::Notice(text) => protocol::Popup::notice(printable(text, false)),
    };
    HostMsg::Popup(popup).encode()
}

/// Look and idle settings: channel colors, LED brightness, how many pixel rows the screen
/// content is moved down (the case hides the top of the panel), and the minutes of quiet
/// before the screen turns off and the lights dim (0 = never).
pub fn settings_message(colors: &[[u8; 3]], cfg: &crate::config::Config) -> Report {
    // On the wire 0 means "pad default" (older apps) and 255 means never.
    let minutes = |m: u8| if m == 0 { protocol::NEVER } else { m.min(240) };
    let mut c = [[0u8; 3]; protocol::CHANNELS];
    for (dst, src) in c.iter_mut().zip(colors) {
        *dst = *src;
    }
    HostMsg::Settings(protocol::Settings {
        channels: colors.len().min(protocol::CHANNELS) as u8,
        colors: c,
        brightness: cfg.brightness.max(1),
        oled_top_plus_one: cfg.oled_top.min(6) + 1,
        screen_off_min: minutes(cfg.screen_off_min),
        lights_dim_min: minutes(cfg.lights_dim_min),
    })
    .encode()
}

/// Short channel names for the screen (5 characters show).
pub fn names_message<'a>(names: impl Iterator<Item = &'a str>) -> Report {
    let mut out = [[0u8; protocol::NAME_LEN]; protocol::CHANNELS];
    for (dst, n) in out.iter_mut().zip(names) {
        *dst = printable(n, true);
    }
    HostMsg::Names(out).encode()
}

/// The name for the logo screen (already checked by the config: capitals, digits, spaces).
pub fn logo_name_message(name: &str) -> Report {
    HostMsg::LogoName(printable(name, true)).encode()
}

/// Tells the pad what its 16 keys do: two reports of 8 keys each.
pub fn key_messages(grid: &crate::config::KeyGrid) -> [Report; 2] {
    use crate::config::KeyAction::*;
    let key = |i: usize| match &grid[i / 4][i % 4] {
        Mute | Focus | Talk | Move | Page | Open(_) => Key::Host,
        &Keyboard { code, mods } => Key::Keyboard { code, mods },
        &Media { tap, hold } => Key::Media { tap, hold },
        Nothing => Key::Nothing,
    };
    [0, 1].map(|half| {
        let first = half * protocol::KEYS_PER_REPORT;
        HostMsg::Keys(protocol::Keys { first: first as u8, keys: core::array::from_fn(|i| key(first + i)) }).encode()
    })
}

pub use protocol::Frame;

/// Live levels, faders and mute state.
pub fn frame_message(f: &Frame) -> Report {
    HostMsg::Frame(*f).encode()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_firmware_is_ours() {
        let (vid, pid, page) = (protocol::VID, protocol::PID, protocol::USAGE_PAGE);
        assert!(is_ours(vid, pid, page, Some("KB16 Mixer")), "this firmware's raw-HID interface");
        assert!(!is_ours(vid, pid, page, Some("KB16-01")), "stock firmware: same ids, other product string");
        assert!(!is_ours(vid, pid, page, None), "no product string");
        assert!(!is_ours(vid, pid, 0x0001, Some("KB16 Mixer")), "the keyboard interface of the same pad");
        assert!(!is_ours(vid, pid, 0x000C, Some("KB16 Mixer")), "the media-key interface of the same pad");
        assert!(!is_ours(0x1EAF, 0x0003, page, Some("KB16 Mixer")), "other ids");
    }

    #[test]
    fn a_device_arrival_wakes_the_search() {
        device_arrived();
        let start = Instant::now();
        assert!(wait_for_arrival(Duration::from_secs(5)));
        assert!(start.elapsed() < Duration::from_secs(1), "woken at once");
        assert!(!wait_for_arrival(Duration::from_millis(50)), "one arrival wakes one search");
    }

    #[test]
    fn notices_reach_the_pad_as_text_only() {
        let notice = HostMsg::decode(&popup_message(&Show::Notice("Config error".into())));
        assert!(matches!(notice, Some(HostMsg::Popup(p)) if p.is_notice() && p.name.starts_with(b"Config error")));
        let level = HostMsg::decode(&popup_message(&Show::Level { name: "Chat".into(), volume: 250, muted: false }));
        assert!(matches!(level, Some(HostMsg::Popup(p)) if p.volume == 100), "a level is clamped, never a notice");
    }
}
