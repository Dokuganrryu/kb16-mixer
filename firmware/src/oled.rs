//! SSD1306 128x32 on I2C2. Frames are drawn into RAM and sent with DMA (async, 400 kHz),
//! only when something changed, so the display never stalls the keys or the LEDs.
//!
//! The case hides the panel's top rows, so every screen is laid out for `ROWS` visible rows
//! and drawn `top` pixels lower (set from the PC app's config, default 3).

use crate::assets::{BIG_COLS, BIG_OFF, LOGO, LOGO_W, SIN};
use crate::font::FONT5X7;
use crate::state::{self, Mixer, NCH, Popup};
use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Async;
use embassy_time::{Duration, Instant, Ticker, Timer};

const W: i32 = 128;
const H: i32 = 32;
/// Rows every layout fits in (32 minus the default top margin).
const ROWS: i32 = 29;
const ADDR: u8 = 0x3C;
const SPLASH_MS: u64 = 1800;
const POPUP_MS: u64 = 1300;
const SCREENSAVER_MS: u64 = 45_000;

type Bus = I2c<'static, Async, Master>;

struct Fb {
    /// Byte 0 is the I2C "data" control byte so the buffer goes out in one transfer.
    buf: [u8; 1 + (W * H / 8) as usize],
    /// Vertical offset applied to everything drawn.
    top: i32,
}

