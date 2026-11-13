//! What the 128x32 screen shows: the frame buffer, the 1-bit drawing, and the choice of screen.
//! The firmware's `oled.rs` sends the buffer to the SSD1306 panel.
//!
//! The screen is exact: nothing on it travels. A value cuts in and follows the knob 1:1, and
//! a screen cuts to the next. The lights carry the glides (`crate::leds`).
//!
//! The case hides the panel's top rows, so every screen is laid out for 29 visible rows and
//! drawn `top` pixels lower (set from the PC app's config, default 3). The layout rows below
//! are those of the layout; the panel row is `top` lower.

use crate::Time;
use crate::assets::{BIG_COLS, BIG_OFF, LOGO_CHARS, LOGO_COLS, LOGO_GAP, LOGO_OFF, MID_COLS, MID_OFF, SIN};
use crate::font::FONT5X7;
use crate::leds::gamma;
use crate::state::{Mixer, NCH, POPUP_MS, Popup};
use kb16_mixer_protocol::{NAME_SHOWN, NEVER, NOTICE};

pub const W: i32 = 128;
pub const H: i32 = 32;
/// How long the logo shows after power-up.
pub const SPLASH_MS: u64 = 1800;
const SCREENSAVER_MS: u64 = 45_000;
/// The logo wipes in over this long (27 frames), after power-up and when the screensaver starts.
const WIPE_MS: u64 = 900;
/// A detent that cannot move the value past 0 or 100 shows an end stop for three 33 ms frames.
const WALL_MS: u32 = 99;
/// After the screen-off limit the panel fades out over this long, with the lights, then sleeps.
pub const FADE_MS: u32 = 700;
/// The SSD1306 control byte that says "display data follows".
const DATA: u8 = 0x40;

/// Mixer: 5x7 label on L2..7 (Focus pill L1..8), the value in the mid font on L12..20 (or
/// `MUTE` on L14..19), and the rail on L25, 2 px thick (L24..25) as far as the sound goes.
/// The rail and the sound stay in one display page for every `top` (0..=6), so sound alone
/// changes only that page.
const LABEL_Y: i32 = 1;
const VALUE_Y: i32 = 11;
const MUTE_Y: i32 = 13;
const RAIL_Y: i32 = 25;
/// The page-2 tick (right edge) and the microphone-muted tick (left edge), on L14..18.
const TICK_Y: (i32, i32) = (14, 18);
/// Readout: the name in the mid font on L3..11, the number in the logo digits on L3..26, and
/// the bar on L22..26 from x 3 to 64, its end stops on L21..27.
const NAME_Y: i32 = 2;
const NUMBER_Y: i32 = 3;
const NUMBER_GAP: i32 = 3;
const MUTED_Y: i32 = 14;
const BAR: (i32, i32, i32, i32) = (3, 64, 22, 26);

pub struct Fb {
    /// Byte 0 is the I2C "data" control byte so the buffer goes out in one transfer. Then one
    /// byte per column and 8-row page, LSB on top, as the SSD1306 takes it.
    pub buf: [u8; 1 + (W * H / 8) as usize],
    /// Vertical offset applied to everything drawn.
    pub top: i32,
}

impl Fb {
    pub fn new(top: i32) -> Fb {
        let mut buf = [0; 1 + (W * H / 8) as usize];
        buf[0] = DATA;
        Fb { buf, top }
    }

    fn clear(&mut self) {
        self.buf[1..].fill(0);
    }

    /// Whether the pixel at (`x`, `y`) of the drawn layout is lit (`top` applies).
    pub fn get(&self, x: i32, y: i32) -> bool {
        let y = y + self.top;
        (0..W).contains(&x) && (0..H).contains(&y) && self.buf[1 + (y / 8 * W + x) as usize] & 1 << (y % 8) != 0
    }

