//! What each control does when it is changed: check the new value, write it into config.toml,
//! and say in the status line what happened.

use super::apps::{add, remove};
use super::checks::{channel_name_problem, kind_problem, page_problem};
use super::choices::*;
use super::controls::{FILLING, checked, child, choice, set_checked, status, text_of};
use super::file::{
    edit, put, rename_channel, set_idle_percent, set_key, set_kind, set_knob, set_number, set_output, set_press,
};
use super::fill::{fill, problem_text};
use super::ids::*;
use super::scene::{hide_chip, saved_chip};
use super::{DEVICES, EDITING, PAGE, read_devices, window};
use crate::config::{self, Config};
use crate::dialog;
use crate::ui::widget;
use crate::win::send;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;

/// What a change did: a line for the status bar, or nothing to report (cancelled).
pub type Outcome = Result<Option<String>, String>;

pub fn saved() -> Outcome {
    Ok(Some("Saved and applied.".into()))
}

fn change_key(hwnd: HWND, cfg: &Config, id: i32) -> Outcome {
    let (r, c) = key_cell(id);
    let page = PAGE.get();
    let Some(picked) = choice(id) else { return Ok(None) };
    let current = &cfg.pages[page][r][c];
    let text = match picked.as_str() {
        ASK_SHORTCUT => match dialog::shortcut(hwnd, shortcut_of(current)) {
            dialog::Shortcut::Set(text) => text,
            dialog::Shortcut::Clear => "none".into(),
            dialog::Shortcut::Cancel => return Ok(None),
        },
        ASK_OPEN => {
            let now = match current {
                config::KeyAction::Open(target) => target.as_str(),
                _ => "",
            };
            match ask_open(hwnd, now) {
                Some(text) => text,
                None => return Ok(None),
            }
        }
        other => other.to_string(),
    };
    let action = config::parse_key(&text)?;
    if let Some(why) = page_problem(cfg, page, r, c, &action) {
        return Err(why.into());
    }
    edit(|doc| {
        set_key(doc, page, r, c, &text);
        Ok(())
    })?;
    let (list, at) = key_choices(&action, &text);
    let column = cfg.channels.get(c).map_or(format!("column {}", c + 1), |ch| format!("{} column", ch.name));
    Ok(Some(format!("Page {}, row {}, {column}: {}. Saved and applied.", page + 1, r + 1, list[at].0)))
}

/// The Open dialog, prefilled with `now`: the config text of the target picked, if any.
fn ask_open(hwnd: HWND, now: &str) -> Option<String> {
    dialog::open_target(hwnd, now).map(|target| format!("open:{target}"))
}

fn change_press(hwnd: HWND, cfg: &Config, id: i32) -> Outcome {
    let (knob, part) = (((id - ID_PRESS) / 3) as usize, ((id - ID_PRESS) % 3) as usize);
    let Some(picked) = choice(id) else { return Ok(None) };
    let press = &cfg.knob_press[knob];
    let current = [&press.tap, &press.hold, &press.double][part];
    let text = if picked == ASK_OPEN {
        let now = match current {
            config::PressAction::Open(target) => target.as_str(),
            _ => "",
        };
        match ask_open(hwnd, now) {
            Some(text) => text,
            None => return Ok(None),
        }
    } else {
        picked
    };
    config::parse_press(&text, true)?;
    edit(|doc| {
        set_press(doc, knob, PRESS_PARTS[part], &text);
        Ok(())
    })?;
    saved()
}

/// The output device: the Windows default, or a device saved with its name beside it.
fn change_output() -> Outcome {
    let Some(picked) = choice(ID_OUTPUT) else { return Ok(None) };
    let name = DEVICES.with_borrow(|d| d.iter().find(|d| d.id == picked).map(|d| d.name.clone()));
    // A device that is not connected stays as it is: picking it again changes nothing.
    if picked != "default" && name.is_none() {
        return Ok(None);
    }
    edit(|doc| {
        set_output(doc, &picked, name.as_deref());
        Ok(())
    })?;
    saved()
}

fn change_idle_lights() -> Outcome {
    let Some(percent) = choice(ID_IDLE_LIGHTS).and_then(|l| super::choices::picked(IDLE_LIGHTS, &l)) else {
        return Ok(None);
    };
    edit(|doc| {
        set_idle_percent(doc, percent);
        Ok(())
    })?;
    saved()
}

