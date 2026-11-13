//! What the 16 key lights show, one 200 Hz frame at a time (index = row * 4 + col). The
//! firmware's `leds.rs` clocks the bytes out to the WS2812 chain.
//!
//! The look is deliberately quiet: one near-white tint per channel, loudness shown as
//! brightness, and amber as the only accent (muted).
//!
//! How a frame is made, and why:
//! - `render_*` picks a target per LED: a perceptual brightness (16 bit) and a colour. Colours
//!   are linear light (what the LED emits, per channel, 0..=255), so a crossfade between two
//!   colours mixes them as light does. Brightness is perceptual, so equal steps look equal.
//! - Each LED glides to its target: exponential smoothing with a quick attack (~35 ms) and a
//!   slow release (~250 ms), so a meter that the host updates ~30 times a second moves at the
//!   full 200 Hz without steps. The smoothing state keeps 24 bits; every step moves at least
//!   one unit and never past the target, so it is monotone and lands exactly.
//! - Gamma 2.4 (a 65-point table) turns brightness into light.
//! - Idle dimming and the fade-in after a mode change are eased curves in time.
//! - A turned knob shows on its own column: for `POPUP_MS` the three meter LEDs show the fader
//!   in place of the sound, with the same glides, so the screen can cut while the lights move.
//! - The 8-bit LEDs get temporal dithering: the light is resolved to a quarter of a step and
//!   the remainder carries to the next frame, so slow fades near black do not stair-step.

use crate::Time;
use crate::assets::SIN;
use crate::state::{Mixer, NCH, POPUP_MS, RIPPLES, Ripple, past};
use kb16_mixer_protocol::NEVER;

pub const N: usize = 16;
/// One frame for the chain: green, red, blue per LED.
pub type Bytes = [u8; N * 3];
/// Full scale of brightness and of light, 16 bit.
const FULL: u32 = 0xFFFF;
/// Total current cap, as a sum of 16-bit channel values: about 45% of all 48 channels at full,
/// ~430 mA for 16 WS2812 at ~20 mA per channel. With the MCU and the screen that stays inside
/// the 500 mA a USB 2 port promises.
const POWER_CAP: u32 = (N as u32) * FULL * 3 * 9 / 20;
const FADE_MS: u64 = 700;
const RIPPLE_MS: u64 = 520;
/// Idle easing: down to the dim level, down to off, and back up on any touch or sound. The
/// screen fades out over the same time from the same moment (with the default limits), so the
/// pad goes quiet as one.
pub const DIM_MS: u64 = 700;
const OFF_MS: u64 = 2000;
const WAKE_MS: u64 = 150;
/// How much light the dimmed lights keep, in percent of normal, unless the host sets it.
/// Light, not perceived brightness: 6% of the light looks like about a third as bright, which
/// reads as clearly dimmed but alive.
pub const DEFAULT_IDLE_PERCENT: u8 = 6;
/// The lights go off after this many times the dim time, at most `OFF_CAP_MIN` minutes.
const OFF_FACTOR: u32 = 3;
const OFF_CAP_MIN: u32 = 240;

/// Smoothing per 5 ms frame, as a fraction of 65536 of the way to the target: 1 - e^(-5/tau).
const ATTACK: u64 = 8724; // tau 35 ms
const RELEASE: u64 = 1297; // tau 250 ms
const RECOLOUR: u64 = 5243; // tau 60 ms: a mute turns amber quickly, but never jumps

// Colours, linear light per channel. The WS2812 blue die is the most efficient, so a raw
// (255, 255, 255) already looks cold; these values are chosen against that.
/// Muted: the one accent. Mostly red with a little green reads as warm amber on the LED; the
/// trace of blue keeps it from turning orange at low brightness.
const AMBER: [u8; 3] = [255, 70, 12];
/// No PC app: ice white, the base of the whole look.
const ICE: [u8; 3] = [185, 212, 255];
/// Channel colours whose component is below this (linear) are not lifted to one step when the
/// LED is nearly dark, so amber stays warm instead of turning white.
const LIFT_MIN: u32 = 64 << 8;

/// Perceptual 0..=65536 (64 steps of 1024) to linear light 0..=65535: round(65535 * x^2.4).
const GAMMA: [u16; 65] = [
    0, 3, 16, 42, 84, 144, 223, 324, 446, 591, 761, 957, 1179, 1429, 1707, 2015, 2352, 2721, 3121, 3553, 4019, 4518,
    5052, 5621, 6225, 6866, 7544, 8259, 9012, 9804, 10635, 11506, 12417, 13368, 14361, 15396, 16473, 17592, 18755,
    19962, 21212, 22507, 23847, 25233, 26664, 28142, 29666, 31237, 32856, 34523, 36238, 38002, 39815, 41678, 43590,
    45552, 47566, 49630, 51745, 53912, 56131, 58403, 60727, 63104, 65535,
];

