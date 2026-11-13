//! `--check`: a short report that is safe to paste into a bug report, and `--selftest`.
//!
//! `--check` only reads: the USB device list, config.toml, the autostart value and Core Audio.
//! It names no program, user, folder, pad name or serial number; the only free text it prints
//! is the product string of a pad with other firmware. Each part is a plain function of what
//! was read, so the tests below need no hardware.

use crate::audio::{Census, Mixer, OutputState};
use crate::{VERSION, autostart, config};
use kb16_mixer_protocol as protocol;
use std::time::Duration;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GETIDLIST_FILTER_ENUMERATOR, CM_GETIDLIST_FILTER_PRESENT, CM_Get_Device_ID_List_SizeW, CM_Get_Device_ID_ListW,
    CR_SUCCESS,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
use windows::core::w;

/// The stm32duino bootloader that the pad restarts into for a flash.
const BOOTLOADER_ID: &str = "USB\\VID_1EAF&PID_0003";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

type Line = (Level, String);

/// One HID interface from the device list, as much of it as the report needs.
#[derive(Clone, Debug, Default)]
struct Hid {
    vendor_id: u16,
    product_id: u16,
    usage_page: u16,
    product: Option<String>,
    release: u16,
}

/// Prints the report and returns the exit code: 0 with no FAIL, else 1.
pub fn run() -> i32 {
    let lines = [
        ("app", app_line(config::custom_dir().is_some())),
        ("pad", pad_line(&hid_devices(), bootloaders())),
        ("settings", settings_line(read_config())),
        ("autostart", autostart_line(autostart::run_value_present(), config::custom_dir().is_some())),
        ("audio", audio_line(census(read_config()).map_err(|e| e.code().0))),
    ];
    let (text, code) = report(&lines);
    to_console();
    print!("{text}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    code
}

/// A Windows app has no console of its own: write to the one it was started from, unless the
/// output already goes to a pipe or a file.
fn to_console() {
    // SAFETY: both calls take plain values and only change this process's standard handles.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|h| h.is_invalid() || h.0.is_null()) {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn hid_devices() -> Vec<Hid> {
    // Enumeration only: no device is opened.
    let Ok(api) = hidapi::HidApi::new() else { return Vec::new() };
    api.device_list()
        .map(|d| Hid {
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            usage_page: d.usage_page(),
            product: d.product_string().map(str::to_string),
            release: d.release_number(),
        })
        .collect()
}

/// USB devices present that are the bootloader. It is not a HID device, so hidapi does not see it.
fn bootloaders() -> usize {
    let filter = w!("USB");
    let flags = CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT;
    let mut len = 0u32;
    // SAFETY: `len` is a live u32 and the filter a static wide string.
    if unsafe { CM_Get_Device_ID_List_SizeW(&mut len, filter, flags) } != CR_SUCCESS {
        return 0;
    }
    let mut ids = vec![0u16; len as usize];
    // SAFETY: the buffer has the length Windows asked for; the call writes only inside it.
    if unsafe { CM_Get_Device_ID_ListW(filter, &mut ids, flags) } != CR_SUCCESS {
        return 0;
    }
    count_bootloaders(&ids)
}

/// Counts the bootloader in a list of NUL-separated device instance ids.
fn count_bootloaders(ids: &[u16]) -> usize {
    ids.split(|&c| c == 0)
        .filter(|id| String::from_utf16_lossy(id).to_ascii_uppercase().starts_with(BOOTLOADER_ID))
        .count()
}

/// None when there is no config.toml yet; the error kind when it cannot be read.
/// Core Audio as the app would use it, on the output device config.toml chooses (the Windows
/// default when the file is missing or wrong).
fn census(config: Option<Result<String, std::io::ErrorKind>>) -> windows::core::Result<Census> {
    let mut mixer = Mixer::new()?;
    if let Some(Ok(cfg)) = config.and_then(Result::ok).map(|text| config::parse(&text)) {
        mixer.output = cfg.output;
    }
    Ok(mixer.census())
}

fn read_config() -> Option<Result<String, std::io::ErrorKind>> {
    match std::fs::read_to_string(config::path()) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        other => Some(other.map_err(|e| e.kind())),
    }
}

