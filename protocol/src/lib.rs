//! The wire protocol between the KB16 mixer firmware and the PC app, shared by both sides.
//!
//! Every message is one 32-byte report on the pad's vendor raw-HID interface (usage page
//! 0xFF60). Byte 0 is [`MAGIC`], byte 1 the message id. Unused bytes are zero.
//!
//! ```text
//! host -> pad
//!   [D1 00]                                        heartbeat
//!   [D1 10 knob vol muted name[16]]                popup on the OLED (vol 255 = text only)
//!   [D1 20 n r0 g0 b0 .. r3 g3 b3 bright top+1 screen dim idle%]
//!                                                  channel colors; LED brightness (byte 15);
//!                                                  OLED top margin + 1 (byte 16); minutes of
//!                                                  quiet before the screen turns off (17) and
//!                                                  the lights dim (18). 0 = default, 255 = never.
//!                                                  Byte 19: percent of normal brightness the
//!                                                  dimmed lights keep, 1..=100; 0 = default
//!   [D1 21 lvl0..3 vol0..3 mutes sel flags]        live frame, ~30 per second
//!   [D1 22 name0[6] .. name3[6]]                   channel names
//!   [D1 23 first (kind a b) x8]                    what 8 keys do, starting at key `first`
//!                                                  (key = row * 4 + col); see `Key`
//!   [D1 24 name[10]]                               name on the logo screen, zero-padded
//!   [D1 7F 'B' 'O' 'O' 'T']                        restart into the bootloader
//! pad -> host
//!   [D1 01 knob delta]   knob turned          [D1 02 knob 0]    knob pressed
//!   [D1 03 row col]      key pressed          [D1 04 row col]   key released
//!   [D1 05 knob 0]       knob let go (newer firmware only; older apps ignore the id)
//! flags: bit0 = keys belong to the mixer, bit1 = mic muted, bit2 = programme audio is playing,
//!        bit3 = the second key page is active.
//! ```
//!
//! This crate only lays out bytes. What a value means (defaults, clamping, which characters
//! the screen can show) stays with the side that acts on it.

#![no_std]
#![forbid(unsafe_code)]

/// USB vendor and product id of the pad (the stock DOIO KB16-01 ids).
pub const VID: u16 = 0xD010;
pub const PID: u16 = 0x1601;
/// USB product string of this firmware. The stock firmware has the same VID, PID and usage page,
/// so the app also checks this string before it talks to a pad.
pub const PRODUCT: &str = "KB16 Mixer";
/// Usage page of the raw-HID interface (the one QMK/VIA use).
pub const USAGE_PAGE: u16 = 0xFF60;

/// First byte of every report.
pub const MAGIC: u8 = 0xD1;
/// Every report is this long.
pub const REPORT_LEN: usize = 32;
pub type Report = [u8; REPORT_LEN];

/// Mixer channels, one per key column.
pub const CHANNELS: usize = 4;
/// Keys described by one [`HostMsg::Keys`] report.
pub const KEYS_PER_REPORT: usize = 8;
/// Bytes of a channel name.
pub const NAME_LEN: usize = 6;
/// Letters of a channel name the mixer screen draws: its strip is 31 px wide and a 5x7 letter
/// takes 6, so a sixth letter would not fit.
pub const NAME_SHOWN: usize = 5;
/// Bytes of popup text.
pub const POPUP_LEN: usize = 16;
/// Bytes of the name on the logo screen.
pub const LOGO_NAME_LEN: usize = 10;
/// Popup volume value that means "just show the text".
pub const NOTICE: u8 = 255;
/// Idle-minutes value that means "never".
pub const NEVER: u8 = 255;

/// Message ids (byte 1).
pub mod id {
    pub const HEARTBEAT: u8 = 0x00;
    pub const TURN: u8 = 0x01;
    pub const PRESS: u8 = 0x02;
    pub const KEY_DOWN: u8 = 0x03;
    pub const KEY_UP: u8 = 0x04;
    pub const KNOB_UP: u8 = 0x05;
    pub const POPUP: u8 = 0x10;
    pub const SETTINGS: u8 = 0x20;
    pub const FRAME: u8 = 0x21;
    pub const NAMES: u8 = 0x22;
    pub const KEYS: u8 = 0x23;
    pub const LOGO_NAME: u8 = 0x24;
    pub const BOOTLOADER: u8 = 0x7F;
}

