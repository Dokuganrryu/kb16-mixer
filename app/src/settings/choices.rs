//! What the dropdowns offer, and which entry shows a setting. Plain data and functions: no
//! windows here, so all of it is tested directly.

use crate::config::{self, ChannelTarget, Config, KeyAction, KnobBinding};
use crate::dialog;

/// What a key can be set to: label shown, text written to the config.
const KEY_CHOICES: &[(&str, &str)] = &[
    ("Mute", "mute"),
    ("Focus", "focus"),
    ("Talk (hold)", "talk"),
    ("Move app", "move"),
    ("Switch page", "page"),
    ("Play / pause", "play"),
    ("Next track", "next"),
    ("Previous track", "prev"),
    ("Next (hold: prev)", "next|prev"),
    ("Stop", "stop"),
    ("F13", "F13"),
    ("F14", "F14"),
    ("F15", "F15"),
    ("F16", "F16"),
    ("F17", "F17"),
    ("F18", "F18"),
    ("F19", "F19"),
    ("F20", "F20"),
    ("F21", "F21"),
    ("F22", "F22"),
    ("F23", "F23"),
    ("F24", "F24"),
    ("Nothing", "none"),
];
/// Dropdown entries that open a dialog instead of being a value.
pub const ASK_SHORTCUT: &str = "\u{1}shortcut";
pub const ASK_OPEN: &str = "\u{1}open";

pub const STEPS: &[(&str, f64)] = &[
    ("1% per click", 1.0),
    ("2% per click", 2.0),
    ("3% per click", 3.0),
    ("5% per click", 5.0),
    ("10% per click", 10.0),
];
pub const DUCKS: &[(&str, f64)] =
    &[("Almost silent (10%)", 0.1), ("A quarter (25%)", 0.25), ("Half (50%)", 0.5), ("Three quarters (75%)", 0.75)];
pub const MINUTES: &[(&str, i64)] = &[
    ("1 minute", 1),
    ("2 minutes", 2),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("30 minutes", 30),
    ("1 hour", 60),
    ("Never", 0),
];
pub const ROW_NAMES: [&str; 4] = ["Row 1 (top)", "Row 2", "Row 3", "Row 4 (bottom)"];
pub const KNOB_NAMES: [&str; 3] = ["Small knob, left", "Small knob, right", "Big knob"];

/// A channel's kind as the window names it, and as config.toml's `target`.
pub const KINDS: [(&str, &str); 4] =
    [("These apps", "apps"), ("Everything else", "other"), ("Microphone", "mic"), ("App in front", "focused")];

pub fn kind_of(target: &ChannelTarget) -> usize {
    match target {
        ChannelTarget::Apps(_) => 0,
        ChannelTarget::Other => 1,
        ChannelTarget::Mic => 2,
        ChannelTarget::Focused => 3,
    }
}

pub fn kind_about(kind: usize) -> &'static str {
    match kind {
        0 => "The apps in this list:",
        1 => "Every app no other channel takes. Moved here:",
        2 => "Your microphone, the one chat apps use.",
        _ => "Whatever app is in front. Moved here:",
    }
}

pub fn kind_entries() -> Vec<(String, String)> {
    KINDS.iter().map(|k| (k.0.to_string(), k.1.to_string())).collect()
}

/// Entries whose value is their label (the label is looked up again when one is picked).
pub fn labelled<T: Copy>(options: &[(&str, T)]) -> Vec<(String, String)> {
    options.iter().map(|(l, _)| (l.to_string(), l.to_string())).collect()
}

/// The entry closest to a value; the first if there are none.
pub fn nearest<T: Copy>(options: &[(&str, T)], distance: impl Fn(T) -> f64) -> usize {
    (0..options.len()).min_by(|&a, &b| distance(options[a].1).total_cmp(&distance(options[b].1))).unwrap_or(0)
}

