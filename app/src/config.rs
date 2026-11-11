//! Settings: parsing and checking config.toml, and the files the app keeps between runs.
//!
//! Files in %APPDATA%\kb16-mixer (or the folder named by `KB16_MIXER_DIR`, see `custom_dir`):
//!   config.toml  what the user edits (written with defaults on first run)
//!   apps.toml    apps moved to a channel with the Move key; only the mixer thread writes it
//!   state.toml   remembered channel levels, per-app trims, and what Focus has turned down

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub const DEFAULT: &str = r##"# kb16-mixer. Most of this is easier in the settings window (click the tray icon).
# If you edit by hand: save, then tray icon > Reload config. A mistake in this file cannot
# break anything: the last working settings stay active and the tray tells you what is wrong.
#
# All of this only moves sliders in the Windows volume mixer. A level you set on the pad
# sticks to the channel: an app that starts later in that channel takes it.

name = "KB16"         # shown big on the pad's idle screen: up to 10 letters, digits or spaces
step = 2              # percent per knob detent
brightness = 0.65     # key lights, 0.1 .. 1.0
oled_top = 3          # rows the screen is moved down; raise if the top looks cut off (0..6)
duck = 0.25           # how far Focus turns the other channels down (0.25 = to a quarter)
screen_off_min = 10   # minutes of quiet before the pad's screen turns off (0 = never)
lights_dim_min = 10   # minutes of quiet before the key lights dim (0 = never)
mixer_keys = true     # false = keys type 1-0/arrows as usual; lights and knobs stay the mixer

# KNOBS. Turn = volume. Press to step through the apps playing in the knob's channel and
# turn just that one (Spotify but not the browser); press on to get back to the whole
# channel, or leave the knob alone for 8 seconds. An app keeps its place in the mix when the
# whole channel is turned. Mute is on the bottom row of keys.
# Each is a channel name, or "master", "mic", "focused" (whatever app is in front).
[knobs]
knob1 = "Chat"        # small knob, top left
knob2 = "Media"       # small knob, top right
knob3 = "Game"        # big knob

# KEYS: four rows of four, top row first. Each column belongs to the channel above it;
# the lights in a column are that channel's level meter. There are two pages of keys:
# [keys] is the mixer page and [keys2] a keypad page; a "page" key switches between them.
#
#   "mute"    mute / unmute this column's channel (amber while muted)
#   "focus"   tap: turn every other channel down so this one stands out; tap again to undo.
#             Hold instead and it only lasts while held. Focus on Game = clutch mode.
#   "talk"    hold: flips the mic while held. Push-to-talk if you are muted, cough button if not.
#   "move"    hold it and tap another key in a column (any key this app handles: mute,
#             focus, talk, page, open): the app in front moves into that column's channel
#             (again to undo). Tap it alone to see which channel the app is in.
#   "page"    tap: switch to the other page of keys; tap again to come back. Hold instead
#             and the other page lasts only while held. The screen shows MIXER or KEYS.
#             If [keys] has a page key, [keys2] needs one too, so there is a way back.
#   "open:<what>"  opens a program, file, folder or web address, as a double-click would:
#             "open:C:\\Tools\\obs64.exe", "open:%USERPROFILE%\\Downloads", "open:https://..."
#   "none"    does nothing
#
# Anything else is typed by the pad itself as a real key press, so it works everywhere,
# games included:
#   media     "play", "next", "prev", "stop";  "next|prev" = tap for next, hold for previous
#   keys      "F13" .. "F24" are free keys nothing else uses: bind them in Discord (deafen),
#             OBS or a game. Also letters, digits, "enter", "esc", "space", arrows ...,
#             with modifiers: "ctrl+shift+m", "win+shift+s"
[keys]
row1 = ["page", "prev",        "play",  "next"]   # keypad page; media keys
row2 = ["move", "win+shift+s", "F13",   "F14"]    # move app; snipping tool; free keys to bind
row3 = ["talk", "focus",       "focus", "focus"]  # push-to-talk; Focus per channel
row4 = ["mute", "mute",        "mute",  "mute"]   # mute per channel

# The keypad page. Same rules; a missing row keeps the default shown here.
[keys2]
row1 = ["page",  "7", "8", "9"]                   # back to the mixer page; digits
row2 = ["0",     "4", "5", "6"]                   # digits
row3 = ["enter", "1", "2", "3"]                   # enter; digits
row4 = ["mute",  "mute", "mute", "mute"]          # mute stays at hand on both pages

# CHANNELS: up to 4, one per key column, left to right.
#   name = "..."        1 to 5 characters: what the pad's screen has room for
#   target = "mic"      your microphone: the meter shows your voice
#   apps = [...]        these apps (exe names, case-insensitive)
#   target = "other"    every app no other channel claims (games land here)
#   target = "focused"  whatever app is in front
# Colors are soft tints of white on purpose; loudness shows as brightness.
[[channel]]
name = "Mic"
color = "#dce6ff"
target = "mic"

[[channel]]
name = "Chat"
color = "#c8d2ff"
apps = ["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe", "Teams.exe", "ms-teams.exe", "Zoom.exe", "TeamSpeak.exe", "ts3client_win64.exe", "Mumble.exe"]

