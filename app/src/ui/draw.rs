//! How each kind of control looks, in each of its states. Pure painting into a canvas: the
//! states (hover, press, focus) come in as numbers from `widget`, which knows the windows.

use super::canvas::{Canvas, R};
use super::theme::{Rgb, Theme, Tok, theme};
use super::{Font, font, line, px, radius};
use windows::Win32::Graphics::Gdi::{
    DRAW_TEXT_FORMAT, DT_CENTER, DT_EDITCONTROL, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DT_WORDBREAK,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

/// How a label looks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Label {
    pub font: Font,
    pub color: Tok,
    pub align: Align,
    /// Several lines, from the top; else one line, centred in its height.
    pub wrap: bool,
    /// A text drawn quieter (text3, regular), such as "(no channel)".
    pub quiet: Option<&'static str>,
}

fn c(t: Tok) -> Rgb {
    theme().get(t)
}

fn one_line(align: Align) -> DRAW_TEXT_FORMAT {
    let a = match align {
        Align::Left => DT_LEFT,
        Align::Center => DT_CENTER,
        Align::Right => DT_RIGHT,
    };
    a | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX
}

/// A design length as a fraction of a pixel, for the glyphs.
fn f(v: f32) -> f32 {
    v * super::dpi() as f32 / 96.0
}

pub fn label(cv: &mut Canvas, r: R, text: &str, l: &Label) {
    let (fnt, color) = match l.quiet {
        Some(q) if q == text => (Font::Small, Tok::Text3),
        _ => (l.font, l.color),
    };
    let flags = if l.wrap { DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL | DT_END_ELLIPSIS } else { one_line(l.align) };
    cv.text(r, text, font(fnt), c(color), flags);
}

pub fn card(cv: &mut Canvas, r: R) {
    cv.round(r, radius(10), line(), Some(c(Tok::Surface)), Some(c(Tok::Line)));
}

/// A segmented track with its indicator at item `pos` (fractional while it slides).
pub fn segments(cv: &mut Canvas, track: R, item: (i32, i32), pos: f32) {
    let t = theme();
    cv.round(track, radius(8), line(), Some(t.get(Tok::Seg)), Some(t.get(Tok::Line)));
    let inset = (track.h - item.1) / 2;
    let x = track.x + inset + (pos * item.0 as f32).round() as i32;
    let ind = R::new(x, track.y + inset, item.0, item.1);
    let edge = if t.look == super::theme::Look::Contrast { Tok::Ind } else { Tok::Line };
    cv.round(ind, radius(6), line(), Some(t.get(Tok::Ind)), Some(t.get(edge)));
}

/// The label of one segment; the parent paints the track and the indicator under it.
pub fn segment(cv: &mut Canvas, r: R, text: &str, selected: bool, hover: f32) {
    let (fnt, color) = if selected {
        (Font::BodyStrong, c(Tok::OnInd))
    } else {
        (Font::Body, c(Tok::Text2).lerp(c(Tok::Text), hover))
    };
    cv.text(r, text, font(fnt), color, one_line(Align::Center));
}

/// The 2 px ring of keyboard focus, 2 px outside `r`, whose corners have `rad`.
pub fn ring(cv: &mut Canvas, r: R, rad: i32) {
    let (gap, w) = (px(2), px(2));
    cv.round(r.grow(gap + w), rad + gap + w, w, None, Some(c(Tok::Ring)));
}

pub fn dot(cv: &mut Canvas, x: i32, cy: i32, d: i32, color: Rgb) {
    cv.round(R::new(x, cy - d / 2, d, d), d / 2, 0, Some(color), None);
}

/// The check mark of a 12 px box at (x, y).
pub fn check(cv: &mut Canvas, x: i32, y: i32, color: Rgb) {
    let p = |a: f32, b: f32| (x as f32 + f(a), y as f32 + f(b));
    cv.stroke(&[p(2.5, 6.2), p(4.8, 8.5), p(9.5, 3.5)], f(1.75), color);
}

/// The chevron of a select, a 10 x 6 box at (x, y).
fn chevron(cv: &mut Canvas, x: i32, y: i32) {
    let p = |a: f32, b: f32| (x as f32 + f(a), y as f32 + f(b));
    cv.stroke(&[p(1.0, 1.0), p(5.0, 5.0), p(9.0, 1.0)], f(1.5), c(Tok::Text2));
}