fn app_line(custom_dir: bool) -> Line {
    let folder = if custom_dir { "the folder in KB16_MIXER_DIR" } else { "the default folder" };
    (Level::Ok, format!("kb16-mixer {VERSION}, settings in {folder}"))
}

fn pad_line(hid: &[Hid], bootloaders: usize) -> Line {
    let pad = |d: &&Hid| d.vendor_id == protocol::VID && d.product_id == protocol::PID;
    // One raw-HID interface per pad; the other interfaces of the same pad are not counted.
    let raw = |d: &&Hid| pad(d) && d.usage_page == protocol::USAGE_PAGE;
    let ours: Vec<&Hid> = hid.iter().filter(raw).filter(|d| d.product.as_deref() == Some(protocol::PRODUCT)).collect();
    let mut others: Vec<&Hid> =
        hid.iter().filter(raw).filter(|d| d.product.as_deref() != Some(protocol::PRODUCT)).collect();
    if ours.is_empty() && others.is_empty() {
        // Other firmware may have no interface on our usage page.
        others.extend(hid.iter().find(pad));
    }
    let total = ours.len() + others.len() + bootloaders;
    if total > 1 {
        let text = format!(
            "{total} pads are connected ({} in app mode, {} with other firmware, {bootloaders} in bootloader mode). \
             The app uses only one. Connect only one pad.",
            ours.len(),
            others.len()
        );
        return (Level::Warn, text);
    }
    let usb = format!("USB {:04X}:{:04X}", protocol::VID, protocol::PID);
    if let Some(d) = ours.first() {
        let version = match d.release {
            protocol::UNVERSIONED_RELEASE => "unknown (an early build)".to_string(),
            r => {
                let (a, b, c) = protocol::release_version(r);
                format!("{a}.{b}.{c}")
            }
        };
        return (Level::Ok, format!("{}, firmware {version} ({usb})", protocol::PRODUCT));
    }
    if bootloaders == 1 {
        return (Level::Fail, "The pad is in bootloader mode (USB 1EAF:0003). Unplug it and plug it in again.".into());
    }
    if let Some(d) = others.first() {
        let product = printable(d.product.as_deref().unwrap_or(""));
        let text = format!(
            "A pad with other firmware is connected ({usb}, product \"{product}\"). \
             This app needs the KB16 Mixer firmware. See docs/install.md."
        );
        return (Level::Warn, text);
    }
    (Level::Fail, "No pad found. Check the USB cable and port.".into())
}

/// Printable ASCII only, at most 32 characters: the product string comes from the device.
fn printable(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(32).collect()
}

fn settings_line(file: Option<Result<String, std::io::ErrorKind>>) -> Line {
    let text = match file {
        None => return (Level::Ok, "No config.toml yet. The app writes the default settings when it starts.".into()),
        Some(Err(kind)) => return (Level::Fail, format!("config.toml cannot be read ({kind}).")),
        Some(Ok(text)) => text,
    };
    match config::parse_located(&text) {
        Ok(cfg) => (Level::Ok, format!("config.toml is valid ({} channels).", cfg.channels.len())),
        Err(config::Problem { at: Some((line, column)), reason }) => {
            (Level::Fail, format!("config.toml, line {line}, column {column}: {}", redact(&reason)))
        }
        Err(config::Problem { at: None, reason }) => (Level::Fail, format!("config.toml: {}", redact(&reason))),
    }
}

