//! The look of the settings window and its dialogs, drawn with plain GDI: the colours
//! (`theme`), a back buffer with smooth rounded shapes (`canvas`), short animations (`anim`),
//! the painting of each kind of control (`draw`), and the glue that makes real Win32 controls
//! paint that way (`widget`). The fonts and the display scale live here.
//!
//! Nothing here exists while no window is open: the fonts are made when the first window opens
//! and deleted when the last one closes.

pub mod anim;
pub mod canvas;
pub mod draw;
pub mod theme;
pub mod widget;

pub use draw::{Align, Label};

use std::cell::Cell;
use windows::Win32::Graphics::Gdi::{DeleteObject, HFONT};
use windows::Win32::UI::WindowsAndMessaging::{
    NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};

/// The type roles of the design.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Font {
    /// 13 px: control text, row labels, dialog prompts.
    Body,
    /// 13 px semibold: the selected tab or segment, the primary button.
    BodyStrong,
    /// 12 px: hints, the status line, the pad status.
    Small,
    /// 12 px semibold: card and column titles, field labels, "Saved", key chips.
    SmallStrong,
}

const FONT_SIZES: [(i32, i32); 4] = [(13, 400), (13, 600), (12, 400), (12, 600)];

thread_local! {
    static DPI: Cell<i32> = const { Cell::new(96) };
    static FONTS: Cell<[HFONT; 4]> = const { Cell::new([HFONT(std::ptr::null_mut()); 4]) };
    /// How many windows use the fonts (the settings window, a dialog over it).
    static USERS: Cell<u32> = const { Cell::new(0) };
}

pub fn dpi() -> i32 {
    DPI.get()
}

/// A length of the design (pixels at 100 %) at the current scale, rounded down.
pub fn px(v: i32) -> i32 {
    v * DPI.get() / 96
}

/// A corner radius at the current scale, rounded to the nearest pixel.
pub fn radius(v: i32) -> i32 {
    (v * DPI.get() + 48) / 96
}

/// Borders and rings stay 1 px up to 149 % and grow with the scale from 150 %.
pub fn line() -> i32 {
    (DPI.get() / 96).max(1)
}

pub fn font(f: Font) -> HFONT {
    FONTS.get()[f as usize]
}

fn make_fonts() {
    let mut metrics = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    // SAFETY: the struct carries its own size and outlives the call.
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            Some((&raw mut metrics).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let face = metrics.lfMessageFont.lfFaceName;
    FONTS.set(FONT_SIZES.map(|(size, weight)| canvas::font(&face, px(size), weight)));
}

fn free_fonts() {
    for f in FONTS.replace([HFONT::default(); 4]) {
        if !f.is_invalid() {
            // SAFETY: the fonts are ours, and no window uses them any more.
            unsafe {
                let _ = DeleteObject(f.into());
            }
        }
    }
    canvas::forget_masks();
}

/// A window opens at `dpi`: the first one makes the fonts and reads the theme.
pub fn begin(dpi: i32) {
    if USERS.replace(USERS.get() + 1) == 0 {
        DPI.set(dpi);
        theme::refresh();
        make_fonts();
    }
}

/// A window closed: the last one frees the fonts.
pub fn end() {
    match USERS.get() {
        0 => {}
        1 => {
            USERS.set(0);
            free_fonts();
        }
        n => USERS.set(n - 1),
    }
}

/// The window moved to a display of another scale: new fonts at `dpi`.
pub fn rescale(dpi: i32) {
    DPI.set(dpi);
    free_fonts();
    make_fonts();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_follow_the_display_scale() {
        for (dpi, px28, r6, border) in [(96, 28, 6, 1), (120, 35, 8, 1), (144, 42, 9, 1), (192, 56, 12, 2)] {
            DPI.set(dpi);
            assert_eq!((px(28), radius(6), line()), (px28, r6, border), "at {dpi} dpi");
        }
        DPI.set(96);
    }
}
