//! Shortcuts as data: which keys the pad can type, how a combination reads on screen and in
//! config.toml, and what the shortcut box has seen so far. No windows here.

/// Modifier bits, the same as the pad's (and `config::parse_key`'s).
pub const CTRL: u8 = 0x01;
pub const SHIFT: u8 = 0x02;
pub const ALT: u8 = 0x04;
pub const WIN: u8 = 0x08;
/// Shown and written in this order: Win+Ctrl+Alt+Shift+key.
const MODS: [(u8, &str, &str); 4] =
    [(WIN, "Win", "win"), (CTRL, "Ctrl", "ctrl"), (ALT, "Alt", "alt"), (SHIFT, "Shift", "shift")];

/// Which modifier a virtual-key code is (generic and left/right codes alike).
fn modifier(vk: u32) -> Option<u8> {
    Some(match vk {
        0x10 | 0xA0 | 0xA1 => SHIFT,
        0x11 | 0xA2 | 0xA3 => CTRL,
        0x12 | 0xA4 | 0xA5 => ALT,
        0x5B | 0x5C => WIN,
        _ => return None,
    })
}

/// Virtual keys the pad can type, with their config names (what `config::parse_key` reads).
const VK_NAMES: &[(u32, &str)] = &[
    (0x0D, "enter"),
    (0x1B, "esc"),
    (0x08, "backspace"),
    (0x09, "tab"),
    (0x20, "space"),
    (0x2C, "printscreen"),
    (0x2D, "insert"),
    (0x24, "home"),
    (0x21, "pageup"),
    (0x2E, "delete"),
    (0x23, "end"),
    (0x22, "pagedown"),
    (0x27, "right"),
    (0x25, "left"),
    (0x28, "down"),
    (0x26, "up"),
];

