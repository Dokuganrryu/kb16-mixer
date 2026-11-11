//! The settings window: three tabs (Keys, Channels, Pad) of plain Win32 controls, created when
//! opened and destroyed when closed, so it costs nothing the rest of the time.
//!
//! Every change is written into config.toml with its comments and layout kept, but only once
//! the app's own parser accepts the result: a change that would stop the file from loading is
//! refused with the reason in the status line, and a file that already fails to load is never
//! written. The file is then loaded through the same path as "Reload config". The controls are
//! filled from the file whenever it changes (hand edits included), so they show what is in force.
//!
//! Apps moved with the pad's Move key live in apps.toml, which only the mixer thread writes. The
//! Channels tab shows every app where the mixer puts it (`audio::assign`, the mixer's own rule),
//! marks the moved ones, and asks the mixer to forget a move when one is removed.

use crate::config::{self, ChannelCfg, ChannelTarget, Config, KeyAction, KnobBinding, PAGES};
use crate::{audio, dialog};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value, value};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    COLOR_WINDOW, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontIndirectW, CreateSolidBrush, DeleteDC,
    DeleteObject, GetDC, GetPixel, GetSysColor, GetSysColorBrush, HBRUSH, HDC, HFONT, ReleaseDC, SelectObject,
    SetBkColor,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    ICC_BAR_CLASSES, ICC_TAB_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx, NMHDR, TCIF_TEXT, TCITEMW,
    TCN_SELCHANGE, WC_TABCONTROLW,
};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

// ------------------------------------------------------------------ what the window shows

/// The tabs, left to right.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Keys,
    Channels,
    Pad,
}

const TABS: [(Tab, &str); 3] = [(Tab::Keys, "Keys"), (Tab::Channels, "Channels"), (Tab::Pad, "Pad")];

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
const ASK_SHORTCUT: &str = "\u{1}shortcut";
const ASK_OPEN: &str = "\u{1}open";

const STEPS: &[(&str, f64)] = &[
    ("1% per click", 1.0),
    ("2% per click", 2.0),
    ("3% per click", 3.0),
    ("5% per click", 5.0),
    ("10% per click", 10.0),
];
const DUCKS: &[(&str, f64)] =
    &[("Almost silent (10%)", 0.1), ("A quarter (25%)", 0.25), ("Half (50%)", 0.5), ("Three quarters (75%)", 0.75)];
const MINUTES: &[(&str, i64)] = &[
    ("1 minute", 1),
    ("2 minutes", 2),
    ("5 minutes", 5),
    ("10 minutes", 10),
    ("30 minutes", 30),
    ("1 hour", 60),
    ("Never", 0),
];
const ROW_NAMES: [&str; 4] = ["Row 1 (top)", "Row 2", "Row 3", "Row 4 (bottom)"];
const KNOB_NAMES: [&str; 3] = ["Small knob, left", "Small knob, right", "Big knob"];

/// A channel's kind as the window names it, and as config.toml's `target`.
const KINDS: [(&str, &str); 4] =
    [("These apps", "apps"), ("Everything else", "other"), ("Microphone", "mic"), ("App in front", "focused")];

fn kind_of(target: &ChannelTarget) -> usize {
    match target {
        ChannelTarget::Apps(_) => 0,
        ChannelTarget::Other => 1,
        ChannelTarget::Mic => 2,
        ChannelTarget::Focused => 3,
    }
}

fn kind_about(kind: usize) -> &'static str {
    match kind {
        0 => "The apps in this list:",
        1 => "Every app no other channel takes. Moved here:",
        2 => "Your microphone, the one chat apps use.",
        _ => "Whatever app is in front. Moved here:",
    }
}

/// Channel names the knobs read as something else.
const KNOB_WORDS: [&str; 5] = ["master", "system", "mic", "focused", "selected"];
/// What the pad's screen has room for.
const CHANNEL_NAME_LEN: usize = kb16_mixer_protocol::NAME_LEN;

// ------------------------------------------------------------------ control ids

const ID_TABS: i32 = 10;
const ID_PAGE: i32 = 20; // + page
const ID_COLUMN: i32 = 30; // + column: channel name over a key column
const ID_KEY: i32 = 100; // + row * 4 + col
const ID_KNOB: i32 = 120; // + knob
const ID_CH_NAME: i32 = 200; // + channel, and the same for the rest of the channel ids
const ID_CH_KIND: i32 = 210;
const ID_CH_ABOUT: i32 = 220;
const ID_CH_LIST: i32 = 230;
const ID_CH_ADD: i32 = 240;
const ID_CH_REMOVE: i32 = 250;
const ID_CH_LABEL: i32 = 260; // + channel * 2 (+ 1)
const ID_NAME: i32 = 300;
const ID_BRIGHTNESS: i32 = 301;
const ID_STEP: i32 = 302;
const ID_DUCK: i32 = 303;
const ID_SHIFT: i32 = 304;
const ID_SCREEN_OFF: i32 = 305;
const ID_LIGHTS_DIM: i32 = 306;
const ID_AUTOSTART: i32 = 307;
const ID_OPEN_FILE: i32 = 308;
const ID_OPEN_FOLDER: i32 = 309;
const ID_VERSION: i32 = 310;
const ID_STATUS: i32 = 400;
const ID_CLOSE: i32 = 401;
/// Labels nothing refers to get ids from here up.
const ID_LABELS: i32 = 1000;

// Win32 message numbers the `windows` crate keeps in other modules.
const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CB_RESETCONTENT: u32 = 0x014B;
const CB_SETCURSEL: u32 = 0x014E;
const CB_SETDROPPEDWIDTH: u32 = 0x0160;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_GETCURSEL: u32 = 0x0188;
const EM_LIMITTEXT: u32 = 0x00C5;
const TBM_GETPOS: u32 = WM_USER;
const TBM_SETPOS: u32 = WM_USER + 5;
const TBM_SETRANGE: u32 = WM_USER + 6;
const TB_ENDTRACK: u32 = 8;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const TCM_SETCURSEL: u32 = 0x130C;
const TCM_GETCURSEL: u32 = 0x130B;
const TCM_INSERTITEMW: u32 = 0x133E;
const SS_CENTER: u32 = 1;

const STATUS_IDLE: &str = "Changes apply the moment you make them (a name once you press Enter or leave its box) and are saved to config.toml.";

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

// ------------------------------------------------------------------ writing config.toml