    fn px(&mut self, x: i32, y: i32, on: bool) {
        let y = y + self.top;
        if !(0..W).contains(&x) || !(0..H).contains(&y) {
            return;
        }
        let i = 1 + (y / 8 * W + x) as usize;
        let bit = 1u8 << (y % 8);
        if on {
            self.buf[i] |= bit;
        } else {
            self.buf[i] &= !bit;
        }
    }

    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, on: bool) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.px(x, y, on);
            }
        }
    }

    /// Filled box with the four corner pixels cut, which reads as rounded at this size.
    fn pill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.rect(x0, y0, x1, y1, true);
        for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
            self.px(x, y, false);
        }
    }

    /// One glyph column: bit n of `bits` sets (`x`, `y` + n) to `on`.
    fn col(&mut self, x: i32, y: i32, mut bits: u32, on: bool) {
        while bits != 0 {
            self.px(x, y + bits.trailing_zeros() as i32, on);
            bits &= bits - 1;
        }
    }

    /// 5x7 text; `on = false` draws dark text (inside a pill).
    fn text(&mut self, x: i32, y: i32, s: &[u8], on: bool) {
        for (i, &ch) in s.iter().enumerate() {
            for (dx, &bits) in FONT5X7[(ch.clamp(32, 126) - 32) as usize].iter().enumerate() {
                self.col(x + i as i32 * 6 + dx as i32, y, bits as u32, on);
            }
        }
    }

    /// Proportional text from its left edge.
    fn write(&mut self, font: Font, mut x: i32, y: i32, s: &[u8]) {
        for &ch in s {
            for k in font.cols(ch) {
                self.col(x, y, font.col(k), true);
                x += 1;
            }
            x += font.gap();
        }
    }
}

/// The proportional fonts: Open Sans SemiBold 16 and 13, and the logo's ExtraBold letters
/// (with the gap between letters, which differs between the logo and the readout number).
#[derive(Clone, Copy)]
enum Font {
    Big,
    Mid,
    Logo(i32),
}

impl Font {
    fn gap(self) -> i32 {
        match self {
            Font::Big => 2,
            Font::Mid => 1,
            Font::Logo(gap) => gap,
        }
    }

    /// The columns of `ch` in the font's table. The logo font lacks most characters; those
    /// have no columns.
    fn cols(self, ch: u8) -> core::ops::Range<usize> {
        let i = (ch.clamp(32, 126) - 32) as usize;
        let (a, b) = match self {
            Font::Big => (BIG_OFF[i], BIG_OFF[i + 1]),
            Font::Mid => (MID_OFF[i], MID_OFF[i + 1]),
            Font::Logo(_) => match LOGO_CHARS.iter().position(|&l| l == ch) {
                Some(i) => (LOGO_OFF[i], LOGO_OFF[i + 1]),
                None => (0, 0),
            },
        };
        a as usize..b as usize
    }

    fn col(self, k: usize) -> u32 {
        match self {
            Font::Big => u16::from_le_bytes(BIG_COLS[k]) as u32,
            Font::Mid => MID_COLS[k] as u32,
            Font::Logo(_) => u32::from_le_bytes(LOGO_COLS[k]),
        }
    }

    fn width(self, s: &[u8]) -> i32 {
        let w: i32 = s.iter().map(|&c| self.cols(c).len() as i32 + self.gap()).sum();
        (w - self.gap()).max(0)
    }

    /// `s` without the letters at its end that do not fit in `max` columns (and without the
    /// spaces the cut leaves at the end).
    fn fit(self, s: &[u8], max: i32) -> &[u8] {
        let mut s = s;
        while !s.is_empty() && self.width(s) > max {
            s = trimmed(&s[..s.len() - 1]);
        }
        s
    }
}

fn trimmed(s: &[u8]) -> &[u8] {
    let end = s.iter().rposition(|&c| c != 0 && c != b' ').map_or(0, |i| i + 1);
    &s[..end]
}

/// A volume as digits, without a percent sign.
fn number(v: u8, out: &mut [u8; 3]) -> &[u8] {
    let v = v.min(100);
    let mut n = 0;
    if v >= 100 {
        out[n] = b'1';
        n += 1;
    }
    if v >= 10 {
        out[n] = b'0' + (v / 10) % 10;
        n += 1;
    }
    out[n] = b'0' + v % 10;
    &out[..n + 1]
}

/// The mixer rail's length in pixels for a volume: 28 at 100.
fn rail(volume: u8) -> i32 {
    volume.min(100) as i32 * 28 / 100
}

/// The name in the logo letters, centred, and a slow wave beneath. `age` is the time since the
/// logo came on: it wipes in from the left over `WIPE_MS`, a light streak crosses it once, and
/// the wave takes about 9 s a period. A name too wide for the logo letters uses the big font,
/// and one too wide even for that loses letters from the end.
fn draw_logo(fb: &mut Fb, age: u64, name: &[u8]) {
    let name = trimmed(name);
    let logo = Font::Logo(LOGO_GAP);
    if logo.width(name) <= W {
        fb.write(logo, (W - logo.width(name)) / 2, 0, name);
    } else {
        let name = Font::Big.fit(name, W);
        fb.write(Font::Big, (W - Font::Big.width(name)) / 2, 4, name);
    }
    let streak = (age.min(10_000) / 5) as i32 - 30;
    if streak < W + 40 {
        for y in 0..H {
            for x in 0..W {
                let d = x - streak - y / 2;
                if (0..4).contains(&d) || d == 6 {
                    fb.px(x, y, false);
                }
            }
        }
    }
    // The wave breaks where a letter reaches down into it (J, Q).
    let low: [bool; W as usize] = core::array::from_fn(|x| (24..30).any(|y| fb.get(x as i32, y)));
    let t = (age / 36) as u32;
    for x in 0..W {
        if (x - 1..=x + 1).any(|cx| low.get(cx as usize).copied().unwrap_or(false)) {
            continue;
        }
        let a = SIN[((x as u32 * 4).wrapping_add(t) & 0xFF) as usize] as i32;
        fb.px(x, 25 + a * 3 / 255, true);
    }
    let reveal = (age.min(WIPE_MS) * 130 / WIPE_MS) as i32;
    if reveal < W - 1 {
        fb.rect(reveal + 1, -fb.top, W - 1, H - 1, false);
    }
}

