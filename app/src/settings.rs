//! The settings window: the pad as a grid of dropdowns. Plain Win32 controls, created when
//! opened and destroyed when closed, so it costs nothing the rest of the time.
//!
//! Every change is written into config.toml (comments and layout preserved) and then loaded
//! through the same path as "Reload config", so the file stays the single source of truth.

use crate::config::{self, Config, KeyAction, KnobBinding};
use std::cell::Cell;
use toml_edit::{Array, DocumentMut, Item, Table, value};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{CreateFontIndirectW, GetSysColorBrush, HFONT};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, w};

// Control ids.
const ID_KEY: i32 = 100; // + row * 4 + col
const ID_KNOB: i32 = 120; // + knob
const ID_BRIGHTNESS: i32 = 130;
const ID_STEP: i32 = 131;
const ID_DUCK: i32 = 132;
const ID_SCREEN_OFF: i32 = 133;
const ID_LIGHTS_DIM: i32 = 134;
const ID_SHIFT: i32 = 135;
const ID_AUTOSTART: i32 = 140;
const ID_OPEN_FILE: i32 = 150;
const ID_CLOSE: i32 = 151;
const ID_STATUS: i32 = 160;

// Win32 message numbers the `windows` crate keeps in other modules.
const CB_ADDSTRING: u32 = 0x0143;
const CB_GETCURSEL: u32 = 0x0147;
const CB_SETCURSEL: u32 = 0x014E;
const CBN_SELCHANGE: u32 = 1;
const TBM_GETPOS: u32 = WM_USER;
const TBM_SETPOS: u32 = WM_USER + 5;
const TBM_SETRANGE: u32 = WM_USER + 6;
const TB_ENDTRACK: u32 = 8;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const SS_CENTER: u32 = 1;

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

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static FONT: Cell<HFONT> = const { Cell::new(HFONT(std::ptr::null_mut())) };
}

/// The open settings window, if any (for the message loop's keyboard navigation).
pub fn window() -> Option<HWND> {
    let h = WINDOW.get();
    (!h.is_invalid()).then_some(h)
}

// ------------------------------------------------------------------ writing config.toml

fn edit(change: impl FnOnce(&mut DocumentMut)) -> Result<(), String> {
    let path = config::path();
    let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read config.toml: {e}"))?;
    let mut doc: DocumentMut =
        text.parse().map_err(|_| "config.toml has a syntax error; fix it first (Open config file)".to_string())?;
    change(&mut doc);
    std::fs::write(&path, doc.to_string()).map_err(|e| format!("cannot write config.toml: {e}"))
}

fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Item {
    let item = &mut doc[name];
    if !item.is_table() {
        *item = Item::Table(Table::new());
    }
    item
}

fn set_key(row: usize, col: usize, text: &str) -> Result<(), String> {
    edit(|doc| {
        let name = format!("row{}", row + 1);
        let keys = table(doc, "keys");
        // A missing or malformed row is rebuilt from the defaults before the one key is set.
        let complete = keys.get(&name).and_then(Item::as_array).is_some_and(|a| a.len() == 4);
        if !complete {
            keys[&name] = value(Array::from_iter(config::DEFAULT_KEYS[0][row]));
        }
        if let Some(cells) = keys[&name].as_array_mut() {
            cells.replace(col, text);
        }
    })
}

fn set_knob(knob: usize, text: &str) -> Result<(), String> {
    edit(|doc| table(doc, "knobs")[&format!("knob{}", knob + 1)] = value(text))
}

fn set_number(name: &str, v: f64) -> Result<(), String> {
    edit(|doc| doc[name] = if v.fract() == 0.0 { value(v as i64) } else { value(v) })
}

// ------------------------------------------------------------------ the window

fn scale(v: i32) -> i32 {
    v * unsafe { GetDpiForSystem() } as i32 / 96
}

fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

