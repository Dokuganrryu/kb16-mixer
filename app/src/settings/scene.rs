//! What the settings window paints itself, under its controls: the background, the tab strip's
//! track and sliding indicator, the pad status, the cards, the footer and its "Saved" chip.

use super::controls::{at, child};
use super::fill::pad_line;
use super::ids::*;
use super::layout::*;
use super::{PAGE, TAB, Tab, window};
use crate::ui::anim::{self, Curve, Paint};
use crate::ui::canvas::{Canvas, R, measure};
use crate::ui::theme::{Tok, theme};
use crate::ui::{Font, font, line, px};
use crate::ui::{draw, widget};
use std::cell::Cell;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER};
use windows::Win32::UI::WindowsAndMessaging::{GWL_STYLE, GetWindowLongPtrW, KillTimer, SetTimer, WS_VISIBLE};

/// Animated parts of the window itself.
pub const PART_TAB: u32 = 10;
pub const PART_PAGE: u32 = 11;
const PART_CHIP: u32 = 12;

thread_local! {
    static CHIP_SHOWN: Cell<bool> = const { Cell::new(false) };
    static CHIP_TIMER: Cell<usize> = const { Cell::new(0) };
}

fn scaled(r: R) -> R {
    at(r.x, r.y, r.w, r.h)
}

fn shown(id: i32) -> bool {
    // SAFETY: reads a child's style; a missing child reads 0.
    unsafe { GetWindowLongPtrW(child(id), GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 }
}

/// Where the pad status sits: right-aligned in the header.
fn pad_status_area() -> R {
    at(WIDTH - 276, 8, 260, 32)
}

/// The window's own painting, for the part `cv` covers.
pub fn paint(cv: &mut Canvas, hwnd: HWND) {
    let t = theme();
    cv.fill(cv.area(), t.get(Tok::Bg));

    let tab_at = anim::value(hwnd, PART_TAB, TAB.get() as f32);
    draw::segments(cv, scaled(TAB_TRACK), (px(TAB_ITEM.0), px(TAB_ITEM.1)), tab_at);

    let area = pad_status_area();
    if cv.area().overlaps(area) {
        let text = pad_line();
        let fnt = font(Font::Small);
        let width = measure(&text, fnt, None).cx;
        cv.text(area, &text, fnt, t.get(Tok::Text2), DT_RIGHT | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
        let dot = if crate::pad::connected_release().is_some() { Tok::Accent } else { Tok::Text3 };
        let d = px(8);
        draw::dot(cv, area.right() - width - px(8) - d, area.y + area.h / 2, d, t.get(dot));
    }

    match super::TABS[TAB.get()].0 {
        Tab::Keys => {
            draw::card(cv, scaled(KEY_CARD));
            let page_at = anim::value(hwnd, PART_PAGE, PAGE.get() as f32);
            draw::segments(cv, scaled(PAGE_TRACK), (px(PAGE_ITEM.0), px(PAGE_ITEM.1)), page_at);
        }
        Tab::Channels => {
            for i in 0..4 {
                if shown(ID_CH_NAME + i) {
                    draw::card(cv, scaled(channel_card(i)));
                }
            }
        }
        Tab::Pad => {
            for card in [SCREEN_CARD, KNOBS_CARD, LIGHTS_CARD, SYSTEM_CARD] {
                draw::card(cv, scaled(card));
            }
        }
    }
    widget::paint_frames(cv, hwnd);

    let footer = px(FOOTER);
    if cv.area().bottom() > footer {
        cv.fill(R::new(0, footer, px(WIDTH), line()), t.get(Tok::Line));
        let shown = anim::value(hwnd, PART_CHIP, CHIP_SHOWN.get() as i32 as f32);
        draw::chip(cv, chip_area(), shown);
    }

    widget::paint_ring(cv, hwnd);
}

fn chip_area() -> R {
    let (x, y, h) = CHIP;
    R::new(px(x), px(y), draw::chip_width(), px(h))
}

/// A tab or page indicator moves from item `from` to `to`; by mouse it slides.
pub fn slide(part: u32, from: usize, to: usize) {
    let Some(hwnd) = window() else { return };
    let track = if part == PART_TAB { TAB_TRACK } else { PAGE_TRACK };
    let ms = if widget::keyboard() { 0 } else { 220 };
    let paint = Paint::Area(hwnd, scaled(track).rect());
    anim::go(hwnd, part, from as f32, to as f32, ms, Curve::InOut, paint);
}

/// A change was saved: the chip fades in, stays 1.6 s, and fades out.
pub fn saved_chip() {
    let Some(hwnd) = window() else { return };
    let was = CHIP_SHOWN.replace(true);
    anim::go(
        hwnd,
        PART_CHIP,
        was as i32 as f32,
        1.0,
        140,
        Curve::Out,
        Paint::Area(hwnd, chip_area().grow(px(2)).rect()),
    );
    stop_chip_timer();
    // SAFETY: a one-shot thread timer whose callback is `chip_out`, which kills it.
    CHIP_TIMER.set(unsafe { SetTimer(None, 0, 1600, Some(chip_out)) });
}

/// The chip goes at once (a refusal, or a problem, replaces what was saved).
pub fn hide_chip() {
    if CHIP_SHOWN.replace(false) {
        stop_chip_timer();
        if let Some(hwnd) = window() {
            anim::go(hwnd, PART_CHIP, 1.0, 0.0, 0, Curve::Ease, Paint::Area(hwnd, chip_area().grow(px(2)).rect()));
        }
    }
}

pub fn stop_chip_timer() {
    let id = CHIP_TIMER.replace(0);
    if id != 0 {
        // SAFETY: the id is the one SetTimer returned for this thread timer.
        unsafe {
            let _ = KillTimer(None, id);
        }
    }
}

unsafe extern "system" fn chip_out(_: HWND, _: u32, _: usize, _: u32) {
    stop_chip_timer();
    CHIP_SHOWN.set(false);
    if let Some(hwnd) = window() {
        anim::go(hwnd, PART_CHIP, 1.0, 0.0, 240, Curve::Ease, Paint::Area(hwnd, chip_area().grow(px(2)).rect()));
    }
}

/// The pad came or went: repaint the pad status.
pub fn pad_status_changed() {
    if let Some(hwnd) = window() {
        anim::redraw(hwnd, Paint::Area(hwnd, pad_status_area().rect()));
    }
}

/// True if the chip shows now (for the tests).
#[cfg(test)]
pub fn chip_shown() -> bool {
    CHIP_SHOWN.get()
}