/// Scribble strip: one 32 px column per channel. `sound` is each channel's swell in pixels of
/// its rail. Inversion means Focus only; a muted channel shows plain `MUTE` and no rail.
fn draw_mixer(fb: &mut Fb, m: &Mixer, sound: &[u8; NCH]) {
    for (c, &sound) in sound.iter().enumerate().take(m.n as usize) {
        let x0 = c as i32 * 32;
        let name = trimmed(&m.names[c]);
        let mut label = [0u8; NAME_SHOWN];
        let n = name.len().min(NAME_SHOWN);
        for (l, ch) in label.iter_mut().zip(name) {
            *l = ch.to_ascii_uppercase();
        }
        let (label, tw) = (&label[..n], n as i32 * 6 - 1);
        let tx = x0 + (32 - tw) / 2;
        let focus = c as u8 == m.selected;
        if focus && n > 0 {
            fb.pill(tx - 2, LABEL_Y, tx + tw + 1, LABEL_Y + 7);
        }
        fb.text(tx, LABEL_Y, label, !focus);

        if m.mutes & 1 << c != 0 {
            fb.text(x0 + 5, MUTE_Y, b"MUTE", true);
            continue;
        }
        let mut digits = [0u8; 3];
        let v = number(m.volume[c], &mut digits);
        fb.write(Font::Mid, x0 + (33 - Font::Mid.width(v)) / 2, VALUE_Y, v);
        let len = rail(m.volume[c]);
        if len > 0 {
            fb.rect(x0 + 2, RAIL_Y, x0 + 1 + len, RAIL_Y, true);
        }
        if sound > 0 {
            fb.rect(x0 + 2, RAIL_Y - 1, x0 + 1 + sound as i32, RAIL_Y, true);
        }
    }
    if m.mic_muted {
        fb.rect(0, TICK_Y.0, 1, TICK_Y.1, true);
    }
    if m.second_page {
        fb.rect(W - 2, TICK_Y.0, W - 1, TICK_Y.1, true);
    }
}

/// A short message on its own, centred: the big font, or the mid font when the big one would
/// fill the width.
fn draw_notice(fb: &mut Fb, text: &[u8]) {
    if Font::Big.width(text) <= W - 4 {
        fb.write(Font::Big, (W - Font::Big.width(text)) / 2, 7, text);
    } else {
        let text = Font::Mid.fit(text, W - 4);
        fb.write(Font::Mid, (W - Font::Mid.width(text)) / 2, 9, text);
    }
}

/// The readout of one knob or key: the name at the left, the number (or `Muted`) at the right,
/// the bar below. `stop` shows the end stop at the top (true) or the bottom of the range.
fn draw_readout<T>(fb: &mut Fb, p: &Popup<T>, stop: Option<bool>) {
    let left = if p.muted {
        let x = W - Font::Big.width(b"Muted");
        fb.write(Font::Big, x, MUTED_Y, b"Muted");
        x
    } else {
        let mut digits = [0u8; 3];
        let v = number(p.volume, &mut digits);
        let font = Font::Logo(NUMBER_GAP);
        let x = W - font.width(v);
        fb.write(font, x, NUMBER_Y, v);
        x
    };
    fb.write(Font::Mid, 2, NAME_Y, Font::Mid.fit(trimmed(&p.name), left - 9));

    // The bar, hollow while muted; past its end, a 1 px track to the end of the range.
    let (x0, x1, y0, y1) = BAR;
    let w = p.volume.min(100) as i32 * (x1 - x0 + 1) / 100;
    if w > 0 {
        fb.rect(x0, y0, x0 + w - 1, y1, true);
        if p.muted && w > 2 {
            fb.rect(x0 + 1, y0 + 1, x0 + w - 2, y1 - 1, false);
        }
    }
    let t0 = x0 + w + if w > 0 { 2 } else { 0 };
    if t0 <= x1 {
        fb.rect(t0, (y0 + y1) / 2, x1, (y0 + y1) / 2, true);
    }
    if let Some(top) = stop {
        let x = if top { x1 + 2 } else { 0 };
        fb.rect(x, y0 - 1, x + 1, y1 + 1, true);
    }
}

