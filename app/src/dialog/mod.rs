//! The settings window's small modal dialogs: "press the keys you want" (`shortcut`, with the
//! key logic in `keys`), and a line of text with a Browse button (`text`). Built from in-memory
//! dialog templates, so Windows does the modality and the keyboard navigation; `look` gives them
//! the settings window's look and places their controls at its scale.

mod keys;
mod look;
mod shortcut;
mod text;

pub use keys::shortcut_label;
pub use shortcut::{Shortcut, shortcut};
pub use text::{open_target, program};

use crate::win;
use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateDialogIndirectParamW, DLGPROC, DS_SETFONT, DialogBoxIndirectParamW, EndDialog, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};
use windows::core::PCWSTR;

thread_local! {
    /// What the open dialog hands back when it ends with OK.
    static TEXT_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

enum Class {
    Button,
    Edit,
    Static,
    /// A class of ours, by its static name.
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
    let style =
        DS_SETFONT as u32 | DS_MODALFRAME | DS_CENTER | WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_CLIPCHILDREN.0;
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
            // SAFETY: the names come from `w!`: static and NUL-terminated.
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

/// Shows a dialog made with `template` and returns the code it ended with (-1 if it could not
/// open).
fn run(owner: HWND, template: &[u32], proc: DLGPROC) -> isize {
    // SAFETY: the template is a whole, DWORD-aligned DLGTEMPLATE that outlives the modal call,
    // and `proc` is one of this module's dialog procedures.
    unsafe { DialogBoxIndirectParamW(Some(win::instance()), template.as_ptr().cast(), Some(owner), proc, LPARAM(0)) }
}

/// A dialog the hidden renderer draws.
pub enum Which {
    /// Started from a shortcut (modifiers, key name).
    Shortcut(Option<(u8, String)>),
    Open(String),
    Program,
}

/// Makes a dialog modeless and hidden (its template has no WS_VISIBLE), for a picture of it.
pub fn hidden(which: Which) -> Option<HWND> {
    let (t, proc) = match which {
        Which::Shortcut(current) => (shortcut::prepare(current), shortcut::PROC),
        Which::Open(now) => (text::prepare_open(&now), text::PROC),
        Which::Program => (text::prepare_program(), text::PROC),
    };
    // SAFETY: as in `run`; the template is copied when the dialog is made.
    unsafe { CreateDialogIndirectParamW(Some(win::instance()), t.as_ptr().cast(), None, proc, LPARAM(0)).ok() }
}

/// Closes a dialog of ours with `code`, which `run` then returns.
fn end(dlg: HWND, code: isize) {
    // SAFETY: called from the dialog's own procedure, on its thread, as EndDialog requires.
    unsafe {
        let _ = EndDialog(dlg, code);
    }
}

fn item_text(dlg: HWND, id: i32) -> String {
    win::dlg_item(Some(dlg), id).map(win::window_text).unwrap_or_default()
}

fn set_item_text(dlg: HWND, id: i32, text: &str) {
    if let Some(h) = win::dlg_item(Some(dlg), id) {
        win::set_window_text(h, text);
    }
}

const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const EM_SETSEL: u32 = 0x00B1;
const ID_FIELD: u16 = 10;
const ID_EXTRA: u16 = 11;
const ID_HINT: u16 = 12;
const ID_CLEAR: u16 = 13;
const ID_PROMPT: u16 = 14;
const ID_NOTE: u16 = 15;
const SS_OWNERDRAW: u32 = 0x000D;
const BS_OWNERDRAW: u32 = 0x000B;
const BS_AUTOCHECKBOX: u32 = 0x0003;
const ES_AUTOHSCROLL: u32 = 0x0080;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_template_is_laid_out_as_windows_reads_it() {
        let items = [
            Item { class: Class::Static, text: "Hi", id: 0xFFFF, style: 0, rect: [7, 7, 50, 10] },
            Item { class: Class::Named(windows::core::w!("ab")), text: "", id: 10, style: 0, rect: [1, 2, 3, 4] },
        ];
        let t = template("T", [100, 50], &items);
        let words: Vec<u16> = t.iter().flat_map(|d| [*d as u16, (d >> 16) as u16]).collect();
        let header_style = t[0];
        assert_ne!(header_style & DS_SETFONT as u32, 0, "the font below is read");
        assert_eq!(&words[4..9], [2, 0, 0, 100, 50], "item count, position, size");
        assert_eq!(&words[9..15], [0, 0, 'T' as u16, 0, 9, 'S' as u16], "no menu or class, title, font size");
        // "Segoe UI" ends at word 22, so the first item starts at the next DWORD, word 24.
        assert_eq!(String::from_utf16_lossy(&words[15..23]), "egoe UI\0");
        assert_eq!(t[12], WS_CHILD.0 | WS_VISIBLE.0, "the first item's style");
        assert_eq!(header_style & WS_VISIBLE.0, 0, "a dialog shows only when it is run");
        assert_eq!(&words[28..35], [7, 7, 50, 10, 0xFFFF, 0xFFFF, 0x0082], "rect, id, the static class");
        assert_eq!(&words[35..39], ['H' as u16, 'i' as u16, 0, 0], "its text and no creation data");
        assert_eq!(&words[44..47], [1, 2, 3], "the second item starts on a DWORD");
        assert_eq!(&words[48..54], [10, 'a' as u16, 'b' as u16, 0, 0, 0], "a named class, empty text");
        assert_eq!(words.len() % 2, 0);
    }
}
