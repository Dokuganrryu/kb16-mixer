//! The settings window's small modal dialogs: "press the keys you want", and a line of text with
//! a Browse button. Built from in-memory dialog templates, so Windows does the modality, the
//! keyboard navigation and the scaling to the display.
//!
//! The shortcut box reads keys only from its own window messages while it has the focus
//! (WM_KEYDOWN / WM_SYSKEYDOWN and their key-up partners). Nothing here observes the keyboard
//! anywhere else: no hooks, no raw input, no polling of key state.

use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_WINDOW, COLOR_WINDOWTEXT, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DrawFocusRect, DrawTextW,
    EndPaint, FillRect, FrameRect, GetSysColor, GetSysColorBrush, HFONT, InvalidateRect, PAINTSTRUCT, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

// ------------------------------------------------------------------ shortcuts, as data

/// Modifier bits, the same as the pad's (and `config::parse_key`'s).
pub const CTRL: u8 = 0x01;
pub const SHIFT: u8 = 0x02;
pub const ALT: u8 = 0x04;
pub const WIN: u8 = 0x08;
/// Shown and written in this order: Win+Ctrl+Alt+Shift+key.
const MODS: [(u8, &str, &str); 4] =
    [(WIN, "Win", "win"), (CTRL, "Ctrl", "ctrl"), (ALT, "Alt", "alt"), (SHIFT, "Shift", "shift")];

/// Which modifier a virtual-key code is (generic and left/right codes alike).
fn modifier(vk: u32) -> Option<u8> {
    Some(match vk {
        0x10 | 0xA0 | 0xA1 => SHIFT,
        0x11 | 0xA2 | 0xA3 => CTRL,
        0x12 | 0xA4 | 0xA5 => ALT,
        0x5B | 0x5C => WIN,
        _ => return None,
    })
}

/// Virtual keys the pad can type, with their config names (what `config::parse_key` reads).
const VK_NAMES: &[(u32, &str)] = &[
    (0x0D, "enter"),
    (0x1B, "esc"),
    (0x08, "backspace"),
    (0x09, "tab"),
    (0x20, "space"),
    (0x2C, "printscreen"),
    (0x2D, "insert"),
    (0x24, "home"),
    (0x21, "pageup"),
    (0x2E, "delete"),
    (0x23, "end"),
    (0x22, "pagedown"),
    (0x27, "right"),
    (0x25, "left"),
    (0x28, "down"),
    (0x26, "up"),
];

/// The config name of a virtual key, if the pad can type it.
pub fn vk_name(vk: u32) -> Option<String> {
    Some(match vk {
        0x41..=0x5A => ((vk as u8 - 0x41 + b'a') as char).to_string(),
        0x30..=0x39 => ((vk as u8 - 0x30 + b'0') as char).to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x70 + 1),
        _ => VK_NAMES.iter().find(|(v, _)| *v == vk)?.1.to_string(),
    })
}