/// What one screen frame needs from the shared state.
pub struct Input<T> {
    pub alive: bool,
    pub mixer: Mixer,
    pub popup: Option<Popup<T>>,
    /// Milliseconds since the last activity.
    pub idle: u64,
    pub name: [u8; kb16_mixer_protocol::LOGO_NAME_LEN],
    /// Meter levels after the lights' ballistics, 0..=255.
    pub meters: [u8; NCH],
}

/// The panel contrast for this frame: `want`, faded out with the lights over the `FADE_MS`
/// after `off_min` minutes of quiet (a smoothstep through the lights' gamma), then 0.
pub fn contrast(want: u8, idle: u64, off_min: u8) -> u8 {
    if off_min == NEVER {
        return want;
    }
    let past = idle.saturating_sub(off_min as u64 * 60_000).min(FADE_MS as u64);
    let t = past * 65536 / FADE_MS as u64;
    let eased = (t * t * (3 * 65536 - 2 * t)) >> 32;
    let left = (65536 - eased).min(0xFFFF) as u32;
    (want as u32 * gamma(left) / 0xFFFF) as u8
}

/// Everything the screen remembers from one frame to the next.
pub struct Screen<T> {
    /// Each channel's sound on its rail, in pixels.
    mpx: [u8; NCH],
    /// The end stop: when it came up (wrapping ms) and whether it is at the top of the range.
    wall: Option<(u32, bool)>,
    /// The readout message on screen, to tell a repeated detent from a new value.
    last: Option<Popup<T>>,
    /// When the logo came on; `None` while it is off.
    logo_at: Option<T>,
}

impl<T: Time> Screen<T> {
    /// The screen at power-up, when the logo comes on.
    pub fn new(now: T) -> Screen<T> {
        Screen { mpx: [0; NCH], wall: None, last: None, logo_at: Some(now) }
    }

    /// Draws one frame into `fb`. `since_boot` is in milliseconds. Returns true for the idle
    /// logo, which may be drawn at half rate and with the panel dimmed.
    pub fn draw(&mut self, fb: &mut Fb, now: T, since_boot: u64, s: &Input<T>) -> bool {
        let clock = now.ms() as u32;
        self.sound(&s.mixer, &s.meters);
        let popup = s.popup.filter(|p| s.alive && now.ms_since(p.at) < POPUP_MS);
        self.end_stop(clock, popup);

        fb.clear();
        if since_boot < SPLASH_MS {
            draw_logo(fb, since_boot, &s.name);
            return false;
        }
        let idle = popup.is_none() && !(s.alive && s.idle < SCREENSAVER_MS);
        if !idle {
            self.logo_at = None;
        }
        match popup {
            Some(p) if p.volume == NOTICE => draw_notice(fb, trimmed(&p.name)),
            Some(p) => {
                let stop = self.wall.filter(|&(at, _)| clock.wrapping_sub(at) < WALL_MS).map(|(_, top)| top);
                draw_readout(fb, &p, stop);
            }
            None if !idle => draw_mixer(fb, &s.mixer, &self.mpx),
            None => {
                let at = *self.logo_at.get_or_insert(now);
                draw_logo(fb, now.ms_since(at), &s.name);
            }
        }
        idle
    }

    /// Each channel's sound through its fader, in pixels of its rail: it follows the lights'
    /// meter, which already rises at once and falls slowly, and moves only when the level is
    /// half a pixel past the edge, so a level on an edge does not flicker.
    fn sound(&mut self, m: &Mixer, levels: &[u8; NCH]) {
        for (c, px) in self.mpx.iter_mut().enumerate() {
            let len = rail(m.volume[c]);
            let halves = if m.mutes & 1 << c != 0 { 0 } else { levels[c] as i32 * len * 2 / 255 };
            let mut p = *px as i32;
            if halves >= 2 * (p + 1) {
                p = halves / 2;
            } else if halves < 2 * p - 1 {
                p = (halves + 1) / 2;
            }
            *px = p.min(len) as u8;
        }
    }

