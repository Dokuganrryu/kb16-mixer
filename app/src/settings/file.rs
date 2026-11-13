//! Writing config.toml: each change from the window is a small edit of the file's TOML, made so
//! that the comments and layout around it stay as the user had them.

use super::choices::KINDS;
use crate::config;
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value, value};

/// Applies `change` to the text of config.toml and checks the result the way the app will load
/// it. Refuses (with the reason, nothing written) when the file already has a problem or when
/// the change would leave one.
pub fn apply_edit(text: &str, change: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> Result<String, String> {
    let problem =
        |e: String| format!("config.toml has a problem, so nothing was saved. Fix it first (Open config file): {e}");
    config::parse(text).map_err(problem)?;
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| problem(e.to_string()))?;
    change(&mut doc)?;
    let new = doc.to_string();
    config::parse(&new).map_err(|e| format!("that would make the config invalid ({e})"))?;
    Ok(new)
}

pub fn edit_file(
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
pub fn edit(change: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> Result<(), String> {
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
pub fn put(item: &mut Item, mut new: Value) {
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

pub fn set_key(doc: &mut DocumentMut, page: usize, row: usize, col: usize, text: &str) {
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

pub fn set_knob(doc: &mut DocumentMut, knob: usize, text: &str) {
    put_in(table(doc, "knobs"), &format!("knob{}", knob + 1), text.into());
}

/// Sets one part ("tap", "hold" or "double") of a knob's press in [knob_press.knobN], made as
/// a section (or inline, inside an inline [knob_press]) if it is missing.
pub fn set_press(doc: &mut DocumentMut, knob: usize, part: &str, text: &str) {
    if doc.get("knob_press").and_then(Item::as_table_like).is_none() {
        let mut t = Table::new();
        t.set_implicit(true);
        doc["knob_press"] = Item::Table(t);
    }
    let presses = table(doc, "knob_press");
    let name = format!("knob{}", knob + 1);
    if presses.get(&name).and_then(Item::as_table_like).is_none() {
        // Inside an inline [knob_press], toml_edit makes the new table inline too.
        presses.insert(&name, Item::Table(Table::new()));
    }
    if let Some(press) = presses.get_mut(&name).and_then(Item::as_table_like_mut) {
        put_in(press, part, text.into());
    }
}

pub fn set_idle_percent(doc: &mut DocumentMut, percent: i64) {
    put_in(table(doc, "lights"), "idle_percent", percent.into());
}

/// Sets `[audio] output`; a device gets its name in a comment beside the id, so the window can
/// still name it while it is unplugged.
pub fn set_output(doc: &mut DocumentMut, id: &str, name: Option<&str>) {
    let audio = table(doc, "audio");
    put_in(audio, "output", id.into());
    if let Some(v) = audio.get_mut("output").and_then(Item::as_value_mut) {
        let comment = name.map(|n| format!("  # {}", n.replace(['\r', '\n'], " ")));
        v.decor_mut().set_suffix(comment.unwrap_or_default());
    }
}

/// The device name in the comment beside `[audio] output`, if there is one. A plain scan of
/// the lines: `output` is a key of [audio] only.
pub fn output_name(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.split('=').next().is_some_and(|k| k.trim() == "output"))?;
    let name = line.split_once('#')?.1.trim();
    (!name.is_empty()).then(|| name.to_string())
}

pub fn set_number(doc: &mut DocumentMut, name: &str, v: f64) {
    put(&mut doc[name], if v.fract() == 0.0 { (v as i64).into() } else { v.into() });
}

fn channels_mut(doc: &mut DocumentMut) -> Result<&mut ArrayOfTables, String> {
    doc.get_mut("channel").and_then(Item::as_array_of_tables_mut).ok_or_else(|| "no [[channel]] sections".to_string())
}

fn channel_mut(doc: &mut DocumentMut, i: usize) -> Result<&mut Table, String> {
    channels_mut(doc)?.get_mut(i).ok_or_else(|| format!("there is no channel {}", i + 1))
}

/// Renames channel `i`, and the knobs that were set to it by its old name.
pub fn rename_channel(doc: &mut DocumentMut, i: usize, old: &str, new: &str) -> Result<(), String> {
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

pub fn set_kind(doc: &mut DocumentMut, i: usize, kind: usize) -> Result<(), String> {
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
pub fn add_app(doc: &mut DocumentMut, i: usize, exe: &str) -> Result<(), String> {
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

pub fn remove_app(doc: &mut DocumentMut, i: usize, exe: &str) -> Result<(), String> {
    let ch = channel_mut(doc, i)?;
    let Some(apps) = ch.get_mut("apps").and_then(Item::as_array_mut) else { return Ok(()) };
    without(apps, exe);
    if apps.is_empty() && ch.get("target").is_none() {
        ch["target"] = value("apps");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelTarget, KeyAction, KnobBinding};

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
    fn presses_lights_and_the_output_device_are_written_in_their_tables() {
        use crate::config::{Output, PressAction};
        let text = edited(config::DEFAULT, |d| {
            set_press(d, 2, "double", "open:C:\\Tools\\obs64.exe");
            set_press(d, 0, "hold", "none");
            set_idle_percent(d, 20);
            set_output(d, "{0.0.0.00000000}.{a}", Some("USB speakers"));
            Ok(())
        });
        let cfg = config::parse(&text).unwrap();
        assert_eq!(cfg.knob_press[2].double, PressAction::Open("C:\\Tools\\obs64.exe".into()));
        assert_eq!(cfg.knob_press[0].hold, PressAction::Nothing);
        assert_eq!(cfg.knob_press[1], config::defaults().knob_press[1], "the other knobs stay");
        assert_eq!(cfg.idle_led_percent, 20);
        assert_eq!(cfg.output, Output::Device("{0.0.0.00000000}.{a}".into()));
        assert!(text.contains("output = \"{0.0.0.00000000}.{a}\"  # USB speakers\n"), "{text}");
        assert_eq!(output_name(&text).as_deref(), Some("USB speakers"));
        assert_eq!(text.lines().count(), config::DEFAULT.lines().count(), "edited in place");
        // Back to the Windows default: the name goes with the id.
        let text = edited(&text, |d| {
            set_output(d, "default", None);
            Ok(())
        });
        assert!(text.contains("output = \"default\"\n"), "{text}");
        assert_eq!(output_name(&text), None);

        // A file without the tables gets them; an inline [knob_press] stays inline.
        let bare = "[[channel]]\nname = \"Game\"\ntarget = \"other\"\n";
        let text = edited(bare, |d| {
            set_press(d, 1, "tap", "pick");
            set_idle_percent(d, 3);
            Ok(())
        });
        let cfg = config::parse(&text).unwrap();
        assert_eq!(cfg.knob_press[1].tap, PressAction::Pick);
        assert_eq!(cfg.knob_press[1].hold, PressAction::Pick, "left out: the default");
        assert_eq!(cfg.idle_led_percent, 3);
        assert!(text.contains("[knob_press.knob2]\ntap = \"pick\""), "{text}");
        assert!(!text.contains("[knob_press]\n"), "no empty parent section: {text}");
        let inline = format!("knob_press = {{ knob1 = {{ tap = \"none\" }} }}\n{bare}");
        let text = edited(&inline, |d| {
            set_press(d, 2, "hold", "mute");
            Ok(())
        });
        let first = text.lines().next().unwrap_or_default();
        assert!(first.starts_with("knob_press = { knob1 = { tap = \"none\" }"), "{text}");
        assert!(first.ends_with("knob3 = { hold = \"mute\" } }"), "still one inline table: {text}");
        assert_eq!(config::parse(&text).unwrap().knob_press[2].hold, PressAction::Mute);
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
}
