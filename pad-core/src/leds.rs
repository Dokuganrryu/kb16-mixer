//! What the 16 key lights show, one 200 Hz frame at a time (index = row * 4 + col). The
//! firmware's `leds.rs` clocks the bytes out to the WS2812 chain.
//!
//! The look is deliberately quiet: one soft tint per channel, loudness shown as brightness,
//! and amber as the only accent (muted). Brightness is perceptual (gamma 2) and resolved to
//! 9 bits: the extra half step alternates every frame, i.e. at 100 Hz, which the eye cannot
//! see, so slow fades do not step.

use crate::Time;
use crate::assets::SIN;
use crate::state::{Mixer, NCH, RIPPLES, Ripple, past};

pub const N: usize = 16;
/// One frame for the chain: green, red, blue per LED.
pub type Bytes = [u8; N * 3];
/// Total current cap: sum of all 8-bit channels (about 45% of full white, ~430 mA).
const POWER_CAP: u32 = (N as u32) * 255 * 3 * 9 / 20;
const FADE_MS: u64 = 700;
const RIPPLE_MS: u64 = 520;
/// How low the lights settle once nothing has played or been touched for a while. It scales
/// light output (after gamma), so it has to be this low to read as dimmed at full brightness.
const IDLE_DIM: u32 = 35; // of 255

const AMBER: [u8; 3] = [255, 70, 12];
const ICE: [u8; 3] = [185, 212, 255];

/// Sine in 0..=255 for a phase in 0..=255.
fn wave(phase: u32) -> u32 {
    SIN[(phase & 0xFF) as usize] as u32
}

fn isqrt(v: u32) -> u32 {
    let mut r = 0;
    let mut bit = 1 << 14;
    while bit != 0 {
        if (r + bit) * (r + bit) <= v {
            r += bit;
        }
        bit >>= 1;
    }
    r
}

struct Frame {
    /// Linear RGB per LED, 0..=65025.
    px: [[u32; 3]; N],
}

impl Frame {
    /// `bright` is perceptual 0..=255; gamma 2 turns it linear.
    fn put(&mut self, led: usize, c: [u8; 3], bright: u32) {
        let b = bright.min(255);
        let lin = b * b;
        for (p, &c) in self.px[led].iter_mut().zip(c.iter()) {
            *p = c as u32 * lin / 255;
        }
    }
}

/// Extra brightness (0..=255) at a key from the blooms spreading out of recent presses.
fn bloom<T: Time>(ripples: &[Option<Ripple<T>>; RIPPLES], now: T, row: u32, col: u32) -> u32 {
    let mut best = 0;
    for r in ripples.iter().flatten() {
        let age = now.ms_since(r.at);
        if age >= RIPPLE_MS {
            continue;
        }
        let (dr, dc) = (row as i32 - r.row as i32, col as i32 - r.col as i32);
        let dist = isqrt(((dr * dr + dc * dc) as u32) << 12); // key units * 64
        let front = (age as u32) * 64 * 8 / 1000; // 8 keys per second
        let off = dist.abs_diff(front);
        if off < 80 {
            let fade = (RIPPLE_MS - age) as u32 * 255 / RIPPLE_MS as u32;
            best = best.max((80 - off) * 255 / 80 * fade / 255);
        }
    }
    best
}

fn render_mixer<T: Time>(
    f: &mut Frame,
    m: &Mixer,
    disp: &[u32; NCH],
    swell: u32,
    ripples: &[Option<Ripple<T>>; RIPPLES],
    now: T,
) {
    let ms = now.ms();
    for led in 0..N {
        let (row, col) = ((led / 4) as u32, led % 4);
        if col >= m.n as usize {
            continue;
        }
        let tint = m.colors[col];
        let muted = m.mutes & (1 << col) != 0;
        let sel = col as u8 == m.selected;
        let glow = bloom(ripples, now, row, col as u32) * 90 / 255;

        if muted {
            // The one accent: a low amber that breathes very slowly.
            let breath = wave((ms / 14) as u32);
            let b = if row == 3 { 135 + breath * 55 / 255 } else { 62 + breath * 14 / 255 };
            f.put(led, AMBER, b + glow);
            continue;
        }
        if row == 3 {
            f.put(led, tint, if sel { 215 } else { 128 } + glow);
            continue;
        }
        // Meter: row 2 is the bottom segment, row 0 the top. The whole pad swells with the sound.
        let floor = if sel { 124 } else { 98 } + swell * 26 / 255;
        let seg = 2 - row;
        let level = disp[col] >> 8;
        let fill = (level * 3).saturating_sub(seg * 255).min(255);
        f.put(led, tint, floor + (255 - floor) * fill / 255 + glow);
    }
}

/// No PC app: ice white with a slow diagonal swell.
fn render_idle<T: Time>(f: &mut Frame, ripples: &[Option<Ripple<T>>; RIPPLES], now: T) {
    let ms = now.ms();
    for led in 0..N {
        let (row, col) = ((led / 4) as u32, (led % 4) as u32);
        let swell = wave((ms / 24) as u32 + (row + col) * 22); // ~6 s
        let glow = bloom(ripples, now, row, col) * 90 / 255;
        f.put(led, ICE, 104 + swell * 78 / 255 + glow);
    }
}

/// What one frame needs from the shared state.
pub struct Input<T> {
    pub alive: bool,
    pub mixer: Mixer,
    pub brightness: u8,
    pub ripples: [Option<Ripple<T>>; RIPPLES],
    pub activity: T,
    pub dim_after: u8,
}