fn child(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

#[allow(clippy::too_many_arguments)]
fn control(parent: HWND, class: PCWSTR, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: i32) -> HWND {
    unsafe {
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            &HSTRING::from(text),
            WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
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
        hwnd
    }
}

fn label(parent: HWND, text: &str, x: i32, y: i32, w: i32) {
    control(parent, w!("STATIC"), text, 0, x, y + 3, w, 18, -1);
}

fn heading(parent: HWND, text: &str, y: i32) {
    control(parent, w!("STATIC"), text, 0, 16, y, 560, 18, -1);
}

fn combo(parent: HWND, id: i32, x: i32, y: i32, w: i32, items: impl Iterator<Item = String>, selected: Option<usize>) {
    let h =
        control(parent, w!("COMBOBOX"), "", (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0 | WS_TABSTOP.0, x, y, w, 400, id);
    for item in items {
        let text = HSTRING::from(item);
        send(h, CB_ADDSTRING, 0, text.as_ptr() as isize);
    }
    if let Some(i) = selected {
        send(h, CB_SETCURSEL, i, 0);
    }
}

fn selection(parent: HWND, id: i32) -> Option<usize> {
    usize::try_from(send(child(parent, id), CB_GETCURSEL, 0, 0)).ok()
}

/// Choices for a key: the standard list, plus the current text if it is something custom
/// (a key combination typed into the file), so opening the window never loses it.
fn key_choices(current: KeyAction, raw: &str) -> (Vec<(String, String)>, usize) {
    let mut list: Vec<(String, String)> = KEY_CHOICES.iter().map(|(l, v)| (l.to_string(), v.to_string())).collect();
    let at = list.iter().position(|(_, v)| config::parse_key(v) == Ok(current)).unwrap_or_else(|| {
        list.push((raw.to_string(), raw.to_string()));
        list.len() - 1
    });
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

fn nearest<T: Copy>(options: &[(&str, T)], distance: impl Fn(T) -> f64) -> usize {
    (0..options.len()).min_by(|&a, &b| distance(options[a].1).total_cmp(&distance(options[b].1))).unwrap_or(0)
}

fn build(hwnd: HWND, cfg: &Config) {
    let raw_keys = config::raw_keys();

    heading(hwnd, "Keys.  Each column belongs to the channel named above it; its lights are that channel's meter.", 14);
    for (c, ch) in cfg.channels.iter().take(4).enumerate() {
        control(hwnd, w!("STATIC"), &ch.name, SS_CENTER, 116 + c as i32 * 118, 40, 112, 18, -1);
    }
    for r in 0..4 {
        label(hwnd, ROW_NAMES[r], 16, 62 + r as i32 * 30, 96);
        for c in 0..4 {
            let (list, at) = key_choices(cfg.pages[0][r][c], &raw_keys[r][c]);
            combo(
                hwnd,
                ID_KEY + (r * 4 + c) as i32,
                116 + c as i32 * 118,
                62 + r as i32 * 30,
                112,
                list.into_iter().map(|(l, _)| l),
                Some(at),
            );
        }
    }

    heading(hwnd, "Knobs.  Turn for volume; press to turn one app in the channel on its own.", 196);
    for (i, name) in ["Small, left", "Small, right", "Big"].iter().enumerate() {
        let x = 16 + i as i32 * 190;
        label(hwnd, name, x, 218, 180);
        combo(
            hwnd,
            ID_KNOB + i as i32,
            x,
            240,
            180,
            knob_choices(cfg).into_iter().map(|(l, _)| l),
            Some(knob_index(cfg, cfg.knobs[i])),
        );
    }

    heading(hwnd, "Feel", 284);
    label(hwnd, "Light brightness", 16, 308, 100);
    let bar = control(hwnd, w!("msctls_trackbar32"), "", WS_TABSTOP.0, 112, 306, 230, 28, ID_BRIGHTNESS);
    send(bar, TBM_SETRANGE, 1, (100 << 16) | 10);
    send(bar, TBM_SETPOS, 1, (cfg.brightness as isize * 100 + 127) / 255);
    label(hwnd, "Knob step", 360, 308, 76);
    combo(
        hwnd,
        ID_STEP,
        440,
        306,
        136,
        STEPS.iter().map(|s| s.0.into()),
        Some(nearest(STEPS, |v| (v - cfg.step as f64).abs())),
    );
    label(hwnd, "Focus turns the others down to", 16, 342, 200);
    combo(
        hwnd,
        ID_DUCK,
        206,
        340,
        136,
        DUCKS.iter().map(|s| s.0.into()),
        Some(nearest(DUCKS, |v| (v - cfg.duck as f64).abs())),
    );
    label(hwnd, "Screen shift", 360, 342, 76);
    combo(hwnd, ID_SHIFT, 440, 340, 136, (0..=6).map(|n| format!("{n} px down")), Some(cfg.oled_top as usize));

    heading(hwnd, "When nothing is playing and nothing is touched", 380);
    label(hwnd, "Screen off after", 16, 404, 100);
    let at = |minutes: u8| {
        nearest(MINUTES, |v| if (v == 0) != (minutes == 0) { 1e9 } else { (v as f64 - minutes as f64).abs() })
    };
    combo(hwnd, ID_SCREEN_OFF, 112, 402, 120, MINUTES.iter().map(|s| s.0.into()), Some(at(cfg.screen_off_min)));
    label(hwnd, "Dim lights after", 260, 404, 100);
    combo(hwnd, ID_LIGHTS_DIM, 356, 402, 120, MINUTES.iter().map(|s| s.0.into()), Some(at(cfg.lights_dim_min)));

    let auto = control(
        hwnd,
        w!("BUTTON"),
        "Start with Windows",
        (BS_AUTOCHECKBOX as u32) | WS_TABSTOP.0,
        16,
        446,
        180,
        22,
        ID_AUTOSTART,
    );
    send(auto, BM_SETCHECK, crate::autostart_enabled() as usize, 0);
    control(hwnd, w!("BUTTON"), "Open config file", WS_TABSTOP.0, 316, 444, 126, 26, ID_OPEN_FILE);
    control(hwnd, w!("BUTTON"), "Close", WS_TABSTOP.0, 450, 444, 126, 26, ID_CLOSE);
    control(
        hwnd,
        w!("STATIC"),
        "Changes apply the moment you pick them and are saved to config.toml.",
        0,
        16,
        482,
        560,
        18,
        ID_STATUS,
    );
}

/// Applies one control's new value. Returns what to show in the status line.
fn changed(hwnd: HWND, id: i32) -> Result<(), String> {
    let cfg = crate::current_config();
    let pick = selection(hwnd, id);
    match (id, pick) {
        (ID_KEY..=115, Some(i)) => {
            let (r, c) = (((id - ID_KEY) / 4) as usize, ((id - ID_KEY) % 4) as usize);
            let (list, _) = key_choices(cfg.pages[0][r][c], &config::raw_keys()[r][c]);
            let Some((_, text)) = list.get(i) else { return Ok(()) };
            set_key(r, c, text)?;
        }
        (ID_KNOB..=122, Some(i)) => {
            let Some((_, text)) = knob_choices(&cfg).into_iter().nth(i) else { return Ok(()) };
            set_knob((id - ID_KNOB) as usize, &text)?;
        }
        (ID_STEP, Some(i)) => set_number("step", STEPS[i.min(STEPS.len() - 1)].1)?,
        (ID_DUCK, Some(i)) => set_number("duck", DUCKS[i.min(DUCKS.len() - 1)].1)?,
        (ID_SHIFT, Some(i)) => set_number("oled_top", i.min(6) as f64)?,
        (ID_SCREEN_OFF, Some(i)) => set_number("screen_off_min", MINUTES[i.min(MINUTES.len() - 1)].1 as f64)?,
        (ID_LIGHTS_DIM, Some(i)) => set_number("lights_dim_min", MINUTES[i.min(MINUTES.len() - 1)].1 as f64)?,
        (ID_BRIGHTNESS, _) => {
            let percent = send(child(hwnd, ID_BRIGHTNESS), TBM_GETPOS, 0, 0).clamp(10, 100);
            set_number("brightness", percent as f64 / 100.0)?;
        }
        _ => return Ok(()),
    }
    crate::reload()
}

fn report(hwnd: HWND, result: Result<(), String>) {
    let text = match result {
        Ok(()) => "Saved and applied.".to_string(),
        Err(e) => format!("Not applied: {e}"),
    };
    unsafe {
        let _ = SetWindowTextW(child(hwnd, ID_STATUS), &HSTRING::from(text));
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_COMMAND => {
                let (id, code) = ((wparam.0 & 0xFFFF) as i32, (wparam.0 >> 16) as u32);
                match id {
                    ID_CLOSE => {
                        let _ = DestroyWindow(hwnd);
                    }
                    ID_OPEN_FILE => {
                        let _ = std::process::Command::new("notepad.exe").arg(config::path()).spawn();
                    }
                    ID_AUTOSTART => crate::set_autostart(send(child(hwnd, ID_AUTOSTART), BM_GETCHECK, 0, 0) == 1),
                    _ if code == CBN_SELCHANGE => report(hwnd, changed(hwnd, id)),
                    _ => {}
                }
                LRESULT(0)
            }
            // The brightness slider, once the mouse or key is released.
            WM_HSCROLL if (wparam.0 & 0xFFFF) as u32 == TB_ENDTRACK => {
                report(hwnd, changed(hwnd, ID_BRIGHTNESS));
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                WINDOW.set(HWND::default());
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Opens the settings window, or brings it forward if it is already open.
pub fn open() {
    unsafe {
        if let Some(existing) = window() {
            let _ = SetForegroundWindow(existing);
            return;
        }
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        if FONT.get().is_invalid() {
            let _ = InitCommonControlsEx(&INITCOMMONCONTROLSEX {
                dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_BAR_CLASSES,
            });
            let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
            let _ = SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                metrics.cbSize,
                Some((&raw mut metrics).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            FONT.set(CreateFontIndirectW(&metrics.lfMessageFont));
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: instance,
                lpszClassName: w!("kb16-mixer-settings"),
                hbrBackground: GetSysColorBrush(windows::Win32::Graphics::Gdi::COLOR_BTNFACE),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
                ..Default::default()
            });
        }
        // Size the window so the client area is 592 x 512 at 96 dpi.
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT { left: 0, top: 0, right: scale(592), bottom: scale(512) };
        let _ = AdjustWindowRect(&mut rect, style, false);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
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
        build(hwnd, &crate::current_config());
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window's writes must leave the rest of the file exactly as the user had it.
    #[test]
    fn edits_keep_the_file_intact() {
        let dir = std::env::temp_dir().join(format!("kb16-settings-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("kb16-mixer")).unwrap();
        // SAFETY: the only test in this binary that touches the environment.
        unsafe { std::env::set_var("APPDATA", &dir) };
        std::fs::write(config::path(), config::DEFAULT).unwrap();

        set_key(0, 1, "ctrl+shift+d").unwrap();
        set_key(2, 3, "none").unwrap();
        set_knob(1, "master").unwrap();
        set_number("brightness", 0.4).unwrap();
        set_number("screen_off_min", 0.0).unwrap();
        set_number("lights_dim_min", 30.0).unwrap();

        let text = std::fs::read_to_string(config::path()).unwrap();
        let cfg = config::load().expect("still a valid config");
        assert_eq!(cfg.pages[0][0][1], KeyAction::Keyboard { code: 0x07, mods: 0x03 });
        assert_eq!(cfg.pages[0][2][3], KeyAction::Nothing);
        assert_eq!(cfg.pages[0][3], [KeyAction::Mute; 4], "untouched rows stay");
        assert_eq!(cfg.pages[1], config::defaults().pages[1], "the keypad page is left alone");
        assert!(text.contains("[keys2]\nrow1 = [\"page\",  \"7\", \"8\", \"9\"]"));
        assert!(cfg.knobs[1] == KnobBinding::Master);
        assert_eq!(cfg.brightness, 102);
        assert_eq!((cfg.screen_off_min, cfg.lights_dim_min), (0, 30));
        assert_eq!(config::raw_keys()[0][1], "ctrl+shift+d");
        // Comments and everything not edited survive.
        assert!(text.contains("# KNOBS. Turn = volume."));
        assert!(text.contains("knob1 = \"Chat\"        # small knob, top left"));
        assert!(text.contains("\"Discord.exe\""));
        assert_eq!(text.lines().count(), config::DEFAULT.lines().count(), "no lines added or lost");

        // A file with no [keys] or [knobs] at all gets them created.
        std::fs::write(config::path(), "[[channel]]\nname = \"Game\"\ntarget = \"other\"\n").unwrap();
        set_key(3, 0, "focus").unwrap();
        set_knob(2, "Game").unwrap();
        let cfg = config::load().expect("valid");
        assert_eq!(cfg.pages[0][3][0], KeyAction::Focus);
        assert_eq!(cfg.pages[0][3][1], KeyAction::Mute, "the rest of the row keeps its default");
        assert!(cfg.knobs[2] == KnobBinding::Channel(0));

        // A broken file is reported, not overwritten.
        std::fs::write(config::path(), "step = = 2").unwrap();
        assert!(set_knob(0, "master").unwrap_err().contains("syntax error"));
        assert_eq!(std::fs::read_to_string(config::path()).unwrap(), "step = = 2");
        let _ = std::fs::remove_dir_all(dir);
    }
}