const BOOT: &[u8; 4] = b"BOOT";

/// What a key does while the PC app is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    /// Report press and release to the PC app (mute, focus, ...).
    Host,
    /// Type a keyboard key: HID usage and modifier bits.
    Keyboard {
        code: u8,
        mods: u8,
    },
    /// Send a media key. With `hold` non-zero, a tap sends `tap` and a long press sends `hold`.
    Media {
        tap: u8,
        hold: u8,
    },
    Nothing,
}

impl Key {
    fn encode(self) -> [u8; 3] {
        match self {
            Key::Host => [0, 0, 0],
            Key::Keyboard { code, mods } => [1, code, mods],
            Key::Media { tap, hold } => [2, tap, hold],
            Key::Nothing => [3, 0, 0],
        }
    }

    fn decode(b: &[u8]) -> Key {
        match b[0] {
            1 => Key::Keyboard { code: b[1], mods: b[2] },
            2 => Key::Media { tap: b[1], hold: b[2] },
            3 => Key::Nothing,
            _ => Key::Host,
        }
    }
}

/// Big readout on the pad's screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Popup {
    pub knob: u8,
    /// 0..=100, or [`NOTICE`].
    pub volume: u8,
    pub muted: bool,
    pub name: [u8; POPUP_LEN],
}

impl Popup {
    /// Text on its own: no value, no bar.
    pub fn notice(name: [u8; POPUP_LEN]) -> Popup {
        Popup { knob: 0, volume: NOTICE, muted: false, name }
    }

    pub fn is_notice(&self) -> bool {
        self.volume == NOTICE
    }
}

/// Look and idle settings, as raw wire values.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// Channels in use.
    pub channels: u8,
    pub colors: [[u8; 3]; CHANNELS],
    /// LED brightness; 0 = pad default.
    pub brightness: u8,
    /// OLED top margin plus one; 0 = pad default.
    pub oled_top_plus_one: u8,
    /// Minutes of quiet before the screen turns off; 0 = pad default, [`NEVER`] = never.
    pub screen_off_min: u8,
    /// Minutes of quiet before the lights dim; 0 = pad default, [`NEVER`] = never.
    pub lights_dim_min: u8,
    /// Brightness the dimmed lights keep, in percent of the normal level (1..=100); 0 = pad
    /// default. Added after the first release: an older host leaves the byte zero, and older
    /// firmware ignores it.
    pub idle_led_percent: u8,
    /// How the screen draws (0 = Classic, 1 = Lean, 2 = Numbers; the pad reads an unknown
    /// value as 0). Added after byte 19: an older host leaves it zero, and older firmware
    /// ignores it.
    pub screen_style: u8,
}

/// Live levels, faders and mute state.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Frame {
    pub levels: [u8; CHANNELS],
    pub volumes: [u8; CHANNELS],
    /// Bit n = channel n muted.
    pub mutes: u8,
    /// Channel Focus is on, 255 = none.
    pub selected: u8,
    /// Keys belong to the mixer (false = they type their fallback keys).
    pub mixer_keys: bool,
    pub mic_muted: bool,
    /// Some app (not the microphone) is audibly playing, as the host judges it: after its own
    /// volume and mute, sustained, and not with the output device muted. The pad counts this
    /// as activity for its idle timers; raw `levels` are only for the meters.
    pub playing: bool,
    /// The host has switched the keys to its second page (the screen shows a small marker).
    pub second_page: bool,
}

/// What [`KEYS_PER_REPORT`] keys do, starting at key `first` (key = row * 4 + col).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Keys {
    pub first: u8,
    pub keys: [Key; KEYS_PER_REPORT],
}

/// Host -> pad.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostMsg {
    Heartbeat,
    Popup(Popup),
    Settings(Settings),
    Frame(Frame),
    Names([[u8; NAME_LEN]; CHANNELS]),
    Keys(Keys),
    /// The name the logo screen shows (the pad keeps it across power cycles).
    LogoName([u8; LOGO_NAME_LEN]),
    Bootloader,
    /// An id this side does not know; it still shows the host is there.
    Unknown(u8),
}

