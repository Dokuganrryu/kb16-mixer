//! Where each key press and knob turn goes: to the PC app over raw HID, to the keyboard report,
//! or to a media key. The firmware's `input` task (`firmware/src/main.rs`) sends what `Router`
//! decides and keeps track of reports that the bus did not take.

use crate::Time;
use kb16_mixer_protocol::{Key, PadMsg};

/// One change that the 1 kHz scan (`firmware/src/keys.rs`) found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Event {
    /// A matrix key; column 4 holds the three knob push buttons (rows 0 to 2).
    Key {
        row: u8,
        col: u8,
        pressed: bool,
    },
    Turn {
        knob: u8,
        cw: bool,
    },
}

/// Fallback keymap (no PC app): the stock layout, 1-0, arrows, Enter.
pub const KEYMAP: [[u8; 4]; 4] = [
    [0x1E, 0x1F, 0x20, 0x21], // 1 2 3 4
    [0x22, 0x23, 0x24, 0x25], // 5 6 7 8
    [0x26, 0x27, 0x52, 0x28], // 9 0 Up Enter
    [0x2A, 0x50, 0x51, 0x4F], // Backspace Left Down Right
];
/// Knob turns and presses without the app: consumer usages, except knob 1, which pages.
pub const CONSUMER_PREV: u16 = 0xB6;
pub const CONSUMER_NEXT: u16 = 0xB5;
pub const CONSUMER_VOL_DOWN: u16 = 0xEA;
pub const CONSUMER_VOL_UP: u16 = 0xE9;
pub const CONSUMER_PLAY: u16 = 0xCD;
pub const CONSUMER_MUTE: u16 = 0xE2;
pub const KEY_PGUP: u8 = 0x4B;
pub const KEY_PGDN: u8 = 0x4E;
pub const KEY_HOME: u8 = 0x4A;
/// A press at least this long on a media key with a second function sends the second one.
pub const HOLD_MS: u64 = 400;

/// Codes that fit in one boot keyboard report.
const SLOTS: usize = 6;

/// One held keyboard key: the matrix key (0 to 15) that pressed it, its HID usage (0 for a
/// modifier-only key) and its modifier bits.
#[derive(Clone, Copy)]
struct Held {
    key: u8,
    code: u8,
    mods: u8,
}

/// The keyboard keys held now, in the order they went down.
///
/// Each press belongs to the matrix key that made it, so a release takes away only what that
/// key added. Two keys can share a usage or a modifier (`ctrl+c` and `c`): the usage and the
/// modifier stay in the report while any key that holds them is down.
#[derive(Clone, Copy)]
pub struct Keyboard {
    held: [Held; 16],
    len: usize,
}

impl Default for Keyboard {
    fn default() -> Keyboard {
        Keyboard::new()
    }
}

impl Keyboard {
    pub const fn new() -> Keyboard {
        Keyboard { held: [Held { key: 0, code: 0, mods: 0 }; 16], len: 0 }
    }

    /// Matrix key `key` went down and types `code` with `mods`.
    pub fn press(&mut self, key: u8, code: u8, mods: u8) {
        self.release(key);
        if let Some(slot) = self.held.get_mut(self.len) {
            *slot = Held { key, code, mods };
            self.len += 1;
        }
    }

    /// Matrix key `key` went up.
    pub fn release(&mut self, key: u8) {
        if let Some(i) = self.held[..self.len].iter().position(|h| h.key == key) {
            // A plain loop, not `copy_within`: that pulls a 1.4 KB `memmove` into the image.
            for j in i + 1..self.len {
                self.held[j - 1] = self.held[j];
            }
            self.len -= 1;
        }
    }

    /// Lets go of everything, as when the bus goes away.
    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// The boot keyboard report, with `extra` (0 = none) typed on top for one report.
    ///
    /// A usage that two keys hold is in the report once. Past six usages, a later key adds
    /// nothing, not even its modifiers, until an earlier key goes up: Ctrl alone with the
    /// usages of other keys would make a shortcut that nobody pressed.
    pub fn report(&self, extra: u8) -> [u8; 8] {
        let mut r = [0u8; 8];
        let mut n = 2;
        let keys = self.held[..self.len].iter().map(|h| (h.code, h.mods));
        for (code, mods) in keys.chain([(extra, 0)]) {
            if code != 0 && !r[2..n].contains(&code) {
                if n == 2 + SLOTS {
                    continue;
                }
                r[n] = code;
                n += 1;
            }
            r[0] |= mods;
        }
        r
    }
}