pub struct ButtonState {
    pub primary: bool,
    pub hover: f32,
    pub press: f32,
    pub enabled: bool,
}

pub fn button(cv: &mut Canvas, r: R, text: &str, s: &ButtonState) {
    let t = theme();
    let inset = if s.press >= 0.5 { px(1).max(1) } else { 0 };
    let (fill, edge, ink, fnt) = if !s.enabled {
        (t.get(Tok::Button), t.get(Tok::Line), t.get(Tok::Text3), Font::Body)
    } else if s.primary {
        let fill = t.get(Tok::Accent).lerp(t.get(Tok::AccentH), s.hover * (1.0 - s.press));
        (fill, fill, t.get(Tok::AccentFg), Font::BodyStrong)
    } else {
        let fill = t.get(Tok::Button).lerp(t.get(Tok::Hover), s.hover).lerp(t.get(Tok::Press), s.press);
        (fill, t.get(Tok::Cborder), t.get(Tok::ButtonText), Font::Body)
    };
    cv.round(r.grow(-inset), radius(6), line(), Some(fill), Some(edge));
    let text_at = R::new(r.x + px(4), r.y + inset, r.w - px(8), r.h);
    cv.text(text_at, text, font(fnt), ink, one_line(Align::Center));
}

/// The closed box of a select: its value and a chevron.
pub fn select(cv: &mut Canvas, r: R, text: &str, hover: f32, open: bool) {
    let t = theme();
    let fill = if open { t.get(Tok::Press) } else { t.get(Tok::Control).lerp(t.get(Tok::Hover), hover) };
    cv.round(r, radius(6), line(), Some(fill), Some(t.get(Tok::Cborder)));
    let text_at = R::new(r.x + px(10), r.y, r.w - px(36), r.h);
    cv.text(text_at, text, font(Font::Body), t.get(Tok::Text), one_line(Align::Left));
    chevron(cv, r.right() - px(20), r.y + (r.h - px(6)) / 2);
}

/// One row of a select's open list: `active` is the row under the mouse or the arrow keys,
/// `current` the value in force, `quiet` the entries that open a dialog.
pub fn popup_row(cv: &mut Canvas, r: R, text: &str, active: bool, current: bool, quiet: bool) {
    let t = theme();
    cv.fill(r, t.get(Tok::Control));
    let inner = R::new(r.x + px(4), r.y, r.w - px(8), r.h);
    if active {
        let (fill, ink) = if t.look == super::theme::Look::Contrast {
            (t.get(Tok::Accent), Tok::AccentFg)
        } else {
            (t.get(Tok::Hover), Tok::Text)
        };
        cv.round(inner, radius(6), 0, Some(fill), None);
        if current {
            check(cv, inner.x + px(8), r.y + (r.h - px(12)) / 2, t.get(Tok::Accent).lerp(t.get(ink), 0.0));
        }
    } else if current {
        check(cv, inner.x + px(8), r.y + (r.h - px(12)) / 2, t.get(Tok::Accent));
    }
    let ink = if active && t.look == super::theme::Look::Contrast {
        Tok::AccentFg
    } else if quiet {
        Tok::Text2
    } else {
        Tok::Text
    };
    let text_at = R::new(inner.x + px(28), r.y, inner.w - px(38), r.h);
    cv.text(text_at, text, font(Font::Body), t.get(ink), one_line(Align::Left));
}

/// The frame of a text box, painted by the parent around the real edit control.
pub fn field(cv: &mut Canvas, r: R, hover: f32, focus: bool, error: bool) {
    let t = theme();
    let edge = if error {
        Tok::Amber
    } else if focus {
        Tok::Accent
    } else {
        Tok::Cborder
    };
    cv.round(r, radius(6), line(), Some(field_fill(&t, hover, focus)), Some(t.get(edge)));
    if focus && !error {
        let rad = radius(6);
        cv.fill(R::new(r.x + rad, r.bottom() - 2 * line(), r.w - 2 * rad, line()), t.get(Tok::Accent));
    }
}