    /// A new readout message that repeats the one on screen at 0 or 100 is a detent that could
    /// not move the value: it raises the end stop. Any other message clears it.
    fn end_stop(&mut self, clock: u32, popup: Option<Popup<T>>) {
        let Some(p) = popup else {
            self.last = None;
            return;
        };
        if self.last.is_some_and(|l| l.at.ms() == p.at.ms()) {
            return;
        }
        let repeat = self.last.is_some_and(|l| l.name == p.name && l.volume == p.volume && !l.muted)
            && !p.muted
            && (p.volume == 0 || p.volume == 100);
        self.wall = repeat.then_some((clock, p.volume == 100));
        self.last = Some(p);
    }
}

/// An unchanged run inside a page longer than this is cheaper to skip than to send: a second
/// window costs a 6-byte address command and a new transfer, about as much as 10 columns.
const SPAN_GAP: usize = 10;

/// The changed columns of display page `page` (0..=3): the first and the last column, both
/// included, where `buf` differs from `sent`, or `None` when the page is unchanged. Both are
/// frames of 512 bytes in panel order, without the I2C control byte (`&fb.buf[1..]`).
pub fn dirty_span(page: u8, buf: &[u8], sent: &[u8]) -> Option<(u8, u8)> {
    let [first, second] = dirty_spans(page, buf, sent);
    Some((first?.0, second.or(first)?.1))
}

