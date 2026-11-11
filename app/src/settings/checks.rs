//! What the Channels tab lists, and the checks that refuse a change in plain words before
//! anything is written.

use super::choices::{KINDS, kind_of};
use crate::audio;
use crate::config::{self, ChannelCfg, ChannelTarget, Config, KeyAction};
use std::collections::HashMap;

/// Channel names the knobs read as something else.
const KNOB_WORDS: [&str; 5] = ["master", "system", "mic", "focused", "selected"];
/// What the pad's screen has room for.
pub const CHANNEL_NAME_LEN: usize = kb16_mixer_protocol::NAME_SHOWN;

/// One app as the Channels tab lists it.
#[derive(Clone, PartialEq, Debug)]
pub struct Member {
    /// As written in the config list, else the exe name as Windows reports it.
    pub exe: String,
    /// Put here with the Move key (apps.toml), which wins over the lists.
    pub moved: bool,
}

/// What each channel holds, exactly as the mixer decides it. An app named in a list, or moved
/// with the pad, appears once: under the channel it really plays in. Channels without a list
/// show only the apps moved into them (the rest of their apps are whatever happens to play).
pub fn members(channels: &[ChannelCfg], moved: &HashMap<String, String>) -> Vec<Vec<Member>> {
    let mut known: Vec<String> = Vec::new();
    let listed = channels.iter().flat_map(|c| match &c.target {
        ChannelTarget::Apps(apps) => apps.as_slice(),
        _ => &[],
    });
    let mut moved_exes: Vec<&String> = moved.keys().collect();
    moved_exes.sort();
    for exe in listed.chain(moved_exes) {
        if !known.iter().any(|k| k.eq_ignore_ascii_case(exe)) {
            known.push(exe.clone());
        }
    }
    let mut out = vec![Vec::new(); channels.len()];
    for exe in known {
        let key = exe.to_lowercase();
        let Some(c) = audio::assign(channels, moved, &key, 0, None) else { continue };
        let by_move = moved.get(&key).is_some_and(|m| m.eq_ignore_ascii_case(&channels[c].name));
        if by_move || matches!(channels[c].target, ChannelTarget::Apps(_)) {
            out[c].push(Member { exe, moved: by_move });
        }
    }
    out
}

/// Why a key change would leave the pad stuck on page 2, if it would.
pub fn page_problem(cfg: &Config, page: usize, row: usize, col: usize, action: &KeyAction) -> Option<&'static str> {
    let mut pages = cfg.pages.clone();
    pages[page][row][col] = action.clone();
    if !config::stuck_on_page_two(&pages) {
        return None;
    }
    Some(if page == 1 {
        "page 2 needs a Switch page key while page 1 has one, or there would be no way back to page 1"
    } else {
        "page 2 has no Switch page key yet; put one on page 2 first, so there is a way back"
    })
}

pub fn channel_name_problem(channels: &[ChannelCfg], i: usize, name: &str) -> Option<String> {
    let fits = name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ');
    if name.is_empty() {
        return Some("a channel needs a name".into());
    }
    if !fits || name.len() > CHANNEL_NAME_LEN {
        return Some(format!(
            "channel names are 1 to {CHANNEL_NAME_LEN} letters, digits or spaces (what the pad shows)"
        ));
    }
    if let Some(other) = channels.iter().enumerate().find(|(j, c)| *j != i && c.name.eq_ignore_ascii_case(name)) {
        return Some(format!("another channel is already called {}", other.1.name));
    }
    let is_mic = matches!(channels[i].target, ChannelTarget::Mic) && name.eq_ignore_ascii_case("mic");
    if KNOB_WORDS.iter().any(|w| w.eq_ignore_ascii_case(name)) && !is_mic {
        return Some(format!("\"{name}\" means something else to the knobs; pick another name"));
    }
    None
}