pub fn field_fill(t: &Theme, hover: f32, focus: bool) -> Rgb {
    if focus { t.get(Tok::Surface) } else { t.get(Tok::Control).lerp(t.get(Tok::Hover), hover) }
}

/// The frame of a list box, painted by the parent around the real list.
pub fn list_frame(cv: &mut Canvas, r: R) {
    cv.round(r, radius(6), line(), Some(c(Tok::Control)), Some(c(Tok::Cborder)));
}

/// One row of a list box. " (moved)" at the end is drawn quieter.
pub fn list_row(cv: &mut Canvas, r: R, text: &str, selected: bool, hover: f32) {
    let t = theme();
    cv.fill(r, t.get(Tok::Control));
    let ink = if selected {
        cv.round(r, radius(4), 0, Some(t.get(Tok::Tint)), None);
        cv.fill(R::new(r.x, r.y + px(2), px(2), r.h - px(4)), t.get(Tok::Accent));
        t.get(Tok::OnInd)
    } else {
        if hover > 0.0 {
            cv.round(r, radius(4), 0, Some(t.get(Tok::Control).lerp(t.get(Tok::Hover), hover)), None);
        }
        t.get(Tok::Text)
    };
    let (name, moved) = match text.strip_suffix(" (moved)") {
        Some(name) => (name, true),
        None => (text, false),
    };
    let at = R::new(r.x + px(8), r.y, r.w - px(12), r.h);
    cv.text(at, name, font(Font::Body), ink, one_line(Align::Left));
    if moved {
        let width = super::canvas::measure(name, font(Font::Body), None).cx;
        let rest = R::new(at.x + width, r.y, (at.w - width).max(0), r.h);
        cv.text(rest, " (moved)", font(Font::Body), t.get(Tok::Text2), one_line(Align::Left));
    }
}

/// A switch: a 40 x 20 track at the left with its knob at `on` (0 off, 1 on), then the label.
pub fn toggle(cv: &mut Canvas, r: R, text: &str, on: f32, hover: f32) {
    let t = theme();
    let track = R::new(r.x, r.y + (r.h - px(20)) / 2, px(40), px(20));
    let off_fill = t.get(Tok::Control).lerp(t.get(Tok::Hover), hover);
    let fill = off_fill.lerp(t.get(Tok::Accent), on);
    let edge = t.get(Tok::Cborder).lerp(t.get(Tok::Accent), on);
    cv.round(track, track.h / 2, line(), Some(fill), Some(edge));
    let knob = px(12);
    let x = track.x + px(3) + (on * px(20) as f32).round() as i32;
    let ink = t.get(Tok::Text2).lerp(t.get(Tok::AccentFg), on);
    cv.round(R::new(x, track.y + (track.h - knob) / 2, knob, knob), knob / 2, 0, Some(ink), None);
    let at = R::new(r.x + px(52), r.y, r.w - px(52), r.h);
    cv.text(at, text, font(Font::Body), t.get(Tok::Text), one_line(Align::Left));
}

/// A slider: a 4 px rail along `rail` filled up to the thumb at `thumb_x`, and the thumb, which
/// grows from 16 to 18 px with `big`.
pub fn slider(cv: &mut Canvas, rail: R, thumb_x: i32, big: f32) {
    let t = theme();
    let h = px(4);
    let y = rail.y + (rail.h - h) / 2;
    cv.round(R::new(rail.x, y, rail.w, h), h / 2, 0, Some(t.get(Tok::Cborder)), None);
    cv.round(R::new(rail.x, y, (thumb_x - rail.x).max(h), h), h / 2, 0, Some(t.get(Tok::Accent)), None);
    let d = px(16) + (big * px(2) as f32).round() as i32;
    let cy = y + h / 2;
    let ring = px(3);
    let around = R::new(thumb_x - d / 2 - ring, cy - d / 2 - ring, d + 2 * ring, d + 2 * ring);
    cv.round(around, around.w / 2, 0, Some(t.get(Tok::Surface)), None);
    cv.round(around.grow(-ring), d / 2, 0, Some(t.get(Tok::Accent)), None);
}