/// Like `dirty_span`, but split in two where the unchanged gap between changed columns is
/// longer than `SPAN_GAP`: then the second span is `Some`. The split is at the longest gap.
pub fn dirty_spans(page: u8, buf: &[u8], sent: &[u8]) -> [Option<(u8, u8)>; 2] {
    const COLS: usize = W as usize;
    let page = page as usize;
    let (Some(a), Some(b)) = (buf.as_chunks::<COLS>().0.get(page), sent.as_chunks::<COLS>().0.get(page)) else {
        return [None; 2];
    };
    let mut changed = (0..COLS).filter(|&x| a[x] != b[x]);
    let Some(first) = changed.next() else { return [None; 2] };
    // The last changed column, and the longest unchanged run between two changed columns.
    let (mut last, mut gap) = (first, (0, first, first));
    for x in changed {
        if x - last - 1 > gap.0 {
            gap = (x - last - 1, last, x);
        }
        last = x;
    }
    if gap.0 > SPAN_GAP {
        [Some((first as u8, gap.1 as u8)), Some((gap.2 as u8, last as u8))]
    } else {
        [Some((first as u8, last as u8)), None]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(changed: &[usize]) -> ([u8; 512], [u8; 512]) {
        let mut buf = [0u8; 512];
        for &i in changed {
            buf[i] = 1;
        }
        (buf, [0u8; 512])
    }

    #[test]
    fn dirty_span_finds_the_first_and_last_changed_column_of_one_page() {
        let (buf, sent) = frames(&[128 + 5, 128 + 90, 3 * 128 + 127]);
        assert_eq!(dirty_span(0, &buf, &sent), None);
        assert_eq!(dirty_span(1, &buf, &sent), Some((5, 90)));
        assert_eq!(dirty_span(2, &buf, &sent), None);
        assert_eq!(dirty_span(3, &buf, &sent), Some((127, 127)));
        assert_eq!(dirty_span(4, &buf, &sent), None, "no such page");
        assert_eq!(dirty_span(0, &buf[..100], &sent), None, "a short frame has no page");
        let (buf, sent) = frames(&[0, 127]);
        assert_eq!(dirty_span(0, &buf, &sent), Some((0, 127)));
    }

    #[test]
    fn dirty_spans_split_only_when_the_gap_is_over_ten_columns() {
        let (buf, sent) = frames(&[20, 31]); // a gap of 10: one span
        assert_eq!(dirty_spans(0, &buf, &sent), [Some((20, 31)), None]);
        let (buf, sent) = frames(&[20, 32]); // a gap of 11: two
        assert_eq!(dirty_spans(0, &buf, &sent), [Some((20, 20)), Some((32, 32))]);
        // Split at the longest gap.
        let (buf, sent) = frames(&[2, 14, 15, 60, 61, 64]);
        assert_eq!(dirty_spans(0, &buf, &sent), [Some((2, 15)), Some((60, 64))]);
        assert_eq!(dirty_span(0, &buf, &sent), Some((2, 64)));
        let (buf, sent) = frames(&[7]);
        assert_eq!(dirty_spans(0, &buf, &sent), [Some((7, 7)), None]);
        assert_eq!(dirty_spans(1, &buf, &sent), [None, None]);
    }

    fn mixer() -> Mixer {
        Mixer {
            n: 4,
            colors: [[255; 3]; NCH],
            level: [0; NCH],
            volume: [80, 55, 100, 30],
            mutes: 0b0100,
            selected: 0,
            mixer_keys: true,
            mic_muted: false,
            playing: true,
            second_page: false,
            names: [*b"Game  ", *b"Music ", *b"Chat  ", *b"Web   "],
        }
    }

    fn input(meters: [u8; NCH]) -> Input<u64> {
        Input { alive: true, mixer: mixer(), popup: None, idle: 0, name: *b"KB16\0\0\0\0\0\0", meters }
    }

    fn popup(at: u64, volume: u8, text: &[u8]) -> Option<Popup<u64>> {
        let mut name = [0; 16];
        name[..text.len()].copy_from_slice(text);
        Some(Popup { at, volume, muted: false, name })
    }

    fn with_popup(p: Option<Popup<u64>>) -> Input<u64> {
        Input { popup: p, ..input([0; NCH]) }
    }

    const T0: u64 = 100_000;

    /// One frame of `screen` at `ms`, well after the boot splash.
    fn frame(screen: &mut Screen<u64>, ms: u64, s: &Input<u64>) -> Fb {
        let mut fb = Fb::new(3);
        screen.draw(&mut fb, ms, ms, s);
        fb
    }

    /// What a screen that has drawn nothing before draws for this input.
    fn fresh(ms: u64, s: &Input<u64>) -> Fb {
        frame(&mut Screen::new(ms), ms, s)
    }

    fn lit_in(fb: &Fb, xs: core::ops::RangeInclusive<i32>, ys: core::ops::RangeInclusive<i32>) -> bool {
        xs.clone().any(|x| ys.clone().any(|y| fb.get(x, y)))
    }

    #[test]
    fn a_readout_is_complete_on_the_frame_of_its_message() {
        let mut screen = Screen::new(T0);
        frame(&mut screen, T0, &input([0; NCH]));
        let s = with_popup(popup(T0 + 33, 64, b"Music"));
        let fb = frame(&mut screen, T0 + 33, &s);
        assert_eq!(fb.buf, fresh(T0 + 33, &s).buf);
        assert!(lit_in(&fb, 72..=127, 3..=26), "the number");
        assert!(fb.get(3, 22) && fb.get(3 + 62 * 64 / 100 - 1, 26), "the bar");
        assert!(fb.get(64, 24), "the track to the end of the range");
        // It stays 39 frames after its message, then the mixer cuts back.
        let end = T0 + 33 + POPUP_MS;
        assert_eq!(frame(&mut screen, end - 1, &s).buf, fb.buf);
        assert_eq!(frame(&mut screen, end, &s).buf, fresh(end, &input([0; NCH])).buf);
    }

    #[test]
    fn the_bar_and_the_number_change_on_the_same_frame() {
        let mut screen = Screen::new(T0);
        let a = frame(&mut screen, T0, &with_popup(popup(T0, 40, b"Music")));
        let s = with_popup(popup(T0 + 33, 42, b"Music"));
        let b = frame(&mut screen, T0 + 33, &s);
        assert_eq!(b.buf, fresh(T0 + 33, &s).buf);
        let differs = |xs: core::ops::RangeInclusive<i32>, ys: core::ops::RangeInclusive<i32>| {
            xs.clone().any(|x| ys.clone().any(|y| a.get(x, y) != b.get(x, y)))
        };
        assert!(differs(3..=64, 22..=26), "the bar moved");
        assert!(differs(72..=127, 3..=20), "the number changed");
    }

    #[test]
    fn a_repeat_at_the_end_of_the_range_shows_the_stop_for_three_frames() {
        let stop_at = |fb: &Fb, x: i32| (21..=27).all(|y| fb.get(x, y) && fb.get(x + 1, y));
        for (volume, x) in [(100, 66), (0, 0)] {
            let mut screen = Screen::new(T0);
            let first = frame(&mut screen, T0, &with_popup(popup(T0, volume, b"Music")));
            assert!(!stop_at(&first, x), "the first message at {volume} is a plain readout");
            let again = with_popup(popup(T0 + 33, volume, b"Music"));
            for f in 0..3 {
                assert!(stop_at(&frame(&mut screen, T0 + 33 * (f + 1), &again), x), "{volume}: frame {f}");
            }
            assert!(!stop_at(&frame(&mut screen, T0 + 33 * 4, &again), x), "{volume}: gone on frame 3");
            assert_eq!(frame(&mut screen, T0 + 33 * 5, &again).buf, first.buf);
        }

        // A mute, another name, or a value that moved shows no stop.
        let muted = |at, name: &[u8]| {
            let mut p = popup(at, 100, name);
            if let Some(p) = p.as_mut() {
                p.muted = true;
            }
            with_popup(p)
        };
        let cases: [[Input<u64>; 2]; 3] = [
            [muted(T0, b"Music"), muted(T0 + 33, b"Music")],
            [with_popup(popup(T0, 100, b"Music")), with_popup(popup(T0 + 33, 100, b"Chat"))],
            [with_popup(popup(T0, 98, b"Music")), with_popup(popup(T0 + 33, 100, b"Music"))],
        ];
        for [a, b] in &cases {
            let mut screen = Screen::new(T0);
            frame(&mut screen, T0, a);
            assert!(!stop_at(&frame(&mut screen, T0 + 33, b), 66));
        }
        let mut screen = Screen::new(T0);
        frame(&mut screen, T0, &with_popup(popup(T0, 100, b"Music")));
        frame(&mut screen, T0 + 33, &with_popup(popup(T0 + 33, 100, b"Music")));
        let moved = frame(&mut screen, T0 + 66, &with_popup(popup(T0 + 66, 98, b"Music")));
        assert!(!stop_at(&moved, 66), "a moved value clears the stop");
    }

    #[test]
    fn sound_alone_changes_only_the_rail_page_for_every_top() {
        for top in 0..=6 {
            let mut screen = Screen::new(T0);
            let mut fb = Fb::new(top);
            screen.draw(&mut fb, T0, T0, &input([0; NCH]));
            let mut sent = [0u8; 512];
            sent.copy_from_slice(&fb.buf[1..]);
            for (i, meters) in [[255, 200, 100, 50], [30, 255, 0, 255], [0; NCH]].into_iter().enumerate() {
                let mut fb = Fb::new(top);
                screen.draw(&mut fb, T0 + 33 * (i as u64 + 1), T0, &input(meters));
                for page in 0..3 {
                    assert_eq!(dirty_span(page, &fb.buf[1..], &sent), None, "top {top}, page {page}");
                }
                assert!(dirty_span(3, &fb.buf[1..], &sent).is_some(), "top {top}: the sound shows");
                sent.copy_from_slice(&fb.buf[1..]);
            }
        }
    }

    #[test]
    fn the_sound_never_passes_the_rail_and_does_not_flicker_on_a_pixel_edge() {
        for volume in [1, 28, 50, 77, 100] {
            let m = Mixer { volume: [volume; NCH], mutes: 0, ..mixer() };
            for level in 0..255u8 {
                let mut screen = Screen::<u64>::new(T0);
                let mut shown = [0; NCH];
                for f in 0..24 {
                    // Two levels on the two sides of a pixel edge, in turn.
                    let l = if f % 2 == 0 { level } else { level + 1 };
                    screen.sound(&m, &[l; NCH]);
                    assert!(screen.mpx[0] as i32 <= rail(volume), "within the rail");
                    if f == 4 {
                        shown = screen.mpx;
                    }
                    if f > 4 {
                        assert_eq!(screen.mpx, shown, "volume {volume}, level {level}");
                    }
                }
            }
        }
        // Up at once, all the way.
        let mut screen = Screen::<u64>::new(T0);
        screen.sound(&Mixer { volume: [100; NCH], mutes: 0, ..mixer() }, &[255; NCH]);
        assert_eq!(screen.mpx, [28; NCH]);
    }

    #[test]
    fn a_muted_channel_shows_mute_and_no_line() {
        let fb = fresh(T0, &input([255; NCH]));
        let chat = 2 * 32;
        assert!(!lit_in(&fb, chat..=chat + 31, 21..=28), "no rail and no sound");
        assert!(lit_in(&fb, chat + 5..=chat + 27, 14..=19), "MUTE");
        assert!(!lit_in(&fb, chat..=chat + 31, 9..=13), "no value");
        // The others have both, and the sound fills the rail as far as the level goes.
        assert!(fb.get(2, 24) && fb.get(2, 25) && fb.get(1 + rail(80), 24));
    }

    #[test]
    fn focus_inverts_only_the_label() {
        let fb = fresh(T0, &input([0; NCH]));
        // "GAME" is 23 px wide, centred in 0..31 from x 4: the pill is 2..28 on L1..8.
        assert!(fb.get(2, 2) && !fb.get(2, 1) && fb.get(28, 7));
        assert!(!lit_in(&fb, 32..=63, 0..=1), "the others are plain");
        // The page-2 tick and the microphone tick sit at the edges, on L14..18.
        let m = Mixer { second_page: true, mic_muted: true, ..mixer() };
        let fb = fresh(T0, &Input { mixer: m, ..input([0; NCH]) });
        assert!((14..=18).all(|y| fb.get(0, y) && fb.get(1, y) && fb.get(126, y) && fb.get(127, y)));
    }

    #[test]
    fn a_long_name_ends_8_px_before_the_number() {
        for muted in [false, true] {
            let mut p = popup(T0, 100, b"Averyverylongnam");
            if let Some(p) = p.as_mut() {
                p.muted = muted;
            }
            let fb = fresh(T0, &with_popup(p));
            let left = if muted { W - Font::Big.width(b"Muted") } else { 72 };
            let end = (0..left).rev().find(|&x| lit_in(&fb, x..=x, 0..=20)).unwrap_or(0);
            assert!(end <= left - 8, "{end} ends 8 px before {left}");
            assert!(end > left - 8 - 12, "{end}: cut by one letter at a time");
            assert!(lit_in(&fb, left..=left + 3, 3..=26), "the number or Muted starts at {left}");
        }
    }

    #[test]
    fn a_long_notice_uses_the_13_px_font() {
        let long = b"Spotify > Media";
        assert!(Font::Big.width(long) > W - 4);
        let mut expected = Fb::new(3);
        expected.write(Font::Mid, (W - Font::Mid.width(long)) / 2, 9, long);
        let s = with_popup(popup(T0, NOTICE, long));
        assert_eq!(fresh(T0, &s).buf, expected.buf);

        let short = b"KEYS";
        let mut expected = Fb::new(3);
        expected.write(Font::Big, (W - Font::Big.width(short)) / 2, 7, short);
        assert_eq!(fresh(T0, &with_popup(popup(T0, NOTICE, short))).buf, expected.buf);
    }

    #[test]
    fn the_idle_logo_wipes_in_and_streaks_once() {
        let mut screen = Screen::new(T0);
        frame(&mut screen, T0, &input([0; NCH]));
        let quiet = Input { idle: SCREENSAVER_MS, ..input([0; NCH]) };
        let start = T0 + 33;
        let mut fb = Fb::new(3);
        assert!(screen.draw(&mut fb, start, start, &quiet), "the idle logo is calm");
        assert!(!lit_in(&fb, 2..=W - 1, -3..=28), "it starts black");
        let letters =
            |fb: &Fb| -> [bool; (W * 24) as usize] { core::array::from_fn(|i| fb.get(i as i32 % W, i as i32 / W)) };
        let rest = letters(&frame(&mut screen, start + 1000, &quiet));
        assert!(rest.iter().any(|&on| on));
        assert_ne!(letters(&frame(&mut screen, start + 500, &quiet)), rest, "the streak crosses");
        for ms in (1000..30_000).step_by(100) {
            assert_eq!(letters(&frame(&mut screen, start + ms, &quiet)), rest, "no second streak at {ms}");
        }
        // A touch cuts to the mixer.
        let back = frame(&mut screen, start + 30_000, &input([0; NCH]));
        assert_eq!(back.buf, fresh(start + 30_000, &input([0; NCH])).buf);
    }

    #[test]
    fn the_boot_logo_runs_on_without_a_second_wipe() {
        let none = Input { alive: false, ..input([0; NCH]) };
        let mut screen = Screen::new(0u64);
        let mut fb = Fb::new(3);
        assert!(!screen.draw(&mut fb, 100, 100, &none), "the splash is not calm");
        screen.draw(&mut fb, SPLASH_MS - 1, SPLASH_MS - 1, &none);
        let splash = fb.buf;
        assert!(screen.draw(&mut fb, SPLASH_MS, SPLASH_MS, &none));
        assert!(lit_in(&fb, 64..=W - 1, 0..=23), "already wiped in");
        // The letters stay; only the wave moves.
        let top = |b: &[u8; 513]| b[1..1 + 2 * W as usize].to_vec();
        assert_eq!(top(&fb.buf), top(&splash));
    }

    #[test]
    fn the_contrast_fades_with_the_lights_after_the_limit() {
        let off = 60_000;
        assert_eq!(contrast(0x30, 0, 1), 0x30);
        assert_eq!(contrast(0x30, off, 1), 0x30, "full up to the limit");
        assert_eq!(contrast(0xCF, off + FADE_MS as u64, 1), 0);
        assert_eq!(contrast(0xCF, off * 10, 1), 0);
        let mut last = 0xCF;
        for idle in off..=off + FADE_MS as u64 {
            let c = contrast(0xCF, idle, 1);
            assert!(c <= last, "monotone");
            last = c;
        }
        // Through the lights' gamma: half way in time is well under half the light.
        assert!(contrast(0xCF, off + FADE_MS as u64 / 2, 1) < 0xCF / 4);
        assert_eq!(contrast(0xCF, off * 10, NEVER), 0xCF, "never sleeps, never fades");
    }
}