[[channel]]
name = "Game"
color = "#e6ecff"
target = "other"

[[channel]]
name = "Media"
color = "#d2f0ff"
apps = ["Spotify.exe", "AppleMusic.exe", "TIDAL.exe", "foobar2000.exe", "MusicBee.exe", "vlc.exe", "mpv.exe",
        "chrome.exe", "firefox.exe", "msedge.exe", "brave.exe", "opera.exe", "opera_gx.exe", "vivaldi.exe", "arc.exe"]
"##;

#[derive(Deserialize)]
struct RawConfig {
    #[serde(default = "default_name")]
    name: String,
    #[serde(default = "default_step")]
    step: f32,
    #[serde(default = "yes")]
    mixer_keys: bool,
    #[serde(default = "default_brightness")]
    brightness: f32,
    #[serde(default = "default_oled_top")]
    oled_top: u8,
    #[serde(default = "default_duck")]
    duck: f32,
    #[serde(default = "default_idle_min")]
    screen_off_min: u32,
    #[serde(default = "default_idle_min")]
    lights_dim_min: u32,
    #[serde(default)]
    knobs: RawKnobs,
    #[serde(default)]
    keys: RawKeys,
    #[serde(default)]
    keys2: RawKeys,
    #[serde(default)]
    channel: Vec<RawChannel>,
}