impl HostMsg {
    pub fn encode(&self) -> Report {
        let mut r = [0u8; REPORT_LEN];
        r[0] = MAGIC;
        match self {
            HostMsg::Heartbeat => r[1] = id::HEARTBEAT,
            HostMsg::Popup(p) => {
                r[1..5].copy_from_slice(&[id::POPUP, p.knob, p.volume, p.muted as u8]);
                r[5..21].copy_from_slice(&p.name);
            }
            HostMsg::Settings(s) => {
                r[1] = id::SETTINGS;
                r[2] = s.channels;
                for (i, c) in s.colors.iter().enumerate() {
                    r[3 + i * 3..6 + i * 3].copy_from_slice(c);
                }
                r[15] = s.brightness;
                r[16] = s.oled_top_plus_one;
                r[17] = s.screen_off_min;
                r[18] = s.lights_dim_min;
                r[19] = s.idle_led_percent;
                r[20] = s.screen_style;
            }
            HostMsg::Frame(f) => {
                r[1] = id::FRAME;
                r[2..6].copy_from_slice(&f.levels);
                r[6..10].copy_from_slice(&f.volumes);
                r[10] = f.mutes;
                r[11] = f.selected;
                r[12] =
                    f.mixer_keys as u8 | (f.mic_muted as u8) << 1 | (f.playing as u8) << 2 | (f.second_page as u8) << 3;
            }
            HostMsg::Names(names) => {
                r[1] = id::NAMES;
                for (i, n) in names.iter().enumerate() {
                    r[2 + i * NAME_LEN..2 + (i + 1) * NAME_LEN].copy_from_slice(n);
                }
            }
            HostMsg::Keys(k) => {
                r[1] = id::KEYS;
                r[2] = k.first;
                for (i, key) in k.keys.iter().enumerate() {
                    r[3 + i * 3..6 + i * 3].copy_from_slice(&key.encode());
                }
            }
            HostMsg::LogoName(name) => {
                r[1] = id::LOGO_NAME;
                r[2..2 + LOGO_NAME_LEN].copy_from_slice(name);
            }
            HostMsg::Bootloader => {
                r[1] = id::BOOTLOADER;
                r[2..6].copy_from_slice(BOOT);
            }
            HostMsg::Unknown(other) => r[1] = *other,
        }
        r
    }

    /// Reads one report. A short report counts as zero-padded. `None` if it is not ours, or
    /// a bootloader request without the password.
    pub fn decode(report: &[u8]) -> Option<HostMsg> {
        if report.len() < 2 || report[0] != MAGIC {
            return None;
        }
        let mut r = [0u8; REPORT_LEN];
        let n = report.len().min(REPORT_LEN);
        r[..n].copy_from_slice(&report[..n]);
        Some(match r[1] {
            id::HEARTBEAT => HostMsg::Heartbeat,
            id::POPUP => {
                let mut name = [0u8; POPUP_LEN];
                name.copy_from_slice(&r[5..21]);
                HostMsg::Popup(Popup { knob: r[2], volume: r[3], muted: r[4] != 0, name })
            }
            id::SETTINGS => {
                let mut colors = [[0u8; 3]; CHANNELS];
                for (i, c) in colors.iter_mut().enumerate() {
                    c.copy_from_slice(&r[3 + i * 3..6 + i * 3]);
                }
                HostMsg::Settings(Settings {
                    channels: r[2],
                    colors,
                    brightness: r[15],
                    oled_top_plus_one: r[16],
                    screen_off_min: r[17],
                    lights_dim_min: r[18],
                    idle_led_percent: r[19],
                    screen_style: r[20],
                })
            }
            id::FRAME => {
                let mut levels = [0u8; CHANNELS];
                let mut volumes = [0u8; CHANNELS];
                levels.copy_from_slice(&r[2..6]);
                volumes.copy_from_slice(&r[6..10]);
                HostMsg::Frame(Frame {
                    levels,
                    volumes,
                    mutes: r[10],
                    selected: r[11],
                    mixer_keys: r[12] & 1 != 0,
                    mic_muted: r[12] & 2 != 0,
                    playing: r[12] & 4 != 0,
                    second_page: r[12] & 8 != 0,
                })
            }
            id::NAMES => {
                let mut names = [[0u8; NAME_LEN]; CHANNELS];
                for (i, n) in names.iter_mut().enumerate() {
                    n.copy_from_slice(&r[2 + i * NAME_LEN..2 + (i + 1) * NAME_LEN]);
                }
                HostMsg::Names(names)
            }
            id::KEYS => {
                let mut keys = [Key::Host; KEYS_PER_REPORT];
                for (i, k) in keys.iter_mut().enumerate() {
                    *k = Key::decode(&r[3 + i * 3..6 + i * 3]);
                }
                HostMsg::Keys(Keys { first: r[2], keys })
            }
            id::LOGO_NAME => {
                let mut name = [0u8; LOGO_NAME_LEN];
                name.copy_from_slice(&r[2..2 + LOGO_NAME_LEN]);
                HostMsg::LogoName(name)
            }
            id::BOOTLOADER if &r[2..6] == BOOT => HostMsg::Bootloader,
            id::BOOTLOADER => return None,
            other => HostMsg::Unknown(other),
        })
    }
}

