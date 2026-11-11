//! Draws the settings window or one of its dialogs into a picture without ever showing it: the
//! window is made hidden, painted through WM_PRINT (it and each child in turn, into one bitmap
//! of its own), and destroyed. Nothing reaches the screen and no other window is read. For the
//! design checks and the pictures in the docs (`examples/render_settings.rs`).

use super::controls::force_dpi;
use super::{Tab, create};
use crate::dialog::Which;
use crate::ui::canvas::{Canvas, R};
use crate::ui::theme::{self, Look};
use crate::ui::widget;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{IntersectClipRect, MapWindowPoints, SelectClipRgn, SetViewportOrgEx};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What to draw.
#[derive(Clone, Copy, Debug)]
pub enum Preview<'a> {
    Tab(Tab),
    /// The Shortcut dialog, started from a key's config text ("win+shift+s").
    Shortcut(&'a str),
    /// The Open dialog, with this text in its box.
    Open(&'a str),
    AddProgram,
}

/// A drawn window: its client area, row by row, as 0x00RRGGBB.
pub struct Picture {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u32>,
    /// From the call that makes the window to the picture being complete.
    pub took: Duration,
}

/// The client area of `hwnd`, with its visible children painted over it.
fn snapshot(hwnd: HWND) -> Option<(i32, i32, Vec<u32>)> {
    let mut client = RECT::default();
    // SAFETY: reads the client size of our own window.
    unsafe { GetClientRect(hwnd, &mut client).ok()? };
    let mut cv = Canvas::new(R::of(client))?;
    let dc = cv.dc();
    widget::printing(true);
    const PRF: isize = 0x4 | 0x8 | 0x10 | 0x2; // client, erase background, children, non-client
    crate::win::send(hwnd, WM_PRINTCLIENT, dc.0 as usize, PRF);
    // The children, from the bottom of the z-order up.
    let mut kids = Vec::new();
    // SAFETY: walks our own window's children; each call only reads the window tree.
    unsafe {
        let mut at = GetWindow(hwnd, GW_CHILD).ok();
        while let Some(k) = at {
            kids.push(k);
            at = GetWindow(k, GW_HWNDNEXT).ok();
        }
    }
    for k in kids.into_iter().rev() {
        // SAFETY: as above; the DC is the canvas's, whose origin and clip are put back after
        // each child. WM_PRINT is answered by the child itself, synchronously.
        unsafe {
            if GetWindowLongPtrW(k, GWL_STYLE) as u32 & WS_VISIBLE.0 == 0 {
                continue;
            }
            let mut r = RECT::default();
            let _ = GetWindowRect(k, &mut r);
            let mut pts = [windows::Win32::Foundation::POINT { x: r.left, y: r.top }];
            MapWindowPoints(None, Some(hwnd), &mut pts);
            let _ = SetViewportOrgEx(dc, pts[0].x, pts[0].y, None);
            IntersectClipRect(dc, 0, 0, r.right - r.left, r.bottom - r.top);
            SendMessageW(k, WM_PRINT, Some(WPARAM(dc.0 as usize)), Some(LPARAM(PRF)));
            SelectClipRgn(dc, None);
            let _ = SetViewportOrgEx(dc, 0, 0, None);
        }
    }
    widget::printing(false);
    let (w, h) = (client.right, client.bottom);
    Some((w, h, cv.pixels().to_vec()))
}

/// How to draw it.
#[derive(Clone, Copy, Default)]
pub struct Options<'a> {
    /// The display scale (96 is 100 %); 0 takes the system's.
    pub dpi: i32,
    /// None: as Windows is set.
    pub look: Option<Look>,
    /// A text for the status line instead of the usual one.
    pub status: Option<&'a str>,
    /// The id of a control to give the keyboard focus (to see its ring).
    pub focus: Option<i32>,
    /// Called once the picture is taken, while the window still exists (to measure it).
    pub while_open: Option<fn()>,
}

/// Draws `what` as `o` says, hidden, and destroys it again.
pub fn render_hidden(what: Preview, o: &Options) -> Result<Picture, String> {
    let (dpi, look, status, focus) = (o.dpi, o.look, o.status, o.focus);
    let dpi = if dpi > 0 { dpi } else { super::controls::system_dpi() };
    theme::force(look);
    force_dpi(Some(dpi));
    let start = Instant::now();
    let picture = match what {
        Preview::Tab(tab) => create(Some(tab), status).ok_or("the window could not be made").and_then(|hwnd| {
            if let Some(id) = focus {
                widget::show_focus(true);
                crate::win::set_focus(super::controls::child(id));
            }
            let shot = snapshot(hwnd).ok_or("the window could not be drawn").map(|s| (s, start.elapsed()));
            widget::show_focus(false);
            if let Some(f) = o.while_open {
                f();
            }
            // SAFETY: the window is ours, on this thread.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            shot
        }),
        Preview::Shortcut(_) | Preview::Open(_) | Preview::AddProgram => {
            crate::ui::begin(dpi);
            let which = match what {
                Preview::Shortcut(text) => {
                    Which::Shortcut(crate::config::parse_key(text).ok().and_then(|a| super::choices::shortcut_of(&a)))
                }
                Preview::Open(text) => Which::Open(text.to_string()),
                _ => Which::Program,
            };
            let shot = crate::dialog::hidden(which).ok_or("the dialog could not be made").and_then(|dlg| {
                let shot = snapshot(dlg).ok_or("the dialog could not be drawn").map(|s| (s, start.elapsed()));
                // SAFETY: the dialog is ours (modeless and hidden), on this thread.
                unsafe {
                    let _ = DestroyWindow(dlg);
                }
                shot
            });
            crate::ui::end();
            shot
        }
    };
    theme::force(None);
    force_dpi(None);
    let ((width, height, pixels), took) = picture?;
    Ok(Picture { width, height, pixels, took })
}