/// Everything the lights remember from one frame to the next.
pub struct Lights<T> {
    /// Meter ballistics, 8.8 fixed point.
    disp: [u32; NCH],
    /// Overall loudness envelope, 8.8.
    swell: u32,
    mode_alive: bool,
    mode_since: T,
    parity: bool,
    /// Idle dimming envelope, 8.8.
    dim: u32,
    dark: bool,
    /// Previous frame before quantising.
    last_px: [[u32; 3]; N],
    last_bytes: Bytes,
    /// Frames in a row with no change.
    still: u32,
    /// Frames since the LEDs were last written.
    unsent: u32,
}

impl<T: Time> Lights<T> {
    pub fn new(now: T) -> Lights<T> {
        Lights {
            disp: [0; NCH],
            swell: 0,
            mode_alive: false,
            mode_since: now,
            parity: false,
            dim: 255 << 8,
            dark: false,
            last_px: [[0; 3]; N],
            last_bytes: [0; N * 3],
            still: 0,
            unsent: 0,
        }
    }

    /// A frame while the PC is asleep: all off, sent once.
    pub fn suspended(&mut self) -> Option<Bytes> {
        if self.dark {
            return None;
        }
        self.dark = true;
        Some([0; N * 3])
    }

    /// The first frame after the PC wakes fades in from `now`.
    pub fn resume(&mut self, now: T) {
        if self.dark {
            self.dark = false;
            self.mode_since = now;
        }
    }

    /// The meter levels after ballistics (8.8 fixed point), which the screen shows too.
    pub fn meters(&self) -> [u16; NCH] {
        self.disp.map(|d| d.min(0xFFFF) as u16)
    }

    /// One frame. Returns the bytes to send, or `None` when the LEDs already show them.
    pub fn frame(&mut self, now: T, s: &Input<T>) -> Option<Bytes> {
        let idle = past(now.ms_since(s.activity), s.dim_after);
        let dim_target = if idle { IDLE_DIM << 8 } else { 255 << 8 };
        if self.dim < dim_target {
            self.dim = (self.dim + 600).min(dim_target); // back up in ~0.4 s
        } else {
            self.dim -= (self.dim - dim_target).min(40); // down over ~6 s
        }
        if s.alive != self.mode_alive {
            self.mode_alive = s.alive;
            self.mode_since = now;
        }

        // Meter ballistics: quick attack, ~170 ms release.
        let mut loudest = 0;
        for (i, (d, &level)) in self.disp.iter_mut().zip(s.mixer.level.iter()).enumerate() {
            let target = if s.alive { (level as u32) << 8 } else { 0 };
            if target > *d {
                *d += (target - *d) / 3 + 1;
            } else {
                *d -= (*d - target) / 16;
            }
            if s.mixer.mutes & (1 << i) == 0 {
                loudest = loudest.max(*d);
            }
        }
        // The swell follows the loudest channel, slower still, so it reads as a breath.
        if loudest > self.swell {
            self.swell += (loudest - self.swell) / 12 + 1;
        } else {
            self.swell -= (self.swell - loudest) / 48;
        }

        let mut f = Frame { px: [[0; 3]; N] };
        if s.alive {
            render_mixer(&mut f, &s.mixer, &self.disp, self.swell >> 8, &s.ripples, now);
        } else {
            render_idle(&mut f, &s.ripples, now);
        }

        // Fade in after a mode change, then the master brightness.
        let env = (now.ms_since(self.mode_since).min(FADE_MS) * 255 / FADE_MS) as u32;
        let master = s.brightness as u32 * env / 255 * (self.dim >> 8) / 255;

        // A picture that has stopped moving needs no half steps and no refresh: the LEDs
        // hold their colour, so the bus (and the interrupts-off window) can rest.
        let scaled = f.px.map(|px| px.map(|v| v * master / 255));
        self.still = if scaled == self.last_px { self.still + 1 } else { 0 };
        self.last_px = scaled;
        let moving = self.still < 20;

        self.parity = !self.parity;
        let mut out = [[0u8; 3]; N];
        let mut total = 0u32;
        for (o, px) in out.iter_mut().zip(scaled.iter()) {
            for (o, &v) in o.iter_mut().zip(px.iter()) {
                // 9-bit value; while moving, the half step shows on alternate frames.
                let v9 = (v + 64) >> 7;
                let half = if moving { v9 & 1 & self.parity as u32 } else { v9 & 1 };
                let v = ((v9 >> 1) + half).min(255);
                *o = v as u8;
                total += v;
            }
        }
        if total > POWER_CAP {
            for px in out.iter_mut() {
                for c in px.iter_mut() {
                    *c = (*c as u32 * POWER_CAP / total) as u8;
                }
            }
        }
        // GRB byte stream.
        let mut bytes = [0u8; N * 3];
        for (i, p) in out.iter().enumerate() {
            bytes[i * 3] = p[1];
            bytes[i * 3 + 1] = p[0];
            bytes[i * 3 + 2] = p[2];
        }
        // Rewrite at least every half second anyway, in case a glitch upset an LED.
        self.unsent += 1;
        if bytes != self.last_bytes || self.unsent >= 100 {
            self.last_bytes = bytes;
            self.unsent = 0;
            Some(bytes)
        } else {
            None
        }
    }
}
