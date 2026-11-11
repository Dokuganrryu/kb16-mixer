//! Putting what is in force into the controls, and following the file when it changes outside
//! the window.

use super::checks::{Member, members};
use super::choices::*;
use super::controls::{child, fill_combo, fill_list, set_checked, set_text, status};
use super::ids::*;
use super::layout::show_tab;
use super::{HIDDEN, MEMBERS, PAGE, SEEN, window};
use crate::config::{self, PAGES};
use crate::win::send;

pub fn version_line() -> String {
    let app = format!("KB16 mixer {}.", env!("CARGO_PKG_VERSION"));
    let pad = match crate::pad::connected_release() {
        None => "Pad not connected.".to_string(),
        Some(kb16_mixer_protocol::UNVERSIONED_RELEASE) => "Pad firmware: an early build without a version.".into(),
        Some(r) => {
            let (a, b, c) = kb16_mixer_protocol::release_version(r);
            format!("Pad firmware {a}.{b}.{c}.")
        }
    };
    format!("{app}   {pad}")
}

/// One line of a channel's app list.
fn member_line(m: &Member) -> String {
    if m.moved { format!("{} (moved)", m.exe) } else { m.exe.clone() }
}

/// The controls of channel column `i` that what it shows hides: all of them when there is no
/// such channel (`kind` None), the list when it is the microphone, else the Add button unless
/// it is a list of apps.
fn channel_hidden(i: usize, kind: Option<usize>) -> Vec<i32> {
    let i = i as i32;
    match kind {
        None => {
            let mut ids =
                [ID_CH_NAME, ID_CH_KIND, ID_CH_ABOUT, ID_CH_LIST, ID_CH_ADD, ID_CH_REMOVE].map(|b| b + i).to_vec();
            ids.extend([ID_CH_LABEL + i * 2, ID_CH_LABEL + i * 2 + 1]);
            ids
        }
        Some(0) => Vec::new(),
        Some(2) => [ID_CH_LIST, ID_CH_ADD, ID_CH_REMOVE].map(|b| b + i).to_vec(),
        Some(_) => vec![ID_CH_ADD + i],
    }
}

/// The config's brightness (0 to 255) on the slider (percent), rounded to the nearest.
fn brightness_percent(brightness: u8) -> isize {
    (brightness as isize * 100 + 127) / 255
}

/// Puts what config.toml says (and what the mixer does with moved apps) into every control.
/// Text boxes are only rewritten with `boxes` (opening, or the file changed elsewhere), so what
/// someone is typing stays as typed. Returns the file's problem, if it has one; the controls
/// then show the settings in force.
pub fn fill(boxes: bool) -> Option<String> {
    let text = std::fs::read_to_string(config::path()).unwrap_or_default();
    let (cfg, problem) = match config::parse(&text) {
        Ok(c) => (c, None),
        Err(e) => (crate::current_config(), Some(e)),
    };
    let raw = config::raw_keys(&text);
    let moved = config::load_apps();
    SEEN.set(Some(text));
    let mut hidden = Vec::new();

    // Keys
    let page = PAGE.get();
    for p in 0..PAGES {
        set_checked(ID_PAGE + p as i32, p == page);
    }
    for c in 0..4 {
        set_text(ID_COLUMN + c as i32, cfg.channels.get(c).map_or("(no channel)", |ch| ch.name.as_str()));
    }
    for (r, (actions, texts)) in cfg.pages[page].iter().zip(&raw[page]).enumerate() {
        for (c, (action, text)) in actions.iter().zip(texts).enumerate() {
            let (list, at) = key_choices(action, text);
            fill_combo(ID_KEY + (r * 4 + c) as i32, list, at);
        }
    }
    for k in 0..3 {
        fill_combo(ID_KNOB + k as i32, knob_choices(&cfg), knob_index(&cfg, cfg.knobs[k]));
    }

    // Channels
    let all = members(&cfg.channels, &moved);
    for i in 0..4 {
        let Some(ch) = cfg.channels.get(i) else {
            hidden.extend(channel_hidden(i, None));
            continue;
        };
        if boxes {
            set_text(ID_CH_NAME + i as i32, &ch.name);
        }
        let kind = kind_of(&ch.target);
        fill_combo(ID_CH_KIND + i as i32, kind_entries(), kind);
        set_text(ID_CH_ABOUT + i as i32, kind_about(kind));
        let here = all.get(i).map_or(&[][..], Vec::as_slice);
        if MEMBERS.with_borrow(|m| m.get(i).map(Vec::as_slice) != Some(here)) {
            fill_list(ID_CH_LIST + i as i32, &here.iter().map(member_line).collect::<Vec<_>>());
        }
        hidden.extend(channel_hidden(i, Some(kind)));
    }
    MEMBERS.set(all);

    // Pad
    if boxes {
        set_text(ID_NAME, &cfg.name);
    }
    let percent = brightness_percent(cfg.brightness);
    if send(child(ID_BRIGHTNESS), TBM_GETPOS, 0, 0) != percent {
        send(child(ID_BRIGHTNESS), TBM_SETPOS, 1, percent);
    }
    fill_combo(ID_STEP, labelled(STEPS), nearest(STEPS, |v| (v - cfg.step as f64).abs()));
    fill_combo(ID_DUCK, labelled(DUCKS), nearest(DUCKS, |v| (v - cfg.duck as f64).abs()));
    fill_combo(ID_SHIFT, shift_entries(), cfg.oled_top as usize);
    fill_combo(ID_SCREEN_OFF, labelled(MINUTES), minutes_at(cfg.screen_off_min));
    fill_combo(ID_LIGHTS_DIM, labelled(MINUTES), minutes_at(cfg.lights_dim_min));
    set_checked(ID_AUTOSTART, crate::autostart_enabled());
    set_text(ID_VERSION, &version_line());

    HIDDEN.set(hidden);
    show_tab();
    problem
}