/// Pad -> host.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PadMsg {
    /// One detent; `delta` is +1 or -1.
    Turn {
        knob: u8,
        delta: i8,
    },
    Press {
        knob: u8,
    },
    KeyDown {
        row: u8,
        col: u8,
    },
    KeyUp {
        row: u8,
        col: u8,
    },
    /// The knob's push button let go. Firmware from before this message never sends it, so a
    /// host must not wait for it.
    KnobUp {
        knob: u8,
    },
}

impl PadMsg {
    pub fn encode(&self) -> Report {
        let (id, a, b) = match *self {
            PadMsg::Turn { knob, delta } => (id::TURN, knob, delta as u8),
            PadMsg::Press { knob } => (id::PRESS, knob, 0),
            PadMsg::KeyDown { row, col } => (id::KEY_DOWN, row, col),
            PadMsg::KeyUp { row, col } => (id::KEY_UP, row, col),
            PadMsg::KnobUp { knob } => (id::KNOB_UP, knob, 0),
        };
        let mut r = [0u8; REPORT_LEN];
        r[..4].copy_from_slice(&[MAGIC, id, a, b]);
        r
    }

    /// Reads one report; `None` if it is too short, not ours, or an unknown id.
    pub fn decode(r: &[u8]) -> Option<PadMsg> {
        if r.len() < 4 || r[0] != MAGIC {
            return None;
        }
        Some(match r[1] {
            id::TURN => PadMsg::Turn { knob: r[2], delta: r[3] as i8 },
            id::PRESS => PadMsg::Press { knob: r[2] },
            id::KEY_DOWN => PadMsg::KeyDown { row: r[2], col: r[3] },
            id::KEY_UP => PadMsg::KeyUp { row: r[2], col: r[3] },
            id::KNOB_UP => PadMsg::KnobUp { knob: r[2] },
            _ => return None,
        })
    }
}

/// The firmware's version as the USB device release number (bcdDevice): `0xJJMN` in binary-coded
/// decimal for version JJ.M.N, so the PC can read it from the device list without opening the
/// pad. Takes the crate version string ("0.2.0"); used in a `const`, a version that does not
/// fit (major over 99, minor or patch over 9) stops the build.
pub const fn device_release(version: &str) -> u16 {
    let v = version.as_bytes();
    let mut parts = [0u16; 3];
    let (mut part, mut i) = (0, 0);
    while i < v.len() && v[i] != b'-' && v[i] != b'+' {
        match v[i] {
            b'.' => part += 1,
            d @ b'0'..=b'9' if part < 3 => parts[part] = parts[part] * 10 + (d - b'0') as u16,
            _ => panic!("version is not major.minor.patch"),
        }
        i += 1;
    }
    let [major, minor, patch] = parts;
    assert!(part == 2 && major <= 99 && minor <= 9 && patch <= 9, "version does not fit bcdDevice");
    (major / 10) << 12 | (major % 10) << 8 | minor << 4 | patch
}