/// The config name of a virtual key, if the pad can type it.
pub fn vk_name(vk: u32) -> Option<String> {
    Some(match vk {
        0x41..=0x5A => ((vk as u8 - 0x41 + b'a') as char).to_string(),
        0x30..=0x39 => ((vk as u8 - 0x30 + b'0') as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        _ => VK_NAMES.iter().find(|(v, _)| *v == vk)?.1.to_string(),
    })
}

/// How a key name reads on screen: "m" -> "M", "pageup" -> "Page Up".
fn key_label(name: &str) -> String {
    let words = match name {
        "esc" => "Esc",
        "printscreen" => "Print Screen",
        "pageup" => "Page Up",
        "pagedown" => "Page Down",
        "backspace" => "Backspace",
        _ => "",
    };
    if !words.is_empty() {
        return words.into();
    }
    let mut chars = name.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// "Ctrl+Shift+M": a key with its modifiers, as shown.
pub fn shortcut_label(mods: u8, key: &str) -> String {
    let mut parts: Vec<String> = MODS.iter().filter(|m| mods & m.0 != 0).map(|m| m.1.to_string()).collect();
    parts.push(key_label(key));
    parts.join("+")
}

/// "ctrl+shift+m": the same, as written to config.toml.
pub fn shortcut_text(mods: u8, key: &str) -> String {
    let mut parts: Vec<&str> = MODS.iter().filter(|m| mods & m.0 != 0).map(|m| m.2).collect();
    parts.push(key);
    parts.join("+")
}

/// What the shortcut box has seen: modifiers held now, and the last full combination.
#[derive(Default)]
pub struct Capture {
    held: u8,
    combo: Option<(u8, String)>,
    /// Modifiers went down after everything was let go, and no key yet: show them as they are.
    pressing: bool,
    /// A key the pad cannot type was pressed (until the next key).
    refused: bool,
}

impl Capture {
    pub fn with(mods: u8, key: Option<String>) -> Capture {
        Capture { combo: key.map(|k| (mods, k)), ..Capture::default() }
    }

    pub fn down(&mut self, vk: u32) {
        if let Some(m) = modifier(vk) {
            if self.held == 0 {
                self.pressing = true;
            }
            self.held |= m;
            return;
        }
        match vk_name(vk) {
            Some(key) => {
                self.combo = Some((self.held, key));
                self.pressing = false;
                self.refused = false;
            }
            None => self.refused = true,
        }
    }

    pub fn up(&mut self, vk: u32) {
        match modifier(vk) {
            Some(m) => self.held &= !m,
            // Print Screen never sends a key-down to a window, only the key-up.
            None if vk == 0x2C => self.down(vk),
            None => {}
        }
        if self.held == 0 && self.combo.is_some() {
            self.pressing = false;
        }
    }

    /// The window lost the focus: whatever was held will not report its release here.
    pub fn let_go(&mut self) {
        self.held = 0;
        self.pressing = false;
    }

    pub fn holding(&self) -> bool {
        self.held != 0
    }

    /// What the box shows; `win` is the "add the Win key" tick.
    pub fn label(&self, win: bool) -> String {
        let extra = if win { WIN } else { 0 };
        if let Some((mods, key)) = &self.combo
            && !self.pressing
        {
            return shortcut_label(mods | extra, key);
        }
        let held = self.held | extra;
        if held == 0 {
            return String::new();
        }
        let mut parts: Vec<&str> = MODS.iter().filter(|m| held & m.0 != 0).map(|m| m.1).collect();
        parts.push("...");
        parts.join("+")
    }

    /// The config text of the combination, if a key was pressed.
    pub fn text(&self, win: bool) -> Option<String> {
        let (mods, key) = self.combo.as_ref()?;
        Some(shortcut_text(mods | if win { WIN } else { 0 }, key))
    }

    pub fn refused(&self) -> bool {
        self.refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyAction, key_name, parse_key};

    #[test]
    fn a_pressed_combination_reads_and_saves_like_the_config() {
        let mut c = Capture::default();
        assert_eq!(c.label(false), "");
        c.down(0x11); // Ctrl
        assert_eq!(c.label(false), "Ctrl+...", "shown as it is pressed");
        c.down(0x10); // Shift
        assert_eq!(c.label(false), "Ctrl+Shift+...");
        c.down(0x4D); // M
        assert_eq!(c.label(false), "Ctrl+Shift+M");
        c.up(0x4D);
        c.up(0x10);
        c.up(0x11);
        assert_eq!(c.label(false), "Ctrl+Shift+M", "kept after letting go");
        assert_eq!(c.text(false).as_deref(), Some("ctrl+shift+m"));
        assert_eq!(c.text(true).as_deref(), Some("win+ctrl+shift+m"), "the Win tick");
        assert_eq!(c.label(true), "Win+Ctrl+Shift+M");

        c.down(0x12); // Alt starts a new one
        assert_eq!(c.label(false), "Alt+...");
        c.down(0x73); // F4: Alt+F4 is just a combination here
        assert_eq!(c.text(false).as_deref(), Some("alt+F4"));
        c.let_go();
        c.down(0x60); // number pad 0
        assert!(c.refused(), "the pad cannot type number-pad keys");
        assert_eq!(c.text(false).as_deref(), Some("alt+F4"), "the last good one stays");
        c.up(0x2C); // Print Screen only reports its release
        assert_eq!(c.text(false).as_deref(), Some("printscreen"));
        c.down(0xA3); // right Ctrl
        c.down(0x25);
        assert_eq!(c.label(false), "Ctrl+Left");
    }

    #[test]
    fn every_key_the_box_takes_parses_back_to_the_same_key() {
        for vk in (0x08..=0x87).filter(|vk| modifier(*vk).is_none()) {
            let Some(name) = vk_name(vk) else { continue };
            for mods in [0, CTRL, CTRL | SHIFT, ALT | WIN, CTRL | SHIFT | ALT | WIN] {
                let action = parse_key(&shortcut_text(mods, &name)).unwrap_or_else(|e| panic!("{name}: {e}"));
                let KeyAction::Keyboard { code, mods: m } = action else { panic!("{name} is not a key") };
                assert_eq!(m, mods);
                assert_eq!(key_name(code).as_deref(), Some(name.as_str()), "usage {code:#x} names {name}");
            }
        }
        assert_eq!(shortcut_label(SHIFT | WIN, "s"), "Win+Shift+S");
        assert_eq!(shortcut_label(0, "pageup"), "Page Up");
        assert_eq!(shortcut_label(CTRL, "F13"), "Ctrl+F13");
    }
}