#[derive(Deserialize, Default)]
struct RawKnobs {
    knob1: Option<String>,
    knob2: Option<String>,
    knob3: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawKeys {
    row1: Option<Vec<String>>,
    row2: Option<Vec<String>>,
    row3: Option<Vec<String>>,
    row4: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawChannel {
    name: String,
    color: Option<String>,
    target: Option<String>,
    #[serde(default)]
    apps: Vec<String>,
}

#[derive(Clone)]
pub enum ChannelTarget {
    Apps(Vec<String>),
    Other,
    Focused,
    Mic,
}

#[derive(Clone)]
pub struct ChannelCfg {
    pub name: String,
    pub color: [u8; 3],
    pub target: ChannelTarget,
}

/// What one of the 16 keys does. Up to `Open` they are handled by this app; the last two are
/// typed by the pad itself as real key presses.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum KeyAction {
    Mute,
    Focus,
    Talk,
    Move,
    Page,
    /// Shell-open this program, file, folder or URL.
    Open(String),
    #[default]
    Nothing,
    Keyboard {
        code: u8,
        mods: u8,
    },
    Media {
        tap: u8,
        hold: u8,
    },
}

pub type KeyGrid = [[KeyAction; 4]; 4];

/// The mixer page ([keys]) and the keypad page ([keys2]).
pub const PAGES: usize = 2;
const PAGE_TABLES: [&str; PAGES] = ["keys", "keys2"];

pub const DEFAULT_KEYS: [[[&str; 4]; 4]; PAGES] = [
    [
        ["page", "prev", "play", "next"],
        ["move", "win+shift+s", "F13", "F14"],
        ["talk", "focus", "focus", "focus"],
        ["mute", "mute", "mute", "mute"],
    ],
    [["page", "7", "8", "9"], ["0", "4", "5", "6"], ["enter", "1", "2", "3"], ["mute", "mute", "mute", "mute"]],
];

fn media_usage(name: &str) -> Option<u8> {
    Some(match name {
        "play" | "pause" | "playpause" => 0xCD,
        "next" => 0xB5,
        "prev" | "previous" => 0xB6,
        "stop" => 0xB7,
        _ => return None,
    })
}

/// Key names the pad can type besides letters, digits and F1-F24, with their HID usage.
const NAMED_KEYS: &[(&str, u8)] = &[
    ("enter", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("printscreen", 0x46),
    ("pause", 0x48),
    ("insert", 0x49),
    ("home", 0x4A),
    ("pageup", 0x4B),
    ("delete", 0x4C),
    ("end", 0x4D),
    ("pagedown", 0x4E),
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
];

/// The config name of a HID keyboard usage, the way `parse_key` reads it back.
pub fn key_name(usage: u8) -> Option<String> {
    Some(match usage {
        0x04..=0x1D => ((b'a' + usage - 0x04) as char).to_string(),
        0x1E..=0x26 => ((b'1' + usage - 0x1E) as char).to_string(),
        0x27 => "0".into(),
        0x3A..=0x45 => format!("F{}", usage - 0x3A + 1),
        0x68..=0x73 => format!("F{}", usage - 0x68 + 13),
        _ => NAMED_KEYS.iter().find(|(_, u)| *u == usage)?.0.into(),
    })
}

/// HID keyboard usage for a key name.
fn key_usage(name: &str) -> Option<u8> {
    let b = name.as_bytes();
    let alias = match name {
        "return" => "enter",
        "escape" => "esc",
        "pgup" => "pageup",
        "del" => "delete",
        "pgdn" => "pagedown",
        other => other,
    };
    if let Some((_, usage)) = NAMED_KEYS.iter().find(|(n, _)| *n == alias) {
        return Some(*usage);
    }
    Some(match name {
        _ if b.len() == 1 && b[0].is_ascii_lowercase() => 0x04 + (b[0] - b'a'),
        _ if b.len() == 1 && (b'1'..=b'9').contains(&b[0]) => 0x1E + (b[0] - b'1'),
        "0" => 0x27,
        _ if b[0] == b'f' => match name[1..].parse::<u8>().ok()? {
            n @ 1..=12 => 0x3A + (n - 1),
            n @ 13..=24 => 0x68 + (n - 13),
            _ => return None,
        },
        _ => return None,
    })
}

pub fn parse_key(text: &str) -> Result<KeyAction, String> {
    // Before lowercasing: a path or URL keeps its case.
    let trimmed = text.trim();
    if trimmed.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("open:")) {
        let target = trimmed[5..].trim().trim_matches('"').trim();
        if target.is_empty() {
            return Err(format!("key {text:?}: open: needs a program, file, folder or web address after it"));
        }
        return Ok(KeyAction::Open(target.to_string()));
    }
    let name = trimmed.to_lowercase();
    let unknown = || format!("key {text:?}: not an action, media key or key name");
    match name.as_str() {
        "" | "none" => return Ok(KeyAction::Nothing),
        "mute" => return Ok(KeyAction::Mute),
        "focus" => return Ok(KeyAction::Focus),
        "talk" => return Ok(KeyAction::Talk),
        "move" => return Ok(KeyAction::Move),
        "page" => return Ok(KeyAction::Page),
        _ => {}
    }
    if let Some((tap, hold)) = name.split_once('|') {
        return match (media_usage(tap.trim()), media_usage(hold.trim())) {
            (Some(tap), Some(hold)) => Ok(KeyAction::Media { tap, hold }),
            _ => Err(format!("key {text:?}: tap|hold works with media keys (play, next, prev, stop)")),
        };
    }
    if let Some(tap) = media_usage(&name) {
        return Ok(KeyAction::Media { tap, hold: 0 });
    }
    let mut mods = 0u8;
    let mut code = None;
    for part in name.split('+').map(str::trim) {
        match part {
            "ctrl" | "control" => mods |= 0x01,
            "shift" => mods |= 0x02,
            "alt" => mods |= 0x04,
            "win" | "gui" | "cmd" => mods |= 0x08,
            "" => return Err(unknown()),
            key => {
                if code.is_some() {
                    return Err(format!("key {text:?}: one key plus modifiers, please"));
                }
                code = Some(key_usage(key).ok_or_else(unknown)?);
            }
        }
    }
    code.map(|code| KeyAction::Keyboard { code, mods }).ok_or_else(unknown)
}

fn parse_grid(raw: RawKeys, page: usize) -> Result<KeyGrid, String> {
    let table = PAGE_TABLES[page];
    let rows = [raw.row1, raw.row2, raw.row3, raw.row4];
    let mut grid = KeyGrid::default();
    for (r, row) in rows.into_iter().enumerate() {
        let names: Vec<String> = match row {
            Some(names) if names.len() == 4 => names,
            Some(names) => return Err(format!("[{table}] row{}: {} entries, a row has 4 keys", r + 1, names.len())),
            None => DEFAULT_KEYS[page][r].iter().map(|s| s.to_string()).collect(),
        };
        for (c, name) in names.iter().enumerate() {
            grid[r][c] = parse_key(name).map_err(|e| format!("[{table}] row{}: {e}", r + 1))?;
        }
    }
    Ok(grid)
}

/// True if a tap could switch to the keypad page with no key there to switch back.
pub fn stuck_on_page_two(pages: &[KeyGrid; PAGES]) -> bool {
    let has_page_key = |g: &KeyGrid| g.iter().flatten().any(|a| *a == KeyAction::Page);
    has_page_key(&pages[0]) && !has_page_key(&pages[1])
}

/// Both pages, refused if they fail `stuck_on_page_two`.
fn parse_pages(keys: RawKeys, keys2: RawKeys) -> Result<[KeyGrid; PAGES], String> {
    let pages = [parse_grid(keys, 0)?, parse_grid(keys2, 1)?];
    if stuck_on_page_two(&pages) {
        return Err("[keys2] has no \"page\" key, so there would be no way back to [keys]".into());
    }
    Ok(pages)
}

#[derive(Clone, Copy, PartialEq)]
pub enum KnobBinding {
    Selected,
    Master,
    Mic,
    Focused,
    Channel(usize),
}

#[derive(Clone)]
pub struct Config {
    /// For the logo screen: 1..=10 characters, capitals, digits and spaces.
    pub name: String,
    pub step: f32,
    pub mixer_keys: bool,
    /// LED brightness byte for the pad, 1..=255.
    pub brightness: u8,
    pub oled_top: u8,
    /// Volume factor for the channels Focus turns down.
    pub duck: f32,
    /// Minutes of quiet before the screen turns off / the lights dim. 0 = never.
    pub screen_off_min: u8,
    pub lights_dim_min: u8,
    pub knobs: [KnobBinding; 3],
    /// [keys] then [keys2]; the pad starts on the first.
    pub pages: [KeyGrid; PAGES],
    pub channels: Vec<ChannelCfg>,
}

impl Config {
    /// The channel a "selected" knob falls back to: the catch-all (games), else the first
    /// app channel.
    pub fn default_selected(&self) -> usize {
        let other = self.channels.iter().position(|c| matches!(c.target, ChannelTarget::Other));
        let any_app = self.channels.iter().position(|c| !matches!(c.target, ChannelTarget::Mic));
        other.or(any_app).unwrap_or(0)
    }
}

fn default_name() -> String {
    "KB16".into()
}

/// The pad draws the logo name in capitals; it has letters for A-Z, 0-9 and space only.
pub fn parse_name(text: &str) -> Result<String, String> {
    let name = text.trim().to_uppercase();
    let fits = name.chars().all(|c| c == ' ' || c.is_ascii_uppercase() || c.is_ascii_digit());
    if name.is_empty() || name.len() > kb16_mixer_protocol::LOGO_NAME_LEN || !fits {
        return Err(format!("name = {text:?}: 1 to 10 letters, digits or spaces"));
    }
    Ok(name)
}

fn default_step() -> f32 {
    2.0
}

fn default_brightness() -> f32 {
    0.65
}

fn default_idle_min() -> u32 {
    10
}

fn default_duck() -> f32 {
    0.25
}

fn default_oled_top() -> u8 {
    3
}

fn yes() -> bool {
    true
}

fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

impl RawConfig {
    /// Turns the file's contents into settings, or says what is wrong in plain words.
    fn check(self) -> Result<Config, String> {
        if self.channel.is_empty() {
            return Err("no [[channel]] sections".into());
        }
        if self.channel.len() > 4 {
            return Err(format!("{} channels, the pad has 4 columns", self.channel.len()));
        }
        let palette = [[220, 230, 255], [200, 210, 255], [230, 236, 255], [210, 240, 255]];
        let mut channels = Vec::new();
        for (i, c) in self.channel.into_iter().enumerate() {
            let shown = kb16_mixer_protocol::NAME_SHOWN;
            if c.name.trim().is_empty() || c.name.chars().count() > shown {
                return Err(format!("channel name {:?}: 1 to {shown} characters, what the pad's screen shows", c.name));
            }
            let color = match c.color.as_deref() {
                None => palette[i],
                Some(text) => parse_color(text).ok_or(format!("channel {}: color {text:?} is not #rrggbb", c.name))?,
            };
            let target = match c.target.as_deref().map(str::to_lowercase).as_deref() {
                Some("other") => ChannelTarget::Other,
                Some("focused") => ChannelTarget::Focused,
                Some("mic") => ChannelTarget::Mic,
                // An explicit "apps" may start empty (the settings window writes that); without a
                // target, an empty list is more likely a mistake.
                Some("apps") => ChannelTarget::Apps(c.apps),
                None if !c.apps.is_empty() => ChannelTarget::Apps(c.apps),
                None => return Err(format!("channel {}: needs apps = [...] or a target", c.name)),
                Some(other) => return Err(format!("channel {}: unknown target {other:?}", c.name)),
            };
            channels.push(ChannelCfg { name: c.name, color, target });
        }
        let bind = |knob: &str, text: Option<String>, default: KnobBinding| -> Result<KnobBinding, String> {
            let Some(text) = text else { return Ok(default) };
            Ok(match text.to_lowercase().as_str() {
                "selected" => KnobBinding::Selected,
                "master" | "system" => KnobBinding::Master,
                "mic" => KnobBinding::Mic,
                "focused" => KnobBinding::Focused,
                name => match channels.iter().position(|c| c.name.eq_ignore_ascii_case(name)) {
                    Some(i) => KnobBinding::Channel(i),
                    None => return Err(format!("{knob} = {text:?}: no channel with that name")),
                },
            })
        };
        Ok(Config {
            name: parse_name(&self.name)?,
            step: in_range("step", self.step, 0.5, 20.0)?,
            mixer_keys: self.mixer_keys,
            brightness: (in_range("brightness", self.brightness, 0.1, 1.0)? * 255.0) as u8,
            oled_top: self.oled_top.min(6),
            duck: in_range("duck", self.duck, 0.0, 1.0)?,
            screen_off_min: self.screen_off_min.min(240) as u8,
            lights_dim_min: self.lights_dim_min.min(240) as u8,
            pages: parse_pages(self.keys, self.keys2)?,
            knobs: [
                bind("knob1", self.knobs.knob1, KnobBinding::Master)?,
                bind("knob2", self.knobs.knob2, KnobBinding::Master)?,
                bind("knob3", self.knobs.knob3, KnobBinding::Selected)?,
            ],
            channels,
        })
    }
}

/// A number outside its range goes to the nearest limit, but TOML also accepts nan, inf and
/// -inf: nan passes `clamp` unchanged, and no limit is "nearest" to it, so these are errors.
fn in_range(key: &str, value: f32, low: f32, high: f32) -> Result<f32, String> {
    if value.is_finite() {
        return Ok(value.clamp(low, high));
    }
    let written = if value.is_nan() {
        "nan"
    } else if value > 0.0 {
        "inf"
    } else {
        "-inf"
    };
    Err(format!("{key} = {written}: a number from {low:?} to {high:?}"))
}

const DIR_NAME: &str = "kb16-mixer";
/// The settings folder's name before the app was renamed.
const OLD_DIR_NAME: &str = "kb16-knobs";

fn base_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// The environment variable that replaces the settings folder.
pub const DIR_VAR: &str = "KB16_MIXER_DIR";

/// The folder named by `KB16_MIXER_DIR`, if set: a portable copy, or a build tried against a
/// throwaway folder. With it, nothing is carried over from an old install and "Start with
/// Windows" is never written (see `main.rs`).
pub fn custom_dir() -> Option<PathBuf> {
    std::env::var_os(DIR_VAR).filter(|d| !d.is_empty()).map(PathBuf::from)
}

pub fn dir() -> PathBuf {
    custom_dir().unwrap_or_else(|| base_dir().join(DIR_NAME))
}

/// Carries the settings of an install from before the rename across, once: only while the new
/// folder does not exist yet. The old folder is left as it is. Returns true if it copied.
pub fn migrate_old_settings() -> bool {
    custom_dir().is_none() && migrate_from(&base_dir())
}

fn migrate_from(base: &std::path::Path) -> bool {
    let (old, new) = (base.join(OLD_DIR_NAME), base.join(DIR_NAME));
    if new.exists() || !old.is_dir() {
        return false;
    }
    // Copy into a side folder and rename it at the end, so a copy cut short is never taken
    // for a finished one.
    let staging = base.join(format!("{DIR_NAME}.migrating"));
    let _ = std::fs::remove_dir_all(&staging);
    let copied = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&staging)?;
        for entry in std::fs::read_dir(&old)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                std::fs::copy(entry.path(), staging.join(entry.file_name()))?;
            }
        }
        std::fs::rename(&staging, &new)
    })();
    if copied.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    copied.is_ok()
}