/// How a key name reads on screen: "m" -> "M", "pageup" -> "Page Up".
fn key_label(name: &str) -> String {
    let words = match name {
        "esc" => "Esc",
        "printscreen" => "Print Screen",
        "pageup" => "Page Up",
        "pagedown" => "Page Down",
        "backspace" => "Backspace",
        _ => "",
    };
    if !words.is_empty() {
        return words.into();
    }
    let mut chars = name.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// "Ctrl+Shift+M": a key with its modifiers, as shown.
pub fn shortcut_label(mods: u8, key: &str) -> String {
    let mut parts: Vec<String> = MODS.iter().filter(|m| mods & m.0 != 0).map(|m| m.1.to_string()).collect();
    parts.push(key_label(key));
    parts.join("+")
}

/// "ctrl+shift+m": the same, as written to config.toml.
pub fn shortcut_text(mods: u8, key: &str) -> String {
    let mut parts: Vec<&str> = MODS.iter().filter(|m| mods & m.0 != 0).map(|m| m.2).collect();
    parts.push(key);
    parts.join("+")
}

/// What the shortcut box has seen: modifiers held now, and the last full combination.
#[derive(Default)]
pub struct Capture {
    held: u8,
    combo: Option<(u8, String)>,
    /// Modifiers went down after everything was let go, and no key yet: show them as they are.
    pressing: bool,
    /// A key the pad cannot type was pressed (until the next key).
    refused: bool,
}

impl Capture {
    pub fn with(mods: u8, key: Option<String>) -> Capture {
        Capture { combo: key.map(|k| (mods, k)), ..Capture::default() }
    }

    pub fn down(&mut self, vk: u32) {
        if let Some(m) = modifier(vk) {
            if self.held == 0 {
                self.pressing = true;
            }
            self.held |= m;
            return;
        }
        match vk_name(vk) {
            Some(key) => {
                self.combo = Some((self.held, key));
                self.pressing = false;
                self.refused = false;
            }
            None => self.refused = true,
        }
    }

    pub fn up(&mut self, vk: u32) {
        match modifier(vk) {
            Some(m) => self.held &= !m,
            // Print Screen never sends a key-down to a window, only the key-up.
            None if vk == 0x2C => self.down(vk),
            None => {}
        }
        if self.held == 0 && self.combo.is_some() {
            self.pressing = false;
        }
    }

    /// The window lost the focus: whatever was held will not report its release here.
    pub fn let_go(&mut self) {
        self.held = 0;
        self.pressing = false;
    }

    pub fn holding(&self) -> bool {
        self.held != 0
    }

    /// What the box shows; `win` is the "add the Win key" tick.
    pub fn label(&self, win: bool) -> String {
        let extra = if win { WIN } else { 0 };
        if self.pressing || self.combo.is_none() {
            let held = self.held | extra;
            if held == 0 {
                return String::new();
            }
            let mut parts: Vec<&str> = MODS.iter().filter(|m| held & m.0 != 0).map(|m| m.1).collect();
            parts.push("...");
            return parts.join("+");
        }
        let (mods, key) = self.combo.as_ref().expect("checked above");
        shortcut_label(mods | extra, key)
    }

    /// The config text of the combination, if a key was pressed.
    pub fn text(&self, win: bool) -> Option<String> {
        let (mods, key) = self.combo.as_ref()?;
        Some(shortcut_text(mods | if win { WIN } else { 0 }, key))
    }

    pub fn refused(&self) -> bool {
        self.refused
    }
}

// ------------------------------------------------------------------ dialog templates

enum Class {
    Button,
    Edit,
    Static,
    Named(PCWSTR),
}

struct Item<'a> {
    class: Class,
    text: &'a str,
    id: u16,
    style: u32,
    rect: [i16; 4],
}

const DS_MODALFRAME: u32 = 0x80;
const DS_CENTER: u32 = 0x800;

/// A DLGTEMPLATE with its items, in dialog units, in a DWORD-aligned buffer.
fn template(title: &str, size: [i16; 2], items: &[Item]) -> Vec<u32> {
    let mut b: Vec<u16> = Vec::new();
    let dword = |b: &mut Vec<u16>, v: u32| b.extend([v as u16, (v >> 16) as u16]);
    let text = |b: &mut Vec<u16>, s: &str| b.extend(s.encode_utf16().chain([0]));
    let style = DS_SETFONT as u32 | DS_MODALFRAME | DS_CENTER | WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0;
    dword(&mut b, style);
    dword(&mut b, 0);
    b.push(items.len() as u16);
    b.extend([0, 0, size[0] as u16, size[1] as u16]);
    b.extend([0, 0]); // no menu, default class
    text(&mut b, title);
    b.push(9);
    text(&mut b, "Segoe UI");
    for item in items {
        if b.len() % 2 == 1 {
            b.push(0);
        }
        dword(&mut b, item.style | WS_CHILD.0 | WS_VISIBLE.0);
        dword(&mut b, 0);
        b.extend(item.rect.map(|v| v as u16));
        b.push(item.id);
        match item.class {
            Class::Button => b.extend([0xFFFF, 0x0080]),
            Class::Edit => b.extend([0xFFFF, 0x0081]),
            Class::Static => b.extend([0xFFFF, 0x0082]),
            Class::Named(name) => b.extend(unsafe { name.as_wide() }.iter().copied().chain([0])),
        }
        text(&mut b, item.text);
        b.push(0);
    }
    if b.len() % 2 == 1 {
        b.push(0);
    }
    b.chunks(2).map(|p| p[0] as u32 | (p[1] as u32) << 16).collect()
}

