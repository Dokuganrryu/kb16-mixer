//! What the 128x32 screen shows: the frame buffer, the 1-bit drawing, and the choice of screen.
//! The firmware's `oled.rs` sends the buffer to the SSD1306 panel.
//!
//! The case hides the panel's top rows, so every screen is laid out for `ROWS` visible rows
//! and drawn `top` pixels lower (set from the PC app's config, default 3).

use crate::Time;
use crate::assets::{BIG_COLS, BIG_OFF, LOGO_CHARS, LOGO_COLS, LOGO_GAP, LOGO_OFF, SIN};
use crate::font::FONT5X7;
use crate::state::{Mixer, NCH, Popup};

pub const W: i32 = 128;
pub const H: i32 = 32;
/// Rows every layout fits in (32 minus the default top margin).
const ROWS: i32 = 29;
/// How long the logo wipes in after power-up.
pub const SPLASH_MS: u64 = 1800;
const POPUP_MS: u64 = 1300;
const SCREENSAVER_MS: u64 = 45_000;
/// The SSD1306 control byte that says "display data follows".
const DATA: u8 = 0x40;

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

    /// 5x7 text; `on = false` draws dark text (inside a pill). Returns the end x.
    fn text(&mut self, mut x: i32, y: i32, s: &[u8], on: bool) -> i32 {
        for &ch in s {
            let g = &FONT5X7[(ch.clamp(32, 126) - 32) as usize];
            for (dx, col) in g.iter().enumerate() {
                for dy in 0..8 {
                    if col & (1 << dy) != 0 {
                        self.px(x + dx as i32, y + dy, on);
                    }
                }
            }
            x += 6;
        }
        x
    }

    /// 5x7 text centred in the 31 px wide strip starting at `x0`.
    fn centred(&mut self, x0: i32, y: i32, s: &[u8], on: bool) {
        self.text(x0 + (31 - (s.len() as i32 * 6 - 1)) / 2, y, s, on);
    }

    fn big_width(s: &[u8]) -> i32 {
        s.iter().map(|&c| glyph(c).len() as i32 + 2).sum::<i32>() - 2
    }

    /// 16 px proportional text.
    fn big(&mut self, mut x: i32, y: i32, s: &[u8]) {
        for &ch in s {
            for (dx, col) in glyph(ch).iter().enumerate() {
                let bits = col[0] as u16 | (col[1] as u16) << 8;
                for dy in 0..16 {
                    if bits & (1 << dy) != 0 {
                        self.px(x + dx as i32, y + dy, true);
                    }
                }
            }
            x += glyph(ch).len() as i32 + 2;
        }
    }
}

/// A logo-font letter; characters it lacks are skipped.
fn logo_glyph(c: u8) -> &'static [[u8; 4]] {
    match LOGO_CHARS.iter().position(|&l| l == c) {
        Some(i) => &LOGO_COLS[LOGO_OFF[i] as usize..LOGO_OFF[i + 1] as usize],
        None => &[],
    }
}

fn logo_width(s: &[u8]) -> i32 {
    s.iter().map(|&c| logo_glyph(c).len() as i32 + LOGO_GAP).sum::<i32>() - LOGO_GAP
}

fn glyph(c: u8) -> &'static [[u8; 2]] {
    let i = (c.clamp(32, 126) - 32) as usize;
    &BIG_COLS[BIG_OFF[i] as usize..BIG_OFF[i + 1] as usize]
}

fn trimmed(s: &[u8]) -> &[u8] {
    let end = s.iter().rposition(|&c| c != 0 && c != b' ').map_or(0, |i| i + 1);
    &s[..end]
}

fn percent(v: u8, out: &mut [u8; 4]) -> &[u8] {
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
    out[n + 1] = b'%';
    &out[..n + 2]
}

/// Segments lit (0..=7) of a 7-segment meter for a level 0..=255.
fn segments(level: u8) -> i32 {
    (level as u32 * 7).div_ceil(255).min(7) as i32
}

/// The name in the logo letters, centred, with a light streak crossing it every few seconds
/// and a slow wave beneath. A name too wide for the logo letters uses the big font, and one
/// too wide even for that loses letters from the end. `reveal` (0..=128) wipes it in during
/// the boot splash.
fn draw_logo(fb: &mut Fb, ms: u64, reveal: i32, name: &[u8]) {
    let name = trimmed(name);
    if logo_width(name) <= W {
        let mut x = (W - logo_width(name)) / 2;
        for &ch in name {
            for col in logo_glyph(ch) {
                let bits = u32::from_le_bytes(*col);
                for y in 0..32 {
                    if bits & 1 << y != 0 {
                        fb.px(x, y, true);
                    }
                }
                x += 1;
            }
            x += LOGO_GAP;
        }
    } else {
        let mut n = name.len();
        while n > 0 && Fb::big_width(&name[..n]) > W {
            n -= 1;
        }
        fb.big((W - Fb::big_width(&name[..n])) / 2, 4, &name[..n]);
    }
    let streak = (ms % 5200) as i32 / 5 - 30; // crosses during the first ~900 ms of each cycle
    for y in 0..H {
        for x in 0..W {
            let d = x - streak - y / 2;
            if ((0..4).contains(&d) || d == 6) && fb.get(x, y) {
                fb.px(x, y, false);
            }
        }
    }
    // The wave breaks where a letter reaches down into it (J, Q).
    let low: [bool; W as usize] = core::array::from_fn(|x| (24..30).any(|y| fb.get(x as i32, y)));
    let t = (ms / 18) as u32;
    for x in 0..W {
        if (x - 1..=x + 1).any(|cx| low.get(cx as usize).copied().unwrap_or(false)) {
            continue;
        }
        let a = SIN[((x as u32 * 4 + t) & 0xFF) as usize] as i32;
        fb.px(x, 25 + a * 3 / 255, true);
    }
    if reveal < W - 1 {
        fb.rect(reveal.max(-1) + 1, 0, W - 1, H - 1, false);
    }
}