pub fn path() -> PathBuf {
    dir().join("config.toml")
}

pub fn parse(text: &str) -> Result<Config, String> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| {
        // toml's message spans several lines (location, source excerpt, reason); keep it on one.
        e.to_string().lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(" ")
    })?;
    raw.check()
}

/// A problem in config.toml, told without quoting the file: where it is (for what the TOML
/// reader finds, 1-based) and why.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub at: Option<(usize, usize)>,
    pub reason: String,
}

/// `parse`, with the place of a TOML error as a line and column (for `--check`).
pub fn parse_located(text: &str) -> Result<Config, Problem> {
    let raw: RawConfig = toml::from_str(text).map_err(|e| Problem {
        at: e.span().map(|s| line_column(text, s.start)),
        reason: e.message().trim().to_string(),
    })?;
    raw.check().map_err(|reason| Problem { at: None, reason })
}

/// Line and column (1-based, in characters) of a byte offset into `text`.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset).unwrap_or(text);
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (before.matches('\n').count() + 1, before[line_start..].chars().count() + 1)
}

pub fn defaults() -> Config {
    parse(DEFAULT).expect("built-in config is valid")
}

/// True the very first time the app runs (no config yet).
pub fn is_first_run() -> bool {
    !path().exists()
}

/// Reads config.toml, writing the defaults first if it does not exist.
pub fn load() -> Result<Config, String> {
    let p = path();
    if !p.exists() {
        let _ = std::fs::create_dir_all(dir());
        std::fs::write(&p, DEFAULT).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    }
    let text = std::fs::read_to_string(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))?;
    parse(&text)
}