fn run(owner: HWND, template: &[u32], proc: DLGPROC) -> isize {
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        DialogBoxIndirectParamW(Some(instance), template.as_ptr().cast(), Some(owner), proc, LPARAM(0))
    }
}

fn item_text(dlg: HWND, id: i32) -> String {
    unsafe {
        let Ok(h) = GetDlgItem(Some(dlg), id) else { return String::new() };
        let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

fn set_item_text(dlg: HWND, id: i32, text: &str) {
    unsafe {
        let _ = SetDlgItemTextW(dlg, id, &HSTRING::from(text));
    }
}

fn send(h: HWND, msg: u32, w: usize, l: isize) -> isize {
    unsafe { SendMessageW(h, msg, Some(WPARAM(w)), Some(LPARAM(l))).0 }
}

const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const EM_SETSEL: u32 = 0x00B1;
const ID_FIELD: u16 = 10;
const ID_EXTRA: u16 = 11;
const ID_HINT: u16 = 12;
const ID_CLEAR: u16 = 13;

// ------------------------------------------------------------------ the shortcut dialog

pub enum Shortcut {
    Set(String),
    Clear,
    Cancel,
}

thread_local! {
    static CAPTURE: RefCell<Capture> = RefCell::new(Capture::default());
    static CAPTURE_FONT: Cell<HFONT> = const { Cell::new(HFONT(std::ptr::null_mut())) };
    static CAPTURE_FOCUSED: Cell<bool> = const { Cell::new(false) };
    static TEXT_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
    static TEXT_INITIAL: RefCell<String> = const { RefCell::new(String::new()) };
    static TEXT_PROGRAM: Cell<bool> = const { Cell::new(false) };
}

const CAPTURE_CLASS: PCWSTR = w!("kb16-mixer-keys");

fn win_ticked(dlg: HWND) -> bool {
    unsafe { GetDlgItem(Some(dlg), ID_EXTRA as i32).is_ok_and(|h| send(h, BM_GETCHECK, 0, 0) == 1) }
}

/// Repaints the box and says in the hint line what the combination needs, if anything.
fn capture_changed(dlg: HWND) {
    unsafe {
        if let Ok(h) = GetDlgItem(Some(dlg), ID_FIELD as i32) {
            let _ = InvalidateRect(Some(h), None, true);
        }
    }
    let hint = CAPTURE.with_borrow(|c| {
        if c.refused() {
            "The pad cannot type that key. Letters, digits, F1 to F24, Enter, arrows and the like work."
        } else if c.text(false).is_none() {
            "Press a key, alone or together with Ctrl, Shift, Alt or Win."
        } else {
            ""
        }
    });
    set_item_text(dlg, ID_HINT as i32, hint);
}

unsafe extern "system" fn capture_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        let dlg = GetParent(hwnd).unwrap_or_default();
        match msg {
            WM_GETDLGCODE => {
                // A plain Tab still moves on to the buttons, so the dialog works without a mouse.
                let m = lparam.0 as *const MSG;
                let plain_tab = !m.is_null()
                    && (*m).message == WM_KEYDOWN
                    && (*m).wParam.0 == 0x09
                    && !CAPTURE.with_borrow(Capture::holding);
                LRESULT(if plain_tab { 0 } else { DLGC_WANTALLKEYS as isize })
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                CAPTURE.with_borrow_mut(|c| c.down(wparam.0 as u32));
                capture_changed(dlg);
                LRESULT(0)
            }
            WM_KEYUP | WM_SYSKEYUP => {
                CAPTURE.with_borrow_mut(|c| c.up(wparam.0 as u32));
                capture_changed(dlg);
                LRESULT(0)
            }
            // Swallowed so Alt+letter does not beep and F10 / Alt do not open the window menu.
            WM_CHAR | WM_SYSCHAR | WM_DEADCHAR | WM_SYSDEADCHAR => LRESULT(0),
            WM_SETFOCUS | WM_KILLFOCUS => {
                CAPTURE_FOCUSED.set(msg == WM_SETFOCUS);
                if msg == WM_KILLFOCUS {
                    CAPTURE.with_borrow_mut(Capture::let_go);
                }
                capture_changed(dlg);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                send(dlg, WM_NEXTDLGCTL, hwnd.0 as usize, 1);
                LRESULT(0)
            }
            WM_SETFONT => {
                CAPTURE_FONT.set(HFONT(wparam.0 as *mut _));
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                FillRect(dc, &rect, GetSysColorBrush(COLOR_WINDOW));
                FrameRect(dc, &rect, GetSysColorBrush(COLOR_WINDOWTEXT));
                let old = SelectObject(dc, CAPTURE_FONT.get().into());
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(GetSysColor(COLOR_WINDOWTEXT)));
                let label = CAPTURE.with_borrow(|c| c.label(win_ticked(dlg)));
                let shown = if label.is_empty() { "(click here, then press the keys)".to_string() } else { label };
                let mut text: Vec<u16> = shown.encode_utf16().collect();
                DrawTextW(dc, &mut text, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
                if CAPTURE_FOCUSED.get() {
                    let inner =
                        RECT { left: rect.left + 3, top: rect.top + 3, right: rect.right - 3, bottom: rect.bottom - 3 };
                    let _ = DrawFocusRect(dc, &inner);
                }
                SelectObject(dc, old);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

unsafe extern "system" fn shortcut_proc(dlg: HWND, msg: u32, wparam: WPARAM, _: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                if WIN_PREFILL.get() {
                    send(GetDlgItem(Some(dlg), ID_EXTRA as i32).unwrap_or_default(), BM_SETCHECK, 1, 0);
                }
                capture_changed(dlg);
                1 // focus goes to the first control: the box
            }
            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                match id {
                    1 => {
                        // OK: only once there is something to keep.
                        if let Some(text) = CAPTURE.with_borrow(|c| c.text(win_ticked(dlg))) {
                            TEXT_RESULT.set(Some(text));
                            let _ = EndDialog(dlg, 1);
                        } else {
                            capture_changed(dlg);
                        }
                    }
                    2 => {
                        let _ = EndDialog(dlg, 0);
                    }
                    _ if id == ID_CLEAR as i32 => {
                        let _ = EndDialog(dlg, 2);
                    }
                    _ if id == ID_EXTRA as i32 => capture_changed(dlg),
                    _ => {}
                }
                1
            }
            _ => 0,
        }
    }
}