fn rename(cfg: &Config, i: usize) -> Outcome {
    let name = text_of(ID_CH_NAME + i as i32).trim().to_string();
    let Some(ch) = cfg.channels.get(i) else { return Ok(None) };
    if name == ch.name {
        return Ok(None);
    }
    if let Some(why) = channel_name_problem(&cfg.channels, i, &name) {
        return Err(why);
    }
    let old = ch.name.clone();
    edit(|doc| rename_channel(doc, i, &old, &name))?;
    Ok(Some(format!("Channel {old} is now called {name}. Saved and applied.")))
}

fn change_kind(cfg: &Config, i: usize) -> Outcome {
    let Some(picked) = choice(ID_CH_KIND + i as i32) else { return Ok(None) };
    let Some(kind) = KINDS.iter().position(|k| k.1 == picked) else { return Ok(None) };
    let Some(ch) = cfg.channels.get(i) else { return Ok(None) };
    if kind == kind_of(&ch.target) {
        return Ok(None);
    }
    if let Some(why) = kind_problem(&cfg.channels, i, kind) {
        return Err(why);
    }
    edit(|doc| set_kind(doc, i, kind))?;
    Ok(Some(format!("{} now plays: {}. Saved and applied.", ch.name, KINDS[kind].0.to_lowercase())))
}

/// The row and column of a key's dropdown.
fn key_cell(id: i32) -> (usize, usize) {
    (((id - ID_KEY) / 4) as usize, ((id - ID_KEY) % 4) as usize)
}

/// The config entry and value a dropdown of the Pad tab stands for, from the entry picked.
fn number_setting(id: i32, picked: &str) -> Option<(&'static str, f64)> {
    let minutes = || super::choices::picked(MINUTES, picked).map(|v| v as f64);
    match id {
        ID_STEP => Some(("step", super::choices::picked(STEPS, picked)?)),
        ID_DUCK => Some(("duck", super::choices::picked(DUCKS, picked)?)),
        ID_SHIFT => Some(("oled_top", picked.parse().ok()?)),
        ID_SCREEN_OFF => Some(("screen_off_min", minutes()?)),
        _ => Some(("lights_dim_min", minutes()?)),
    }
}

/// The slider's position as the config's brightness (0.1 to 1).
fn slider_brightness(position: isize) -> f64 {
    position.clamp(BRIGHTNESS_PERCENT.0, BRIGHTNESS_PERCENT.1) as f64 / 100.0
}

fn change_number(id: i32) -> Outcome {
    let Some(picked) = choice(id) else { return Ok(None) };
    let Some((name, v)) = number_setting(id, &picked) else { return Ok(None) };
    edit(|doc| {
        set_number(doc, name, v);
        Ok(())
    })?;
    saved()
}

const LOGO_NAME_RULE: &str = "the name on the screen is 1 to 10 letters, digits or spaces";

fn change_name(cfg: &Config) -> Outcome {
    let typed = text_of(ID_NAME);
    let name = config::parse_name(&typed).map_err(|_| LOGO_NAME_RULE.to_string())?;
    if name == cfg.name {
        return Ok(None);
    }
    edit(|doc| {
        put(&mut doc["name"], typed.trim().into());
        Ok(())
    })?;
    saved()
}

fn change_autostart() -> Outcome {
    let on = checked(ID_AUTOSTART);
    if config::custom_dir().is_some() {
        set_checked(ID_AUTOSTART, false);
        return Err(format!("Start with Windows stays off while {} is set (a portable or test copy)", config::DIR_VAR));
    }
    crate::set_autostart(on);
    Ok(Some(
        if on { "KB16 mixer now starts with Windows." } else { "KB16 mixer no longer starts with Windows." }.into(),
    ))
}

/// The command id Enter sends from a text box.
pub const ENTER: i32 = IDOK.0;

fn apply_box(cfg: &Config, id: i32) -> Outcome {
    match id {
        ID_NAME => change_name(cfg),
        _ => rename(cfg, (id - ID_CH_NAME) as usize),
    }
}

