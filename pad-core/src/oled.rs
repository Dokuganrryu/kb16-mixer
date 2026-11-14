//! What the 128x32 screen shows: the frame buffer, the 1-bit drawing, and the choice of screen.
//! The firmware's `oled.rs` sends the buffer to the SSD1306 panel.
//!
//! One motion engine serves three screen styles (`state::STYLE_*`, chosen in the PC app):
//! a readout pushes the screen beneath it up and leaves the same way, its bar glides to the
//! value a little behind the number, meters glide with a short hold, a page change slides
//! sideways (a quick change back only nudges), and a mute change inverts the channel name for
//! 200 ms. A style decides only how a mixer column and a readout look:
//! - Classic: name, 7-segment meter with a peak mark, dotted fader, percent per column.
//! - Lean: name, one solid fader with a thin track, a 1 px meter line, no percent.
//! - Numbers: label, the volume in the 13 px font, a rail that swells with the sound.
//!
//! The case hides the panel's top rows, so every screen is laid out for 29 visible rows and
//! drawn `top` pixels lower (set from the PC app's config, default 3). The layout rows below
//! are those of the layout; the panel row is `top` lower.

use crate::Time;
use crate::assets::{BIG_COLS, BIG_OFF, LOGO_CHARS, LOGO_COLS, LOGO_GAP, LOGO_OFF, MID_COLS, MID_OFF, SIN};
use crate::font::FONT5X7;
use crate::leds::{DIM_MS, WAKE_MS, ease_out, gamma, progress, smoothstep};
use crate::state::{Mixer, NCH, POPUP_MS, Popup, STYLE_CLASSIC, STYLE_LEAN, STYLE_NUMBERS};
use kb16_mixer_protocol::{NAME_SHOWN, NEVER, NOTICE};

pub const W: i32 = 128;
pub const H: i32 = 32;
/// Rows every layout fits in (32 minus the default top margin).
const ROWS: i32 = 29;
/// How long the logo shows after power-up.
pub const SPLASH_MS: u64 = 1800;
const SCREENSAVER_MS: u64 = 45_000;
/// The logo wipes in over this long (27 frames), after power-up and when the screensaver starts.
const WIPE_MS: u64 = 900;
/// A detent that cannot move the value past 0 or 100 shows an end stop for three 33 ms frames.
const WALL_MS: u32 = 99;
/// A readout pushes the screen beneath it up and out; it leaves faster than it came.
const ENTER_MS: u32 = 100;
const EXIT_MS: u32 = 70;
const _: () = assert!(EXIT_MS < ENTER_MS);
/// How far a readout travels: far enough that it and the screen it pushes never overlap.
const PUSH: i32 = 32;
/// The time between two frames while the screen moves.
const FRAME_MS: u32 = 33;
/// A page change slides the screen sideways, in from the side of the new page.
const SLIDE: i32 = 24;
const SLIDE_MS: u32 = 140;
/// A page change back within this time is the end of a held page key (a peek) or a quick
/// double tap: it only nudges, as the user goes back to what they just saw.
const PEEK_WINDOW_MS: u32 = 1500;
const PEEK: i32 = 4;
const PEEK_MS: u32 = 60;
/// A channel name stays inverted this long after its mute changes.
const MUTE_FLASH_MS: u32 = 200;
/// Frames a meter waits before it drops one step, so a level on a step edge does not flicker.
/// A drop of two or more steps shows at once.
const METER_HOLD: u8 = 6;
/// Meter positions are in eighths of a pixel column.
const SUB: u16 = 8;
/// The SSD1306 control byte that says "display data follows".
const DATA: u8 = 0x40;

/// Lean mixer: the fader on L20..22 with its track on L21, the meter line on L24 (one display
/// page for every `top` 0..=6), `MUTE` in a pill on L18..26.
const LEAN_BAR_Y: (i32, i32) = (20, 22);
const LEAN_METER_Y: i32 = 24;
const LEAN_MUTE_Y: i32 = 18;
/// Lean readout: name and `%` in 5x7 on L6..12, the number in the 16 px font (digits L1..12),
/// the bar on L19..23 from x 3 to 124, end stops just outside it.
const LEAN_TEXT_Y: i32 = 6;
const LEAN_BAR: (i32, i32, i32, i32) = (3, 124, 19, 23);

/// Numbers mixer: 5x7 label on L2..7 (Focus pill L1..8), the value in the mid font on L12..20
/// (or `MUTE` on L14..19), and the rail on L25, 2 px thick (L24..25) as far as the sound goes.
/// The rail and the sound stay in one display page for every `top` (0..=6).
const LABEL_Y: i32 = 1;
const VALUE_Y: i32 = 11;
const MUTE_Y: i32 = 13;
const RAIL_Y: i32 = 25;
/// The page-2 tick (right edge) and the microphone-muted tick (left edge), on L14..18.
const TICK_Y: (i32, i32) = (14, 18);
/// Numbers readout: the name in the mid font on L3..11, the number (or `Muted`) in the 16 px
/// font on L0..11, the bar on L22..26 from x 3 to 124.
const NAME_Y: i32 = 2;
const NUMBER_Y: i32 = -1;
const BAR: (i32, i32, i32, i32) = (3, 124, 22, 26);

pub struct Fb {
    /// Byte 0 is the I2C "data" control byte so the buffer goes out in one transfer. Then one
    /// byte per column and 8-row page, LSB on top, as the SSD1306 takes it.
    pub buf: [u8; 1 + (W * H / 8) as usize],
    /// Vertical offset applied to everything drawn.
    pub top: i32,
    /// Sideways offset applied to everything drawn (a page change slides the frame).
    left: i32,
}

impl Fb {
    pub fn new(top: i32) -> Fb {
        let mut buf = [0; 1 + (W * H / 8) as usize];
        buf[0] = DATA;
        Fb { buf, top, left: 0 }
    }

    fn clear(&mut self) {
        self.buf[1..].fill(0);
    }

    /// Whether the pixel at (`x`, `y`) of the drawn layout is lit (`top` applies).
    pub fn get(&self, x: i32, y: i32) -> bool {
        let (x, y) = (x + self.left, y + self.top);
        (0..W).contains(&x) && (0..H).contains(&y) && self.buf[1 + (y / 8 * W + x) as usize] & 1 << (y % 8) != 0
    }