/// Perceptual brightness (0..=65535) to linear light (0..=65535). Monotone; 0 and full exact.
pub fn gamma(p: u32) -> u32 {
    if p >= FULL {
        return FULL;
    }
    let (i, frac) = ((p >> 10) as usize, p & 1023);
    let (a, b) = (GAMMA[i] as u32, GAMMA[i + 1] as u32);
    a + (((b - a) * frac) >> 10)
}

/// The brightness whose light is `lin`: the largest `p` with `gamma(p) <= lin`.
pub fn ungamma(lin: u32) -> u32 {
    let (mut lo, mut hi) = (0, FULL);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if gamma(mid) <= lin { lo = mid } else { hi = mid - 1 }
    }
    lo
}

/// 0..=65536 progress of `ms` through `dur`.
fn progress(ms: u64, dur: u64) -> u64 {
    ms.min(dur) * 65536 / dur
}

/// Slow in, slow out: 3t^2 - 2t^3, in and out 0..=65536.
fn smoothstep(t: u64) -> u64 {
    (t * t * (3 * 65536 - 2 * t)) >> 32
}

/// Quick start, gentle landing: 1 - (1 - t)^2. A wake must answer at once.
fn ease_out(t: u64) -> u64 {
    65536 - (((65536 - t) * (65536 - t)) >> 16)
}

/// Sine in 0..=255 for a phase in 0..=255.
fn wave(phase: u32) -> u32 {
    SIN[(phase & 0xFF) as usize] as u32
}