/// While a name is typed: says what is wrong with it, or how to apply it. Nothing is written.
fn typing(cfg: &Config, id: i32) {
    let typed = text_of(id);
    let (problem, unchanged) = match id {
        ID_NAME => {
            let parsed = config::parse_name(&typed);
            (parsed.is_err().then(|| LOGO_NAME_RULE.to_string()), parsed.is_ok_and(|n| n == cfg.name))
        }
        _ => {
            let i = (id - ID_CH_NAME) as usize;
            let name = typed.trim();
            let Some(ch) = cfg.channels.get(i) else { return };
            (if name == ch.name { None } else { channel_name_problem(&cfg.channels, i, name) }, name == ch.name)
        }
    };
    match problem {
        Some(why) => status(&format!("Not changed: {why}.")),
        None if !unchanged => status("Press Enter or leave the box to apply the new name."),
        None => {}
    }
}

/// Applies one control's new value; then the window shows what is in force.
pub fn changed(hwnd: HWND, id: i32, code: u32) {
    // While the window is torn down its boxes still report losing focus; nothing to apply then.
    if FILLING.get() || window().is_none() {
        return;
    }
    let cfg = crate::current_config();
    let outcome = match (id, code) {
        (ID_PAGE..=21, BN_CLICKED) => {
            let page = (id - ID_PAGE) as usize;
            super::scene::slide(super::scene::PART_PAGE, PAGE.replace(page), page);
            Ok(None)
        }
        (ID_KEY..=115, CBN_SELCHANGE) => change_key(hwnd, &cfg, id),
        (ID_KNOB..=122, CBN_SELCHANGE) => match choice(id) {
            Some(text) => edit(|doc| {
                set_knob(doc, (id - ID_KNOB) as usize, &text);
                Ok(())
            })
            .and_then(|()| saved()),
            None => Ok(None),
        },
        (ID_PRESS..=138, CBN_SELCHANGE) => change_press(hwnd, &cfg, id),
        // The device list is read again as it opens, so a device plugged in since shows.
        (ID_OUTPUT, CBN_DROPDOWN) => {
            if read_devices() {
                fill(false);
            }
            return;
        }
        (ID_OUTPUT, CBN_SELCHANGE) => change_output(),
        (ID_IDLE_LIGHTS, CBN_SELCHANGE) => change_idle_lights(),
        // A name is applied once it is complete (Enter, or leaving the box), never letter by
        // letter: each apply reloads everything, and the pad keeps its logo name in flash.
        (ID_CH_NAME..=203 | ID_NAME, EN_SETFOCUS) => {
            EDITING.set(Some(id));
            return;
        }
        (ID_CH_NAME..=203 | ID_NAME, EN_CHANGE) => {
            widget::set_error(child(id), false);
            return typing(&cfg, id);
        }
        (ID_CH_NAME..=203 | ID_NAME, EN_KILLFOCUS) => {
            EDITING.set(None);
            apply_box(&cfg, id)
        }
        // Enter in a text box (IsDialogMessage turns it into IDOK).
        (ENTER, _) => match EDITING.get() {
            Some(id) => apply_box(&cfg, id),
            None => return,
        },
        (ID_CH_KIND..=213, CBN_SELCHANGE) => change_kind(&cfg, (id - ID_CH_KIND) as usize),
        (ID_CH_ADD..=243, BN_CLICKED) => add(hwnd, &cfg, (id - ID_CH_ADD) as usize),
        (ID_CH_REMOVE..=253, BN_CLICKED) => remove(&cfg, (id - ID_CH_REMOVE) as usize),
        (ID_STEP | ID_DUCK | ID_SHIFT | ID_SCREEN_OFF | ID_LIGHTS_DIM, CBN_SELCHANGE) => change_number(id),
        (ID_BRIGHTNESS, _) => {
            let v = slider_brightness(send(child(ID_BRIGHTNESS), TBM_GETPOS, 0, 0));
            edit(|doc| {
                set_number(doc, "brightness", v);
                Ok(())
            })
            .and_then(|()| saved())
        }
        (ID_AUTOSTART, BN_CLICKED) => change_autostart(),
        (ID_OPEN_FILE, BN_CLICKED) => {
            let _ = std::process::Command::new("notepad.exe").arg(config::path()).spawn();
            Ok(Some(
                "config.toml is open in Notepad. Save it there; this window picks it up when you come back.".into(),
            ))
        }
        (ID_OPEN_FOLDER, BN_CLICKED) => {
            let _ = std::process::Command::new("explorer.exe").arg(config::dir()).spawn();
            Ok(None)
        }
        (ID_CLOSE, BN_CLICKED) => {
            // SAFETY: the window is ours and on this thread; its WM_DESTROY cleans up after it.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return;
        }
        _ => return,
    };
    // The text box a refusal is about, if any: its frame turns amber until the next keystroke.
    let refused_box = match (id, code) {
        (ID_CH_NAME..=203 | ID_NAME, EN_KILLFOCUS) => Some(id),
        (ENTER, _) => EDITING.get(),
        _ => None,
    };
    match (outcome, fill(false)) {
        (_, Some(e)) => {
            hide_chip();
            status(&problem_text(&e));
        }
        (Ok(Some(text)), None) => {
            if shows_saved(&text) {
                saved_chip();
            }
            status(&text);
        }
        (Ok(None), None) => {}
        (Err(why), None) => {
            hide_chip();
            if let Some(b) = refused_box {
                widget::set_error(child(b), true);
            }
            status(&format!("Not changed: {why}."));
        }
    }
}

