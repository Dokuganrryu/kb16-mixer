//! A line of text with a Browse button: what an "open:" key opens, or a program to add to a
//! channel by name.

use super::{Class, EM_SETSEL, ID_EXTRA, ID_FIELD, ID_HINT, Item, TEXT_RESULT};
use super::{end, item_text, run, set_item_text, template};
use crate::win::{self, send};
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, PWSTR};

thread_local! {
    static TEXT_INITIAL: RefCell<String> = const { RefCell::new(String::new()) };
    static TEXT_PROGRAM: Cell<bool> = const { Cell::new(false) };
}

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

/// What OK keeps of the line: a program name, or the text as typed. None if that is empty.
fn kept(text: &str, program: bool) -> Option<String> {
    let kept = if program { program_name(text) } else { Some(text.trim().to_string()) };
    kept.filter(|t| !t.is_empty())
}

/// The file picker's filter: pairs of name and pattern, NUL-separated, ending in two NULs.
fn filter(program: bool) -> &'static str {
    if program {
        "Programs\0*.exe\0\0"
    } else {
        "Programs and shortcuts\0*.exe;*.lnk;*.bat;*.cmd;*.url\0All files\0*.*\0\0"
    }
}

fn browse(dlg: HWND) {
    let mut file = [0u16; 1024];
    let program = TEXT_PROGRAM.get();
    let filter: Vec<u16> = filter(program).encode_utf16().collect();
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
    // SAFETY: the struct carries its own size; the filter (double-NUL-terminated) and the file
    // buffer, whose length is given in nMaxFile, both outlive the call.
    if unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
        let path = String::from_utf16_lossy(&file[..len]);
        let shown = if program { program_name(&path).unwrap_or_default() } else { path };
        set_item_text(dlg, ID_FIELD as i32, &shown);
    }
}

unsafe extern "system" fn text_proc(dlg: HWND, msg: u32, wparam: WPARAM, _: LPARAM) -> isize {
    match msg {
        WM_INITDIALOG => {
            set_item_text(dlg, ID_FIELD as i32, &TEXT_INITIAL.with_borrow(Clone::clone));
            if let Some(field) = win::dlg_item(Some(dlg), ID_FIELD as i32) {
                send(field, EM_SETSEL, 0, -1);
            }
            1
        }
        WM_COMMAND => {
            match (wparam.0 & 0xFFFF) as i32 {
                1 => match kept(&item_text(dlg, ID_FIELD as i32), TEXT_PROGRAM.get()) {
                    Some(t) => {
                        TEXT_RESULT.set(Some(t));
                        end(dlg, 1);
                    }
                    None => set_item_text(dlg, ID_HINT as i32, "Type something first, or press Cancel."),
                },
                2 => end(dlg, 0),
                id if id == ID_EXTRA as i32 => browse(dlg),
                _ => {}
            }
            1
        }
        _ => 0,
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

    #[test]
    fn program_names_are_what_windows_lists() {
        assert_eq!(program_name(" spotify ").as_deref(), Some("spotify.exe"));
        assert_eq!(program_name("\"C:\\Apps\\Spotify.exe\"").as_deref(), Some("Spotify.exe"));
        assert_eq!(program_name("D:/Games/game.bin").as_deref(), Some("game.bin"));
        assert_eq!(program_name("   "), None);
        assert_eq!(program_name("C:\\Apps\\"), None);
        assert_eq!(program_name("a|b"), None);
    }

    #[test]
    fn ok_keeps_only_something_usable() {
        assert_eq!(kept("  https://example.com  ", false).as_deref(), Some("https://example.com"));
        assert_eq!(kept("C:\\Tools\\obs64.exe", true).as_deref(), Some("obs64.exe"), "a program by name");
        assert_eq!(kept("   ", false), None);
        assert_eq!(kept("C:\\Apps\\", true), None);
        for program in [true, false] {
            let f = filter(program);
            assert!(f.ends_with("\0\0"), "the list ends in two NULs");
            assert_eq!(f.matches('\0').count() % 2, 1, "names and patterns come in pairs");
        }
    }
}