/// Applies `change` to the text of config.toml and checks the result the way the app will load
/// it. Refuses (with the reason, nothing written) when the file already has a problem or when
/// the change would leave one.
fn apply_edit(text: &str, change: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> Result<String, String> {
    let problem =
        |e: String| format!("config.toml has a problem, so nothing was saved. Fix it first (Open config file): {e}");
    config::parse(text).map_err(problem)?;
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| problem(e.to_string()))?;
    change(&mut doc)?;
    let new = doc.to_string();
    config::parse(&new).map_err(|e| format!("that would make the config invalid ({e})"))?;
    Ok(new)
}

fn edit_file(
    path: &std::path::Path,
    change: impl FnOnce(&mut DocumentMut) -> Result<(), String>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read config.toml: {e}"))?;
    let new = apply_edit(&text, change)?;
    if new != text {
        config::write_whole(path, &new).map_err(|e| format!("cannot write config.toml: {e}"))?;
    }
    Ok(())
}

/// Edits config.toml and makes the result live, mixer included, before the window shows it.
fn edit(change: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> Result<(), String> {
    edit_file(&config::path(), change)?;
    crate::reload()?;
    crate::mixer_caught_up();
    Ok(())
}

/// The table `name`, written as a section or inline (`knobs = { ... }`); edited as it is, so
/// its other entries and its form stay. Made if it is missing or not a table.
fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut dyn TableLike {
    if doc.get(name).and_then(Item::as_table_like).is_none() {
        doc[name] = Item::Table(Table::new());
    }
    doc[name].as_table_like_mut().expect("a table")
}

/// Sets a value and keeps the comment that followed the old one.
fn put(item: &mut Item, mut new: Value) {
    if let Some(old) = item.as_value() {
        *new.decor_mut() = old.decor().clone();
    }
    *item = Item::Value(new);
}

/// `put` for an entry that may not exist yet.
fn put_in(table: &mut dyn TableLike, key: &str, new: Value) {
    match table.get_mut(key) {
        Some(item) => put(item, new),
        None => {
            table.insert(key, Item::Value(new));
        }
    }
}

fn set_key(doc: &mut DocumentMut, page: usize, row: usize, col: usize, text: &str) {
    let name = format!("row{}", row + 1);
    let keys = table(doc, ["keys", "keys2"][page]);
    // A missing or malformed row is rebuilt from the defaults before the one key is set.
    let complete = keys.get(&name).and_then(Item::as_array).is_some_and(|a| a.len() == 4);
    if !complete {
        put_in(keys, &name, Array::from_iter(config::DEFAULT_KEYS[page][row]).into());
    }
    if let Some(cells) = keys.get_mut(&name).and_then(Item::as_array_mut) {
        cells.replace(col, text);
    }
}

fn set_knob(doc: &mut DocumentMut, knob: usize, text: &str) {
    put_in(table(doc, "knobs"), &format!("knob{}", knob + 1), text.into());
}

fn set_number(doc: &mut DocumentMut, name: &str, v: f64) {
    put(&mut doc[name], if v.fract() == 0.0 { (v as i64).into() } else { v.into() });
}

fn channels_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, String> {
    doc.get_mut("channel").and_then(Item::as_array_of_tables_mut).ok_or_else(|| "no [[channel]] sections".to_string())
}

fn channel_mut(doc: &mut DocumentMut, i: usize) -> Result<&mut Table, String> {
    channels_mut(doc)?.get_mut(i).ok_or_else(|| format!("there is no channel {}", i + 1))
}

/// Renames channel `i`, and the knobs that were set to it by its old name.
fn rename_channel(doc: &mut DocumentMut, i: usize, old: &str, new: &str) -> Result<(), String> {
    put(&mut channel_mut(doc, i)?["name"], new.into());
    if let Some(knobs) = doc.get_mut("knobs").and_then(Item::as_table_like_mut) {
        for k in ["knob1", "knob2", "knob3"] {
            if let Some(item) = knobs.get_mut(k)
                && item.as_str().is_some_and(|v| v.eq_ignore_ascii_case(old))
            {
                put(item, new.into());
            }
        }
    }
    Ok(())
}

fn set_kind(doc: &mut DocumentMut, i: usize, kind: usize) -> Result<(), String> {
    let ch = channel_mut(doc, i)?;
    if kind == 0 {
        // Without a target the list decides; an empty list needs `target = "apps"` to load.
        let listed = ch.get("apps").and_then(Item::as_array).is_some_and(|a| !a.is_empty());
        if listed {
            ch.remove("target");
        } else {
            ch["target"] = value("apps");
            if ch.get("apps").and_then(Item::as_array).is_none() {
                ch["apps"] = value(Array::new());
            }
        }
    } else {
        // A list stays in the file, unused, so switching back brings it back; an empty one goes.
        put(&mut ch["target"], KINDS[kind].1.into());
        if ch.get("apps").and_then(Item::as_array).is_some_and(Array::is_empty) {
            ch.remove("apps");
        }
    }
    Ok(())
}

fn without(array: &mut Array, exe: &str) {
    array.retain(|v| !v.as_str().is_some_and(|a| a.eq_ignore_ascii_case(exe)));
}

/// Adds an app to channel `i`'s list and takes it out of every other list, so it is in one
/// place only.
fn add_app(doc: &mut DocumentMut, i: usize, exe: &str) -> Result<(), String> {
    for ch in channels_mut(doc)?.iter_mut() {
        if let Some(apps) = ch.get_mut("apps").and_then(Item::as_array_mut) {
            without(apps, exe);
        }
    }
    let ch = channel_mut(doc, i)?;
    match ch.get_mut("apps").and_then(Item::as_array_mut) {
        Some(apps) => apps.push(exe),
        None => ch["apps"] = value(Array::from_iter([exe])),
    }
    Ok(())
}

fn remove_app(doc: &mut DocumentMut, i: usize, exe: &str) -> Result<(), String> {
    let ch = channel_mut(doc, i)?;
    let Some(apps) = ch.get_mut("apps").and_then(Item::as_array_mut) else { return Ok(()) };
    without(apps, exe);
    if apps.is_empty() && ch.get("target").is_none() {
        ch["target"] = value("apps");
    }
    Ok(())
}

// ------------------------------------------------------------------ checks in plain words

/// Why a key change would leave the pad stuck on page 2, if it would.
fn page_problem(cfg: &Config, page: usize, row: usize, col: usize, action: &KeyAction) -> Option<&'static str> {
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

fn channel_name_problem(channels: &[ChannelCfg], i: usize, name: &str) -> Option<String> {
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

fn kind_problem(channels: &[ChannelCfg], i: usize, kind: usize) -> Option<String> {
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

// ------------------------------------------------------------------ the window's state

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static FONT: Cell<HFONT> = const { Cell::new(HFONT(std::ptr::null_mut())) };
    static TAB: Cell<usize> = const { Cell::new(0) };
    static PAGE: Cell<usize> = const { Cell::new(0) };
    /// The tab the controls being created belong to (None: always shown).
    static BUILDING: Cell<Option<usize>> = const { Cell::new(None) };
    static NEXT_LABEL: Cell<i32> = const { Cell::new(ID_LABELS) };
    static CONTROLS: RefCell<Vec<(HWND, i32, Option<usize>)>> = const { RefCell::new(Vec::new()) };
    /// Controls hidden by what they show (a channel that does not exist, a list a kind has not).
    static HIDDEN: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Each dropdown's entries as (label, value), so a pick maps to exactly what was shown.
    static CHOICES: RefCell<HashMap<i32, Vec<(String, String)>>> = RefCell::new(HashMap::new());
    static MEMBERS: RefCell<Vec<Vec<Member>>> = const { RefCell::new(Vec::new()) };
    /// The config text the controls were last filled from.
    static SEEN: RefCell<Option<String>> = const { RefCell::new(None) };
    /// Set while the window itself writes into its controls, so that is not taken as typing.
    static FILLING: Cell<bool> = const { Cell::new(false) };
    /// The text box being typed in, if any: Enter applies it.
    static EDITING: Cell<Option<i32>> = const { Cell::new(None) };
    /// The tab pages' colour, for the labels, tick boxes and slider on them.
    static PANE: Cell<Option<COLORREF>> = const { Cell::new(None) };
    static PANE_BRUSH: Cell<HBRUSH> = const { Cell::new(HBRUSH(std::ptr::null_mut())) };
}

/// The open settings window, if any (for the message loop's keyboard navigation).
pub fn window() -> Option<HWND> {
    let h = WINDOW.get();
    (!h.is_invalid()).then_some(h)
}

fn dpi() -> i32 {
    // KB16_MIXER_DPI (developer setting): lay the window out as at another display scale.
    std::env::var("KB16_MIXER_DPI").ok().and_then(|v| v.parse().ok()).unwrap_or(unsafe { GetDpiForSystem() } as i32)
}

fn scale(v: i32) -> i32 {
    v * dpi() / 96
}

fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

fn child(id: i32) -> HWND {
    unsafe { GetDlgItem(window(), id).unwrap_or_default() }
}

fn text_of(id: i32) -> String {
    let h = child(id);
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

fn set_text(id: i32, text: &str) {
    if text_of(id) != text {
        FILLING.set(true);
        unsafe {
            let _ = SetWindowTextW(child(id), &HSTRING::from(text));
        }
        FILLING.set(false);
    }
}

#[allow(clippy::too_many_arguments)]
fn control(parent: HWND, class: PCWSTR, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
    let id = if id < 0 { NEXT_LABEL.replace(NEXT_LABEL.get() + 1) } else { id };
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(style) | WS_CHILD,
            scale(x),
            scale(y),
            scale(w),
            scale(h),
            Some(parent),
            Some(HMENU(id as usize as *mut _)),
            None,
            None,
        )
        .unwrap_or_default();
        send(hwnd, WM_SETFONT, FONT.get().0 as usize, 1);
        CONTROLS.with_borrow_mut(|c| c.push((hwnd, id, BUILDING.get())));
        hwnd
    }
}

fn label(parent: HWND, text: &str, x: i32, y: i32, w: i32) {
    control(parent, w!("STATIC"), text, 0, x, y + 3, w, 18, -1);
}

fn combo(parent: HWND, id: i32, x: i32, y: i32, w: i32) {
    let h =
        control(parent, w!("COMBOBOX"), "", CBS_DROPDOWNLIST as u32 | WS_VSCROLL.0 | WS_TABSTOP.0, x, y, w, 400, id);
    send(h, CB_SETDROPPEDWIDTH, scale(w.max(200)) as usize, 0);
}

fn button(parent: HWND, text: &str, x: i32, y: i32, w: i32, id: i32) {
    control(parent, w!("BUTTON"), text, WS_TABSTOP.0, x, y, w, 26, id);
}

/// Fills a dropdown, touching it only if its entries changed (an open list stays open).
fn fill_combo(id: i32, entries: Vec<(String, String)>, at: usize) {
    let h = child(id);
    let same = CHOICES.with_borrow(|c| c.get(&id) == Some(&entries));
    if !same {
        send(h, CB_RESETCONTENT, 0, 0);
        for (label, _) in &entries {
            let text = HSTRING::from(label.as_str());
            send(h, CB_ADDSTRING, 0, text.as_ptr() as isize);
        }
        CHOICES.with_borrow_mut(|c| c.insert(id, entries));
    }
    if send(h, CB_GETCURSEL, 0, 0) != at as isize {
        send(h, CB_SETCURSEL, at, 0);
    }
}

fn choice(id: i32) -> Option<String> {
    let at = usize::try_from(send(child(id), CB_GETCURSEL, 0, 0)).ok()?;
    CHOICES.with_borrow(|c| c.get(&id).and_then(|e| e.get(at)).map(|(_, v)| v.clone()))
}

fn labelled<T: Copy>(options: &[(&str, T)]) -> Vec<(String, String)> {
    options.iter().map(|(l, _)| (l.to_string(), l.to_string())).collect()
}

fn nearest<T: Copy>(options: &[(&str, T)], distance: impl Fn(T) -> f64) -> usize {
    (0..options.len()).min_by(|&a, &b| distance(options[a].1).total_cmp(&distance(options[b].1))).unwrap_or(0)
}

fn minutes_at(minutes: u8) -> usize {
    nearest(MINUTES, |v| if (v == 0) != (minutes == 0) { 1e9 } else { (v as f64 - minutes as f64).abs() })
}

/// How a key's current setting reads in its dropdown when it is none of the standard entries.
fn key_label(action: &KeyAction, raw: &str) -> String {
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
fn key_choices(current: &KeyAction, raw: &str) -> (Vec<(String, String)>, usize) {
    let mut list: Vec<(String, String)> = KEY_CHOICES.iter().map(|(l, v)| (l.to_string(), v.to_string())).collect();
    let at = list.iter().position(|(_, v)| config::parse_key(v).as_ref() == Ok(current)).unwrap_or_else(|| {
        list.push((key_label(current, raw), raw.to_string()));
        list.len() - 1
    });
    list.push(("Shortcut...".into(), ASK_SHORTCUT.into()));
    list.push(("Open...".into(), ASK_OPEN.into()));
    (list, at)
}

fn knob_choices(cfg: &Config) -> Vec<(String, String)> {
    let mut list: Vec<(String, String)> = cfg.channels.iter().map(|c| (c.name.clone(), c.name.clone())).collect();
    list.push(("Master volume".into(), "master".into()));
    list.push(("Microphone".into(), "mic".into()));
    list.push(("App in front".into(), "focused".into()));
    list.push(("Focused channel".into(), "selected".into()));
    list
}

fn knob_index(cfg: &Config, binding: KnobBinding) -> usize {
    let n = cfg.channels.len();
    match binding {
        KnobBinding::Channel(i) => i,
        KnobBinding::Master => n,
        KnobBinding::Mic => n + 1,
        KnobBinding::Focused => n + 2,
        KnobBinding::Selected => n + 3,
    }
}

fn version_line() -> String {
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

// ------------------------------------------------------------------ building and filling

/// Client area at 96 dpi.
const WIDTH: i32 = 680;
const HEIGHT: i32 = 532;
const COL_X: i32 = 126;
const COL_W: i32 = 136;
const CH_X: i32 = 24;
const CH_W: i32 = 150;
const CH_STEP: i32 = 160;

fn build(hwnd: HWND) {
    let on = |tab: Tab| BUILDING.set(TABS.iter().position(|t| t.0 == tab));

    // ---- Keys
    on(Tab::Keys);
    let radio = BS_AUTORADIOBUTTON as u32 | BS_PUSHLIKE as u32 | WS_TABSTOP.0;
    control(hwnd, w!("BUTTON"), "Page 1", radio | WS_GROUP.0, 24, 46, 96, 28, ID_PAGE);
    control(hwnd, w!("BUTTON"), "Page 2", radio, 124, 46, 96, 28, ID_PAGE + 1);
    label(hwnd, "The pad starts on page 1. A Switch page key flips to the other page.", 232, 50, 420);
    for c in 0..4 {
        control(hwnd, w!("STATIC"), "", SS_CENTER, COL_X + c * COL_W, 88, COL_W - 8, 18, ID_COLUMN + c);
    }
    for r in 0..4 {
        label(hwnd, ROW_NAMES[r as usize], 24, 108 + r * 32, 100);
        for c in 0..4 {
            combo(hwnd, ID_KEY + r * 4 + c, COL_X + c * COL_W, 108 + r * 32, COL_W - 8);
        }
    }
    label(
        hwnd,
        "Keys that type (media keys, F13 to F24, shortcuts) are pressed by the pad itself, so they work everywhere.",
        24,
        238,
        630,
    );
    for i in 0..3 {
        let x = 24 + i * 214;
        label(hwnd, KNOB_NAMES[i as usize], x, 276, 200);
        combo(hwnd, ID_KNOB + i, x, 298, 200);
    }
    label(
        hwnd,
        "Press a knob to pick one app in its channel. It goes back to the whole channel after 8 seconds.",
        24,
        332,
        630,
    );

    // ---- Channels
    on(Tab::Channels);
    for i in 0..4 {
        let x = CH_X + i * CH_STEP;
        control(hwnd, w!("STATIC"), "Name", 0, x, 46, CH_W, 18, ID_CH_LABEL + i * 2);
        let name = control(
            hwnd,
            w!("EDIT"),
            "",
            WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32,
            x,
            66,
            CH_W,
            24,
            ID_CH_NAME + i,
        );
        send(name, EM_LIMITTEXT, CHANNEL_NAME_LEN, 0);
        control(hwnd, w!("STATIC"), "Plays", 0, x, 98, CH_W, 18, ID_CH_LABEL + i * 2 + 1);
        combo(hwnd, ID_CH_KIND + i, x, 118, CH_W);
        control(hwnd, w!("STATIC"), "", 0, x, 150, CH_W, 34, ID_CH_ABOUT + i);
        let list = WS_BORDER.0 | WS_VSCROLL.0 | WS_TABSTOP.0 | LBS_NOINTEGRALHEIGHT as u32;
        control(hwnd, w!("LISTBOX"), "", list, x, 188, CH_W, 176, ID_CH_LIST + i);
        button(hwnd, "Add...", x, 370, 72, ID_CH_ADD + i);
        button(hwnd, "Remove", x + 78, 370, 72, ID_CH_REMOVE + i);
    }
    label(hwnd, "Windows has one volume slider per program. All tabs of one browser move together.", 24, 404, 630);
    label(
        hwnd,
        "(moved) = put there with the pad's Move key, which wins over the lists. Remove undoes it.",
        24,
        424,
        630,
    );

    // ---- Pad
    on(Tab::Pad);
    let rows = [
        "Name on the screen",
        "Light brightness",
        "Knob step",
        "Focus turns the others down to",
        "Screen shift",
        "Screen off after",
        "Dim lights after",
    ];
    for (i, text) in rows.iter().enumerate() {
        label(hwnd, text, 24, 50 + i as i32 * 36, 200);
    }
    let name = control(
        hwnd,
        w!("EDIT"),
        "",
        WS_BORDER.0 | WS_TABSTOP.0 | ES_AUTOHSCROLL as u32 | ES_UPPERCASE as u32,
        232,
        50,
        160,
        24,
        ID_NAME,
    );
    send(name, EM_LIMITTEXT, kb16_mixer_protocol::LOGO_NAME_LEN, 0);
    label(hwnd, "1 to 10 letters, digits or spaces", 404, 50, 250);
    let bar = control(hwnd, w!("msctls_trackbar32"), "", WS_TABSTOP.0, 226, 84, 250, 30, ID_BRIGHTNESS);
    send(bar, TBM_SETRANGE, 1, (100 << 16) | 10);
    combo(hwnd, ID_STEP, 232, 122, 180);
    combo(hwnd, ID_DUCK, 232, 158, 180);
    combo(hwnd, ID_SHIFT, 232, 194, 180);
    control(hwnd, w!("STATIC"), "Moves the picture down if its top is cut off.", 0, 424, 197, 240, 36, -1);
    combo(hwnd, ID_SCREEN_OFF, 232, 230, 180);
    combo(hwnd, ID_LIGHTS_DIM, 232, 266, 180);
    control(hwnd, w!("STATIC"), "Counted while nothing plays or is touched.", 0, 424, 236, 240, 40, -1);
    control(
        hwnd,
        w!("BUTTON"),
        "Start with Windows",
        BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0,
        24,
        308,
        300,
        24,
        ID_AUTOSTART,
    );
    button(hwnd, "Open config file", 24, 346, 160, ID_OPEN_FILE);
    button(hwnd, "Open settings folder", 192, 346, 180, ID_OPEN_FOLDER);
    control(hwnd, w!("STATIC"), "", 0, 24, 392, 630, 18, ID_VERSION);

    // ---- around the tabs. The tab strip comes last so it sits under the pages.
    BUILDING.set(None);
    control(hwnd, w!("STATIC"), STATUS_IDLE, 0, 16, 468, 548, 54, ID_STATUS);
    button(hwnd, "Close", 572, 470, 92, ID_CLOSE);
    let tabs = control(hwnd, WC_TABCONTROLW, "", WS_CLIPSIBLINGS.0 | WS_TABSTOP.0, 8, 8, WIDTH - 16, 448, ID_TABS);
    for (i, (_, name)) in TABS.iter().enumerate() {
        let mut text: Vec<u16> = name.encode_utf16().chain([0]).collect();
        let item = TCITEMW { mask: TCIF_TEXT, pszText: PWSTR(text.as_mut_ptr()), ..Default::default() };
        send(tabs, TCM_INSERTITEMW, i, &raw const item as isize);
    }
    send(tabs, TCM_SETCURSEL, TAB.get(), 0);
    PANE.set(pane_color(tabs));
    if let Some(color) = PANE.get() {
        PANE_BRUSH.set(unsafe { CreateSolidBrush(color) });
    }
}

/// The colour the tab control fills its pages with. It differs between Windows versions and
/// themes, so it is read from the control itself, once.
fn pane_color(tabs: HWND) -> Option<COLORREF> {
    const PRF_CLIENT: isize = 4;
    const PRF_ERASEBKGND: isize = 8;
    unsafe {
        let mut r = RECT::default();
        GetClientRect(tabs, &mut r).ok()?;
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, r.right, r.bottom);
        let old = SelectObject(dc, bitmap.into());
        send(tabs, WM_PRINTCLIENT, dc.0 as usize, PRF_CLIENT | PRF_ERASEBKGND);
        let color = GetPixel(dc, r.right / 2, r.bottom - scale(24));
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        (color.0 != 0xFFFF_FFFF).then_some(color)
    }
}

/// Shows the controls of the open tab, minus those hidden by what they show.
fn show_tab() {
    let tab = TAB.get();
    let hidden = HIDDEN.with_borrow(Clone::clone);
    CONTROLS.with_borrow(|controls| {
        for &(h, id, on) in controls {
            let show = on.is_none_or(|t| t == tab) && !hidden.contains(&id);
            unsafe {
                let visible = GetWindowLongPtrW(h, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0;
                if visible != show {
                    let _ = ShowWindow(h, if show { SW_SHOWNA } else { SW_HIDE });
                }
            }
        }
    });
}

/// Puts what config.toml says (and what the mixer does with moved apps) into every control.
/// Text boxes are only rewritten with `boxes` (opening, or the file changed elsewhere), so what
/// someone is typing stays as typed. Returns the file's problem, if it has one; the controls
/// then show the settings in force.
fn fill(boxes: bool) -> Option<String> {
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
        send(child(ID_PAGE + p as i32), BM_SETCHECK, (p == page) as usize, 0);
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
        let ids = [ID_CH_NAME, ID_CH_KIND, ID_CH_ABOUT, ID_CH_LIST, ID_CH_ADD, ID_CH_REMOVE].map(|b| b + i as i32);
        let Some(ch) = cfg.channels.get(i) else {
            hidden.extend(ids);
            hidden.extend([ID_CH_LABEL + i as i32 * 2, ID_CH_LABEL + i as i32 * 2 + 1]);
            continue;
        };
        if boxes {
            set_text(ID_CH_NAME + i as i32, &ch.name);
        }
        let kind = kind_of(&ch.target);
        fill_combo(ID_CH_KIND + i as i32, KINDS.iter().map(|k| (k.0.to_string(), k.1.to_string())).collect(), kind);
        set_text(ID_CH_ABOUT + i as i32, kind_about(kind));
        let list = child(ID_CH_LIST + i as i32);
        let here = all.get(i).map_or(&[][..], Vec::as_slice);
        let shown: Vec<String> =
            here.iter().map(|m| if m.moved { format!("{} (moved)", m.exe) } else { m.exe.clone() }).collect();
        if MEMBERS.with_borrow(|m| m.get(i).map(Vec::as_slice) != Some(here)) {
            send(list, LB_RESETCONTENT, 0, 0);
            for s in &shown {
                let text = HSTRING::from(s.as_str());
                send(list, LB_ADDSTRING, 0, text.as_ptr() as isize);
            }
        }
        match kind {
            0 => {}
            2 => hidden.extend([ID_CH_LIST, ID_CH_ADD, ID_CH_REMOVE].map(|b| b + i as i32)),
            _ => hidden.push(ID_CH_ADD + i as i32),
        }
    }
    MEMBERS.set(all);

    // Pad
    if boxes {
        set_text(ID_NAME, &cfg.name);
    }
    let percent = (cfg.brightness as isize * 100 + 127) / 255;
    if send(child(ID_BRIGHTNESS), TBM_GETPOS, 0, 0) != percent {
        send(child(ID_BRIGHTNESS), TBM_SETPOS, 1, percent);
    }
    fill_combo(ID_STEP, labelled(STEPS), nearest(STEPS, |v| (v - cfg.step as f64).abs()));
    fill_combo(ID_DUCK, labelled(DUCKS), nearest(DUCKS, |v| (v - cfg.duck as f64).abs()));
    let shifts: Vec<(String, String)> = (0..=6).map(|n| (format!("{n} px down"), n.to_string())).collect();
    fill_combo(ID_SHIFT, shifts, cfg.oled_top as usize);
    fill_combo(ID_SCREEN_OFF, labelled(MINUTES), minutes_at(cfg.screen_off_min));
    fill_combo(ID_LIGHTS_DIM, labelled(MINUTES), minutes_at(cfg.lights_dim_min));
    send(child(ID_AUTOSTART), BM_SETCHECK, crate::autostart_enabled() as usize, 0);
    set_text(ID_VERSION, &version_line());

    HIDDEN.set(hidden);
    show_tab();
    problem
}

fn status(text: &str) {
    set_text(ID_STATUS, text);
}

fn problem_text(e: &str) -> String {
    format!(
        "config.toml has a problem, so the settings in force stay as they are and nothing is saved until it is fixed (Open config file on the Pad tab): {e}"
    )
}

/// Re-reads config.toml if it changed since the window last showed it (edited by hand, or
/// reloaded from the tray). A file that loads is made live; one that does not is reported.
fn catch_up() {
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

// ------------------------------------------------------------------ changes

/// What a change did: a line for the status bar, or nothing to report (cancelled).
type Outcome = Result<Option<String>, String>;

fn saved() -> Outcome {
    Ok(Some("Saved and applied.".into()))
}

fn change_key(hwnd: HWND, cfg: &Config, id: i32) -> Outcome {
    let (r, c) = (((id - ID_KEY) / 4) as usize, ((id - ID_KEY) % 4) as usize);
    let page = PAGE.get();
    let Some(picked) = choice(id) else { return Ok(None) };
    let current = &cfg.pages[page][r][c];
    let text = match picked.as_str() {
        ASK_SHORTCUT => {
            let now = match current {
                &KeyAction::Keyboard { code, mods } => config::key_name(code).map(|k| (mods, k)),
                _ => None,
            };
            match dialog::shortcut(hwnd, now) {
                dialog::Shortcut::Set(text) => text,
                dialog::Shortcut::Clear => "none".into(),
                dialog::Shortcut::Cancel => return Ok(None),
            }
        }
        ASK_OPEN => {
            let now = match current {
                KeyAction::Open(target) => target.as_str(),
                _ => "",
            };
            match dialog::open_target(hwnd, now) {
                Some(target) => format!("open:{target}"),
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

fn add(hwnd: HWND, cfg: &Config, i: usize) -> Outcome {
    let Some(ch) = cfg.channels.get(i) else { return Ok(None) };
    let here: Vec<String> =
        MEMBERS.with_borrow(|m| m.get(i).map(|v| v.iter().map(|m| m.exe.to_lowercase()).collect())).unwrap_or_default();
    let moved = config::load_apps();
    let live: Vec<String> = crate::live_apps().unwrap_or_default().into_iter().filter(|e| !here.contains(e)).collect();
    let picked = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return Ok(None) };
        for (k, exe) in live.iter().enumerate() {
            let place = audio::assign(&cfg.channels, &moved, exe, 0, None)
                .map_or(String::new(), |c| format!("   (now in {})", cfg.channels[c].name));
            let _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("{exe}{place}")));
        }
        if live.is_empty() {
            let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("No other app has a volume slider right now"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 10_000, w!("Type a program name..."));
        let mut rect = RECT::default();
        let _ = GetWindowRect(child(ID_CH_ADD + i as i32), &mut rect);
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
    };
    let exe = match picked {
        0 => return Ok(None),
        10_000 => match dialog::program(hwnd) {
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

fn remove(cfg: &Config, i: usize) -> Outcome {
    let Some(ch) = cfg.channels.get(i) else { return Ok(None) };
    let at = usize::try_from(send(child(ID_CH_LIST + i as i32), LB_GETCURSEL, 0, 0)).ok();
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

fn change_number(id: i32) -> Outcome {
    let Some(picked) = choice(id) else { return Ok(None) };
    let pick = |options: &[(&str, f64)]| options.iter().find(|o| o.0 == picked).map(|o| o.1);
    let (name, v) = match id {
        ID_STEP => ("step", pick(STEPS)),
        ID_DUCK => ("duck", pick(DUCKS)),
        ID_SHIFT => ("oled_top", picked.parse().ok()),
        ID_SCREEN_OFF => ("screen_off_min", MINUTES.iter().find(|o| o.0 == picked).map(|o| o.1 as f64)),
        _ => ("lights_dim_min", MINUTES.iter().find(|o| o.0 == picked).map(|o| o.1 as f64)),
    };
    let Some(v) = v else { return Ok(None) };
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
    let on = send(child(ID_AUTOSTART), BM_GETCHECK, 0, 0) == 1;
    if config::custom_dir().is_some() {
        send(child(ID_AUTOSTART), BM_SETCHECK, 0, 0);
        return Err(format!("Start with Windows stays off while {} is set (a portable or test copy)", config::DIR_VAR));
    }
    crate::set_autostart(on);
    Ok(Some(
        if on { "KB16 mixer now starts with Windows." } else { "KB16 mixer no longer starts with Windows." }.into(),
    ))
}

/// The command id Enter sends from a text box.
const ENTER: i32 = IDOK.0;

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
fn changed(hwnd: HWND, id: i32, code: u32) {
    // While the window is torn down its boxes still report losing focus; nothing to apply then.
    if FILLING.get() || window().is_none() {
        return;
    }
    let cfg = crate::current_config();
    let outcome = match (id, code) {
        (ID_PAGE..=21, BN_CLICKED) => {
            PAGE.set((id - ID_PAGE) as usize);
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
        // A name is applied once it is complete (Enter, or leaving the box), never letter by
        // letter: each apply reloads everything, and the pad keeps its logo name in flash.
        (ID_CH_NAME..=203 | ID_NAME, EN_SETFOCUS) => {
            EDITING.set(Some(id));
            return;
        }
        (ID_CH_NAME..=203 | ID_NAME, EN_CHANGE) => return typing(&cfg, id),
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
            let percent = send(child(ID_BRIGHTNESS), TBM_GETPOS, 0, 0).clamp(10, 100);
            edit(|doc| {
                set_number(doc, "brightness", percent as f64 / 100.0);
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
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            return;
        }
        _ => return,
    };
    match (outcome, fill(false)) {
        (_, Some(e)) => status(&problem_text(&e)),
        (Ok(Some(text)), None) => status(&text),
        (Ok(None), None) => {}
        (Err(why), None) => status(&format!("Not changed: {why}.")),
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                changed(hwnd, (wparam.0 & 0xFFFF) as i32, (wparam.0 >> 16) as u32);
                LRESULT(0)
            }
            WM_NOTIFY => {
                let hdr = &*(lparam.0 as *const NMHDR);
                if hdr.idFrom == ID_TABS as usize && hdr.code == TCN_SELCHANGE {
                    TAB.set(send(child(ID_TABS), TCM_GETCURSEL, 0, 0).max(0) as usize);
                    show_tab();
                }
                LRESULT(0)
            }
            // The brightness slider, once the mouse or key is released.
            WM_HSCROLL if (wparam.0 & 0xFFFF) as u32 == TB_ENDTRACK => {
                changed(hwnd, ID_BRIGHTNESS, 0);
                LRESULT(0)
            }
            // Labels, tick boxes and the slider take the colour of the tab page under them.
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                let from = HWND(lparam.0 as *mut _);
                let on_page = CONTROLS.with_borrow(|c| c.iter().any(|(h, _, tab)| *h == from && tab.is_some()));
                let (color, brush) = match PANE.get() {
                    Some(c) if on_page => (c, PANE_BRUSH.get()),
                    _ => (COLORREF(GetSysColor(COLOR_WINDOW)), GetSysColorBrush(COLOR_WINDOW)),
                };
                SetBkColor(HDC(wparam.0 as *mut _), color);
                LRESULT(brush.0 as isize)
            }
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 != WA_INACTIVE => {
                catch_up();
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_CLOSE => {
                if let Some(id) = EDITING.get() {
                    changed(hwnd, id, EN_KILLFOCUS);
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                WINDOW.set(HWND::default());
                CONTROLS.with_borrow_mut(Vec::clear);
                CHOICES.with_borrow_mut(HashMap::clear);
                MEMBERS.with_borrow_mut(Vec::clear);
                HIDDEN.with_borrow_mut(Vec::clear);
                SEEN.set(None);
                EDITING.set(None);
                NEXT_LABEL.set(ID_LABELS);
                if !PANE_BRUSH.get().is_invalid() {
                    let _ = DeleteObject(PANE_BRUSH.replace(HBRUSH::default()).into());
                }
                PANE.set(None);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Opens the settings window, or brings it forward if it is already open.
pub fn open() {
    open_at(None, None);
}

/// Opens the window on `tab` (or where it was), with `note` in the status line.
pub fn open_at(tab: Option<Tab>, note: Option<&str>) {
    if let Some(t) = tab {
        TAB.set(TABS.iter().position(|x| x.0 == t).unwrap_or(0));
    }
    unsafe {
        if let Some(existing) = window() {
            send(child(ID_TABS), TCM_SETCURSEL, TAB.get(), 0);
            show_tab();
            if let Some(n) = note {
                status(n);
            }
            let _ = SetForegroundWindow(existing);
            return;
        }
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        if FONT.get().is_invalid() {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES | ICC_TAB_CLASSES,
            });
            let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
            let _ = SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                Some((&raw mut metrics).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            let font = &mut metrics.lfMessageFont;
            font.lfHeight = font.lfHeight * dpi() / GetDpiForSystem() as i32;
            FONT.set(CreateFontIndirectW(font));
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("kb16-mixer-settings"),
                hbrBackground: GetSysColorBrush(COLOR_WINDOW),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                ..Default::default()
            });
        }
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT { left: 0, top: 0, right: scale(WIDTH), bottom: scale(HEIGHT) };
        let _ = AdjustWindowRect(&mut rect, style, false);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            w!("kb16-mixer-settings"),
            w!("KB16 mixer settings"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return crate::log("could not create the settings window");
        };
        WINDOW.set(hwnd);
        build(hwnd);
        match fill(true) {
            Some(e) => status(&problem_text(&e)),
            None => status(note.unwrap_or(STATUS_IDLE)),
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited(text: &str, change: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> String {
        apply_edit(text, change).expect("accepted")
    }

    /// The window's writes must leave the rest of the file exactly as the user had it.
    #[test]
    fn edits_keep_the_file_intact() {
        let mut text = config::DEFAULT.to_string();
        for change in [
            |d: &mut DocumentMut| set_key(d, 0, 0, 1, "ctrl+shift+d"),
            |d: &mut DocumentMut| set_key(d, 0, 2, 3, "none"),
            |d: &mut DocumentMut| set_knob(d, 1, "master"),
            |d: &mut DocumentMut| set_number(d, "brightness", 0.4),
            |d: &mut DocumentMut| set_number(d, "screen_off_min", 0.0),
            |d: &mut DocumentMut| set_number(d, "lights_dim_min", 30.0),
        ] {
            text = edited(&text, |d| {
                change(d);
                Ok(())
            });
        }
        let cfg = config::parse(&text).expect("still a valid config");
        assert_eq!(cfg.pages[0][0][1], KeyAction::Keyboard { code: 0x07, mods: 0x03 });
        assert_eq!(cfg.pages[0][2][3], KeyAction::Nothing);
        assert_eq!(cfg.pages[0][3], [const { KeyAction::Mute }; 4], "untouched rows stay");
        assert_eq!(cfg.pages[1], config::defaults().pages[1], "the keypad page is left alone");
        assert!(text.contains("[keys2]\nrow1 = [\"page\",  \"7\", \"8\", \"9\"]"));
        assert!(cfg.knobs[1] == KnobBinding::Master);
        assert_eq!(cfg.brightness, 102);
        assert_eq!((cfg.screen_off_min, cfg.lights_dim_min), (0, 30));
        assert_eq!(config::raw_keys(&text)[0][0][1], "ctrl+shift+d");
        // Comments and everything not edited survive, the edited lines' comments included.
        assert!(text.contains("# KNOBS. Turn = volume."));
        assert!(text.contains("knob1 = \"Chat\"        # small knob, top left"));
        assert!(text.contains("knob2 = \"master\"       # small knob, top right"), "{text}");
        assert!(text.contains("brightness = 0.4     # key lights"));
        assert!(text.contains("\"Discord.exe\""));
        assert_eq!(text.lines().count(), config::DEFAULT.lines().count(), "no lines added or lost");

        // A file with no [keys] or [knobs] at all gets them created.
        let bare = "[[channel]]\nname = \"Game\"\ntarget = \"other\"\n";
        let text = edited(bare, |d| {
            set_key(d, 0, 3, 0, "focus");
            set_knob(d, 2, "Game");
            Ok(())
        });
        let cfg = config::parse(&text).expect("valid");
        assert_eq!(cfg.pages[0][3][0], KeyAction::Focus);
        assert_eq!(cfg.pages[0][3][1], KeyAction::Mute, "the rest of the row keeps its default");
        assert!(cfg.knobs[2] == KnobBinding::Channel(0));
    }

    #[test]
    fn hand_written_inline_tables_keep_their_other_entries() {
        let text = "knobs = { knob1 = \"Chat\", knob2 = \"Media\", knob3 = \"Game\" }\n\
                    keys = { row1 = [\"page\", \"prev\", \"play\", \"next\"], row4 = [\"mute\", \"mute\", \"mute\", \"mute\"] }\n\
                    keys2 = { row1 = [\"page\", \"a\", \"b\", \"c\"], row2 = [\"d\", \"e\", \"f\", \"g\"] }\n\
                    [[channel]]\nname = \"Chat\"\napps = [\"x.exe\"]\n\
                    [[channel]]\nname = \"Media\"\napps = [\"y.exe\"]\n\
                    [[channel]]\nname = \"Game\"\ntarget = \"other\"\n";
        let text = edited(text, |d| {
            set_knob(d, 0, "master");
            set_key(d, 0, 0, 1, "F20");
            set_key(d, 1, 1, 0, "F21");
            Ok(())
        });
        let cfg = config::parse(&text).expect("still valid");
        assert!(cfg.knobs == [KnobBinding::Master, KnobBinding::Channel(1), KnobBinding::Channel(2)], "{text}");
        assert_eq!(cfg.pages[0][0][1], KeyAction::Keyboard { code: 0x6F, mods: 0 });
        assert_eq!(cfg.pages[0][0][2], KeyAction::Media { tap: 0xCD, hold: 0 }, "the rest of row1 stays");
        assert_eq!(config::raw_keys(&text)[0][3], ["mute"; 4].map(String::from), "row4 stays");
        assert_eq!(cfg.pages[1][0][1], KeyAction::Keyboard { code: 0x04, mods: 0 }, "keys2 row1 stays");
        assert_eq!(cfg.pages[1][1][0], KeyAction::Keyboard { code: 0x70, mods: 0 });
        assert_eq!(cfg.pages[1][1][1], KeyAction::Keyboard { code: 0x08, mods: 0 }, "keys2 row2 keeps its others");
        assert!(text.starts_with("knobs = { knob1 = \"master\", knob2 = \"Media\""), "still inline: {text}");
    }

    #[test]
    fn page_two_is_edited_in_its_own_table() {
        let text = edited(config::DEFAULT, |d| {
            set_key(d, 1, 1, 2, "ctrl+F5");
            Ok(())
        });
        let cfg = config::parse(&text).unwrap();
        assert_eq!(cfg.pages[1][1][2], KeyAction::Keyboard { code: 0x3E, mods: 0x01 });
        assert_eq!(cfg.pages[0], config::defaults().pages[0], "page 1 untouched");
        assert!(text.contains("row2 = [\"0\",     \"4\", \"ctrl+F5\", \"6\"]                   # digits"), "{text}");
        assert_eq!(text.lines().count(), config::DEFAULT.lines().count());
        // A config without [keys2] gets the table, built from page 2's defaults.
        let bare = "[[channel]]\nname = \"Game\"\ntarget = \"other\"\n";
        let cfg = config::parse(&edited(bare, |d| {
            set_key(d, 1, 0, 3, "F20");
            Ok(())
        }))
        .unwrap();
        assert_eq!(cfg.pages[1][0][0], KeyAction::Page, "the way back stays");
        assert_eq!(cfg.pages[1][0][3], KeyAction::Keyboard { code: 0x6F, mods: 0 });
    }

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
    fn a_broken_file_is_reported_and_never_overwritten() {
        let dir = std::env::temp_dir().join(format!("kb16-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        for broken in ["step = = 2", "[[channel]]\nname = \"A\"\ntarget = \"gmae\"\n"] {
            std::fs::write(&path, broken).unwrap();
            let e = edit_file(&path, |d| {
                set_knob(d, 0, "master");
                Ok(())
            })
            .unwrap_err();
            assert!(e.contains("has a problem"), "{e}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        }
        std::fs::write(&path, config::DEFAULT).unwrap();
        edit_file(&path, |d| {
            set_number(d, "step", 5.0);
            Ok(())
        })
        .unwrap();
        assert_eq!(config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap().step, 5.0);
        assert!(!dir.join("config.tmp").exists(), "no temporary file left behind");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn channels_are_renamed_retyped_and_given_apps() {
        // Rename Chat: the knob set to it follows, its comment stays.
        let text = edited(config::DEFAULT, |d| rename_channel(d, 1, "Chat", "Talk"));
        let cfg = config::parse(&text).unwrap();
        assert_eq!(cfg.channels[1].name, "Talk");
        assert!(cfg.knobs[0] == KnobBinding::Channel(1));
        assert!(text.contains("knob1 = \"Talk\"        # small knob, top left"));
        assert!(text.contains("# CHANNELS: up to 4"));

        // Media becomes the app in front, then a list again: its list comes back.
        let text = edited(config::DEFAULT, |d| set_kind(d, 3, 3));
        assert!(matches!(config::parse(&text).unwrap().channels[3].target, ChannelTarget::Focused));
        let text = edited(&text, |d| set_kind(d, 3, 0));
        let ChannelTarget::Apps(apps) = &config::parse(&text).unwrap().channels[3].target else { panic!() };
        assert!(apps.iter().any(|a| a == "Spotify.exe"));

        // The catch-all becomes an empty list, which still loads, and back: the file is as it was.
        let text = edited(config::DEFAULT, |d| set_kind(d, 2, 0));
        assert_eq!(edited(&text, |d| set_kind(d, 2, 1)), config::DEFAULT);
        // As a list, it gets one app.
        assert!(matches!(&config::parse(&text).unwrap().channels[2].target, ChannelTarget::Apps(a) if a.is_empty()));
        let text = edited(&text, |d| add_app(d, 2, "Discord.exe"));
        let cfg = config::parse(&text).unwrap();
        assert!(matches!(&cfg.channels[2].target, ChannelTarget::Apps(a) if a == &["Discord.exe"]));
        let ChannelTarget::Apps(chat) = &cfg.channels[1].target else { panic!() };
        assert!(!chat.iter().any(|a| a.eq_ignore_ascii_case("discord.exe")), "in one list only");

        // Remove the last app of a list without a target: it stays a (now empty) list.
        let one = "[[channel]]\nname = \"A\"\napps = [\"a.exe\"]\n";
        let text = edited(one, |d| remove_app(d, 0, "A.EXE"));
        assert!(matches!(&config::parse(&text).unwrap().channels[0].target, ChannelTarget::Apps(a) if a.is_empty()));
        assert!(text.contains("target = \"apps\""));
    }

    #[test]
    fn channel_names_and_kinds_are_checked_in_plain_words() {
        let c = config::defaults().channels;
        assert_eq!(channel_name_problem(&c, 1, "Voice"), None);
        assert!(channel_name_problem(&c, 1, "").unwrap().contains("needs a name"));
        assert!(channel_name_problem(&c, 1, "Discord").unwrap().contains("1 to 6"));
        assert!(channel_name_problem(&c, 1, "game").unwrap().contains("already called Game"));
        assert!(channel_name_problem(&c, 1, "Master").unwrap().contains("knobs"));
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