impl Fb {
    fn clear(&mut self) {
        self.buf[1..].fill(0);
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

/// The name, with a light streak crossing it every few seconds and a slow wave beneath.
/// `reveal` (0..=128) wipes it in during the boot splash.
fn draw_logo(fb: &mut Fb, ms: u64, reveal: i32) {
    let x0 = (W - LOGO_W as i32) / 2;
    let streak = (ms % 5200) as i32 / 5 - 30; // crosses during the first ~900 ms of each cycle
    for (cx, col) in LOGO.iter().enumerate() {
        let x = x0 + cx as i32;
        if x > reveal {
            break;
        }
        for (page, &byte) in col.iter().enumerate() {
            for bit in 0..8 {
                if byte & (1 << bit) == 0 {
                    continue;
                }
                let y = (page * 8 + bit) as i32;
                let d = x - streak - y / 2;
                fb.px(x, y, !((0..4).contains(&d) || d == 6));
            }
        }
    }
    let t = (ms / 18) as u32;
    for x in 0..W.min(reveal + 1) {
        let a = SIN[((x as u32 * 4 + t) & 0xFF) as usize] as i32;
        fb.px(x, 25 + a * 3 / 255, true);
    }
}

/// Scribble strip: one 32 px column per channel, laid out in ROWS rows:
/// name 0..8, live meter 11..14, fader 17..19, value 21..28.
fn draw_mixer(fb: &mut Fb, m: &Mixer, peaks: &[u8; NCH]) {
    for c in 0..m.n as usize {
        let x0 = c as i32 * 32;
        let muted = m.mutes & (1 << c) != 0;
        let name = trimmed(&m.names[c]);
        let name = &name[..name.len().min(5)];
        if c as u8 == m.selected {
            fb.pill(x0, 0, x0 + 30, 8);
            fb.centred(x0, 1, name, false);
        } else {
            fb.centred(x0, 1, name, true);
        }

        // Live meter: 7 segments with a peak-hold marker.
        let lit = ((state::meter(c) as i32 * 7 + 254) / 255).min(7);
        let peak = ((peaks[c] as i32 * 7 + 254) / 255).min(7);
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
fn draw_popup(fb: &mut Fb, p: &Popup, shown_vol: u32) {
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

async fn cmd(i2c: &mut Bus, bytes: &[u8]) -> bool {
    let mut b = [0u8; 32];
    b[1..=bytes.len()].copy_from_slice(bytes); // b[0] = 0x00: command stream
    i2c.write(ADDR, &b[..=bytes.len()]).await.is_ok()
}

async fn init(i2c: &mut Bus) -> bool {
    // The sequence QMK uses for this panel (SSD1306, 128x32, rotation 0).
    cmd(
        i2c,
        &[
            0xAE, 0xD5, 0x80, 0xA8, 0x1F, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xD3, 0x00, 0xDA, 0x02, 0x81, 0xCF,
            0xD9, 0xF1, 0xDB, 0x20, 0xA4, 0xA6, 0x2E, 0xAF,
        ],
    )
    .await
}

#[embassy_executor::task]
pub async fn run(mut i2c: Bus) {
    Timer::after_millis(100).await; // panel power-up
    crate::diag::log(0x4000);
    let mut ready = init(&mut i2c).await;
    crate::diag::log(0x4100 | ready as u16);
    let mut fb = Fb { buf: [0; 1 + (W * H / 8) as usize], top: state::DEFAULT_OLED_TOP as i32 };
    fb.buf[0] = 0x40;
    let mut sent = [0xFFu8; (W * H / 8) as usize];
    let mut peaks = [0u8; NCH];
    let mut peak_at = [Instant::now(); NCH];
    let mut shown_vol = 0u32; // popup bar, x10 for smooth easing
    let mut popup_open = false;
    let mut asleep = false;
    let mut contrast = 0u8;
    let boot = Instant::now();
    let mut was_calm = false;
    let mut skip = false;
    let mut ticker = Ticker::every(Duration::from_millis(33));

    loop {
        ticker.next().await;
        // The idle logo moves slowly; drawing it at half rate halves the bus traffic.
        skip = was_calm && !skip;
        if skip && ready {
            let now = Instant::now();
            let busy =
                state::with(|s| s.host_alive(now) && now.saturating_duration_since(s.activity).as_millis() < 500);
            if !busy {
                continue;
            }
        }
        if !ready {
            Timer::after_millis(500).await;
            ready = init(&mut i2c).await;
            // Init switches the panel on; the sleep check below switches it off again if it
            // should be asleep, instead of freezing the last picture on screen.
            if ready {
                asleep = false;
            }
            sent.fill(0xFF);
            contrast = 0;
            continue;
        }
        let now = Instant::now();
        let ms = now.as_millis();
        let since_boot = now.saturating_duration_since(boot).as_millis();
        let (alive, mixer, popup, activity, top, off_after) =
            state::with(|s| (s.host_alive(now), s.mixer, s.popup, s.activity, s.oled_top, s.screen_off_min));
        let idle = now.saturating_duration_since(activity).as_millis();
        fb.top = top as i32;

        // Sleep after long inactivity; any key, knob or app audio the host hears wakes it.
        let want_sleep = state::suspended() || (since_boot > SPLASH_MS && state::past(idle, off_after));
        if want_sleep != asleep {
            asleep = want_sleep;
            ready = cmd(&mut i2c, &[if asleep { 0xAE } else { 0xAF }]).await;
        }
        if asleep {
            continue;
        }

        for c in 0..NCH {
            let l = state::meter(c);
            if l >= peaks[c] {
                peaks[c] = l;
                peak_at[c] = now;
            } else if now.saturating_duration_since(peak_at[c]).as_millis() > 900 {
                peaks[c] = peaks[c].saturating_sub(6);
            }
        }

        fb.clear();
        let popup = popup.filter(|p| alive && now.saturating_duration_since(p.at).as_millis() < POPUP_MS);
        let calm = if since_boot < SPLASH_MS {
            draw_logo(&mut fb, ms, (since_boot * 160 / 900) as i32);
            false
        } else if let Some(p) = popup {
            let target = p.volume.min(100) as u32 * 10;
            // A fresh popup starts at the value; while it stays open the bar glides.
            shown_vol = if !popup_open {
                target
            } else if target > shown_vol {
                shown_vol + (target - shown_vol + 2) / 3
            } else {
                shown_vol - (shown_vol - target + 2) / 3
            };
            draw_popup(&mut fb, &p, shown_vol);
            false
        } else if alive && idle < SCREENSAVER_MS {
            draw_mixer(&mut fb, &mixer, &peaks);
            false
        } else {
            draw_logo(&mut fb, ms, W);
            true
        };
        popup_open = popup.is_some();
        was_calm = calm;

        // Dim the panel on the idle logo to spare it.
        let want = if calm { 0x30 } else { 0xCF };
        if want != contrast {
            contrast = want;
            ready &= cmd(&mut i2c, &[0x81, want]).await;
        }

        if fb.buf[1..] != sent[..] {
            ready &= cmd(&mut i2c, &[0x21, 0, 127, 0x22, 0, 3]).await;
            ready &= i2c.write(ADDR, &fb.buf).await.is_ok();
            sent.copy_from_slice(&fb.buf[1..]);
        }
    }
}