/// The changes the "Saved" chip confirms: the ones saved to config.toml, and Start with Windows.
fn shows_saved(text: &str) -> bool {
    text.ends_with("Saved and applied.") || text.contains("starts with Windows.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_control_maps_to_its_setting() {
        assert_eq!(key_cell(ID_KEY), (0, 0));
        assert_eq!(key_cell(ID_KEY + 6), (1, 2), "row 2, column 3");
        assert_eq!(key_cell(ID_KEY + 15), (3, 3));
        assert_eq!(number_setting(ID_STEP, "5% per click"), Some(("step", 5.0)));
        assert_eq!(number_setting(ID_DUCK, "A quarter (25%)"), Some(("duck", 0.25)));
        assert_eq!(number_setting(ID_SHIFT, "3"), Some(("oled_top", 3.0)), "the shift entries' value is a number");
        assert_eq!(number_setting(ID_SCREEN_OFF, "Never"), Some(("screen_off_min", 0.0)));
        assert_eq!(number_setting(ID_LIGHTS_DIM, "1 hour"), Some(("lights_dim_min", 60.0)));
        assert_eq!(number_setting(ID_STEP, "Never"), None, "an entry of another list changes nothing");
        assert_eq!(number_setting(ID_SHIFT, "3 px down"), None);
        assert_eq!(slider_brightness(40), 0.4);
        assert_eq!(ID_PRESS + 8, 138, "the last press select ends its range in `changed`");
        assert_eq!(slider_brightness(0), 0.1, "the lights never go fully dark from the slider");
        assert_eq!(slider_brightness(250), 1.0);
    }

    #[test]
    fn the_saved_chip_confirms_saved_changes_only() {
        assert!(shows_saved("Page 1, row 2, Chat column: F13. Saved and applied."));
        assert!(shows_saved("KB16 mixer now starts with Windows."));
        assert!(shows_saved("KB16 mixer no longer starts with Windows."));
        assert!(!shows_saved(
            "config.toml is open in Notepad. Save it there; this window picks it up when you come back."
        ));
        assert!(!shows_saved("Spotify.exe is already in Media."));
    }

    #[test]
    fn every_pad_tab_entry_writes_a_valid_config() {
        let lists: [(i32, Vec<(String, String)>); 5] = [
            (ID_STEP, labelled(STEPS)),
            (ID_DUCK, labelled(DUCKS)),
            (ID_SHIFT, shift_entries()),
            (ID_SCREEN_OFF, labelled(MINUTES)),
            (ID_LIGHTS_DIM, labelled(MINUTES)),
        ];
        for (id, entries) in lists {
            for (label, value) in entries {
                let (name, v) = number_setting(id, &value).unwrap_or_else(|| panic!("{label}"));
                let text = super::super::file::apply_edit(config::DEFAULT, |d| {
                    set_number(d, name, v);
                    Ok(())
                });
                assert!(text.is_ok(), "{label}: {text:?}");
            }
        }
    }
}