    fn px(&mut self, x: i32, y: i32, on: bool) {
        let (x, y) = (x + self.left, y + self.top);
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

    /// Sets the box from (`x0`, `y0`) to (`x1`, `y1`), both included, one byte per column and
    /// display page instead of one pixel at a time.
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, on: bool) {
        let (x0, x1) = ((x0 + self.left).max(0), (x1 + self.left).min(W - 1));
        let (y0, y1) = ((y0 + self.top).max(0), (y1 + self.top).min(H - 1));
        if x0 > x1 || y0 > y1 {
            return;
        }
        for page in y0 / 8..=y1 / 8 {
            let lo = (y0 - page * 8).max(0);
            let hi = (y1 - page * 8).min(7);
            let mask = (0xFFu8 << lo) & (0xFFu8 >> (7 - hi));
            let row = 1 + (page * W) as usize;
            for b in &mut self.buf[row + x0 as usize..=row + x1 as usize] {
                if on {
                    *b |= mask;
                } else {
                    *b &= !mask;
                }
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

    /// The pill's edge only, 1 px wide, with the same cut corners.
    fn outline(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.rect(x0 + 1, y0, x1 - 1, y0, true);
        self.rect(x0 + 1, y1, x1 - 1, y1, true);
        self.rect(x0, y0 + 1, x0, y1 - 1, true);
        self.rect(x1, y0 + 1, x1, y1 - 1, true);
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

    /// 5x7 text centred in the 31 px wide strip starting at `x0`.
    fn centred(&mut self, x0: i32, y: i32, s: &[u8], on: bool) {
        self.text(x0 + (31 - (s.len() as i32 * 6 - 1)) / 2, y, s, on);
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

/// The proportional fonts: Open Sans SemiBold 16 and 13, and the logo's ExtraBold letters.
#[derive(Clone, Copy)]
enum Font {
    Big,
    Mid,
    Logo,
}

impl Font {
    fn gap(self) -> i32 {
        match self {
            Font::Big => 2,
            Font::Mid => 1,
            Font::Logo => LOGO_GAP,
        }
    }

    /// The columns of `ch` in the font's table. The logo font lacks most characters; those
    /// have no columns.
    fn cols(self, ch: u8) -> core::ops::Range<usize> {
        let i = (ch.clamp(32, 126) - 32) as usize;
        let (a, b) = match self {
            Font::Big => (BIG_OFF[i], BIG_OFF[i + 1]),
            Font::Mid => (MID_OFF[i], MID_OFF[i + 1]),
            Font::Logo => match LOGO_CHARS.iter().position(|&l| l == ch) {
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
            Font::Logo => u32::from_le_bytes(LOGO_COLS[k]),
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
fn number(v: u8, out: &mut [u8; 4]) -> &[u8] {
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

/// A volume as digits and a percent sign.
fn percent(v: u8, out: &mut [u8; 4]) -> &[u8] {
    let n = number(v, out).len();
    out[n] = b'%';
    &out[..n + 1]
}

/// The Numbers rail's length in pixels for a volume: 28 at 100.
fn rail(volume: u8) -> i32 {
    volume.min(100) as i32 * 28 / 100
}

/// Steps lit (0..=`steps`) of a meter for a level 0..=255.
fn steps_lit(level: u8, steps: u8) -> u8 {
    (level as u32 * steps as u32).div_ceil(255).min(steps as u32) as u8
}

/// A style's meter for channel `c`: how many steps it has, and how many pixel columns a step is.
fn meter_scale(style: u8, m: &Mixer, c: usize) -> (u8, u16) {
    match style {
        STYLE_LEAN => (27, 1),
        STYLE_NUMBERS => (rail(m.volume[c]) as u8, 1),
        _ => (7, 3),
    }
}

/// The name in the logo letters, centred, and a slow wave beneath. `age` is the time since the
/// logo came on: it wipes in from the left over `WIPE_MS`, a light streak crosses it once, and
/// the wave takes about 9 s a period. A name too wide for the logo letters uses the big font,
/// and one too wide even for that loses letters from the end.
fn draw_logo(fb: &mut Fb, age: u64, name: &[u8]) {
    let name = trimmed(name);
    if Font::Logo.width(name) <= W {
        fb.write(Font::Logo, (W - Font::Logo.width(name)) / 2, 0, name);
    } else {
        let name = Font::Big.fit(name, W);
        fb.write(Font::Big, (W - Font::Big.width(name)) / 2, 4, name);
    }
    let streak = (age.min(10_000) / 5) as i32 - 30;
    if streak < W + 40 {
        draw_streak(fb, streak);
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

/// The light streak at `streak`: a dark band 4 px wide and a 1 px line after a 2 px gap,
/// slanted one column per two rows. Only its own columns are visited on each row.
fn draw_streak(fb: &mut Fb, streak: i32) {
    for y in 0..H {
        let x = streak + y / 2;
        for d in [0, 1, 2, 3, 6] {
            if (0..W).contains(&(x + d)) {
                fb.px(x + d, y, false);
            }
        }
    }
}

/// Scribble strip: one 32 px column per channel, in the look of `style`. `cols` is each
/// meter's lit width in pixel columns, `peaks` the levels the Classic peak marks hold, and
/// `flash` has a bit set for each channel whose name shows inverted.
fn draw_mixer(fb: &mut Fb, style: u8, m: &Mixer, cols: &[u8; NCH], peaks: &[u8; NCH], flash: u8) {
    for c in 0..(m.n as usize).min(NCH) {
        match style {
            STYLE_LEAN => lean_column(fb, m, c, cols[c], flash),
            STYLE_NUMBERS => numbers_column(fb, m, c, cols[c]),
            _ => classic_column(fb, m, c, cols[c], peaks[c], flash),
        }
    }
    if style == STYLE_NUMBERS {
        if m.mic_muted {
            fb.rect(0, TICK_Y.0, 1, TICK_Y.1, true);
        }
        if m.second_page {
            fb.rect(W - 2, TICK_Y.0, W - 1, TICK_Y.1, true);
        }
    } else {
        // Mic muted: a rule under the names for as long as it lasts.
        if m.mic_muted {
            fb.rect(0, 9, W - 2, 9, true);
        }
        // Second key page: a short tick in the right-most pixel column, which nothing else uses.
        if m.second_page {
            fb.rect(W - 1, 2, W - 1, 6, true);
        }
    }
}

/// Classic and Lean: the name on L1..7, in a pill (L0..8) for the selected channel; a mute
/// change flips that for a moment. The selected channel keeps the pill's outline while it
/// flashes, so the selection stays in view.
fn pill_name(fb: &mut Fb, m: &Mixer, c: usize, flash: u8) {
    let x0 = c as i32 * 32;
    let name = trimmed(&m.names[c]);
    let name = &name[..name.len().min(NAME_SHOWN)];
    match (c as u8 == m.selected, flash & 1 << c != 0) {
        (true, false) | (false, true) => {
            fb.pill(x0, 0, x0 + 30, 8);
            fb.centred(x0, 1, name, false);
        }
        (true, true) => {
            fb.outline(x0, 0, x0 + 30, 8);
            fb.centred(x0, 1, name, true);
        }
        (false, false) => fb.centred(x0, 1, name, true),
    }
}

/// Classic: name 0..8, live meter 11..14, fader 17..19, value 21..28.
fn classic_column(fb: &mut Fb, m: &Mixer, c: usize, cols: u8, peak: u8, flash: u8) {
    let x0 = c as i32 * 32;
    let muted = m.mutes & (1 << c) != 0;
    pill_name(fb, m, c, flash);

    // Live meter: 7 segments of 3 columns with a peak-hold marker; while the meter glides,
    // a width between segments lights part of the next one.
    let lit = cols as i32 / 3;
    let part = cols as i32 % 3;
    let peak = steps_lit(peak, 7) as i32;
    for s in 0..7 {
        let sx = x0 + 2 + s * 4;
        if !muted && s < lit {
            fb.rect(sx, 11, sx + 2, 14, true);
        } else if !muted && s == lit && part > 0 {
            fb.rect(sx, 11, sx + part - 1, 14, true);
        } else if !muted && s == peak - 1 {
            fb.rect(sx, 11, sx + 2, 11, true);
            fb.rect(sx, 14, sx + 2, 14, true);
        } else {
            fb.px(sx + 1, 14, true);
        }
    }

    // Fader: dotted track, solid fill.
    for x in (x0 + 2..=x0 + 28).step_by(2) {
        fb.px(x, 18, true);
    }
    let w = m.volume[c] as i32 * 27 / 100;
    if w > 0 {
        fb.rect(x0 + 2, 17, x0 + 1 + w, 19, true);
    }

    if muted {
        fb.pill(x0 + 2, 21, x0 + 28, ROWS - 1);
        fb.centred(x0, 22, b"MUTE", false);
    } else {
        let mut tmp = [0u8; 4];
        fb.centred(x0, 22, percent(m.volume[c], &mut tmp), true);
    }
}

/// Lean: the name, open rows, then one solid fader with a thin track and the meter line under it.
fn lean_column(fb: &mut Fb, m: &Mixer, c: usize, cols: u8, flash: u8) {
    let x0 = c as i32 * 32;
    pill_name(fb, m, c, flash);
    if m.mutes & (1 << c) != 0 {
        fb.pill(x0 + 2, LEAN_MUTE_Y, x0 + 28, LEAN_MUTE_Y + 8);
        fb.centred(x0, LEAN_MUTE_Y + 1, b"MUTE", false);
        return;
    }
    let (y0, y1) = LEAN_BAR_Y;
    let w = m.volume[c] as i32 * 27 / 100;
    if w > 0 {
        fb.rect(x0 + 2, y0, x0 + 1 + w, y1, true);
    }
    let t0 = x0 + 2 + w + if w > 0 { 1 } else { 0 };
    if t0 <= x0 + 28 {
        fb.rect(t0, (y0 + y1) / 2, x0 + 28, (y0 + y1) / 2, true);
    }
    if cols > 0 {
        fb.rect(x0 + 2, LEAN_METER_Y, x0 + 1 + cols as i32, LEAN_METER_Y, true);
    }
}

/// Numbers: the upper-case label (Focus inverts it), the value, the rail. A muted channel shows
/// plain `MUTE` and no rail.
fn numbers_column(fb: &mut Fb, m: &Mixer, c: usize, sound: u8) {
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
        return;
    }
    let mut digits = [0u8; 4];
    let v = number(m.volume[c], &mut digits);
    fb.write(Font::Mid, x0 + (33 - Font::Mid.width(v)) / 2, VALUE_Y, v);
    let len = rail(m.volume[c]);
    if len > 0 {
        fb.rect(x0 + 2, RAIL_Y, x0 + 1 + len, RAIL_Y, true);
    }
    let sound = (sound as i32).min(len);
    if sound > 0 {
        fb.rect(x0 + 2, RAIL_Y - 1, x0 + 1 + sound, RAIL_Y, true);
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

/// The readout of one knob or key in the look of `style`. `shown` is where the bar is (volume
/// x10, gliding), `stop` shows the end stop at the top (true) or the bottom of the range.
fn draw_readout<T>(fb: &mut Fb, style: u8, p: &Popup<T>, shown: u32, stop: Option<bool>) {
    match style {
        STYLE_LEAN => lean_readout(fb, p, shown, stop),
        STYLE_NUMBERS => numbers_readout(fb, p, shown, stop),
        _ => classic_readout(fb, p, shown, stop),
    }
}

/// Classic: name left and value right in the 16 px font, a framed bar below (outline 19..28,
/// fill 21..26, dotted while muted). The end stop fills the gap inside the frame's end.
fn classic_readout<T>(fb: &mut Fb, p: &Popup<T>, shown: u32, stop: Option<bool>) {
    let mut tmp = [0u8; 4];
    let right: &[u8] = if p.muted { b"MUTED" } else { percent(p.volume, &mut tmp) };
    let rw = Font::Big.width(right);
    fb.write(Font::Big, W - rw, 0, right);
    fb.write(Font::Big, 0, 0, Font::Big.fit(trimmed(&p.name), W - rw - 6));

    fb.rect(1, 19, W - 2, 19, true);
    fb.rect(1, 28, W - 2, 28, true);
    fb.rect(0, 20, 0, 27, true);
    fb.rect(W - 1, 20, W - 1, 27, true);
    let w = (shown * 122 / 1000) as i32;
    for x in 3..3 + w {
        for y in 21..=26 {
            if !p.muted || (x + y) % 3 == 0 {
                fb.px(x, y, true);
            }
        }
    }
    if let Some(top) = stop {
        let x = if top { W - 3 } else { 1 };
        fb.rect(x, 20, x + 1, 27, true);
    }
}

/// Lean: name in 5x7 at the left, the number in the 16 px font with a 5x7 `%` at the right,
/// a frameless bar below.
fn lean_readout<T>(fb: &mut Fb, p: &Popup<T>, shown: u32, stop: Option<bool>) {
    let (x0, x1, _, _) = LEAN_BAR;
    let left = if p.muted {
        let x = x1 + 1 - Font::Big.width(b"Muted");
        fb.write(Font::Big, x, 0, b"Muted");
        x
    } else {
        let mut digits = [0u8; 4];
        let v = number(p.volume, &mut digits);
        let nw = Font::Big.width(v);
        let x = x1 + 1 - (nw + 2 + 5);
        fb.write(Font::Big, x, 0, v);
        fb.text(x + nw + 2, LEAN_TEXT_Y, b"%", true);
        x
    };
    let name = trimmed(&p.name);
    let fits = ((left - 8 - x0 + 1) / 6).max(0) as usize;
    fb.text(x0, LEAN_TEXT_Y, trimmed(&name[..name.len().min(fits)]), true);
    bar(fb, LEAN_BAR, shown, p.muted, stop);
}

/// Numbers: the name in the mid font at the left, the number (or `Muted`) in the 16 px font at
/// the right, the bar below.
fn numbers_readout<T>(fb: &mut Fb, p: &Popup<T>, shown: u32, stop: Option<bool>) {
    let mut digits = [0u8; 4];
    let right: &[u8] = if p.muted { b"Muted" } else { number(p.volume, &mut digits) };
    let left = W - Font::Big.width(right);
    fb.write(Font::Big, left, NUMBER_Y, right);
    fb.write(Font::Mid, 2, NAME_Y, Font::Mid.fit(trimmed(&p.name), left - 9));
    bar(fb, BAR, shown, p.muted, stop);
}

/// A frameless bar in (x0, x1, y0, y1) filled to `shown` (volume x10), hollow while muted; past
/// its end, a 1 px track to the end of the range; the end stop just outside either end.
fn bar(fb: &mut Fb, (x0, x1, y0, y1): (i32, i32, i32, i32), shown: u32, muted: bool, stop: Option<bool>) {
    let w = (shown.min(1000) * (x1 - x0 + 1) as u32 / 1000) as i32;
    if w > 0 {
        fb.rect(x0, y0, x0 + w - 1, y1, true);
        if muted && w > 2 {
            fb.rect(x0 + 1, y0 + 1, x0 + w - 2, y1 - 1, false);
        }
    }
    let t0 = x0 + w + if w > 0 { 2 } else { 0 };
    if t0 <= x1 {
        fb.rect(t0, (y0 + y1) / 2, x1, (y0 + y1) / 2, true);
    }
    if let Some(top) = stop {
        let x = if top { x1 + 2 } else { x0 - 3 };
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
    /// The screen style, one of `state::STYLE_*`; any other value draws Classic.
    pub style: u8,
}

/// A value that eases from `from` to `to` over `dur` ms with a cubic ease-out: it starts fast,
/// settles gently, and lands on `to` exactly.
#[derive(Clone, Copy)]
struct Tween {
    from: i32,
    to: i32,
    at: u32,
    dur: u32,
}

impl Tween {
    const fn rest(at: i32) -> Tween {
        Tween { from: at, to: at, at: 0, dur: 0 }
    }

    fn value(&self, ms: u32) -> i32 {
        let t = ms.wrapping_sub(self.at);
        if t >= self.dur {
            return self.to;
        }
        let left = 256 - (t * 256 / self.dur) as i32; // 256..=1
        let eased = (left * left * left) >> 16; // (1 - x)^3 in 256ths
        self.to + (self.from - self.to) * eased / 256
    }

    /// Heads for `to` from wherever the value is now, so a change of mind mid-motion carries
    /// on from the current position instead of jumping back to the start.
    fn retarget(&mut self, ms: u32, to: i32, dur: u32) {
        *self = Tween { from: self.value(ms), to, at: ms, dur };
    }
}

/// One step of an exponential glide towards `target`: a `1/div` part of the gap, at least one
/// unit. It never passes the target and lands on it exactly.
fn glide(pos: u16, target: u16, div: u16) -> u16 {
    if target > pos { pos + (target - pos).div_ceil(div) } else { pos - (pos - target).div_ceil(div) }
}

/// The panel contrast for this frame: `want`, faded out with the lights over the `DIM_MS`
/// after `off_min` minutes of quiet (a smoothstep through the lights' gamma), then 0. The
/// panel turned on `since_wake` ms ago: it fades in with the lights' wake ease-out, so a wake
/// does not snap on at full brightness.
pub fn contrast(want: u8, idle: u64, off_min: u8, since_wake: u64) -> u8 {
    let faded = if off_min == NEVER {
        want as u32
    } else {
        let past = idle.saturating_sub(off_min as u64 * 60_000);
        let left = (65536 - smoothstep(progress(past, DIM_MS))).min(0xFFFF) as u32;
        want as u32 * gamma(left) / 0xFFFF
    };
    ((faded as u64 * ease_out(progress(since_wake, WAKE_MS))) >> 16) as u8
}

/// Everything the screen remembers from one frame to the next.
pub struct Screen<T> {
    peaks: [u8; NCH],
    peak_at: [T; NCH],
    /// Readout bar, volume x10 for smooth easing.
    shown_vol: u32,
    /// The readout was on screen last frame (also while it left).
    popup_open: bool,
    /// Per meter: the steps it heads for, the frames it has wanted fewer, and where it is now
    /// (in `SUB`ths of a column).
    meter_to: [u8; NCH],
    meter_low: [u8; NCH],
    meter_pos: [u16; NCH],
    /// How far the readout sits below its place (0 = in, `PUSH` = gone), and the message on
    /// screen, kept while it leaves.
    push: Tween,
    shown: Option<Popup<T>>,
    /// Sideways offset of the whole frame after a page change.
    slide: Tween,
    page: Option<bool>,
    page_at: u32,
    mutes: Option<u8>,
    mute_at: [Option<u32>; NCH],
    /// The style of the last frame; a change redraws the next frame at rest.
    style: u8,
    /// No frame drawn yet: the first one shows the state as it is, without motion.
    fresh: bool,
    moving: bool,
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
        Screen {
            peaks: [0; NCH],
            peak_at: [now; NCH],
            shown_vol: 0,
            popup_open: false,
            meter_to: [0; NCH],
            meter_low: [0; NCH],
            meter_pos: [0; NCH],
            push: Tween::rest(PUSH),
            shown: None,
            slide: Tween::rest(0),
            page: None,
            page_at: 0,
            mutes: None,
            mute_at: [None; NCH],
            style: STYLE_CLASSIC,
            fresh: true,
            moving: false,
            wall: None,
            last: None,
            logo_at: Some(now),
        }
    }

    /// True while the last frame drawn was part of a motion (or showed the end stop). A frame
    /// drawn while it is false shows the state at rest.
    pub fn moving(&self) -> bool {
        self.moving
    }

    /// Draws one frame into `fb`. `since_boot` is in milliseconds. Returns true for the calm
    /// idle logo, which may be drawn at half rate and with the panel dimmed.
    pub fn draw(&mut self, fb: &mut Fb, now: T, since_boot: u64, s: &Input<T>) -> bool {
        let ms = now.ms();
        // Motion only looks at short spans, so a wrapping 32-bit clock does (cheaper on the M3).
        let clock = ms as u32;
        let style = match s.style {
            STYLE_LEAN | STYLE_NUMBERS => s.style,
            _ => STYLE_CLASSIC,
        };
        let restyled = core::mem::replace(&mut self.style, style) != style;
        let fresh = core::mem::replace(&mut self.fresh, false) || restyled;
        for ((peak, at), &l) in self.peaks.iter_mut().zip(self.peak_at.iter_mut()).zip(s.meters.iter()) {
            if l >= *peak {
                *peak = l;
                *at = now;
            } else if now.ms_since(*at) > 900 {
                *peak = peak.saturating_sub(6);
            }
        }
        let (cols, meters_moving) = self.meters(style, &s.mixer, &s.meters, fresh);
        // Numbers inverts the label for Focus only; it still follows the mutes, so a later
        // switch to another style does not flash a change it saw long ago.
        let popup = s.popup.filter(|p| s.alive && now.ms_since(p.at) < POPUP_MS);
        let flash = self.mute_flash(clock, s, popup.is_some()) * (style != STYLE_NUMBERS) as u8;
        self.page_change(clock, s, fresh);
        fb.left = self.slide.value(clock);
        let mut moving = fb.left != 0;
        self.end_stop(clock, popup);

        fb.clear();
        let calm = if since_boot < SPLASH_MS {
            draw_logo(fb, since_boot, &s.name);
            self.shown = None;
            self.push = Tween::rest(PUSH);
            false
        } else {
            let sliding = clock.wrapping_sub(self.page_at) < SLIDE_MS;
            match popup {
                // The slide is the page's one entrance: the notice that names the page comes
                // in with it, not with a push of its own.
                Some(_) if fresh || sliding => self.push = Tween::rest(0),
                // From where it is now, but one frame into its motion: the first frame already
                // shows the readout, never the screen beneath unchanged.
                Some(_) if self.push.to != 0 => {
                    let from = self.push.value(clock);
                    self.push = Tween { from, to: 0, at: clock.wrapping_sub(FRAME_MS), dur: ENTER_MS };
                }
                None if !s.alive => self.push = Tween::rest(PUSH),
                None if self.push.to != PUSH => self.push.retarget(clock, PUSH, EXIT_MS),
                _ => {}
            }
            if popup.is_some() {
                self.shown = popup;
            }
            let d = self.push.value(clock);
            if d >= PUSH {
                self.shown = None;
            }
            moving |= d != self.push.to;

            // The screen beneath sits right above the readout, so the two move as one sheet.
            let top = fb.top;
            let mut calm = false;
            let mut logo_on = false;
            if self.shown.is_none() || d > 0 {
                fb.top = top + if self.shown.is_some() { d - PUSH } else { 0 };
                if s.alive && s.idle < SCREENSAVER_MS {
                    // Meters and mute flashes move only while the mixer shows.
                    moving |= meters_moving || flash != 0;
                    draw_mixer(fb, style, &s.mixer, &cols, &self.peaks, flash);
                } else {
                    let at = *self.logo_at.get_or_insert(now);
                    draw_logo(fb, now.ms_since(at), &s.name);
                    logo_on = true;
                    calm = self.shown.is_none();
                }
            }
            if !logo_on {
                self.logo_at = None;
            }
            if let Some(p) = self.shown {
                fb.top = top + d;
                if p.volume == NOTICE {
                    draw_notice(fb, trimmed(&p.name));
                } else {
                    let target = p.volume.min(100) as u32 * 10;
                    // A new readout starts at the value; while it stays open the bar glides.
                    self.shown_vol = if self.popup_open && !fresh {
                        glide(self.shown_vol as u16, target as u16, 3) as u32
                    } else {
                        target
                    };
                    moving |= self.shown_vol != target;
                    let stop = self.wall.filter(|&(at, _)| clock.wrapping_sub(at) < WALL_MS).map(|(_, top)| top);
                    moving |= stop.is_some();
                    draw_readout(fb, style, &p, self.shown_vol, stop);
                }
            }
            fb.top = top;
            calm
        };
        self.popup_open = self.shown.is_some();
        fb.left = 0;
        self.moving = moving;
        calm && !moving
    }

    /// Meters go up at once and down only after a short hold; the meter then glides there, fast
    /// up and slower down, like a needle. Returns each meter's lit columns, and whether any
    /// meter is still on its way.
    fn meters(&mut self, style: u8, m: &Mixer, levels: &[u8; NCH], fresh: bool) -> ([u8; NCH], bool) {
        let mut cols = [0u8; NCH];
        let mut moving = false;
        for c in 0..NCH {
            let (steps, step_cols) = meter_scale(style, m, c);
            let want = steps_lit(levels[c], steps);
            let (to, low) = (&mut self.meter_to[c], &mut self.meter_low[c]);
            if fresh || want > *to || *to - want >= 2 || (want < *to && *low + 1 >= METER_HOLD) {
                *to = want;
                *low = 0;
            } else if want < *to {
                *low += 1;
            } else {
                *low = 0;
            }
            let target = *to as u16 * step_cols * SUB;
            let pos = &mut self.meter_pos[c];
            *pos = if fresh { target } else { glide(*pos, target, if target > *pos { 2 } else { 3 }) };
            cols[c] = ((*pos + SUB / 2) / SUB) as u8;
            moving |= *pos != target || *low != 0;
        }
        (cols, moving)
    }

    /// The channels whose name shows inverted because their mute just changed. While a readout
    /// covers the mixer (`covered`), a flash waits: it plays in full when the mixer comes back.
    fn mute_flash(&mut self, ms: u32, s: &Input<T>, covered: bool) -> u8 {
        let changed = self.mutes.map_or(0, |m| m ^ s.mixer.mutes);
        self.mutes = s.alive.then_some(s.mixer.mutes);
        let mut flash = 0;
        for (c, at) in self.mute_at.iter_mut().enumerate() {
            if changed & 1 << c != 0 || (covered && at.is_some_and(|t| ms.wrapping_sub(t) < MUTE_FLASH_MS)) {
                *at = Some(ms);
            }
            if s.alive && at.is_some_and(|t| ms.wrapping_sub(t) < MUTE_FLASH_MS) {
                flash |= 1 << c;
            }
        }
        flash
    }

    /// A page change slides the frame in from the new page's side: page 2 from the right.
    fn page_change(&mut self, ms: u32, s: &Input<T>, fresh: bool) {
        let page = s.alive.then_some(s.mixer.second_page);
        if let (Some(was), Some(is)) = (self.page, page)
            && was != is
            && !fresh
        {
            let peek = ms.wrapping_sub(self.page_at) < PEEK_WINDOW_MS;
            let (dist, dur) = if peek { (PEEK, PEEK_MS) } else { (SLIDE, SLIDE_MS) };
            let dir = if is { 1 } else { -1 };
            let from = (self.slide.value(ms) + dir * dist).clamp(-SLIDE, SLIDE);
            self.slide = Tween { from, to: 0, at: ms, dur };
            self.page_at = ms;
        }
        if fresh {
            self.slide = Tween::rest(0);
        }
        self.page = page;
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

    const STYLES: [u8; 3] = [STYLE_CLASSIC, STYLE_LEAN, STYLE_NUMBERS];

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

    #[test]
    fn a_glide_lands_exactly_without_overshoot_or_a_limit_cycle() {
        for div in 2..=4 {
            for start in (0..=224).step_by(7) {
                for target in (0..=224).step_by(5) {
                    let mut pos = start;
                    for _ in 0..40 {
                        let next = glide(pos, target, div);
                        assert!(next == target || next.abs_diff(target) < pos.abs_diff(target), "closer every step");
                        assert!(next == target || (next > target) == (pos > target), "never passes the target");
                        pos = next;
                    }
                    assert_eq!(pos, target);
                    assert_eq!(glide(pos, target, div), target, "stays at rest");
                }
            }
        }
    }

    #[test]
    fn a_tween_eases_out_monotonically_and_lands_exactly() {
        let t = Tween { from: PUSH, to: 0, at: 1000, dur: ENTER_MS };
        assert_eq!(t.value(1000), PUSH);
        assert_eq!(t.value(1000 + ENTER_MS), 0);
        assert_eq!(t.value(1000 + 60_000), 0);
        assert_eq!(t.value(999), 0, "a clock that wrapped long ago counts as done");
        let mut last = PUSH;
        for ms in 1000..1000 + ENTER_MS {
            let v = t.value(ms);
            assert!(v <= last && v >= 0);
            last = v;
        }
        // Ease-out: most of the way is covered in the first third.
        assert!(t.value(1000 + ENTER_MS / 3) < PUSH / 3);
    }

    #[test]
    fn a_retarget_carries_on_from_where_the_value_is() {
        let mut t = Tween { from: 0, to: PUSH, at: 0, dur: EXIT_MS };
        let mid = t.value(30);
        assert!(mid > 0 && mid < PUSH);
        t.retarget(30, 0, ENTER_MS);
        assert_eq!(t.value(30), mid, "no jump");
        assert_eq!(t.value(30 + ENTER_MS), 0);
    }

    #[test]
    fn the_contrast_fades_with_the_lights_after_the_limit() {
        let off = 60_000;
        let awake = WAKE_MS;
        assert_eq!(contrast(0x30, 0, 1, awake), 0x30);
        assert_eq!(contrast(0x30, off, 1, awake), 0x30, "full up to the limit");
        assert_eq!(contrast(0xCF, off + DIM_MS, 1, awake), 0);
        assert_eq!(contrast(0xCF, off * 10, 1, awake), 0);
        let mut last = 0xCF;
        for idle in off..=off + DIM_MS {
            let c = contrast(0xCF, idle, 1, awake);
            assert!(c <= last, "monotone");
            last = c;
        }
        // Through the lights' gamma: half way in time is well under half the light.
        assert!(contrast(0xCF, off + DIM_MS / 2, 1, awake) < 0xCF / 4);
        assert_eq!(contrast(0xCF, off * 10, NEVER, awake), 0xCF, "never sleeps, never fades");
    }

    #[test]
    fn the_contrast_fades_in_with_the_lights_on_a_wake() {
        for off_min in [1, NEVER] {
            assert_eq!(contrast(0xCF, 0, off_min, 0), 0, "the panel turns on dark");
            assert_eq!(contrast(0xCF, 0, off_min, WAKE_MS), 0xCF, "full after the lights' wake time");
            assert_eq!(contrast(0xCF, 0, off_min, 60_000), 0xCF);
            let mut last = 0;
            for ms in 0..=WAKE_MS {
                let c = contrast(0xCF, 0, off_min, ms);
                assert!(c >= last, "monotone");
                last = c;
            }
            // Ease-out: a wake answers at once, so the first 33 ms frame is already bright.
            assert!(contrast(0xCF, 0, off_min, 33) > 0xCF / 3);
        }
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
            names: [*b"GAME  ", *b"MUSIC ", *b"CHAT  ", *b"WEB   "],
        }
    }

    fn input(style: u8, meters: [u8; NCH]) -> Input<u64> {
        Input { alive: true, mixer: mixer(), popup: None, idle: 0, name: *b"KB16\0\0\0\0\0\0", meters, style }
    }

    fn popup(at: u64, volume: u8, text: &[u8]) -> Option<Popup<u64>> {
        let mut name = [0; 16];
        name[..text.len()].copy_from_slice(text);
        Some(Popup { at, volume, muted: false, name })
    }

    fn with_popup(style: u8, p: Option<Popup<u64>>) -> Input<u64> {
        Input { popup: p, ..input(style, [0; NCH]) }
    }

    const T0: u64 = 100_000;

    /// One frame of `screen` at `ms`, well after the boot splash.
    fn frame(screen: &mut Screen<u64>, ms: u64, s: &Input<u64>) -> Fb {
        let mut fb = Fb::new(3);
        screen.draw(&mut fb, ms, ms, s);
        fb
    }

    /// Draws `frames` frames 33 ms apart from `at`; returns the last frame and the time after.
    fn run(screen: &mut Screen<u64>, at: u64, frames: u64, s: &Input<u64>) -> (Fb, u64) {
        let mut fb = Fb::new(3);
        let mut ms = at;
        for _ in 0..frames {
            screen.draw(&mut fb, ms, ms, s);
            ms += 33;
        }
        (fb, ms)
    }

    /// The frame without motion: what a screen that has drawn nothing before draws.
    fn settled(s: &Input<u64>) -> Fb {
        frame(&mut Screen::new(T0), T0, s)
    }

    fn lit_in(fb: &Fb, xs: core::ops::RangeInclusive<i32>, ys: core::ops::RangeInclusive<i32>) -> bool {
        xs.clone().any(|x| ys.clone().any(|y| fb.get(x, y)))
    }

    #[test]
    fn a_classic_meter_glides_up_through_part_segments_and_settles_on_the_same_frame() {
        let mut screen = Screen::new(T0);
        let (_, ms) = run(&mut screen, T0, 1, &input(STYLE_CLASSIC, [0; NCH]));
        let high = input(STYLE_CLASSIC, [255, 0, 0, 0]);
        let mut fb = Fb::new(3);
        screen.draw(&mut fb, ms, ms, &high);
        assert!(screen.moving());
        // Half way after one frame: 21 columns -> 10.5, drawn as 3 segments and 2 columns.
        assert_eq!(screen.meter_pos[0], 21 * SUB / 2);
        assert!(fb.get(2 + 3 * 4, 12) && fb.get(3 + 3 * 4, 12) && !fb.get(4 + 3 * 4, 12));
        let (fb, _) = run(&mut screen, ms + 33, 10, &high);
        assert!(!screen.moving());
        assert_eq!(fb.buf, settled(&high).buf);
    }

    #[test]
    fn a_classic_meter_on_a_segment_edge_does_not_flicker() {
        let mut screen = Screen::new(T0);
        // 110 and 108 sit on the two sides of the segment edge at 109.3.
        let (a, b) = (input(STYLE_CLASSIC, [110, 0, 0, 0]), input(STYLE_CLASSIC, [108, 0, 0, 0]));
        let first = settled(&a);
        let mut fb = Fb::new(3);
        for i in 0..60 {
            screen.draw(&mut fb, T0 + i * 33, T0, if i % 2 == 0 { &a } else { &b });
            assert_eq!(fb.buf, first.buf, "frame {i}");
        }
        // Held below the edge, it drops after the hold and, once the peak marker has fallen
        // back (after 900 ms), matches the plain frame.
        let (fb, _) = run(&mut screen, T0 + 2000, 40, &b);
        assert!(!screen.moving());
        assert_eq!(fb.buf, settled(&b).buf);
    }

    #[test]
    fn a_meter_never_passes_its_track_and_does_not_flicker_on_a_step_edge() {
        for style in STYLES {
            for volume in [1, 28, 50, 77, 100] {
                let m = Mixer { volume: [volume; NCH], mutes: 0, ..mixer() };
                let (steps, step_cols) = meter_scale(style, &m, 0);
                for level in 0..255u8 {
                    let mut screen = Screen::<u64>::new(T0);
                    let mut shown = [0; NCH];
                    for f in 0..40 {
                        // Two levels on the two sides of a step edge, in turn.
                        let l = if f % 2 == 0 { level } else { level + 1 };
                        let (cols, _) = screen.meters(style, &m, &[l; NCH], f == 0);
                        assert!(cols[0] as u16 <= steps as u16 * step_cols, "within the track");
                        if f == 20 {
                            shown = cols;
                        }
                        if f > 20 {
                            assert_eq!(cols, shown, "style {style}, volume {volume}, level {level}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_readout_pushes_in_fast_leaves_faster_and_a_retrigger_carries_on() {
        for style in STYLES {
            let base = input(style, [0; NCH]);
            let mut screen = Screen::new(T0);
            let (_, ms) = run(&mut screen, T0, 2, &base);
            let pop = |at| with_popup(style, popup(at, 64, b"MUSIC"));
            let p = pop(ms);
            let mut fb = Fb::new(3);
            screen.draw(&mut fb, ms, ms, &p);
            // One frame into its motion: no frame shows the mixer unchanged.
            let path: [i32; 4] = core::array::from_fn(|f| screen.push.value(ms as u32 + 33 * f as u32));
            assert_eq!(path, [9, 1, 0, 0], "style {style}");
            assert_ne!(fb.buf, settled(&base).buf, "style {style}: the first frame shows the readout");
            assert!(screen.moving());
            let entered = ms + ENTER_MS as u64;
            screen.draw(&mut fb, entered, entered, &p);
            assert!(!screen.moving());
            assert_eq!(fb.buf, settled(&p).buf, "style {style}: it lands exactly");

            // It times out and leaves; half way out a new readout comes, from where the old one is.
            let gone = ms + POPUP_MS;
            screen.draw(&mut fb, gone, gone, &p);
            let mid = gone + 30;
            let d = screen.push.value(mid as u32);
            assert!(d > 0 && d < PUSH);
            screen.draw(&mut fb, mid, mid, &pop(mid));
            let back = screen.push.value(mid as u32);
            assert!(back > 0 && back < d, "carries on from {d}, not from the bottom, one frame in: {back}");
            assert!(screen.push.value((mid + 33) as u32) < back);

            // Let it go for good: it is out within the exit time, and the mixer is back as before.
            let end = mid + POPUP_MS;
            screen.draw(&mut fb, end, end, &pop(mid));
            let (fb, _) = run(&mut screen, end + EXIT_MS as u64, 2, &base);
            assert!(!screen.moving());
            assert_eq!(fb.buf, settled(&base).buf, "style {style}");
        }
    }

    #[test]
    fn the_bar_glides_behind_the_number_and_lands_on_the_value() {
        for style in STYLES {
            let mut screen = Screen::new(T0);
            frame(&mut screen, T0, &with_popup(style, popup(T0, 40, b"MUSIC")));
            let s = with_popup(style, popup(T0 + 33, 90, b"MUSIC"));
            let fb = frame(&mut screen, T0 + 33, &s);
            assert!(screen.moving());
            assert!(screen.shown_vol > 400 && screen.shown_vol < 900, "on its way: {}", screen.shown_vol);
            assert_ne!(fb.buf, settled(&s).buf, "style {style}");
            // The number is already the new one: only the bar is still on its way.
            let number =
                |fb: &Fb| -> [bool; 64 * 13] { core::array::from_fn(|i| fb.get(64 + i as i32 % 64, i as i32 / 64)) };
            assert_eq!(number(&fb), number(&settled(&s)), "style {style}");
            let mut last = screen.shown_vol;
            let mut ms = T0 + 66;
            while screen.moving() {
                let fb = frame(&mut screen, ms, &s);
                assert!(screen.shown_vol >= last && screen.shown_vol <= 900, "never passes the value");
                last = screen.shown_vol;
                ms += 33;
                assert!(ms < T0 + 33 * 20, "style {style}: lands within 20 frames");
                if !screen.moving() {
                    assert_eq!(fb.buf, settled(&s).buf, "style {style}");
                }
            }
            assert_eq!(screen.shown_vol, 900);
        }
    }

    /// Whether the end stop shows at the top (or the bottom) end of the readout bar.
    fn stop_shows(style: u8, fb: &Fb, top: bool) -> bool {
        let (x, ys) = match style {
            STYLE_CLASSIC => (if top { 125 } else { 1 }, 20..=27),
            STYLE_LEAN => (if top { 126 } else { 0 }, 18..=24),
            _ => (if top { 126 } else { 0 }, 21..=27),
        };
        ys.clone().all(|y| fb.get(x, y) && fb.get(x + 1, y))
    }

    #[test]
    fn a_repeat_at_the_end_of_the_range_shows_the_stop_for_three_frames() {
        for style in STYLES {
            for volume in [100, 0] {
                let top = volume == 100;
                let mut screen = Screen::new(T0);
                let first = frame(&mut screen, T0, &with_popup(style, popup(T0, volume, b"MUSIC")));
                assert!(!stop_shows(style, &first, top), "{style}: the first message at {volume} is a plain readout");
                assert!(!stop_shows(style, &first, !top));
                let again = with_popup(style, popup(T0 + 33, volume, b"MUSIC"));
                for f in 0..3 {
                    let fb = frame(&mut screen, T0 + 33 * (f + 1), &again);
                    assert!(stop_shows(style, &fb, top), "{style}, {volume}: frame {f}");
                    assert!(screen.moving());
                }
                assert!(!stop_shows(style, &frame(&mut screen, T0 + 33 * 4, &again), top), "{style}: gone on frame 3");
                assert_eq!(frame(&mut screen, T0 + 33 * 5, &again).buf, first.buf);
            }

            // A mute, another name, or a value that moved shows no stop.
            let muted = |at, name: &[u8]| {
                let mut p = popup(at, 100, name);
                if let Some(p) = p.as_mut() {
                    p.muted = true;
                }
                with_popup(style, p)
            };
            let cases: [[Input<u64>; 2]; 3] = [
                [muted(T0, b"MUSIC"), muted(T0 + 33, b"MUSIC")],
                [with_popup(style, popup(T0, 100, b"MUSIC")), with_popup(style, popup(T0 + 33, 100, b"CHAT"))],
                [with_popup(style, popup(T0, 98, b"MUSIC")), with_popup(style, popup(T0 + 33, 100, b"MUSIC"))],
            ];
            for [a, b] in &cases {
                let mut screen = Screen::new(T0);
                frame(&mut screen, T0, a);
                assert!(!stop_shows(style, &frame(&mut screen, T0 + 33, b), true));
            }
        }
    }

    #[test]
    fn a_page_change_slides_in_from_the_new_pages_side_and_lands_exactly() {
        for style in STYLES {
            let one = input(style, [0; NCH]);
            let two = Input { mixer: Mixer { second_page: true, ..mixer() }, ..input(style, [0; NCH]) };
            let mut screen = Screen::new(T0);
            let (_, ms) = run(&mut screen, T0, 2, &one);
            let mut fb = Fb::new(3);
            screen.draw(&mut fb, ms, ms, &two);
            assert_eq!(screen.slide.value(ms as u32), SLIDE, "page 2 comes from the right");
            assert!(fb.buf[1..1 + SLIDE as usize].iter().all(|&b| b == 0));
            let (fb, ms) = run(&mut screen, ms + SLIDE_MS as u64, 1, &two);
            assert!(!screen.moving());
            assert_eq!(fb.buf, settled(&two).buf);

            // Back soon after (a held page key let go): only a nudge, from the left.
            let mut fb = Fb::new(3);
            screen.draw(&mut fb, ms, ms, &one);
            assert_eq!(screen.slide.value(ms as u32), -PEEK);
            let (fb, ms) = run(&mut screen, ms + PEEK_MS as u64, 1, &one);
            assert_eq!(fb.buf, settled(&one).buf);

            // Forth and back mid-slide: it carries on from where it is.
            let later = ms + PEEK_WINDOW_MS as u64;
            let mut fb = Fb::new(3);
            screen.draw(&mut fb, later, later, &two);
            let at = screen.slide.value((later + 40) as u32);
            screen.draw(&mut fb, later + 40, later + 40, &one);
            assert_eq!(screen.slide.value((later + 40) as u32), at - PEEK);
        }
    }

    #[test]
    fn a_page_notice_comes_in_with_the_slide_not_with_a_push() {
        let one = input(STYLE_CLASSIC, [0; NCH]);
        let mut screen = Screen::new(T0);
        let (_, ms) = run(&mut screen, T0, 2, &one);
        let two = Input {
            mixer: Mixer { second_page: true, ..mixer() },
            popup: popup(ms, NOTICE, b"KEYS"),
            ..input(STYLE_CLASSIC, [0; NCH])
        };
        let mut fb = Fb::new(3);
        screen.draw(&mut fb, ms, ms, &two);
        assert_eq!(screen.push.value(ms as u32), 0);
    }

    #[test]
    fn a_mute_change_inverts_the_name_briefly_in_classic_and_lean() {
        for style in [STYLE_CLASSIC, STYLE_LEAN] {
            let base = input(style, [0; NCH]);
            let mut screen = Screen::new(T0);
            let (_, ms) = run(&mut screen, T0, 2, &base);
            let muted = Input { mixer: Mixer { mutes: 0b0101, ..mixer() }, ..input(style, [0; NCH]) };
            let mut fb = Fb::new(3);
            screen.draw(&mut fb, ms, ms, &muted);
            // GAME is centred from x 4: x 2 is inside the pill, left of the letters.
            assert!(fb.get(1, 0) && fb.get(0, 1) && !fb.get(0, 0), "channel 0 (selected) keeps the pill's outline");
            assert!(!fb.get(2, 4) && lit_in(&fb, 4..=26, 1..=7), "hollow, with lit letters, while it flashes");
            assert!(screen.moving());
            let (fb, _) = run(&mut screen, ms + MUTE_FLASH_MS as u64, 1, &muted);
            assert!(fb.get(1, 0) && fb.get(2, 4), "and in its filled pill again after");
            assert!(!screen.moving());
            assert_eq!(fb.buf, settled(&muted).buf);
        }
        // Numbers inverts for Focus only.
        let mut screen = Screen::new(T0);
        let (_, ms) = run(&mut screen, T0, 2, &input(STYLE_NUMBERS, [0; NCH]));
        let muted = Input { mixer: Mixer { mutes: 0b0101, ..mixer() }, ..input(STYLE_NUMBERS, [0; NCH]) };
        assert_eq!(frame(&mut screen, ms, &muted).buf, settled(&muted).buf);
        assert!(!screen.moving());
    }

    #[test]
    fn a_mute_flash_under_a_readout_plays_when_the_mixer_comes_back() {
        for style in [STYLE_CLASSIC, STYLE_LEAN] {
            let mut screen = Screen::new(T0);
            let (_, ms) = run(&mut screen, T0, 2, &input(style, [0; NCH]));
            // MUSIC (channel 1, not selected) mutes, and its readout comes up.
            let muted = Input { mixer: Mixer { mutes: 0b0110, ..mixer() }, ..input(style, [0; NCH]) };
            let covered = Input { popup: popup(ms, 55, b"MUSIC"), ..muted };
            let mut t = ms;
            while t < ms + POPUP_MS {
                frame(&mut screen, t, &covered);
                t += 33;
            }
            // The readout times out and leaves; the mixer beneath shows the flash, in a pill.
            let gone = ms + POPUP_MS;
            frame(&mut screen, gone, &covered);
            let fb = frame(&mut screen, gone + EXIT_MS as u64, &covered);
            assert!(fb.get(33, 0), "style {style}: the flash plays once the mixer is back");
            assert!(screen.moving());
            let fb = frame(&mut screen, gone + MUTE_FLASH_MS as u64, &covered);
            assert!(!fb.get(33, 0), "style {style}: for 200 ms");
            assert!(!screen.moving());
            assert_eq!(fb.buf, settled(&muted).buf, "style {style}");
        }
    }

    #[test]
    fn the_first_frame_shows_the_state_without_motion() {
        for style in STYLES {
            let s = Input { popup: popup(T0, 64, b"MUSIC"), ..input(style, [255, 120, 0, 60]) };
            let mut screen = Screen::new(T0);
            frame(&mut screen, T0, &s);
            assert!(!screen.moving());
        }
    }

    #[test]
    fn a_style_switch_redraws_the_next_frame_at_rest() {
        for from in STYLES {
            for to in STYLES {
                // Everything on its way: meters rising, the bar gliding, the page sliding.
                let mut screen = Screen::new(T0);
                let a = Input { popup: popup(T0, 40, b"MUSIC"), ..input(from, [0; NCH]) };
                frame(&mut screen, T0, &a);
                let two = Mixer { second_page: true, ..mixer() };
                let b = Input { mixer: two, popup: popup(T0 + 33, 90, b"MUSIC"), ..input(from, [255, 200, 90, 30]) };
                frame(&mut screen, T0 + 33, &b);
                assert!(screen.moving());
                let c = Input { style: to, ..b };
                let fb = frame(&mut screen, T0 + 66, &c);
                if from != to {
                    assert_eq!(fb.buf, settled(&c).buf, "{from} -> {to}");
                    assert!(!screen.moving());
                }
                // And the mixer beneath, once the readout has gone.
                let d = Input { popup: None, ..c };
                let (fb, _) = run(&mut screen, T0 + 66 + POPUP_MS + EXIT_MS as u64, 12, &d);
                assert_eq!(fb.buf, settled(&d).buf, "{from} -> {to}: the mixer");
            }
        }
    }

    #[test]
    fn an_unknown_style_draws_classic() {
        let mut classic = Screen::new(T0);
        let mut unknown = Screen::new(T0);
        for (i, style) in [3u8, 7, 255].into_iter().enumerate() {
            let ms = T0 + 33 * 5 * i as u64;
            for f in 0..5 {
                let at = ms + 33 * f;
                let s = Input { popup: popup(ms, 30 + 20 * f as u8, b"MUSIC"), ..input(style, [60 * f as u8; NCH]) };
                let a = frame(&mut classic, at, &Input { style: STYLE_CLASSIC, ..s });
                let b = frame(&mut unknown, at, &s);
                assert_eq!(a.buf, b.buf, "style {style}, frame {f}");
            }
        }
    }

    #[test]
    fn sound_alone_changes_only_the_meter_page_in_lean_and_numbers_for_every_top() {
        for style in [STYLE_LEAN, STYLE_NUMBERS] {
            for top in 0..=6 {
                let mut screen = Screen::new(T0);
                let mut fb = Fb::new(top);
                screen.draw(&mut fb, T0, T0, &input(style, [0; NCH]));
                let mut sent = [0u8; 512];
                sent.copy_from_slice(&fb.buf[1..]);
                let mut ms = T0;
                for meters in [[255, 200, 100, 50], [30, 255, 0, 255], [0; NCH]] {
                    let mut seen = false;
                    for _ in 0..20 {
                        ms += 33;
                        let mut fb = Fb::new(top);
                        screen.draw(&mut fb, ms, T0, &input(style, meters));
                        for page in 0..3 {
                            assert_eq!(dirty_span(page, &fb.buf[1..], &sent), None, "{style}, top {top}, page {page}");
                        }
                        seen |= dirty_span(3, &fb.buf[1..], &sent).is_some();
                        sent.copy_from_slice(&fb.buf[1..]);
                    }
                    assert!(seen, "{style}, top {top}: the sound shows");
                }
            }
        }
    }

    #[test]
    fn lean_has_one_bar_a_meter_line_and_no_percent() {
        let fb = settled(&input(STYLE_LEAN, [255, 0, 0, 0]));
        // GAME at 80: the bar is 21 px (x 2..22) on L20..22, the track on L21 from x 24 to 28.
        assert!((2..=22).all(|x| (20..=22).all(|y| fb.get(x, y))));
        assert!(!fb.get(23, 21) && (24..=28).all(|x| fb.get(x, 21)) && !fb.get(24, 20) && !fb.get(24, 22));
        // The meter line under it, all the way for a full level.
        assert!((2..=28).all(|x| fb.get(x, 24)) && !fb.get(29, 24) && !fb.get(2, 23));
        // Open rows between the name and the bar, and nothing below the meter: no percent.
        for c in [0, 1, 3] {
            let x0 = c * 32;
            assert!(!lit_in(&fb, x0..=x0 + 31, 10..=19), "column {c}");
            assert!(!lit_in(&fb, x0..=x0 + 31, 25..=28), "column {c}");
        }
        assert!(!lit_in(&fb, 32..=63, 23..=24), "no sound, no meter line");
        // A muted channel: MUTE in a pill, no bar.
        assert!(fb.get(66, 20) && lit_in(&fb, 64..=95, 18..=26) && !fb.get(66, 18));
        // The classic marks: the mic rule under the names, the page-2 tick at the right edge.
        let m = Mixer { second_page: true, mic_muted: true, ..mixer() };
        let fb = settled(&Input { mixer: m, ..input(STYLE_LEAN, [0; NCH]) });
        assert!((0..=W - 2).all(|x| fb.get(x, 9)) && (2..=6).all(|y| fb.get(W - 1, y)));
    }

    #[test]
    fn the_lean_readout_has_a_small_name_a_16_px_number_and_a_frameless_bar() {
        let s = with_popup(STYLE_LEAN, popup(T0, 64, b"MUSIC"));
        let fb = settled(&s);
        let mut expected = Fb::new(3);
        let x = 125 - (Font::Big.width(b"64") + 7);
        expected.write(Font::Big, x, 0, b"64");
        expected.text(x + Font::Big.width(b"64") + 2, LEAN_TEXT_Y, b"%", true);
        expected.text(3, LEAN_TEXT_Y, b"MUSIC", true);
        bar(&mut expected, LEAN_BAR, 640, false, None);
        assert_eq!(fb.buf, expected.buf);
        // The 16 px digits (L1..12), not the 24 px logo digits.
        assert!(lit_in(&fb, x..=124, 1..=12) && !lit_in(&fb, 0..=127, 13..=18));
        // A long name stops 8 px before the number.
        let fb = settled(&with_popup(STYLE_LEAN, popup(T0, 100, b"Averyverylongnam")));
        let left = 125 - (Font::Big.width(b"100") + 7);
        let end = (0..left).rev().find(|&x| lit_in(&fb, x..=x, 0..=14)).unwrap_or(0);
        assert!(end <= left - 8 && end > left - 8 - 8, "{end} ends 8 px before {left}, cut by one letter at a time");
    }

    #[test]
    fn a_numbers_muted_channel_shows_mute_and_no_line() {
        let fb = settled(&input(STYLE_NUMBERS, [255; NCH]));
        let chat = 2 * 32;
        assert!(!lit_in(&fb, chat..=chat + 31, 21..=28), "no rail and no sound");
        assert!(lit_in(&fb, chat + 5..=chat + 27, 14..=19), "MUTE");
        assert!(!lit_in(&fb, chat..=chat + 31, 9..=13), "no value");
        // The others have both, and the sound fills the rail as far as the level goes.
        assert!(fb.get(2, 24) && fb.get(2, 25) && fb.get(1 + rail(80), 24));
    }

    #[test]
    fn numbers_focus_inverts_only_the_label() {
        let fb = settled(&input(STYLE_NUMBERS, [0; NCH]));
        // "GAME" is 23 px wide, centred in 0..31 from x 4: the pill is 2..28 on L1..8.
        assert!(fb.get(2, 2) && !fb.get(2, 1) && fb.get(28, 7));
        assert!(!lit_in(&fb, 32..=63, 0..=1), "the others are plain");
        // The page-2 tick and the microphone tick sit at the edges, on L14..18.
        let m = Mixer { second_page: true, mic_muted: true, ..mixer() };
        let fb = settled(&Input { mixer: m, ..input(STYLE_NUMBERS, [0; NCH]) });
        assert!((14..=18).all(|y| fb.get(0, y) && fb.get(1, y) && fb.get(126, y) && fb.get(127, y)));
    }

    #[test]
    fn the_numbers_readout_uses_the_16_px_font_and_a_long_name_ends_8_px_before_it() {
        for muted in [false, true] {
            let mut p = popup(T0, 100, b"Averyverylongnam");
            if let Some(p) = p.as_mut() {
                p.muted = muted;
            }
            let fb = settled(&with_popup(STYLE_NUMBERS, p));
            let left = W - Font::Big.width(if muted { b"Muted" } else { b"100" });
            let end = (0..left).rev().find(|&x| lit_in(&fb, x..=x, 0..=14)).unwrap_or(0);
            assert!(end <= left - 8, "{end} ends 8 px before {left}");
            assert!(end > left - 8 - 12, "{end}: cut by one letter at a time");
            assert!(lit_in(&fb, left..=left + 3, 0..=11), "the number or Muted starts at {left}");
            assert!(!lit_in(&fb, left..=127, 12..=20), "16 px, not the 24 px digits");
        }
    }

    #[test]
    fn a_long_notice_uses_the_13_px_font() {
        let long = b"Spotify > Media";
        assert!(Font::Big.width(long) > W - 4);
        let mut expected = Fb::new(3);
        expected.write(Font::Mid, (W - Font::Mid.width(long)) / 2, 9, long);
        for style in STYLES {
            let s = with_popup(style, popup(T0, NOTICE, long));
            assert_eq!(settled(&s).buf, expected.buf);
        }
        let short = b"KEYS";
        let mut expected = Fb::new(3);
        expected.write(Font::Big, (W - Font::Big.width(short)) / 2, 7, short);
        assert_eq!(settled(&with_popup(STYLE_CLASSIC, popup(T0, NOTICE, short))).buf, expected.buf);
    }

    /// A frame with every pixel set at random, its offsets set too.
    fn noise(seed: u32, top: i32, left: i32) -> Fb {
        let mut fb = Fb::new(top);
        fb.left = left;
        let mut x = seed | 1;
        for b in fb.buf[1..].iter_mut() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
        fb
    }

    #[test]
    fn the_streak_clears_the_same_pixels_as_a_scan_of_the_whole_frame() {
        for streak in -40..W + 40 {
            for (top, left) in [(0, 0), (3, 0), (3, -24), (6, 24)] {
                let mut want = noise((streak + 47) as u32, top, left);
                let mut got = noise((streak + 47) as u32, top, left);
                for y in 0..H {
                    for x in 0..W {
                        let d = x - streak - y / 2;
                        if (0..4).contains(&d) || d == 6 {
                            want.px(x, y, false);
                        }
                    }
                }
                draw_streak(&mut got, streak);
                assert_eq!(got.buf, want.buf, "streak {streak}, top {top}, left {left}");
            }
        }
    }

    #[test]
    fn a_rect_sets_the_same_pixels_as_one_px_at_a_time() {
        let mut seed = 1u32;
        for (top, left) in [(0, 0), (3, 0), (6, -24), (-2, 30)] {
            for x0 in (-5..W + 5).step_by(7) {
                for y0 in -9..H + 2 {
                    for (w, h) in [(0, 0), (1, 1), (3, 9), (40, 2), (200, 40), (-2, 3)] {
                        for on in [true, false] {
                            seed += 1;
                            let (x1, y1) = (x0 + w, y0 + h);
                            let mut want = noise(seed, top, left);
                            let mut got = noise(seed, top, left);
                            for y in y0..=y1 {
                                for x in x0..=x1 {
                                    want.px(x, y, on);
                                }
                            }
                            got.rect(x0, y0, x1, y1, on);
                            assert_eq!(got.buf, want.buf, "({x0},{y0})..({x1},{y1}) top {top} left {left}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_idle_logo_wipes_in_and_streaks_once() {
        let mut screen = Screen::new(T0);
        frame(&mut screen, T0, &input(STYLE_CLASSIC, [0; NCH]));
        let quiet = Input { idle: SCREENSAVER_MS, ..input(STYLE_CLASSIC, [0; NCH]) };
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
        // A touch brings the mixer back.
        let back = frame(&mut screen, start + 30_000, &input(STYLE_CLASSIC, [0; NCH]));
        assert_eq!(back.buf, settled(&input(STYLE_CLASSIC, [0; NCH])).buf);
    }

    #[test]
    fn the_boot_logo_runs_on_without_a_second_wipe() {
        let none = Input { alive: false, ..input(STYLE_CLASSIC, [0; NCH]) };
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
}