/// "Press the keys you want" for one pad key. `current` prefills it (mods, key name).
pub fn shortcut(owner: HWND, current: Option<(u8, String)>) -> Shortcut {
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(capture_proc),
            hInstance: instance,
            lpszClassName: CAPTURE_CLASS,
            hCursor: LoadCursorW(None, IDC_IBEAM).unwrap_or_default(),
            ..Default::default()
        });
    }
    let (mods, key) = current.map_or((0, None), |(m, k)| (m, Some(k)));
    // The Win part of a prefilled shortcut lives in the tick box, like one made here.
    CAPTURE.set(Capture::with(mods & !WIN, key));
    TEXT_RESULT.set(None);
    let tick = BS_AUTOCHECKBOX as u32 | WS_TABSTOP.0;
    let items = [
        Item {
            class: Class::Static,
            text: "Press the keys you want, for example Ctrl+Shift+M.",
            id: 0xFFFF,
            style: 0,
            rect: [7, 7, 250, 10],
        },
        Item {
            class: Class::Named(CAPTURE_CLASS),
            text: "",
            id: ID_FIELD,
            style: WS_TABSTOP.0,
            rect: [7, 20, 250, 20],
        },
        Item { class: Class::Static, text: "", id: ID_HINT, style: 0, rect: [7, 44, 250, 18] },
        Item { class: Class::Button, text: "Add the Win key", id: ID_EXTRA, style: tick, rect: [7, 64, 250, 12] },
        Item {
            class: Class::Static,
            text: "Windows keeps some combinations for itself (Win+L, Ctrl+Alt+Del, often the Win key). \
                   Those can be typed into the config file instead.",
            id: 0xFFFF,
            style: 0,
            rect: [7, 80, 250, 20],
        },
        Item { class: Class::Button, text: "Clear", id: ID_CLEAR, style: WS_TABSTOP.0, rect: [7, 108, 56, 15] },
        Item {
            class: Class::Button,
            text: "OK",
            id: 1,
            style: BS_DEFPUSHBUTTON as u32 | WS_TABSTOP.0,
            rect: [141, 108, 56, 15],
        },
        Item { class: Class::Button, text: "Cancel", id: 2, style: WS_TABSTOP.0, rect: [201, 108, 56, 15] },
    ];
    let t = template("Shortcut", [264, 130], &items);
    let win = mods & WIN != 0;
    WIN_PREFILL.set(win);
    match run(owner, &t, Some(shortcut_proc)) {
        1 => TEXT_RESULT.take().map_or(Shortcut::Cancel, Shortcut::Set),
        2 => Shortcut::Clear,
        _ => Shortcut::Cancel,
    }
}