/// The release number firmware sent before it carried its version.
pub const UNVERSIONED_RELEASE: u16 = 0x0300;

/// (major, minor, patch) from a release number made by [`device_release`].
pub const fn release_version(bcd: u16) -> (u8, u8, u8) {
    let major = (bcd >> 12) * 10 + ((bcd >> 8) & 0xF);
    (major as u8, ((bcd >> 4) & 0xF) as u8, (bcd & 0xF) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_firmware_version_travels_as_bcd() {
        assert_eq!(device_release("0.1.0"), 0x0010);
        assert_eq!(device_release("0.2.0"), 0x0020);
        assert_eq!(device_release("12.3.4"), 0x1234);
        assert_eq!(device_release("1.0.9-beta"), 0x0109);
        for v in ["0.1.0", "12.3.4", "99.9.9", "1.0.0"] {
            let (a, b, c) = release_version(device_release(v));
            let mut text = [0u8; 8];
            let n = fmt3(&mut text, a, b, c);
            assert_eq!(&text[..n], v.as_bytes());
        }
    }

    fn fmt3(out: &mut [u8], a: u8, b: u8, c: u8) -> usize {
        let mut n = 0;
        for (i, x) in [a, b, c].into_iter().enumerate() {
            if i > 0 {
                out[n] = b'.';
                n += 1;
            }
            if x >= 10 {
                out[n] = b'0' + x / 10;
                n += 1;
            }
            out[n] = b'0' + x % 10;
            n += 1;
        }
        n
    }

    fn host_roundtrip(m: HostMsg) -> Report {
        let r = m.encode();
        assert_eq!(HostMsg::decode(&r), Some(m), "{m:?}");
        r
    }

    #[test]
    fn host_messages_round_trip() {
        host_roundtrip(HostMsg::Heartbeat);
        host_roundtrip(HostMsg::Bootloader);
        host_roundtrip(HostMsg::Unknown(0x55));
        host_roundtrip(HostMsg::Names([*b"MIC   ", *b"CHAT\0\0", *b"GAME  ", *b"MEDIA "]));
        host_roundtrip(HostMsg::LogoName(*b"DESK PC\0\0\0"));
        host_roundtrip(HostMsg::Popup(Popup { knob: 2, volume: NOTICE, muted: true, name: *b"Config error\0\0\0\0" }));
        host_roundtrip(HostMsg::Settings(Settings {
            channels: 4,
            colors: [[1, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]],
            brightness: 166,
            oled_top_plus_one: 4,
            screen_off_min: NEVER,
            lights_dim_min: 30,
            idle_led_percent: 6,
            screen_style: 2,
        }));
        for flags in 0..16u8 {
            host_roundtrip(HostMsg::Frame(Frame {
                levels: [0, 40, 200, 255],
                volumes: [100, 0, 55, 7],
                mutes: 0b1010,
                selected: 255,
                mixer_keys: flags & 1 != 0,
                mic_muted: flags & 2 != 0,
                playing: flags & 4 != 0,
                second_page: flags & 8 != 0,
            }));
        }
        host_roundtrip(HostMsg::Keys(Keys {
            first: 8,
            keys: [
                Key::Host,
                Key::Keyboard { code: 0x68, mods: 0x03 },
                Key::Media { tap: 0xB5, hold: 0xB6 },
                Key::Media { tap: 0xCD, hold: 0 },
                Key::Nothing,
                Key::Host,
                Key::Keyboard { code: 0x04, mods: 0 },
                Key::Nothing,
            ],
        }));
    }

    #[test]
    fn pad_messages_round_trip() {
        for m in [
            PadMsg::Turn { knob: 0, delta: 1 },
            PadMsg::Turn { knob: 2, delta: -1 },
            PadMsg::Press { knob: 1 },
            PadMsg::KeyDown { row: 3, col: 0 },
            PadMsg::KeyUp { row: 0, col: 3 },
            PadMsg::KnobUp { knob: 2 },
        ] {
            assert_eq!(PadMsg::decode(&m.encode()), Some(m));
        }
    }

    /// The exact bytes both sides agreed on before this crate existed.
    #[test]
    fn wire_layout_is_fixed() {
        let mut expect = [0u8; REPORT_LEN];
        expect[..6].copy_from_slice(&[0xD1, 0x7F, b'B', b'O', b'O', b'T']);
        assert_eq!(HostMsg::Bootloader.encode(), expect);

        let popup = HostMsg::Popup(Popup { knob: 0, volume: 42, muted: true, name: *b"Chat\0\0\0\0\0\0\0\0\0\0\0\0" });
        let r = popup.encode();
        assert_eq!(&r[..9], &[0xD1, 0x10, 0, 42, 1, b'C', b'h', b'a', b't']);
        assert!(r[9..].iter().all(|&b| b == 0));

        let s = HostMsg::Settings(Settings {
            channels: 2,
            colors: [[0xAA, 0xBB, 0xCC], [1, 2, 3], [0; 3], [0; 3]],
            brightness: 9,
            oled_top_plus_one: 4,
            screen_off_min: 255,
            lights_dim_min: 10,
            idle_led_percent: 40,
            screen_style: 1,
        })
        .encode();
        assert_eq!(&s[..21], &[0xD1, 0x20, 2, 0xAA, 0xBB, 0xCC, 1, 2, 3, 0, 0, 0, 0, 0, 0, 9, 4, 255, 10, 40, 1]);
        assert!(s[21..].iter().all(|&b| b == 0));

        let f = HostMsg::Frame(Frame {
            levels: [1, 2, 3, 4],
            volumes: [5, 6, 7, 8],
            mutes: 9,
            selected: 10,
            mixer_keys: false,
            mic_muted: true,
            playing: true,
            second_page: true,
        })
        .encode();
        assert_eq!(&f[..13], &[0xD1, 0x21, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 14]);

        let mut names = [[b' '; NAME_LEN]; CHANNELS];
        names[3] = *b"MEDIA\0";
        let n = HostMsg::Names(names).encode();
        assert_eq!(&n[2..8], b"      ");
        assert_eq!(&n[20..26], b"MEDIA\0");

        let mut keys = [Key::Host; KEYS_PER_REPORT];
        keys[0] = Key::Keyboard { code: 0x68, mods: 2 };
        keys[1] = Key::Media { tap: 0xB5, hold: 0xB6 };
        keys[7] = Key::Nothing;
        let k = HostMsg::Keys(Keys { first: 8, keys }).encode();
        assert_eq!(&k[..9], &[0xD1, 0x23, 8, 1, 0x68, 2, 2, 0xB5, 0xB6]);
        assert_eq!(&k[24..27], &[3, 0, 0]);

        let l = HostMsg::LogoName(*b"KB16\0\0\0\0\0\0").encode();
        assert_eq!(&l[..7], &[0xD1, 0x24, b'K', b'B', b'1', b'6', 0]);
        assert!(l[7..].iter().all(|&b| b == 0));

        assert_eq!(&PadMsg::Turn { knob: 1, delta: -1 }.encode()[..4], &[0xD1, 0x01, 1, 0xFF]);
        assert_eq!(&PadMsg::Press { knob: 2 }.encode()[..4], &[0xD1, 0x02, 2, 0]);
        assert_eq!(&PadMsg::KeyDown { row: 3, col: 1 }.encode()[..4], &[0xD1, 0x03, 3, 1]);
        assert_eq!(&PadMsg::KeyUp { row: 3, col: 1 }.encode()[..4], &[0xD1, 0x04, 3, 1]);
        assert_eq!(&PadMsg::KnobUp { knob: 1 }.encode()[..4], &[0xD1, 0x05, 1, 0]);
    }

    /// A notice must reach the pad as the text-only value, never as a clamped percentage.
    #[test]
    fn notices_are_text_only() {
        let n = Popup::notice(*b"Config error\0\0\0\0");
        let r = HostMsg::Popup(n).encode();
        assert_eq!(r[3], NOTICE);
        let Some(HostMsg::Popup(back)) = HostMsg::decode(&r) else { panic!() };
        assert!(back.is_notice());
        assert!(!Popup { knob: 0, volume: 100, muted: false, name: [0; POPUP_LEN] }.is_notice());
    }

    /// A host from before byte 19 existed sends it as zero (or cuts the report short): the
    /// pad then reads "default", never a stray percentage.
    #[test]
    fn settings_from_an_older_host_mean_default_idle_lights() {
        let mut old = [0u8; 19];
        old.copy_from_slice(&[0xD1, 0x20, 4, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 166, 4, 255, 30]);
        for report in [&old[..], &[&old[..], &[0u8; 13][..]].concat()[..]] {
            let Some(HostMsg::Settings(s)) = HostMsg::decode(report) else { panic!() };
            assert_eq!(s.idle_led_percent, 0);
            assert_eq!(s.screen_style, 0);
            assert_eq!((s.brightness, s.oled_top_plus_one, s.screen_off_min, s.lights_dim_min), (166, 4, 255, 30));
        }
        for pct in [0, 1, 6, 100, 255] {
            let s = Settings {
                channels: 4,
                colors: [[0; 3]; CHANNELS],
                brightness: 0,
                oled_top_plus_one: 0,
                screen_off_min: 0,
                lights_dim_min: 0,
                idle_led_percent: pct,
                screen_style: 0,
            };
            assert_eq!(host_roundtrip(HostMsg::Settings(s))[19], pct);
        }
    }

    /// A host from before byte 20 existed sends it as zero or cuts the report at 20 bytes:
    /// the pad then draws the Classic screen.
    #[test]
    fn settings_from_an_older_host_mean_the_classic_screen() {
        let mut old = [0u8; 20];
        old.copy_from_slice(&[0xD1, 0x20, 4, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 166, 4, 255, 30, 25]);
        for report in [&old[..], &[&old[..], &[0u8; 12][..]].concat()[..]] {
            let Some(HostMsg::Settings(s)) = HostMsg::decode(report) else { panic!() };
            assert_eq!(s.screen_style, 0);
            assert_eq!(s.idle_led_percent, 25);
        }
        for style in [0, 1, 2, 3, 255] {
            let s = Settings {
                channels: 4,
                colors: [[0; 3]; CHANNELS],
                brightness: 0,
                oled_top_plus_one: 0,
                screen_off_min: 0,
                lights_dim_min: 0,
                idle_led_percent: 0,
                screen_style: style,
            };
            assert_eq!(host_roundtrip(HostMsg::Settings(s))[20], style);
        }
    }

    #[test]
    fn foreign_and_short_reports() {
        assert_eq!(HostMsg::decode(&[]), None);
        assert_eq!(HostMsg::decode(&[0xD1]), None);
        assert_eq!(HostMsg::decode(&[0x00, 0x00]), None);
        assert_eq!(HostMsg::decode(&[0xD1, 0x7F, b'B', b'O', b'O', b'X']), None, "wrong password");
        assert_eq!(HostMsg::decode(&[0xD1, 0x00]), Some(HostMsg::Heartbeat));
        // A short report reads as zero-padded.
        assert_eq!(
            HostMsg::decode(&[0xD1, 0x21, 9]),
            Some(HostMsg::Frame(Frame {
                levels: [9, 0, 0, 0],
                volumes: [0; 4],
                mutes: 0,
                selected: 0,
                mixer_keys: false,
                mic_muted: false,
                playing: false,
                second_page: false,
            }))
        );
        // Unknown key kinds are host keys.
        let mut r = [0u8; REPORT_LEN];
        r[..6].copy_from_slice(&[0xD1, 0x23, 0, 9, 1, 2]);
        let Some(HostMsg::Keys(k)) = HostMsg::decode(&r) else { panic!() };
        assert_eq!(k.keys[0], Key::Host);

        assert_eq!(PadMsg::decode(&[0xD1, 0x01, 0]), None, "too short");
        assert_eq!(PadMsg::decode(&[0xD2, 0x01, 0, 1]), None);
        assert_eq!(PadMsg::decode(&[0xD1, 0x09, 0, 1]), None);
        assert_eq!(PadMsg::decode(&[0xD1, 0x05, 2, 0]), Some(PadMsg::KnobUp { knob: 2 }));
    }

    /// xorshift64*: small, deterministic, no dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    /// Decodes `r` both ways. A message that decodes must also survive its own round trip, so a
    /// field the decoder reads is always a field the encoder writes.
    fn decode_both(r: &[u8]) {
        if let Some(m) = HostMsg::decode(r) {
            assert_eq!(HostMsg::decode(&m.encode()), Some(m), "{r:02X?}");
        }
        if let Some(m) = PadMsg::decode(r) {
            assert_eq!(PadMsg::decode(&m.encode()), Some(m), "{r:02X?}");
        }
    }

    #[test]
    fn no_report_makes_a_decoder_panic() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut decoded = 0;
        for i in 0..200_000 {
            let mut r = [0u8; REPORT_LEN];
            for b in r.iter_mut() {
                *b = rng.next() as u8;
            }
            // Mostly our magic byte and a known id, so every decoder branch runs.
            if i % 8 != 0 {
                r[0] = MAGIC;
            }
            if i % 4 != 0 {
                let ids = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x10, 0x20, 0x21, 0x22, 0x23, 0x24, 0x7F];
                r[1] = ids[(rng.next() % ids.len() as u64) as usize];
            }
            if r[1] == id::BOOTLOADER && i % 3 == 0 {
                r[2..6].copy_from_slice(BOOT);
            }
            let len = if i % 5 == 0 { (rng.next() % 33) as usize } else { REPORT_LEN };
            decode_both(&r[..len]);
            decoded += HostMsg::decode(&r[..len]).is_some() as u32;
        }
        assert!(decoded > 100_000, "too few reports decoded ({decoded}) to cover the branches");
    }

    #[test]
    fn edge_case_reports() {
        // Longer than a report: the extra bytes are ignored.
        let long = [MAGIC; 40];
        decode_both(&long);
        for fill in [0x00, 0xFF, MAGIC] {
            let r = [fill; REPORT_LEN];
            decode_both(&r);
            for len in 0..=REPORT_LEN {
                decode_both(&r[..len]);
            }
        }
        assert_eq!(HostMsg::decode(&[0; REPORT_LEN]), None);
        assert_eq!(HostMsg::decode(&[0xFF; REPORT_LEN]), None);
        assert_eq!(PadMsg::decode(&[0; REPORT_LEN]), None);
        assert_eq!(PadMsg::decode(&[0xFF; REPORT_LEN]), None);

        // Every id, with the wrong report id (magic byte) and with the right one, at every length.
        for id in 0..=255u8 {
            for magic in [MAGIC, MAGIC ^ 1, 0x00, 0xFF] {
                let mut r = [0xFFu8; REPORT_LEN];
                r[0] = magic;
                r[1] = id;
                for len in 0..=REPORT_LEN {
                    decode_both(&r[..len]);
                    if magic != MAGIC {
                        assert_eq!(HostMsg::decode(&r[..len]), None);
                        assert_eq!(PadMsg::decode(&r[..len]), None);
                    }
                }
            }
        }

        // A popup with volume 255 is a notice, and stays one.
        let mut r = [0u8; REPORT_LEN];
        r[..4].copy_from_slice(&[MAGIC, id::POPUP, 0, 255]);
        let Some(HostMsg::Popup(p)) = HostMsg::decode(&r) else { panic!() };
        assert!(p.is_notice());
        // Other volumes above 100 decode as sent; the pad clamps them when it shows them.
        r[3] = 254;
        let Some(HostMsg::Popup(p)) = HostMsg::decode(&r) else { panic!() };
        assert_eq!(p.volume, 254);

        // A truncated bootloader request never counts.
        let boot = HostMsg::Bootloader.encode();
        for len in 0..6 {
            assert_eq!(HostMsg::decode(&boot[..len]), None, "length {len}");
        }
        assert_eq!(HostMsg::decode(&boot[..6]), Some(HostMsg::Bootloader));
    }

    #[test]
    fn every_release_number_reads_back() {
        for bcd in 0..=u16::MAX {
            let (major, minor, patch) = release_version(bcd);
            assert!(major <= 165 && minor <= 15 && patch <= 15);
        }
    }
}