pub fn problem_text(e: &str) -> String {
    format!(
        "config.toml has a problem, so the settings in force stay as they are and nothing is saved until it is fixed (Open config file on the Pad tab): {e}"
    )
}

/// Re-reads config.toml if it changed since the window last showed it (edited by hand, or
/// reloaded from the tray). A file that loads is made live; one that does not is reported.
pub fn catch_up() {
    let text = std::fs::read_to_string(config::path()).unwrap_or_default();
    if SEEN.with_borrow(|s| s.as_deref() == Some(text.as_str())) {
        return;
    }
    let loaded = config::parse(&text).is_ok() && crate::reload().is_ok();
    match fill(true) {
        Some(e) => status(&problem_text(&e)),
        None if loaded => {
            status("config.toml was changed outside this window; the window shows it and it is in force.")
        }
        None => {}
    }
}

/// The app reloaded the config from the tray: show what is in force now.
pub fn reloaded() {
    if window().is_some()
        && let Some(e) = fill(true)
    {
        status(&problem_text(&e));
    }
}

/// The mixer moved an app with the Move key: the Channels tab shows it.
pub fn apps_moved() {
    if window().is_some() {
        fill(false);
    }
}

/// The pad came or went: the version line says so.
pub fn pad_changed() {
    if window().is_some() {
        set_text(ID_VERSION, &version_line());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_each_channel_column_hides() {
        assert!(channel_hidden(1, Some(0)).is_empty(), "a list of apps shows everything");
        assert_eq!(channel_hidden(1, Some(1)), [ID_CH_ADD + 1], "the catch-all lists moved apps, adds none");
        assert_eq!(channel_hidden(3, Some(3)), [ID_CH_ADD + 3]);
        assert_eq!(channel_hidden(0, Some(2)), [ID_CH_LIST, ID_CH_ADD, ID_CH_REMOVE], "the microphone has no list");
        let gone = channel_hidden(2, None);
        assert_eq!(gone.len(), 8, "a missing channel hides its six controls and two labels");
        assert!(gone.contains(&(ID_CH_NAME + 2)) && gone.contains(&(ID_CH_LABEL + 5)));
        assert!(gone.iter().all(|id| (id - 2) % 10 == 0 || *id >= ID_CH_LABEL), "only column 3");
    }

    #[test]
    fn lists_and_the_slider_show_the_config() {
        assert_eq!(member_line(&Member { exe: "obs64.exe".into(), moved: true }), "obs64.exe (moved)");
        assert_eq!(member_line(&Member { exe: "Spotify.exe".into(), moved: false }), "Spotify.exe");
        assert_eq!(brightness_percent(255), 100);
        assert_eq!(brightness_percent(0), 0);
        assert_eq!(brightness_percent(102), 40, "0.4 in the file reads as 40 %");
        assert_eq!(brightness_percent(26), 10);
    }
}