thread_local! {
    static WIN_PREFILL: Cell<bool> = const { Cell::new(false) };
}

// ------------------------------------------------------------------ a line of text

/// Turns what was typed or browsed for into a program name as Windows lists it: "spotify" and
/// "C:\Apps\Spotify.exe" both become "Spotify.exe". None if nothing usable is left.
pub fn program_name(text: &str) -> Option<String> {
    let name = text.trim().trim_matches('"').trim();
    let name = name.rsplit(['\\', '/']).next().unwrap_or("").trim();
    if name.is_empty() || name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        return None;
    }
    Some(if name.contains('.') { name.to_string() } else { format!("{name}.exe") })
}

fn browse(dlg: HWND) {
    let mut file = [0u16; 1024];
    let program = TEXT_PROGRAM.get();
    let filter = if program {
        "Programs\0*.exe\0\0"
    } else {
        "Programs and shortcuts\0*.exe;*.lnk;*.bat;*.cmd;*.url\0All files\0*.*\0\0"
    };
    let filter: Vec<u16> = filter.encode_utf16().collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: dlg,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        // Keep a shortcut as the shortcut, and never change the app's working folder.
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR | OFN_NODEREFERENCELINKS,
        ..Default::default()
    };
    if unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
        let path = String::from_utf16_lossy(&file[..len]);
        let shown = if program { program_name(&path).unwrap_or_default() } else { path };
        set_item_text(dlg, ID_FIELD as i32, &shown);
    }
}

unsafe extern "system" fn text_proc(dlg: HWND, msg: u32, wparam: WPARAM, _: LPARAM) -> isize {
    unsafe {
        match msg {
            WM_INITDIALOG => {
                set_item_text(dlg, ID_FIELD as i32, &TEXT_INITIAL.with_borrow(Clone::clone));
                if let Ok(field) = GetDlgItem(Some(dlg), ID_FIELD as i32) {
                    send(field, EM_SETSEL, 0, -1);
                }
                1
            }
            WM_COMMAND => {
                match (wparam.0 & 0xFFFF) as i32 {
                    1 => {
                        let text = item_text(dlg, ID_FIELD as i32);
                        let kept = if TEXT_PROGRAM.get() { program_name(&text) } else { Some(text.trim().to_string()) };
                        match kept.filter(|t| !t.is_empty()) {
                            Some(t) => {
                                TEXT_RESULT.set(Some(t));
                                let _ = EndDialog(dlg, 1);
                            }
                            None => set_item_text(dlg, ID_HINT as i32, "Type something first, or press Cancel."),
                        }
                    }
                    2 => {
                        let _ = EndDialog(dlg, 0);
                    }
                    id if id == ID_EXTRA as i32 => browse(dlg),
                    _ => {}
                }
                1
            }
            _ => 0,
        }
    }
}

fn ask_text(owner: HWND, title: &str, prompt: &str, hint: &str, initial: &str, program: bool) -> Option<String> {
    TEXT_INITIAL.set(initial.to_string());
    TEXT_PROGRAM.set(program);
    TEXT_RESULT.set(None);
    let tab = WS_TABSTOP.0;
    let items = [
        Item { class: Class::Static, text: prompt, id: 0xFFFF, style: 0, rect: [7, 7, 270, 10] },
        Item {
            class: Class::Edit,
            text: "",
            id: ID_FIELD,
            style: tab | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
            rect: [7, 20, 210, 14],
        },
        Item { class: Class::Button, text: "Browse...", id: ID_EXTRA, style: tab, rect: [221, 19, 56, 16] },
        Item { class: Class::Static, text: hint, id: ID_HINT, style: 0, rect: [7, 40, 270, 28] },
        Item { class: Class::Button, text: "OK", id: 1, style: tab | BS_DEFPUSHBUTTON as u32, rect: [161, 72, 56, 15] },
        Item { class: Class::Button, text: "Cancel", id: 2, style: tab, rect: [221, 72, 56, 15] },
    ];
    let t = template(title, [284, 94], &items);
    (run(owner, &t, Some(text_proc)) == 1).then(|| TEXT_RESULT.take()).flatten()
}

