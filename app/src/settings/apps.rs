//! The Channels tab's Add and Remove buttons: which apps a channel holds, in config.toml's lists
//! and in the Move key's apps.toml.

use super::MEMBERS;
use super::changes::Outcome;
use super::controls::{child, list_pick};
use super::file::{add_app, edit, remove_app};
use super::ids::*;
use crate::config::{self, ChannelTarget, Config};
use crate::{audio, dialog};
use std::collections::HashMap;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

/// The Add menu's command that asks for a program by name.
const TYPE_A_NAME: usize = 10_000;

/// Where an app plays under the settings in force now, in words.
fn now_in(exe: &str) -> String {
    let cfg = crate::current_config();
    match audio::assign(&cfg.channels, &config::load_apps(), &exe.to_lowercase(), 0, None) {
        Some(c) => format!("it now plays in {}", cfg.channels[c].name),
        None => "no channel takes it now".into(),
    }
}

/// Takes an app out of the Move key's list, through the mixer when it runs.
fn unmove(exe: &str) {
    let exe = exe.to_lowercase();
    if !config::load_apps().contains_key(&exe) {
        return;
    }
    let (done, answer) = std::sync::mpsc::channel();
    let asked = crate::tell_mixer(crate::pad::PadEvent::Unmove { exe: exe.clone(), done });
    if asked && answer.recv_timeout(std::time::Duration::from_secs(1)).is_ok() {
        return;
    }
    // No mixer to ask (its audio could not start): nothing else writes the file.
    let mut apps = config::load_apps();
    if apps.remove(&exe).is_some() {
        config::save_apps(&apps);
    }
}

/// The Add menu's lines for the apps playing now: each with the channel it plays in.
fn menu_lines(cfg: &Config, moved: &HashMap<String, String>, live: &[String]) -> Vec<String> {
    live.iter()
        .map(|exe| {
            let place = audio::assign(&cfg.channels, moved, exe, 0, None)
                .map_or(String::new(), |c| format!("   (now in {})", cfg.channels[c].name));
            format!("{exe}{place}")
        })
        .collect()
}

/// Shows the Add menu under `under` and waits for a pick: 0 for none, k + 1 for `lines[k]`, or
/// TYPE_A_NAME.
fn pick_from_menu(hwnd: HWND, under: HWND, lines: &[String]) -> usize {
    // SAFETY: the menu is made, shown and destroyed here; its strings outlive each AppendMenuW,
    // which copies them. TrackPopupMenu returns only once the menu is closed.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return 0 };
        for (k, line) in lines.iter().enumerate() {
            let _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(line.as_str()));
        }
        if lines.is_empty() {
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("No other app has a volume slider right now"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, TYPE_A_NAME, w!("Type a program name..."));
        let mut rect = RECT::default();
        let _ = GetWindowRect(under, &mut rect);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN,
            rect.left,
            rect.bottom,
            None,
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        picked.0 as usize
    }
}

pub fn add(hwnd: HWND, cfg: &Config, i: usize) -> Outcome {
    let Some(ch) = cfg.channels.get(i) else { return Ok(None) };
    let here: Vec<String> =
        MEMBERS.with_borrow(|m| m.get(i).map(|v| v.iter().map(|m| m.exe.to_lowercase()).collect())).unwrap_or_default();
    let moved = config::load_apps();
    let live: Vec<String> = crate::live_apps().unwrap_or_default().into_iter().filter(|e| !here.contains(e)).collect();
    let exe = match pick_from_menu(hwnd, child(ID_CH_ADD + i as i32), &menu_lines(cfg, &moved, &live)) {
        0 => return Ok(None),
        TYPE_A_NAME => match dialog::program(hwnd) {
            Some(exe) => exe,
            None => return Ok(None),
        },
        k => live[k - 1].clone(),
    };
    if here.contains(&exe.to_lowercase()) {
        return Ok(Some(format!("{exe} is already in {}.", ch.name)));
    }
    let was = audio::assign(&cfg.channels, &moved, &exe.to_lowercase(), 0, None)
        .filter(|&c| c != i && MEMBERS.with_borrow(|m| m[c].iter().any(|m| m.exe.eq_ignore_ascii_case(&exe))));
    // The list first: if it is refused, a move made on the pad is not lost either.
    edit(|doc| add_app(doc, i, &exe))?;
    unmove(&exe);
    let from = was.map_or(String::new(), |c| format!(" (taken out of {})", cfg.channels[c].name));
    Ok(Some(format!("{exe} added to {}{from}. Saved and applied.", ch.name)))
}

pub fn remove(cfg: &Config, i: usize) -> Outcome {
    let Some(ch) = cfg.channels.get(i) else { return Ok(None) };
    let at = list_pick(ID_CH_LIST + i as i32);
    let Some(member) = at.and_then(|at| MEMBERS.with_borrow(|m| m.get(i).and_then(|v| v.get(at)).cloned())) else {
        return Err("pick an app in the list first".into());
    };
    if member.moved {
        unmove(&member.exe);
    }
    let listed =
        matches!(&ch.target, ChannelTarget::Apps(apps) if apps.iter().any(|a| a.eq_ignore_ascii_case(&member.exe)));
    if listed {
        edit(|doc| remove_app(doc, i, &member.exe))?;
    }
    Ok(Some(format!("{} is out of {}; {}.", member.exe, ch.name, now_in(&member.exe))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_add_menu_says_where_each_app_plays_now() {
        let cfg = config::defaults();
        let mut moved = HashMap::new();
        moved.insert("obs64.exe".to_string(), "Chat".to_string());
        let live = ["discord.exe", "obs64.exe", "unknown.exe"].map(String::from);
        let lines = menu_lines(&cfg, &moved, &live);
        assert_eq!(lines[0], "discord.exe   (now in Chat)", "by its list");
        assert_eq!(lines[1], "obs64.exe   (now in Chat)", "by a move");
        assert_eq!(lines[2], "unknown.exe   (now in Game)", "the catch-all takes the rest");
        assert!(menu_lines(&cfg, &moved, &[]).is_empty());
    }
}