/// Replaces a file so that it is never seen half written. The new text goes to a side file,
/// which is flushed to the disk (`sync_all`) before it takes the old file's place in one rename.
/// After a crash, a full disk or a power cut the file is therefore whole: the old text or the
/// new one. The rename itself is not flushed, so a power cut just after it can leave the old
/// text. Not tested with a real power cut.
pub fn write_whole(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()
    })();
    written.and_then(|()| std::fs::rename(&tmp, path)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Both pages' key tables as written in `text` (for showing a custom entry unchanged).
pub fn raw_keys(text: &str) -> [[[String; 4]; 4]; PAGES] {
    #[derive(Deserialize, Default)]
    struct File {
        #[serde(default)]
        keys: RawKeys,
        #[serde(default)]
        keys2: RawKeys,
    }
    let file: File = toml::from_str(text).unwrap_or_default();
    let mut out: [[[String; 4]; 4]; PAGES] = Default::default();
    for (page, keys) in [file.keys, file.keys2].into_iter().enumerate() {
        let rows = [keys.row1, keys.row2, keys.row3, keys.row4];
        for (r, row) in rows.into_iter().enumerate() {
            for c in 0..4 {
                out[page][r][c] = match &row {
                    Some(names) if names.len() == 4 => names[c].clone(),
                    _ => DEFAULT_KEYS[page][r][c].to_string(),
                };
            }
        }
    }
    out
}

fn apps_path() -> PathBuf {
    dir().join("apps.toml")
}

/// Apps moved by holding Move and tapping a column: exe (lowercase) -> channel name.
pub fn load_apps() -> HashMap<String, String> {
    let text = std::fs::read_to_string(apps_path()).unwrap_or_default();
    let map: HashMap<String, String> = toml::from_str(&text).unwrap_or_default();
    map.into_iter().map(|(exe, channel)| (exe.to_lowercase(), channel)).collect()
}

pub fn save_apps(apps: &HashMap<String, String>) {
    let mut lines: Vec<String> = apps.iter().map(|(exe, ch)| format!("{exe:?} = {ch:?}")).collect();
    lines.sort();
    let header = "# Apps moved with the Move key on the pad: \"app.exe\" = \"Channel\". They win over the lists\n\
                  # in config.toml. Delete a line to undo, or use Remove in the settings window.";
    if let Err(e) = write_whole(&apps_path(), &format!("{header}\n{}\n", lines.join("\n"))) {
        crate::log(&format!("could not save apps.toml: {e}"));
    }
}

/// Channels that kept their place but got a new name, (old, new): what is remembered by name
/// (levels, apps moved there) follows them. A name that still exists elsewhere, or a new name
/// that was already there, is a reshuffle, not a rename. A change of case only is a rename:
/// levels are kept under the exact name.
pub fn renames(before: &[String], now: &[ChannelCfg]) -> Vec<(String, String)> {
    let had = |name: &str| before.iter().any(|b| b.eq_ignore_ascii_case(name));
    let has = |name: &str| now.iter().any(|c| c.name.eq_ignore_ascii_case(name));
    before
        .iter()
        .zip(now)
        .filter(|(old, c)| **old != c.name && (old.eq_ignore_ascii_case(&c.name) || (!has(old) && !had(&c.name))))
        .map(|(old, c)| (old.clone(), c.name.clone()))
        .collect()
}

/// What the pad remembers between runs.
#[derive(Serialize, Deserialize, Default, PartialEq, Clone)]
pub struct Saved {
    /// Channel levels set on the pad, 0.0..=1.0, by channel name.
    #[serde(default)]
    pub levels: HashMap<String, f32>,
    /// Apps Focus has turned down right now (exe -> their real level). Written in the same pass
    /// of the mixer loop that turns them down, not after the usual wait (see `worker.rs`), and
    /// flushed to the disk (see `write_whole`). If the PC is switched off mid-focus, they get
    /// their real level back on the next start. A power cut in the moment between the turn-down
    /// and that write (a few milliseconds) loses it.
    #[serde(default)]
    pub restore: HashMap<String, f32>,
    /// Apps turned on their own inside a channel: exe -> share of the channel level (0..1).
    #[serde(default)]
    pub trims: HashMap<String, f32>,
}

pub fn load_state() -> Saved {
    let text = std::fs::read_to_string(dir().join("state.toml")).unwrap_or_default();
    toml::from_str(&text).unwrap_or_default()
}

/// Whole or not at all (`write_whole`): a torn file would lose every level, trim and the list of
/// apps Focus has turned down.
pub fn save_state(state: &Saved) {
    match toml::to_string(state) {
        Ok(text) => {
            if let Err(e) = write_whole(&dir().join("state.toml"), &text) {
                crate::log(&format!("could not save state.toml: {e}"));
            }
        }
        Err(e) => crate::log(&format!("could not encode state: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(text: &str) -> String {
        parse(text).err().expect("should be rejected")
    }

    #[test]
    fn built_in_config_is_the_game_and_chat_layout() {
        let c = defaults();
        let names: Vec<&str> = c.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Mic", "Chat", "Game", "Media"]);
        assert!(matches!(c.channels[0].target, ChannelTarget::Mic));
        assert!(
            c.knobs == [KnobBinding::Channel(1), KnobBinding::Channel(3), KnobBinding::Channel(2)],
            "Chat, Media, Game"
        );
        assert_eq!(c.default_selected(), 2);
        let [mixer, keypad] = c.pages;
        assert_eq!(mixer[3], [const { KeyAction::Mute }; 4], "bottom row mutes");
        assert_eq!(keypad[3], [const { KeyAction::Mute }; 4], "on both pages");
        assert_eq!(mixer[2][0], KeyAction::Talk);
        assert_eq!(mixer[2][1..], [const { KeyAction::Focus }; 3]);
        assert_eq!((&mixer[0][0], &keypad[0][0]), (&KeyAction::Page, &KeyAction::Page));
        assert_eq!(mixer[0][1..], [0xB6, 0xCD, 0xB5].map(|tap| KeyAction::Media { tap, hold: 0 }), "prev play next");
        assert_eq!(mixer[1][0], KeyAction::Move);
        assert_eq!(mixer[1][1], KeyAction::Keyboard { code: 0x16, mods: 0x0A }, "win+shift+s");
        assert_eq!(mixer[1][2], KeyAction::Keyboard { code: 0x68, mods: 0 }, "F13");
        assert_eq!(keypad[0][1], KeyAction::Keyboard { code: 0x24, mods: 0 }, "7");
        assert_eq!(keypad[1][0], KeyAction::Keyboard { code: 0x27, mods: 0 }, "0");
        assert_eq!(keypad[2][0], KeyAction::Keyboard { code: 0x28, mods: 0 }, "enter");
    }

    #[test]
    fn the_keypad_page_falls_back_to_its_defaults_and_always_has_a_way_back() {
        let one_channel = "[[channel]]\nname = \"A\"\ntarget = \"other\"\n";
        let c = parse(one_channel).unwrap();
        assert_eq!(c.pages, defaults().pages, "no [keys] or [keys2]: both defaults");
        let c = parse(&format!("[keys2]\nrow2 = [\"a\", \"b\", \"c\", \"d\"]\n{one_channel}")).unwrap();
        assert_eq!(c.pages[1][1][0], KeyAction::Keyboard { code: 0x04, mods: 0 });
        assert_eq!(c.pages[1][0][0], KeyAction::Page, "rows left out keep the default");

        let stuck = format!("[keys2]\nrow1 = [\"1\", \"2\", \"3\", \"4\"]\n{one_channel}");
        assert!(problem(&stuck).contains("no way back"));
        let unreachable = format!("[keys]\nrow1 = [\"move\", \"F13\", \"F14\", \"F15\"]\n{stuck}");
        assert!(parse(&unreachable).is_ok(), "no page key on either page is fine");
        assert!(problem(&format!("[keys2]\nrow3 = [\"x\"]\n{one_channel}")).contains("[keys2] row3"));
    }

    #[test]
    fn mistakes_are_explained_not_swallowed() {
        assert!(problem("step = 2").contains("no [[channel]]"));
        assert!(problem("[[channel]]\nname = \"A\"\ncolor = \"red\"\ntarget = \"other\"").contains("not #rrggbb"));
        assert!(problem("[[channel]]\nname = \"A\"").contains("needs apps"));
        assert!(problem("[[channel]]\nname = \"A\"\ntarget = \"gmae\"").contains("unknown target"));
        assert!(
            problem("[knobs]\nknob1 = \"Chta\"\n[[channel]]\nname = \"Chat\"\napps = [\"x.exe\"]")
                .contains("no channel with that name")
        );
        assert!(problem("[[channel]\nname = 1").contains("line 1"), "syntax errors carry the line");
        let five = "[[channel]]\nname = \"A\"\ntarget = \"other\"\n".repeat(5);
        assert!(problem(&five).contains("4 columns"));
        assert!(problem("[[channel]]\nname = \"Gaming\"\ntarget = \"other\"").contains("1 to 5"), "what the pad shows");
        assert!(problem("[[channel]]\nname = \"\"\ntarget = \"other\"").contains("1 to 5"));
        assert!(parse("[[channel]]\nname = \"Gamer\"\ntarget = \"other\"").is_ok());
    }

    #[test]
    fn a_toml_error_has_a_line_and_a_column() {
        let at = |text: &str| parse_located(text).err().and_then(|p| p.at);
        assert_eq!(at("[[channel]\nname = 1"), Some((1, 11)), "syntax: where the second bracket should be");
        assert_eq!(at("step = 2\n[[channel]]\nname = 1\ntarget = \"other\""), Some((3, 8)), "type: the value");
        assert_eq!(at("[[channel]]\nname = \"A\""), None, "a rule of this app, not of TOML");
        assert_eq!(line_column("ab\ncd\u{e9}f", 7), (2, 4), "columns count characters");
        assert!(parse_located(DEFAULT).is_ok());
    }

    #[test]
    fn key_names() {
        assert_eq!(parse_key("F24"), Ok(KeyAction::Keyboard { code: 0x73, mods: 0 }));
        assert_eq!(parse_key("f1"), Ok(KeyAction::Keyboard { code: 0x3A, mods: 0 }));
        assert_eq!(parse_key("Ctrl+Shift+M"), Ok(KeyAction::Keyboard { code: 0x10, mods: 0x03 }));
        assert_eq!(parse_key("0"), Ok(KeyAction::Keyboard { code: 0x27, mods: 0 }));
        assert_eq!(parse_key(" None "), Ok(KeyAction::Nothing));
        assert!(parse_key("F25").is_err());
        assert!(parse_key("ctrl+").is_err());
        assert!(parse_key("a+b").unwrap_err().contains("one key"));
        assert!(parse_key("mute|focus").unwrap_err().contains("media"));
        assert_eq!(parse_key(" Open:C:\\Tools\\OBS.exe "), Ok(KeyAction::Open("C:\\Tools\\OBS.exe".into())));
        assert_eq!(parse_key("open: \"D:\\My Games\" "), Ok(KeyAction::Open("D:\\My Games".into())));
        assert_eq!(parse_key("OPEN:https://example.com/A"), Ok(KeyAction::Open("https://example.com/A".into())));
        assert!(parse_key("open:").unwrap_err().contains("needs a program"));
        assert!(parse_key("open: \"\"").is_err());
        assert!(parse_key("opener").is_err(), "only the prefix counts");
        let short = "[keys]\nrow1 = [\"mute\"]\n[[channel]]\nname = \"A\"\ntarget = \"other\"";
        assert!(problem(short).contains("a row has 4 keys"));
    }

    #[test]
    fn the_logo_name_is_checked() {
        let with = |name: &str| parse(&format!("name = {name:?}\n[[channel]]\nname = \"A\"\ntarget = \"other\""));
        assert_eq!(defaults().name, "KB16");
        assert_eq!(with(" Desk pc ").unwrap().name, "DESK PC");
        assert_eq!(with("ABCDEFGHIJ").unwrap().name.len(), 10);
        for bad in ["", "   ", "ABCDEFGHIJK", "Ünï", "KB-16"] {
            assert!(
                problem(&format!("name = {bad:?}\n[[channel]]\nname = \"A\"\ntarget = \"other\"")).contains("1 to 10")
            );
        }
    }

    #[test]
    fn numbers_that_are_not_finite_are_rejected() {
        let with = |line: &str| parse(&format!("{line}\n[[channel]]\nname = \"A\"\ntarget = \"other\""));
        for (key, range) in [("step", "0.5 to 20.0"), ("brightness", "0.1 to 1.0"), ("duck", "0.0 to 1.0")] {
            for bad in ["nan", "inf", "-inf"] {
                let e = with(&format!("{key} = {bad}")).err().expect("should be rejected");
                assert_eq!(e, format!("{key} = {bad}: a number from {range}"));
            }
        }
        let c = with("step = 5\nbrightness = 0.5\nduck = 0.5").unwrap();
        assert_eq!((c.step, c.brightness, c.duck), (5.0, 127, 0.5));
    }

    #[test]
    fn out_of_range_numbers_are_clamped() {
        let c = parse("step = 900\nbrightness = 7\noled_top = 40\nscreen_off_min = 100000\n[[channel]]\nname = \"A\"\ntarget = \"other\"").unwrap();
        assert_eq!(c.screen_off_min, 240);
        assert_eq!(c.lights_dim_min, 10, "default");
        assert_eq!(c.step, 20.0);
        assert_eq!(c.brightness, 255);
        assert_eq!(c.oled_top, 6);
    }

    #[test]
    fn settings_from_before_the_rename_are_carried_across_once() {
        let base = std::env::temp_dir().join(format!("kb16-migrate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        assert!(!migrate_from(&base), "nothing to migrate");

        std::fs::create_dir_all(base.join(OLD_DIR_NAME)).unwrap();
        std::fs::write(base.join(OLD_DIR_NAME).join("config.toml"), "step = 5").unwrap();
        std::fs::write(base.join(OLD_DIR_NAME).join("state.toml"), "[levels]").unwrap();
        assert!(migrate_from(&base));
        assert_eq!(std::fs::read_to_string(base.join(DIR_NAME).join("config.toml")).unwrap(), "step = 5");
        assert_eq!(std::fs::read_to_string(base.join(DIR_NAME).join("state.toml")).unwrap(), "[levels]");
        assert!(base.join(OLD_DIR_NAME).join("config.toml").exists(), "the old folder stays");
        assert!(!base.join(format!("{DIR_NAME}.migrating")).exists());

        std::fs::write(base.join(DIR_NAME).join("config.toml"), "step = 3").unwrap();
        assert!(!migrate_from(&base), "only once");
        assert_eq!(std::fs::read_to_string(base.join(DIR_NAME).join("config.toml")).unwrap(), "step = 3");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_whole_write_leaves_the_complete_file_and_no_side_file() {
        let dir = std::env::temp_dir().join(format!("kb16-write-whole-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("state.toml");
        let long = "[levels]
"
        .to_string()
            + &"\"Game\" = 0.5
"
            .repeat(2000);
        for text in [
            long.as_str(),
            "[levels]
\"Chat\" = 0.25
",
        ] {
            write_whole(&file, text).unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), text, "the whole text, the old one replaced");
            let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert_eq!(names, ["state.toml"], "no side file is left");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_renamed_channel_is_told_from_a_reshuffle() {
        let before: Vec<String> = ["Mic", "Chat", "Game", "Media"].map(String::from).to_vec();
        let with = |names: [&str; 4]| {
            let mut c = defaults().channels;
            for (ch, n) in c.iter_mut().zip(names) {
                ch.name = n.into();
            }
            renames(&before, &c)
        };
        assert_eq!(with(["Mic", "Talk", "Game", "Media"]), [("Chat".to_string(), "Talk".to_string())]);
        assert!(with(["Mic", "Chat", "Game", "Media"]).is_empty());
        assert_eq!(
            with(["Mic", "CHAT", "Game", "Media"]),
            [("Chat".to_string(), "CHAT".to_string())],
            "case only: the level is remembered by the exact name"
        );
        assert!(with(["Mic", "game", "chat", "Media"]).is_empty(), "swapped, whatever the case");
        assert!(with(["Mic", "Game", "Chat", "Media"]).is_empty(), "swapped, not renamed");
        assert!(renames(&[], &defaults().channels).is_empty(), "first load");
    }

    #[test]
    fn a_config_without_mic_or_catch_all_still_picks_a_channel() {
        let c = parse("[[channel]]\nname = \"A\"\ntarget = \"mic\"\n[[channel]]\nname = \"B\"\napps = [\"b.exe\"]")
            .unwrap();
        assert_eq!(c.default_selected(), 1);
    }
}