/// What an "open:" key should open.
pub fn open_target(owner: HWND, current: &str) -> Option<String> {
    ask_text(
        owner,
        "Open",
        "Program, folder, file or web address to open:",
        "For example C:\\Tools\\obs64.exe, %USERPROFILE%\\Downloads or https://example.com. \
         It opens the way a double-click would.",
        current,
        false,
    )
}

/// A program to add to a channel by name.
pub fn program(owner: HWND) -> Option<String> {
    ask_text(
        owner,
        "Add a program",
        "Program name, as Task Manager shows it under Details:",
        "For example Spotify.exe. It joins the channel the next time it makes sound.",
        "",
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{KeyAction, key_name, parse_key};

    #[test]
    fn a_pressed_combination_reads_and_saves_like_the_config() {
        let mut c = Capture::default();
        assert_eq!(c.label(false), "");
        c.down(0x11); // Ctrl
        assert_eq!(c.label(false), "Ctrl+...", "shown as it is pressed");
        c.down(0x10); // Shift
        assert_eq!(c.label(false), "Ctrl+Shift+...");
        c.down(0x4D); // M
        assert_eq!(c.label(false), "Ctrl+Shift+M");
        c.up(0x4D);
        c.up(0x10);
        c.up(0x11);
        assert_eq!(c.label(false), "Ctrl+Shift+M", "kept after letting go");
        assert_eq!(c.text(false).as_deref(), Some("ctrl+shift+m"));
        assert_eq!(c.text(true).as_deref(), Some("win+ctrl+shift+m"), "the Win tick");
        assert_eq!(c.label(true), "Win+Ctrl+Shift+M");

        c.down(0x12); // Alt starts a new one
        assert_eq!(c.label(false), "Alt+...");
        c.down(0x73); // F4: Alt+F4 is just a combination here
        assert_eq!(c.text(false).as_deref(), Some("alt+F4"));
        c.let_go();
        c.down(0x60); // number pad 0
        assert!(c.refused(), "the pad cannot type number-pad keys");
        assert_eq!(c.text(false).as_deref(), Some("alt+F4"), "the last good one stays");
        c.up(0x2C); // Print Screen only reports its release
        assert_eq!(c.text(false).as_deref(), Some("printscreen"));
        c.down(0xA3); // right Ctrl
        c.down(0x25);
        assert_eq!(c.label(false), "Ctrl+Left");
    }

    #[test]
    fn every_key_the_box_takes_parses_back_to_the_same_key() {
        for vk in (0x08..=0x87).filter(|vk| modifier(*vk).is_none()) {
            let Some(name) = vk_name(vk) else { continue };
            for mods in [0, CTRL, CTRL | SHIFT, ALT | WIN, CTRL | SHIFT | ALT | WIN] {
                let action = parse_key(&shortcut_text(mods, &name)).unwrap_or_else(|e| panic!("{name}: {e}"));
                let KeyAction::Keyboard { code, mods: m } = action else { panic!("{name} is not a key") };
                assert_eq!(m, mods);
                assert_eq!(key_name(code).as_deref(), Some(name.as_str()), "usage {code:#x} names {name}");
            }
        }
        assert_eq!(shortcut_label(SHIFT | WIN, "s"), "Win+Shift+S");
        assert_eq!(shortcut_label(0, "pageup"), "Page Up");
        assert_eq!(shortcut_label(CTRL, "F13"), "Ctrl+F13");
    }

    #[test]
    fn program_names_are_what_windows_lists() {
        assert_eq!(program_name(" spotify ").as_deref(), Some("spotify.exe"));
        assert_eq!(program_name("\"C:\\Apps\\Spotify.exe\"").as_deref(), Some("Spotify.exe"));
        assert_eq!(program_name("D:/Games/game.bin").as_deref(), Some("game.bin"));
        assert_eq!(program_name("   "), None);
        assert_eq!(program_name("C:\\Apps\\"), None);
        assert_eq!(program_name("a|b"), None);
    }
}