/// The MINUTES entry for a time; 0 (never) only ever matches "Never".
pub fn minutes_at(minutes: u8) -> usize {
    nearest(MINUTES, |v| if (v == 0) != (minutes == 0) { 1e9 } else { (v as f64 - minutes as f64).abs() })
}

/// The value a picked entry of `options` stands for.
pub fn picked<T: Copy>(options: &[(&str, T)], label: &str) -> Option<T> {
    options.iter().find(|o| o.0 == label).map(|o| o.1)
}

pub fn shift_entries() -> Vec<(String, String)> {
    (0..=6).map(|n| (format!("{n} px down"), n.to_string())).collect()
}

/// How a key's current setting reads in its dropdown when it is none of the standard entries.
pub fn key_label(action: &KeyAction, raw: &str) -> String {
    match action {
        &KeyAction::Keyboard { code, mods } => {
            config::key_name(code).map_or(raw.to_string(), |k| dialog::shortcut_label(mods, &k))
        }
        KeyAction::Open(target) => {
            let short = target.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or(target);
            let short: String = short.chars().take(22).collect();
            format!("Open {short}")
        }
        _ => raw.to_string(),
    }
}

/// Entries for a key: the standard list, the current setting if it is something else (so
/// opening the window never loses it), then the two that ask.
pub fn key_choices(current: &KeyAction, raw: &str) -> (Vec<(String, String)>, usize) {
    let mut list: Vec<(String, String)> = KEY_CHOICES.iter().map(|(l, v)| (l.to_string(), v.to_string())).collect();
    let at = list.iter().position(|(_, v)| config::parse_key(v).as_ref() == Ok(current)).unwrap_or_else(|| {
        list.push((key_label(current, raw), raw.to_string()));
        list.len() - 1
    });
    list.push(("Shortcut...".into(), ASK_SHORTCUT.into()));
    list.push(("Open...".into(), ASK_OPEN.into()));
    (list, at)
}

/// A keyboard key's setting as the Shortcut dialog prefills it: (modifiers, key name).
pub fn shortcut_of(action: &KeyAction) -> Option<(u8, String)> {
    match action {
        &KeyAction::Keyboard { code, mods } => config::key_name(code).map(|k| (mods, k)),
        _ => None,
    }
}

pub fn knob_choices(cfg: &Config) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = cfg.channels.iter().map(|c| (c.name.clone(), c.name.clone())).collect();
    list.push(("Master volume".into(), "master".into()));
    list.push(("Microphone".into(), "mic".into()));
    list.push(("App in front".into(), "focused".into()));
    list.push(("Focused channel".into(), "selected".into()));
    list
}

