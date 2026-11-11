//! The link to the pad: its vendor raw-HID interface (usage page 0xFF60), 32-byte reports
//! that start with 0xD1. The wire format lives in the `kb16-mixer-protocol` crate.
//!
//! Two handles are opened on the same interface. `listen` blocks on one for knob and key
//! events, so nothing polls; `Link` is the other, used by the mixer to write.

use hidapi::{HidApi, HidDevice};
use kb16_mixer_protocol::{self as protocol, HostMsg, Key, PadMsg, Report};
use std::ffi::CString;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

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
    /// Not from the pad: the app is closing; undo anything temporary first.
    Shutdown,
}

fn find(api: &mut HidApi) -> Option<CString> {
    let _ = api.refresh_devices();
    api.device_list()
        .find(|d| {
            d.vendor_id() == protocol::VID && d.product_id() == protocol::PID && d.usage_page() == protocol::USAGE_PAGE
        })
        .map(|d| d.path().to_owned())
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
        let Some(dev) = find(&mut api).and_then(|p| api.open_path(&p).ok()) else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
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
    pub fn new() -> Link {
        Link { api: HidApi::new().ok(), dev: None, last_write: Instant::now() }
    }

    pub fn is_open(&self) -> bool {
        self.dev.is_some()
    }

    pub fn open(&mut self) -> bool {
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
    let key = |i: usize| match grid[i / 4][i % 4] {
        Mute | Focus | Talk | Move | Page => Key::Host,
        Keyboard { code, mods } => Key::Keyboard { code, mods },
        Media { tap, hold } => Key::Media { tap, hold },
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
    fn notices_reach_the_pad_as_text_only() {
        let notice = HostMsg::decode(&popup_message(&Show::Notice("Config error".into())));
        assert!(matches!(notice, Some(HostMsg::Popup(p)) if p.is_notice() && p.name.starts_with(b"Config error")));
        let level = HostMsg::decode(&popup_message(&Show::Level { name: "Chat".into(), volume: 250, muted: false }));
        assert!(matches!(level, Some(HostMsg::Popup(p)) if p.volume == 100), "a level is clamped, never a notice");
    }
}