/// The reason without the user's own words: text in double quotes (the user's values), and a
/// channel's name, become "...". Text in backticks is the TOML reader's own (`]]`, a key name).
fn redact(reason: &str) -> String {
    let mut out = String::new();
    let mut quote = None;
    for c in reason.chars() {
        match quote {
            Some(q) if c == q => {
                out.push_str("...\"");
                quote = None;
            }
            Some(_) => {}
            None if c == '"' => {
                out.push('"');
                quote = Some(c);
            }
            None => out.push(c),
        }
    }
    if quote.is_some() {
        out.push_str("...\"");
    }
    // "channel Game: needs apps = [...]" names the channel without quotes.
    if let Some(rest) = out.strip_prefix("channel ")
        && let Some((_, why)) = rest.split_once(": ")
    {
        out = format!("a channel: {why}");
    }
    out
}

fn autostart_line(present: bool, custom_dir: bool) -> Line {
    let state = if present { "Start with Windows is on." } else { "Start with Windows is off." };
    let note = if custom_dir { " This copy does not change it (KB16_MIXER_DIR is set)." } else { "" };
    (Level::Ok, format!("{state}{note}"))
}

/// `audio` is the census, or the HRESULT of a Core Audio that does not answer.
fn audio_line(audio: Result<Census, i32>) -> Line {
    let c = match audio {
        Ok(c) => c,
        Err(code) => return (Level::Fail, format!("Core Audio does not answer (error 0x{code:08X}).")),
    };
    if c.outputs == 0 {
        return (Level::Fail, "No audio output device is active.".into());
    }
    if !c.default_output && c.output != OutputState::Chosen {
        return (Level::Warn, "No default output device is set.".into());
    }
    let mic = if c.mic { "a microphone" } else { "no microphone" };
    let (level, output) = match c.output {
        OutputState::Default => (Level::Ok, "Output: the Windows default."),
        OutputState::Chosen => (Level::Ok, "Output: the device chosen in the settings."),
        OutputState::ChosenMissing => {
            (Level::Warn, "The output device chosen in the settings is not active; the Windows default is used.")
        }
    };
    let muted = if c.output_muted { " That output device is muted." } else { "" };
    (level, format!("{} output device(s), {} app session(s), {mic}. {output}{muted}", c.outputs, c.sessions))
}

/// The report text, one line per topic, and the exit code.
fn report(lines: &[(&str, Line)]) -> (String, i32) {
    let mut text = String::new();
    for (topic, (level, message)) in lines {
        let level = match level {
            Level::Ok => "OK",
            Level::Warn => "WARN",
            Level::Fail => "FAIL",
        };
        text.push_str(&format!("{topic:<9}  {level:<4}  {message}\n"));
    }
    let problems = lines.iter().filter(|(_, (l, _))| *l != Level::Ok).count();
    text.push_str(&match problems {
        0 => "Result: OK\n".to_string(),
        1 => "Result: 1 problem\n".to_string(),
        n => format!("Result: {n} problems\n"),
    });
    let failed = lines.iter().any(|(_, (l, _))| *l == Level::Fail);
    (text, i32::from(failed))
}