/// 0..=255 points of brightness to 16 bit.
const fn pt(points: u32) -> u32 {
    points * 257
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

/// One frame's targets.
struct Frame {
    /// Perceptual brightness, 0..=65535.
    bright: [u32; N],
    /// Linear colour; `None` keeps the colour the LED has, so a light that goes out fades in
    /// its own colour.
    colour: [Option<[u8; 3]>; N],
}

impl Frame {
    fn put(&mut self, led: usize, c: [u8; 3], bright: u32) {
        self.bright[led] = bright.min(FULL);
        self.colour[led] = Some(c);
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

// Brightness levels in points (0..=255, perceptual). Rows 0..=2 are the meter, row 3 the mute
// keys. Focus lifts its channel a little, never with a new colour, so the pad stays one family.
const METER_FLOOR: u32 = 115;
const METER_FLOOR_FOCUS: u32 = 141;
const SWELL: u32 = 26;
const MUTE_KEY: u32 = 144;
const MUTE_KEY_FOCUS: u32 = 220;
/// On the second key page the meter rows rest, even and dim, in ice white: the keys there are
/// not channels, and the mute row still is.
const PAGE2_REST: u32 = 84;

/// `faders`: a bit per channel whose meter shows its volume in place of its sound.
fn render_mixer<T: Time>(
    f: &mut Frame,
    m: &Mixer,
    disp: &[u32; NCH],
    faders: u8,
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
        let glow = pt(bloom(ripples, now, row, col as u32) * 90 / 255);

        if muted {
            // A low amber that breathes very slowly (~3.6 s), the mute key a little brighter.
            let breath = wave((ms / 14) as u32);
            let b = if row == 3 { 150 + breath * 50 / 255 } else { 79 + breath * 14 / 255 };
            f.put(led, AMBER, pt(b) + glow);
            continue;
        }
        if row == 3 {
            f.put(led, tint, pt(if sel { MUTE_KEY_FOCUS } else { MUTE_KEY }) + glow);
            continue;
        }
        if m.second_page {
            f.put(led, ICE, pt(PAGE2_REST) + glow);
            continue;
        }
        // Meter: row 2 is the bottom segment, row 0 the top. The whole pad swells with the sound.
        // A fader lifts its column's floor as focus does.
        let fader = faders & (1 << col) != 0;
        let floor = pt(if sel || fader { METER_FLOOR_FOCUS } else { METER_FLOOR }) + swell * SWELL / 255;
        let level = if fader { m.volume[col] as u32 * 0xFF00 / 100 } else { disp[col] };
        let seg = 2 - row;
        let fill = (level * 3).saturating_sub(seg * 0xFF00).min(0xFF00);
        f.put(led, tint, floor + (FULL - floor) * fill / 0xFF00 + glow);
    }
}

/// No PC app: ice white with a slow diagonal swell.
fn render_idle<T: Time>(f: &mut Frame, ripples: &[Option<Ripple<T>>; RIPPLES], now: T) {
    let ms = now.ms();
    for led in 0..N {
        let (row, col) = ((led / 4) as u32, (led % 4) as u32);
        let swell = wave((ms / 24) as u32 + (row + col) * 22); // ~6 s
        let glow = bloom(ripples, now, row, col) * 90 / 255;
        f.put(led, ICE, pt(121 + swell * 69 / 255 + glow));
    }
}

/// What one frame needs from the shared state.
pub struct Input<T> {
    pub alive: bool,
    pub mixer: Mixer,
    pub brightness: u8,
    pub ripples: [Option<Ripple<T>>; RIPPLES],
    /// The host that is there has sent a frame (`State::frame_seen`).
    pub frame_seen: bool,
    pub activity: T,
    pub dim_after: u8,
    /// Light the dimmed LEDs keep, in percent of normal; 0 = [`DEFAULT_IDLE_PERCENT`].
    pub idle_percent: u8,
}

/// Minutes of quiet before the lights go off: three times the dim time, at most four hours,
/// never before they dim.
pub fn off_after(dim_after: u8) -> u8 {
    if dim_after == NEVER {
        return NEVER;
    }
    let d = dim_after as u32;
    (d * OFF_FACTOR).min(OFF_CAP_MIN).max(d) as u8
}

/// The perceptual level the dimmed lights sit at.
pub fn idle_level(percent: u8) -> u32 {
    let pct = if percent == 0 { DEFAULT_IDLE_PERCENT } else { percent.min(100) };
    ungamma(FULL * pct as u32 / 100)
}

/// A level that eases from one value to another over time; a new target starts from wherever
/// the level is, so an interrupted fade never jumps.
#[derive(Clone, Copy)]
pub struct Ease<T> {
    from: u32,
    to: u32,
    since: T,
}

impl<T: Time> Ease<T> {
    pub fn new(level: u32, now: T) -> Ease<T> {
        Ease { from: level, to: level, since: now }
    }

    /// The level now: up with an ease-out over `WAKE_MS`, down with a smoothstep over `DIM_MS`,
    /// or over `OFF_MS` when going dark.
    pub fn value(&self, now: T) -> u32 {
        let ms = now.ms_since(self.since);
        let (from, to) = (self.from as u64, self.to as u64);
        if to >= from {
            (from + (((to - from) * ease_out(progress(ms, WAKE_MS))) >> 16)) as u32
        } else {
            let dur = if to == 0 { OFF_MS } else { DIM_MS };
            (from - (((from - to) * smoothstep(progress(ms, dur))) >> 16)) as u32
        }
    }

    pub fn aim(&mut self, to: u32, now: T) {
        if to != self.to {
            self.from = self.value(now);
            self.to = to;
            self.since = now;
        }
    }
}

/// One exponential smoothing step from `cur` toward `target`: at least one unit, never past it.
pub fn glide(cur: u32, target: u32, k: u64) -> u32 {
    let step = |d: u32| (((d as u64 * k) >> 16) as u32).clamp(1, d);
    if target > cur {
        cur + step(target - cur)
    } else if target < cur {
        cur - step(cur - target)
    } else {
        cur
    }
}

/// One LED channel's light (0..=65535) to an 8-bit value, with the remainder carried in `err`
/// (quarter steps). Above 16 steps the dither uses quarter steps: a pattern repeats at least at
/// 50 Hz and swings 1/16 or less, which nobody sees. From 2 to 16 steps it uses half steps
/// (100 Hz). Below 2 steps a one-step swing would be a large share of the light, so it rounds.
pub fn dither(lin: u32, err: &mut u8) -> u8 {
    let q = (lin.min(FULL) * 1020) >> 16; // quarter steps, 0..=1019
    if q < 8 {
        *err = 0;
        return ((q + 2) >> 2) as u8;
    }
    let (v, e) = if q < 64 { (q & !1, *err as u32 & 2) } else { (q, *err as u32) };
    let acc = v + e;
    *err = (acc & 3) as u8;
    (acc >> 2) as u8
}

/// Everything the lights remember from one frame to the next.
pub struct Lights<T> {
    /// Meter ballistics, 8.8 fixed point. The screen's meters use them too.
    disp: [u32; NCH],
    /// Overall loudness envelope, 8.8.
    swell: u32,
    /// The volumes of the last host frame; `None` until this host's first frame, which is not a
    /// change anybody made.
    vol: Option<[u8; NCH]>,
    /// When each channel's volume last changed.
    touch: [Option<T>; NCH],
    mode_alive: bool,
    mode_since: T,
    /// Idle dimming, perceptual 0..=65535 (full = awake).
    dim: Ease<T>,
    dark: bool,
    /// Smoothed perceptual brightness per LED, 16.8 fixed point.
    bright: [u32; N],
    /// Smoothed linear colour per LED, 8.8 fixed point.
    colour: [[u32; 3]; N],
    /// Dither remainder per LED channel, in quarter steps.
    err: [[u8; 3]; N],
    last_bytes: Bytes,
    /// Frames since the LEDs were last written.
    unsent: u32,
}

impl<T: Time> Lights<T> {
    pub fn new(now: T) -> Lights<T> {
        Lights {
            disp: [0; NCH],
            swell: 0,
            vol: None,
            touch: [None; NCH],
            mode_alive: false,
            mode_since: now,
            dim: Ease::new(FULL, now),
            dark: false,
            bright: [0; N],
            colour: [ICE.map(|c| (c as u32) << 8); N],
            // Staggered so that LEDs at the same level do not step on the same frame.
            err: core::array::from_fn(|i| core::array::from_fn(|c| ((i + c) & 3) as u8)),
            last_bytes: [0; N * 3],
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
        let quiet = now.ms_since(s.activity);
        let dim_target = if past(quiet, off_after(s.dim_after)) {
            0
        } else if past(quiet, s.dim_after) {
            idle_level(s.idle_percent)
        } else {
            FULL
        };
        self.dim.aim(dim_target, now);
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

        // The pad finds the moved channel by comparing volumes with the last frame.
        let known = (s.alive && s.frame_seen).then_some(s.mixer.volume);
        if let (Some(was), Some(is)) = (self.vol, known) {
            for ((t, a), b) in self.touch.iter_mut().zip(was).zip(is) {
                if a != b {
                    *t = Some(now);
                }
            }
        }
        self.vol = known;
        let mut faders = 0;
        for (i, t) in self.touch.iter().enumerate() {
            if t.is_some_and(|t| now.ms_since(t) < POPUP_MS) {
                faders |= 1 << i;
            }
        }

        let mut f = Frame { bright: [0; N], colour: [None; N] };
        if s.alive {
            render_mixer(&mut f, &s.mixer, &self.disp, faders, self.swell, &s.ripples, now);
        } else {
            render_idle(&mut f, &s.ripples, now);
        }

        // Master level, perceptual: brightness setting x fade-in after a mode change x idle.
        let fade = smoothstep(progress(now.ms_since(self.mode_since), FADE_MS)) as u32;
        let master = gamma((((pt(s.brightness as u32) * fade) >> 16) * self.dim.value(now)) >> 16);

        let mut light = [[0u32; 3]; N];
        let mut total = 0u32;
        let targets = f.bright.iter().zip(f.colour.iter());
        let state = self.bright.iter_mut().zip(self.colour.iter_mut());
        for (light, ((&target, want), (bright, colour))) in light.iter_mut().zip(targets.zip(state)) {
            let k = if target << 8 > *bright { ATTACK } else { RELEASE };
            *bright = glide(*bright, target << 8, k);
            if let Some(c) = want {
                for (cur, &t) in colour.iter_mut().zip(c.iter()) {
                    *cur = glide(*cur, (t as u32) << 8, RECOLOUR);
                }
            }
            let lin = (gamma(*bright >> 8) * master) >> 16;
            for (l, &c) in light.iter_mut().zip(colour.iter()) {
                // 8.8 colour to a 16-bit fraction (255 -> 65535); fits u32, no division.
                *l = (lin * (c + (c >> 8))) >> 16;
                total += *l;
            }
        }

        // Over the cap, everything scales down together (16-bit fraction, one division a frame).
        let cap = if total > POWER_CAP { POWER_CAP as u64 * 65536 / total as u64 } else { 65536 };
        let mut out = [[0u8; 3]; N];
        for ((o, l), (err, colour)) in out.iter_mut().zip(light.iter()).zip(self.err.iter_mut().zip(self.colour.iter()))
        {
            for ((o, &l), e) in o.iter_mut().zip(l.iter()).zip(err.iter_mut()) {
                let l = ((l as u64 * cap) >> 16) as u32;
                *o = dither(l, e);
            }
            // Nearly dark, one channel can round to a step while the others round to nothing,
            // and a white key glows blue. Any lit LED shows one step of each colour it has.
            if o.iter().any(|&v| v > 0) {
                for (v, &c) in o.iter_mut().zip(colour.iter()) {
                    if c >= LIFT_MIN {
                        *v = (*v).max(1);
                    }
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