/// Where a knob's binding sits in `knob_choices`.
pub fn knob_index(cfg: &Config, binding: KnobBinding) -> usize {
    let n = cfg.channels.len();
    match binding {
        KnobBinding::Channel(i) => i,
        KnobBinding::Master => n,
        KnobBinding::Mic => n + 1,
        KnobBinding::Focused => n + 2,
        KnobBinding::Selected => n + 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_shows_as_the_entry_that_writes_it() {
        let cfg = config::defaults();
        let raw = config::raw_keys(config::DEFAULT);
        for (page, rows) in cfg.pages.iter().enumerate() {
            for (r, keys) in rows.iter().enumerate() {
                for (c, action) in keys.iter().enumerate() {
                    let text = &raw[page][r][c];
                    let (list, at) = key_choices(action, text);
                    assert_eq!(config::parse_key(&list[at].1).as_ref(), Ok(action), "page {page} row {r} col {c}");
                    assert_eq!(&list[list.len() - 2].1, ASK_SHORTCUT);
                    assert_eq!(&list[list.len() - 1].1, ASK_OPEN);
                }
            }
        }
        // Row 2, column 3 of page 1 is F13: one of the 23 standard entries, so the dialogs
        // follow at 23 and 24.
        let (list, at) = key_choices(&cfg.pages[0][1][2], &raw[0][1][2]);
        assert_eq!((list.len(), at, list[at].0.as_str()), (25, 10, "F13"));
    }

    #[test]
    fn a_setting_outside_the_list_gets_an_entry_of_its_own() {
        let shortcut = config::parse_key("ctrl+shift+m").unwrap();
        let (list, at) = key_choices(&shortcut, "ctrl+shift+m");
        assert_eq!(list[at], ("Ctrl+Shift+M".to_string(), "ctrl+shift+m".to_string()));
        assert_eq!(list.len(), 26, "the standard entries, this one, and the two that ask");
        assert_eq!(shortcut_of(&shortcut), Some((0x03, "m".to_string())));
        assert_eq!(shortcut_of(&KeyAction::Mute), None);

        let open = KeyAction::Open("C:\\Program Files\\Some Long Folder Name\\a-very-long-program-name.exe\\".into());
        assert_eq!(key_label(&open, "x"), "Open a-very-long-program-na", "the last part, 22 characters");
        assert_eq!(key_label(&KeyAction::Open("https://example.com/".into()), "x"), "Open example.com");
        assert_eq!(key_label(&KeyAction::Talk, "talk"), "talk", "anything else reads as written");
    }

    #[test]
    fn numbers_show_as_the_nearest_entry() {
        assert_eq!(STEPS[nearest(STEPS, |v| (v - 4.0f64).abs())].0, "3% per click", "the first of a tie");
        assert_eq!(STEPS[nearest(STEPS, |v| (v - 40.0f64).abs())].0, "10% per click");
        assert_eq!(DUCKS[nearest(DUCKS, |v| (v - 0.3f64).abs())].0, "A quarter (25%)");
        assert_eq!(nearest::<f64>(&[], |v| v), 0);
        assert_eq!(MINUTES[minutes_at(0)].0, "Never");
        assert_eq!(MINUTES[minutes_at(1)].0, "1 minute", "never is not the closest to 1");
        assert_eq!(MINUTES[minutes_at(45)].0, "30 minutes");
        assert_eq!(MINUTES[minutes_at(200)].0, "1 hour");
        assert_eq!(picked(DUCKS, "Half (50%)"), Some(0.5));
        assert_eq!(picked(MINUTES, "Never"), Some(0));
        assert_eq!(picked(STEPS, "7% per click"), None);
        assert_eq!(labelled(STEPS)[0], ("1% per click".to_string(), "1% per click".to_string()));
        assert_eq!(shift_entries()[6], ("6 px down".to_string(), "6".to_string()));
    }

    #[test]
    fn each_knob_binding_has_its_own_entry() {
        let cfg = config::defaults();
        let list = knob_choices(&cfg);
        let bindings = (0..cfg.channels.len()).map(KnobBinding::Channel).chain([
            KnobBinding::Master,
            KnobBinding::Mic,
            KnobBinding::Focused,
            KnobBinding::Selected,
        ]);
        for b in bindings {
            let value = &list[knob_index(&cfg, b)].1;
            let text = super::super::file::apply_edit(config::DEFAULT, |d| {
                super::super::file::set_knob(d, 0, value);
                Ok(())
            });
            let got = config::parse(&text.unwrap()).map(|c| c.knobs[0]);
            // The default microphone channel is called Mic, which the knobs read as the
            // microphone itself: the same sound, so that one may come back as the other.
            let same = got == Ok(b) || (value == "Mic" && got == Ok(KnobBinding::Mic));
            assert!(same, "{value}");
        }
        for (kind, (_, target)) in KINDS.iter().enumerate() {
            let t = format!("[[channel]]\nname = \"A\"\ntarget = \"{target}\"\n");
            assert_eq!(kind_of(&config::parse(&t).unwrap().channels[0].target), kind);
        }
        assert_eq!(kind_entries().len(), 4);
        assert!(kind_about(2).contains("microphone"));
    }
}
