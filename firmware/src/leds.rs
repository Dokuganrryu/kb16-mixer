//! 16 WS2812 LEDs on PA10 (one per key, index = row * 4 + col), refreshed at 200 Hz.
//!
//! The look is deliberately quiet: one soft tint per channel, loudness shown as brightness,
//! and amber as the only accent (muted). Brightness is perceptual (gamma 2) and resolved to
//! 9 bits: the extra half step alternates every frame, i.e. at 100 Hz, which the eye cannot
//! see, so slow fades do not step.
//!
//! Bits are clocked out against the Cortex-M cycle counter with interrupts off (~0.5 ms per
//! frame) from a routine that lives in RAM, so flash wait states cannot stretch a pulse.

use crate::assets::SIN;
use crate::state::{self, METER, Mixer, NCH, RIPPLES, Ripple};
use core::sync::atomic::Ordering;
use embassy_stm32::gpio::Output;
use embassy_time::{Duration, Instant, Ticker};

const N: usize = 16;
// 72 MHz: 1 cycle = 13.9 ns. WS2812B wants T0H ~0.3 us, T1H ~0.75 us, bit 1.25 us.
// The wait loop overshoots by a few cycles, so the targets sit low in each window.
const T0H: u32 = 16;
const T1H: u32 = 50;
const BIT: u32 = 90;
/// Total current cap: sum of all 8-bit channels (about 45% of full white, ~430 mA).
const POWER_CAP: u32 = (N as u32) * 255 * 3 * 9 / 20;
const FADE_MS: u64 = 700;
const RIPPLE_MS: u64 = 520;
/// How low the lights settle once nothing has played or been touched for a while.
const IDLE_DIM: u32 = 70; // of 255

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
        for k in 0..3 {
            self.px[led][k] = c[k] as u32 * lin / 255;
        }
    }
}