/// The status line: up to three lines in the small font, amber with a dot for a warning.
pub fn status(cv: &mut Canvas, r: R, text: &str, warn: bool) {
    let t = theme();
    let fnt = font(Font::Small);
    let (ink, x) = if warn {
        dot(cv, r.x, r.y + r.h / 2, px(8), t.get(Tok::Amber));
        (t.get(Tok::Amber), r.x + px(16))
    } else {
        (t.get(Tok::Text2), r.x)
    };
    let w = r.right() - x;
    let lines = px(16) * 3;
    let h = super::canvas::measure(text, fnt, Some(w)).cy.min(lines);
    let at = R::new(x, r.y + (r.h - h) / 2, w, h);
    cv.text(at, text, fnt, ink, DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL | DT_END_ELLIPSIS);
}

/// The "Saved" chip at `shown` (0 gone, 1 fully in): it fades in from the colour under it and
/// rises 2 px.
pub fn chip(cv: &mut Canvas, r: R, shown: f32) {
    if shown <= 0.0 {
        return;
    }
    let t = theme();
    let under = t.get(Tok::Bg);
    let r = r.at(0, ((1.0 - shown) * px(2) as f32).round() as i32);
    cv.round(r, r.h / 2, 0, Some(under.lerp(t.get(Tok::Tint), shown)), None);
    check(cv, r.x + px(8), r.y + (r.h - px(12)) / 2, under.lerp(t.get(Tok::Accent), shown));
    let at = R::new(r.x + px(26), r.y, r.w - px(26), r.h);
    cv.text(at, "Saved", font(Font::SmallStrong), under.lerp(t.get(Tok::OnInd), shown), one_line(Align::Left));
}

/// The width of the "Saved" chip: 8 + check 12 + 6 + text + 10.
pub fn chip_width() -> i32 {
    px(36) + super::canvas::measure("Saved", font(Font::SmallStrong), None).cx
}

/// Key chips for a shortcut ("Win+Shift+S"), centred in `r`, with a quiet "+" between them.
pub fn key_chips(cv: &mut Canvas, r: R, label: &str) {
    let t = theme();
    let fnt = font(Font::SmallStrong);
    let parts: Vec<&str> = label.split('+').collect();
    let plus = super::canvas::measure("+", font(Font::Small), None).cx;
    let gap = px(6);
    let widths: Vec<i32> = parts.iter().map(|p| super::canvas::measure(p, fnt, None).cx + px(16)).collect();
    let total: i32 = widths.iter().sum::<i32>() + (parts.len() as i32 - 1) * (plus + 2 * gap);
    let mut x = r.x + (r.w - total) / 2;
    let h = px(24);
    let y = r.y + (r.h - h) / 2;
    for (i, (part, w)) in parts.iter().zip(&widths).enumerate() {
        if i > 0 {
            cv.text(R::new(x + gap, y, plus, h), "+", font(Font::Small), t.get(Tok::Text3), one_line(Align::Center));
            x += plus + 2 * gap;
        }
        let chip = R::new(x, y, *w, h);
        cv.round(chip, radius(6), line(), Some(t.get(Tok::Control)), Some(t.get(Tok::Cborder)));
        cv.text(chip, part, fnt, t.get(Tok::Text), one_line(Align::Center));
        x += w;
    }
}

/// The shortcut box: a dashed border at rest, a solid accent one with the focus.
pub fn capture_box(cv: &mut Canvas, r: R, focused: bool) {
    let t = theme();
    let rad = radius(8);
    if focused {
        cv.round(r, rad, line(), Some(t.get(Tok::Surface)), Some(t.get(Tok::Accent)));
        return;
    }
    cv.round(r, rad, line(), Some(t.get(Tok::Surface)), Some(t.get(Tok::Cborder)));
    // Gaps in the straight edges make the dashes: 4 px on, 3 px off.
    let (on, off, w) = (px(4), px(3), line());
    let surface = t.get(Tok::Surface);
    let mut x = r.x + rad + on;
    while x < r.right() - rad - off {
        cv.fill(R::new(x, r.y, off, w), surface);
        cv.fill(R::new(x, r.bottom() - w, off, w), surface);
        x += on + off;
    }
    let mut y = r.y + rad + on;
    while y < r.bottom() - rad - off {
        cv.fill(R::new(r.x, y, w, off), surface);
        cv.fill(R::new(r.right() - w, y, w, off), surface);
        y += on + off;
    }
}