/// Scribble strip: one 32 px column per channel, laid out in ROWS rows:
/// name 0..8, live meter 11..14, fader 17..19, value 21..28.
fn draw_mixer(fb: &mut Fb, m: &Mixer, meters: &[u8; NCH], peaks: &[u8; NCH]) {
    for c in 0..m.n as usize {
        let x0 = c as i32 * 32;
        let muted = m.mutes & (1 << c) != 0;
        let name = trimmed(&m.names[c]);
        let name = &name[..name.len().min(kb16_mixer_protocol::NAME_SHOWN)];
        if c as u8 == m.selected {
            fb.pill(x0, 0, x0 + 30, 8);
            fb.centred(x0, 1, name, false);
        } else {
            fb.centred(x0, 1, name, true);
        }

        // Live meter: 7 segments with a peak-hold marker.
        let lit = segments(meters[c]);
        let peak = segments(peaks[c]);
        for s in 0..7 {
            let sx = x0 + 2 + s * 4;
            if !muted && s < lit {
                fb.rect(sx, 11, sx + 2, 14, true);
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
            let p = percent(m.volume[c], &mut tmp);
            fb.centred(x0, 22, p, true);
        }
    }
    // Mic muted: a rule under the names for as long as it lasts.
    if m.mic_muted {
        fb.rect(0, 9, W - 2, 9, true);
    }
    // Second key page: a short tick in the right-most pixel column, which nothing else uses.
    if m.second_page {
        fb.rect(W - 1, 2, W - 1, 6, true);
    }
}

/// A short message on its own, centred.
fn draw_notice(fb: &mut Fb, text: &[u8]) {
    let mut n = text.len();
    while n > 0 && Fb::big_width(&text[..n]) > W {
        n -= 1;
    }
    let text = &text[..n];
    fb.big((W - Fb::big_width(text)) / 2, (ROWS - 16) / 2, text);
}

/// Big readout while a knob moves: name left, value right, bar below.
fn draw_popup<T>(fb: &mut Fb, p: &Popup<T>, shown_vol: u32) {
    if p.volume == kb16_mixer_protocol::NOTICE {
        return draw_notice(fb, trimmed(&p.name));
    }
    let mut tmp = [0u8; 4];
    let right: &[u8] = if p.muted { b"MUTED" } else { percent(p.volume, &mut tmp) };
    let rw = Fb::big_width(right);
    fb.big(W - rw, 0, right);

    let name = trimmed(&p.name);
    let mut n = name.len();
    while n > 0 && Fb::big_width(&name[..n]) > W - rw - 6 {
        n -= 1;
    }
    fb.big(0, 0, &name[..n]);

    // Bar: outline 19..28, fill 21..26.
    fb.rect(1, 19, W - 2, 19, true);
    fb.rect(1, 28, W - 2, 28, true);
    fb.rect(0, 20, 0, 27, true);
    fb.rect(W - 1, 20, W - 1, 27, true);
    let w = (shown_vol * 122 / 1000) as i32;
    for x in 3..3 + w {
        for y in 21..=26 {
            if !p.muted || (x + y) % 3 == 0 {
                fb.px(x, y, true);
            }
        }
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

/// Everything the screen remembers from one frame to the next.
pub struct Screen<T> {
    peaks: [u8; NCH],
    peak_at: [T; NCH],
    /// Popup bar, x10 for smooth easing.
    shown_vol: u32,
    popup_open: bool,
}

impl<T: Time> Screen<T> {
    pub fn new(now: T) -> Screen<T> {
        Screen { peaks: [0; NCH], peak_at: [now; NCH], shown_vol: 0, popup_open: false }
    }

    /// Draws one frame into `fb`. `since_boot` is in milliseconds. Returns true for the calm
    /// idle logo, which may be drawn at half rate and with the panel dimmed.
    pub fn draw(&mut self, fb: &mut Fb, now: T, since_boot: u64, s: &Input<T>) -> bool {
        let ms = now.ms();
        for ((peak, at), &l) in self.peaks.iter_mut().zip(self.peak_at.iter_mut()).zip(s.meters.iter()) {
            if l >= *peak {
                *peak = l;
                *at = now;
            } else if now.ms_since(*at) > 900 {
                *peak = peak.saturating_sub(6);
            }
        }

        fb.clear();
        let popup = s.popup.filter(|p| s.alive && now.ms_since(p.at) < POPUP_MS);
        let calm = if since_boot < SPLASH_MS {
            draw_logo(fb, ms, (since_boot * 160 / 900) as i32, &s.name);
            false
        } else if let Some(p) = popup {
            let target = p.volume.min(100) as u32 * 10;
            // A fresh popup starts at the value; while it stays open the bar glides.
            self.shown_vol = if !self.popup_open {
                target
            } else if target > self.shown_vol {
                self.shown_vol + (target - self.shown_vol).div_ceil(3)
            } else {
                self.shown_vol - (self.shown_vol - target).div_ceil(3)
            };
            draw_popup(fb, &p, self.shown_vol);
            false
        } else if s.alive && s.idle < SCREENSAVER_MS {
            draw_mixer(fb, &s.mixer, &s.meters, &self.peaks);
            false
        } else {
            draw_logo(fb, ms, W, &s.name);
            true
        };
        self.popup_open = popup.is_some();
        calm
    }
}