/// Extra brightness (0..=255) at a key from the blooms spreading out of recent presses.
fn bloom(ripples: &[Option<Ripple>; RIPPLES], now: Instant, row: u32, col: u32) -> u32 {
    let mut best = 0;
    for r in ripples.iter().flatten() {
        let age = now.saturating_duration_since(r.at).as_millis();
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

fn render_mixer(
    f: &mut Frame,
    m: &Mixer,
    disp: &[u32; NCH],
    swell: u32,
    ripples: &[Option<Ripple>; RIPPLES],
    now: Instant,
) {
    let ms = now.as_millis();
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
fn render_idle(f: &mut Frame, ripples: &[Option<Ripple>; RIPPLES], now: Instant) {
    let ms = now.as_millis();
    for led in 0..N {
        let (row, col) = ((led / 4) as u32, (led % 4) as u32);
        let swell = wave((ms / 24) as u32 + (row + col) * 22); // ~6 s
        let glow = bloom(ripples, now, row, col) * 90 / 255;
        f.put(led, ICE, 104 + swell * 78 / 255 + glow);
    }
}

#[embassy_executor::task]
pub async fn run(_pin: Output<'static>) {
    let timed = cycle_counter_works();
    crate::diag::log(0x2000 | timed as u16);
    let mut disp = [0u32; NCH]; // meter ballistics, 8.8 fixed point
    let mut swell = 0u32; // overall loudness envelope, 8.8
    let mut mode_alive = false;
    let mut mode_since = Instant::now();
    let mut parity = false;
    let mut dim = 255u32 << 8; // idle dimming envelope, 8.8
    let mut dark = false;
    let mut last_px = [[0u32; 3]; N]; // previous frame before quantising
    let mut last_bytes = [0u8; N * 3];
    let mut still = 0u32; // frames in a row with no change
    let mut unsent = 0u32; // frames since the LEDs were last written
    let mut ticker = Ticker::every(Duration::from_millis(5));

    loop {
        ticker.next().await;
        // PC asleep: lights off, and nothing to do until it wakes.
        if state::suspended() {
            if !dark {
                dark = true;
                send(timed, &[0; N * 3]);
            }
            continue;
        }
        if dark {
            dark = false;
            mode_since = Instant::now(); // fade back in
        }
        let now = Instant::now();
        let (alive, mixer, brightness, ripples, activity, dim_after) =
            state::with(|s| (s.host_alive(now), s.mixer, s.brightness, s.ripples, s.activity, s.lights_dim_min));
        let idle = state::past(now.saturating_duration_since(activity).as_millis(), dim_after);
        let dim_target = if idle { IDLE_DIM << 8 } else { 255 << 8 };
        if dim < dim_target {
            dim = (dim + 600).min(dim_target); // back up in ~0.4 s
        } else {
            dim -= (dim - dim_target).min(40); // down over ~6 s
        }
        if alive != mode_alive {
            mode_alive = alive;
            mode_since = now;
        }

        // Meter ballistics: quick attack, ~170 ms release.
        let mut loudest = 0;
        for i in 0..NCH {
            let target = if alive { (mixer.level[i] as u32) << 8 } else { 0 };
            if target > disp[i] {
                disp[i] += (target - disp[i]) / 3 + 1;
            } else {
                disp[i] -= (disp[i] - target) / 16;
            }
            METER[i].store(disp[i].min(0xFFFF) as u16, Ordering::Relaxed);
            if mixer.mutes & (1 << i) == 0 {
                loudest = loudest.max(disp[i]);
            }
        }
        // The swell follows the loudest channel, slower still, so it reads as a breath.
        if loudest > swell {
            swell += (loudest - swell) / 12 + 1;
        } else {
            swell -= (swell - loudest) / 48;
        }

        let mut f = Frame { px: [[0; 3]; N] };
        if alive {
            render_mixer(&mut f, &mixer, &disp, swell >> 8, &ripples, now);
        } else {
            render_idle(&mut f, &ripples, now);
        }

        // Fade in after a mode change, then the master brightness.
        let env = (now.saturating_duration_since(mode_since).as_millis().min(FADE_MS) * 255 / FADE_MS) as u32;
        let master = brightness as u32 * env / 255 * (dim >> 8) / 255;

        // A picture that has stopped moving needs no half steps and no refresh: the LEDs
        // hold their colour, so the bus (and the interrupts-off window) can rest.
        let mut scaled = [[0u32; 3]; N];
        for i in 0..N {
            for k in 0..3 {
                scaled[i][k] = f.px[i][k] * master / 255;
            }
        }
        still = if scaled == last_px { still + 1 } else { 0 };
        last_px = scaled;
        let moving = still < 20;

        parity = !parity;
        let mut out = [[0u8; 3]; N];
        let mut total = 0u32;
        for i in 0..N {
            for k in 0..3 {
                // 9-bit value; while moving, the half step shows on alternate frames.
                let v9 = (scaled[i][k] + 64) >> 7;
                let half = if moving { v9 & 1 & parity as u32 } else { v9 & 1 };
                let v = ((v9 >> 1) + half).min(255);
                out[i][k] = v as u8;
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
        unsent += 1;
        if bytes != last_bytes || unsent >= 100 {
            send(timed, &bytes);
            last_bytes = bytes;
            unsent = 0;
        }
    }
}

fn send(timed: bool, bytes: &[u8; N * 3]) {
    if timed {
        write(bytes);
    } else {
        write_untimed(bytes);
    }
}

/// True if the DWT cycle counter runs (it does on this Cortex-M3; checked so a clone
/// without it can never hang the LED loop with interrupts off).
fn cycle_counter_works() -> bool {
    let a = cortex_m::peripheral::DWT::cycle_count();
    cortex_m::asm::delay(200);
    cortex_m::peripheral::DWT::cycle_count().wrapping_sub(a) >= 100
}

const DWT_CYCCNT: *const u32 = 0xE000_1004 as *const u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;
const SET: u32 = 1 << 10;
const CLEAR: u32 = 1 << 26;

/// Clocks the bytes out on PA10. Runs from RAM, interrupts off, timed by the cycle counter.
#[unsafe(link_section = ".data.ws2812")]
#[inline(never)]
fn write(bytes: &[u8; N * 3]) {
    let was_enabled = cortex_m::register::primask::read().is_active();
    cortex_m::interrupt::disable();
    unsafe {
        for &byte in bytes.iter() {
            let mut mask = 0x80u8;
            while mask != 0 {
                let high = if byte & mask != 0 { T1H } else { T0H };
                let t = DWT_CYCCNT.read_volatile();
                GPIOA_BSRR.write_volatile(SET);
                while DWT_CYCCNT.read_volatile().wrapping_sub(t) < high {}
                GPIOA_BSRR.write_volatile(CLEAR);
                while DWT_CYCCNT.read_volatile().wrapping_sub(t) < BIT {}
                mask >>= 1;
            }
        }
        if was_enabled {
            cortex_m::interrupt::enable();
        }
    }
}

/// Fallback bit timing from instruction counts (only if the cycle counter is missing).
fn write_untimed(bytes: &[u8; N * 3]) {
    cortex_m::interrupt::free(|_| unsafe {
        for &byte in bytes.iter() {
            let mut mask = 0x80u8;
            while mask != 0 {
                let one = byte & mask != 0;
                GPIOA_BSRR.write_volatile(SET);
                cortex_m::asm::delay(if one { 44 } else { 12 });
                GPIOA_BSRR.write_volatile(CLEAR);
                cortex_m::asm::delay(if one { 24 } else { 56 });
                mask >>= 1;
            }
        }
    });
}