/// What the firmware must send for one event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Nothing,
    /// A knob turn or press for the PC app.
    Host(PadMsg),
    /// A key that the PC app handles went down: send `PadMsg::KeyDown`. A release of the same
    /// key that is still owed from before is not owed any more.
    HostDown {
        row: u8,
        col: u8,
    },
    /// Send `PadMsg::KeyUp`; if the bus does not take it, it is owed until it does.
    HostUp {
        row: u8,
        col: u8,
    },
    /// The held keys changed: send `Keyboard::report(0)`.
    Keyboard,
    /// Type this usage on top of the held keys, then send the report without it.
    TapKey(u8),
    /// Send this consumer usage, then its release.
    TapMedia(u16),
}

/// The routing of events, and the keys held down. `T` is the clock (`crate::Time`).
pub struct Router<T> {
    pub keyboard: Keyboard,
    /// What each matrix key did when it went down, so its release always matches, whatever
    /// the host changed since.
    down: [Option<(Key, T)>; 16],
}

impl<T: Time> Default for Router<T> {
    fn default() -> Router<T> {
        Router::new()
    }
}

impl<T: Time> Router<T> {
    pub const fn new() -> Router<T> {
        Router { keyboard: Keyboard::new(), down: [None; 16] }
    }

    /// Routes one event. `alive`: the PC app is running. `mixer_keys`: the app wants the
    /// matrix keys (else they type the fallback keymap). `keymap`: what each key does then.
    pub fn route(&mut self, ev: Event, alive: bool, mixer_keys: bool, keymap: &[Key; 16], now: T) -> Action {
        match ev {
            Event::Turn { knob, cw } if alive => Action::Host(PadMsg::Turn { knob, delta: if cw { 1 } else { -1 } }),
            Event::Turn { knob, cw } => match (knob, cw) {
                (1, true) => Action::TapKey(KEY_PGUP),
                (1, false) => Action::TapKey(KEY_PGDN),
                (0, true) => Action::TapMedia(CONSUMER_NEXT),
                (0, false) => Action::TapMedia(CONSUMER_PREV),
                (_, true) => Action::TapMedia(CONSUMER_VOL_UP),
                (_, false) => Action::TapMedia(CONSUMER_VOL_DOWN),
            },
            // Not in the 4x5 matrix: the scan never sends it.
            Event::Key { row, col, .. } if row >= 4 || col > 4 => Action::Nothing,
            // The app times knob holds from press to release; without it a knob acts on the press.
            Event::Key { row, col: 4, pressed: false } if alive => Action::Host(PadMsg::KnobUp { knob: row }),
            Event::Key { col: 4, pressed: false, .. } => Action::Nothing,
            Event::Key { row, col: 4, pressed: true } if alive => Action::Host(PadMsg::Press { knob: row }),
            Event::Key { row, col: 4, pressed: true } => match row {
                0 => Action::TapMedia(CONSUMER_PLAY),
                1 => Action::TapKey(KEY_HOME),
                _ => Action::TapMedia(CONSUMER_MUTE),
            },
            Event::Key { row, col, pressed: true } => {
                let i = row * 4 + col;
                let action = if alive && mixer_keys {
                    keymap[i as usize]
                } else {
                    Key::Keyboard { code: KEYMAP[row as usize][col as usize], mods: 0 }
                };
                self.down[i as usize] = Some((action, now));
                match action {
                    Key::Host => Action::HostDown { row, col },
                    Key::Keyboard { code, mods } => {
                        self.keyboard.press(i, code, mods);
                        Action::Keyboard
                    }
                    Key::Media { tap, hold: 0 } => Action::TapMedia(tap as u16),
                    Key::Media { .. } | Key::Nothing => Action::Nothing,
                }
            }
            Event::Key { row, col, pressed: false } => {
                let i = row * 4 + col;
                let Some((action, since)) = self.down[i as usize].take() else { return Action::Nothing };
                match action {
                    Key::Host => Action::HostUp { row, col },
                    Key::Keyboard { .. } => {
                        self.keyboard.release(i);
                        Action::Keyboard
                    }
                    Key::Media { tap, hold } if hold != 0 => {
                        let long = now.ms_since(since) >= HOLD_MS;
                        Action::TapMedia(if long { hold } else { tap } as u16)
                    }
                    Key::Media { .. } | Key::Nothing => Action::Nothing,
                }
            }
        }
    }
}