pub fn kind_problem(channels: &[ChannelCfg], i: usize, kind: usize) -> Option<String> {
    if kind != 0
        && let Some(other) = channels.iter().enumerate().find(|(j, c)| *j != i && kind_of(&c.target) == kind)
    {
        return Some(format!("{} is already \"{}\"; only one channel can be", other.1.name, KINDS[kind].0));
    }
    if kind != 2 && channels[i].name.eq_ignore_ascii_case("mic") {
        return Some("rename the channel first: to the knobs, \"Mic\" means the microphone".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::file::{apply_edit, set_key};
    use super::*;

    #[test]
    fn there_is_always_a_way_back_from_page_two() {
        let cfg = config::defaults();
        // The default page 2 has one Switch page key, top left.
        assert!(page_problem(&cfg, 1, 0, 0, &KeyAction::Nothing).unwrap().contains("no way back"));
        assert_eq!(page_problem(&cfg, 1, 0, 1, &KeyAction::Page), None, "a second one is fine");
        assert_eq!(page_problem(&cfg, 0, 0, 0, &KeyAction::Nothing), None, "page 1 may drop its page key");
        let mut no_page = cfg.clone();
        no_page.pages[0][0][0] = KeyAction::Nothing;
        no_page.pages[1][0][0] = KeyAction::Nothing;
        assert!(page_problem(&no_page, 0, 1, 1, &KeyAction::Page).unwrap().contains("first"));
        // The file check refuses the same thing, even where the window's own check is skipped.
        let stuck = apply_edit(config::DEFAULT, |d| {
            set_key(d, 1, 0, 0, "none");
            Ok(())
        });
        assert!(stuck.unwrap_err().contains("no way back"));
    }

    #[test]
    fn channel_names_and_kinds_are_checked_in_plain_words() {
        let c = config::defaults().channels;
        assert_eq!(channel_name_problem(&c, 1, "Voice"), None);
        assert!(channel_name_problem(&c, 1, "").unwrap().contains("needs a name"));
        assert!(channel_name_problem(&c, 1, "Gamer").is_none(), "5 letters fit the screen");
        assert!(channel_name_problem(&c, 1, "Gaming").unwrap().contains("1 to 5"), "a sixth does not");
        assert!(channel_name_problem(&c, 1, "game").unwrap().contains("already called Game"));
        let mut no_mic_name = c.clone();
        no_mic_name[0].name = "Voice".into();
        assert!(channel_name_problem(&no_mic_name, 1, "MIC").unwrap().contains("knobs"));
        assert_eq!(channel_name_problem(&c, 0, "MIC"), None, "the microphone channel may be Mic");
        assert!(kind_problem(&c, 1, 2).unwrap().contains("Mic is already"), "one microphone");
        assert!(kind_problem(&c, 1, 1).unwrap().contains("Game is already"), "one catch-all");
        assert_eq!(kind_problem(&c, 1, 3), None);
        assert!(kind_problem(&c, 0, 0).unwrap().contains("rename"));
    }

    #[test]
    fn the_channels_tab_shows_what_the_mixer_uses() {
        let cfg = config::defaults();
        let mut moved = HashMap::new();
        moved.insert("spotify.exe".to_string(), "Game".to_string()); // moved off its list
        moved.insert("obs64.exe".to_string(), "Chat".to_string()); // in no list
        moved.insert("vlc.exe".to_string(), "Mic".to_string()); // the mixer ignores this one
        let m = members(&cfg.channels, &moved);
        let names = |c: usize| m[c].iter().map(|m| (m.exe.as_str(), m.moved)).collect::<Vec<_>>();
        assert!(m[0].is_empty(), "the microphone holds no apps");
        assert!(names(1).contains(&("Discord.exe", false)));
        assert!(names(1).contains(&("obs64.exe", true)));
        assert_eq!(names(2), [("Spotify.exe", true)], "the catch-all shows what was moved into it");
        assert!(!names(3).iter().any(|(e, _)| *e == "Spotify.exe"), "not where its list says");
        assert!(names(3).contains(&("vlc.exe", false)), "a move the mixer ignores is not shown");
        let total: usize = m.iter().map(Vec::len).sum();
        assert_eq!(total, 9 + 15 + 1, "every listed or moved app exactly once");
    }
}