/// Writes what the engine sees into selftest.txt (sessions, channels, live levels). Unlike
/// `--check` it names the programs, so it is for the user's own eyes first.
pub fn selftest() {
    let mut out = Vec::new();
    match (Mixer::new(), config::load()) {
        (Ok(mut mixer), Ok(cfg)) => {
            mixer.moved = config::load_apps();
            mixer.reset(&cfg.channels, &config::load_state().levels);
            mixer.refresh(&cfg.channels, true);
            out.push("sessions:".to_string());
            out.extend(mixer.describe());
            let mut peak = [0u8; 4];
            for _ in 0..30 {
                for (max, l) in peak.iter_mut().zip(mixer.levels(&cfg.channels).0) {
                    *max = (*max).max(l);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            for (i, c) in cfg.channels.iter().enumerate() {
                let r = mixer.channel_readout(i, &cfg.channels);
                out.push(format!(
                    "channel {} {:<6} live {} vol {} muted {} peak {}",
                    i + 1,
                    c.name,
                    r.live,
                    r.volume,
                    r.muted,
                    peak[i]
                ));
            }
        }
        (Err(e), _) => out.push(format!("audio init failed: {e}")),
        (_, Err(e)) => out.push(format!("config problem: {e}")),
    }
    let _ = std::fs::write(config::dir().join("selftest.txt"), out.join("\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours(release: u16) -> Hid {
        Hid {
            vendor_id: protocol::VID,
            product_id: protocol::PID,
            usage_page: protocol::USAGE_PAGE,
            product: Some(protocol::PRODUCT.into()),
            release,
        }
    }

    fn stock(product: &str) -> Hid {
        Hid { product: Some(product.into()), ..ours(0x0100) }
    }

    /// The other interfaces of a pad (keyboard, media keys): never counted as a second pad.
    fn keyboard_of(pad: &Hid) -> Hid {
        Hid { usage_page: 0x01, ..pad.clone() }
    }

    #[test]
    fn the_pad_line_tells_the_five_cases_apart() {
        let pad = ours(protocol::device_release("0.1.0"));
        assert_eq!(
            pad_line(&[keyboard_of(&pad), pad.clone()], 0),
            (Level::Ok, "KB16 Mixer, firmware 0.1.0 (USB D010:1601)".into())
        );
        assert_eq!(
            pad_line(&[ours(protocol::UNVERSIONED_RELEASE)], 0).1,
            "KB16 Mixer, firmware unknown (an early build) (USB D010:1601)"
        );
        assert_eq!(
            pad_line(&[], 1),
            (Level::Fail, "The pad is in bootloader mode (USB 1EAF:0003). Unplug it and plug it in again.".into())
        );
        assert_eq!(pad_line(&[], 0), (Level::Fail, "No pad found. Check the USB cable and port.".into()));
        let other = stock("KB16-01\u{7}\u{e9}");
        assert_eq!(
            pad_line(&[keyboard_of(&other), other.clone()], 0),
            (
                Level::Warn,
                "A pad with other firmware is connected (USB D010:1601, product \"KB16-01\"). \
                 This app needs the KB16 Mixer firmware. See docs/install.md."
                    .into()
            )
        );
        assert_eq!(pad_line(&[keyboard_of(&other)], 0).0, Level::Warn, "other firmware without a raw interface");
        let (level, text) = pad_line(&[pad.clone(), other], 1);
        assert_eq!(level, Level::Warn);
        assert!(
            text.starts_with("3 pads are connected (1 in app mode, 1 with other firmware, 1 in bootloader"),
            "{text}"
        );
        assert!(pad_line(&[pad.clone(), pad], 0).1.starts_with("2 pads"));
        let mouse = Hid { vendor_id: 0x046D, product_id: 0xC077, usage_page: 1, ..Hid::default() };
        assert_eq!(pad_line(&[mouse], 0).0, Level::Fail, "other devices do not count");
    }

    #[test]
    fn only_the_bootloader_id_counts() {
        let list: Vec<u16> =
            "USB\\VID_1EAF&PID_0003\\8D6F23A5\0usb\\vid_1eaf&pid_0003\\X\0USB\\VID_D010&PID_1601\\Y\0\0"
                .encode_utf16()
                .collect();
        assert_eq!(count_bootloaders(&list), 2);
        assert_eq!(count_bootloaders(&[0]), 0);
    }

    #[test]
    fn the_settings_line_never_quotes_the_file() {
        assert_eq!(settings_line(None).0, Level::Ok, "first start: no file yet");
        let read = |text: &str| settings_line(Some(Ok(text.into())));
        let valid = read("[[channel]]\nname = \"A\"\ntarget = \"other\"");
        assert_eq!(valid, (Level::Ok, "config.toml is valid (1 channels).".into()));
        let (level, text) = read(
            "[[channel]]\nname = \"A\"\ntarget = \"other\"\n[keys]\nrow1 = [\"open:C:\\\\Users\\\\Someone\\\\x.exe\"",
        );
        assert_eq!(level, Level::Fail);
        assert!(text.starts_with("config.toml, line 5, column "), "{text}");
        assert!(!text.contains("Someone"), "{text}");
        let (_, text) = read(
            "[keys]\nrow1 = [\"open:C:\\\\Users\\\\Someone\\\\x.exe\"]\n[[channel]]\nname = \"A\"\ntarget = \"other\"",
        );
        assert_eq!(text, "config.toml: [keys] row1: 1 entries, a row has 4 keys");
        let (_, text) = read("name = \"DESK PC 12345\"\n[[channel]]\nname = \"A\"\ntarget = \"other\"");
        assert!(text.starts_with("config.toml: name = \"...\": ") && !text.contains("DESK"), "{text}");
        let (_, text) = read("[[channel]]\nname = \"Desk\"");
        assert_eq!(text, "config.toml: a channel: needs apps = [...] or a target", "the name is left out");
        let unreadable = settings_line(Some(Err(std::io::ErrorKind::PermissionDenied)));
        assert_eq!(unreadable, (Level::Fail, "config.toml cannot be read (permission denied).".into()));
    }

    #[test]
    fn quoted_words_are_left_out() {
        assert_eq!(redact("key \"open:x\": not an action"), "key \"...\": not an action");
        assert_eq!(redact("unclosed array table, expected `]]`"), "unclosed array table, expected `]]`");
        assert_eq!(redact("cut \"short"), "cut \"...\"");
    }

    #[test]
    fn the_audio_and_autostart_lines() {
        let census = Census { outputs: 2, default_output: true, sessions: 5, mic: true, ..Census::default() };
        assert_eq!(
            audio_line(Ok(census)),
            (Level::Ok, "2 output device(s), 5 app session(s), a microphone. Output: the Windows default.".into())
        );
        let muted = Census { output_muted: true, mic: false, ..census };
        assert!(
            audio_line(Ok(muted))
                .1
                .ends_with("no microphone. Output: the Windows default. That output device is muted.")
        );
        assert_eq!(audio_line(Ok(Census { outputs: 0, ..census })).0, Level::Fail);
        assert_eq!(audio_line(Ok(Census { default_output: false, ..census })).0, Level::Warn);
        let chosen = Census { output: OutputState::Chosen, default_output: false, ..census };
        assert_eq!(
            audio_line(Ok(chosen)),
            (
                Level::Ok,
                "2 output device(s), 5 app session(s), a microphone. Output: the device chosen in the settings.".into()
            ),
            "no default is needed then"
        );
        let gone = audio_line(Ok(Census { output: OutputState::ChosenMissing, ..census }));
        assert_eq!(gone.0, Level::Warn);
        assert!(
            gone.1.ends_with("The output device chosen in the settings is not active; the Windows default is used.")
        );
        assert_eq!(audio_line(Err(0x8007_0005_u32 as i32)).1, "Core Audio does not answer (error 0x80070005).");
        assert_eq!(autostart_line(true, false), (Level::Ok, "Start with Windows is on.".into()));
        assert!(autostart_line(false, true).1.contains("KB16_MIXER_DIR"));
    }

    #[test]
    fn the_report_counts_problems_and_fails_only_on_fail() {
        let ok = |m: &str| (Level::Ok, m.to_string());
        let (text, code) = report(&[("app", ok("a")), ("autostart", ok("b"))]);
        assert_eq!(text, "app        OK    a\nautostart  OK    b\nResult: OK\n");
        assert_eq!(code, 0);
        let (text, code) = report(&[("pad", (Level::Warn, "w".into())), ("audio", ok("c"))]);
        assert!(text.contains("pad        WARN  w\n") && text.ends_with("Result: 1 problem\n"), "{text}");
        assert_eq!(code, 0, "a warning alone does not fail");
        let (text, code) = report(&[("pad", (Level::Fail, "f".into())), ("audio", (Level::Warn, "w".into()))]);
        assert!(text.ends_with("Result: 2 problems\n"));
        assert_eq!(code, 1);
    }
}
