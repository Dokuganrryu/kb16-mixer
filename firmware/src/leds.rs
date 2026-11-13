//! 16 WS2812 LEDs on PA10 (one per key, index = row * 4 + col), refreshed at 200 Hz. What
//! they show is worked out in `pad_core::leds`; this file clocks it out.
//!
//! Bits are clocked out against the Cortex-M cycle counter with interrupts off (~0.5 ms per
//! frame) from a routine that lives in RAM, so flash wait states cannot stretch a pulse.

use crate::state::{self, METER, Tick};
use core::sync::atomic::Ordering;
use embassy_stm32::gpio::Output;
use embassy_time::{Duration, Ticker};
use pad_core::leds::{Bytes, Input, Lights};

// 72 MHz: 1 cycle = 13.9 ns. WS2812B wants T0H ~0.3 us, T1H ~0.75 us, bit 1.25 us.
// The wait loop overshoots by a few cycles, so the targets sit low in each window.
const T0H: u32 = 16;
const T1H: u32 = 50;
const BIT: u32 = 90;

#[embassy_executor::task]
pub async fn run(_pin: Output<'static>) {
    let timed = cycle_counter_works();
    crate::diag::log(0x2000 | timed as u16);
    let mut lights = Lights::new(Tick::now());
    let mut ticker = Ticker::every(Duration::from_millis(5));

    loop {
        ticker.next().await;
        // PC asleep: lights off, and nothing to do until it wakes.
        if state::suspended() {
            if let Some(off) = lights.suspended() {
                send(timed, &off);
            }
            continue;
        }
        lights.resume(Tick::now()); // fade back in
        let now = Tick::now();
        let input = state::with(|s| Input {
            alive: s.host_alive(now),
            mixer: s.mixer,
            brightness: s.brightness,
            ripples: s.ripples,
            activity: s.activity,
            dim_after: s.lights_dim_min,
            idle_percent: s.lights_idle_percent,
        });
        let bytes = lights.frame(now, &input);
        for (m, v) in METER.iter().zip(lights.meters()) {
            m.store(v, Ordering::Relaxed);
        }
        if let Some(bytes) = bytes {
            send(timed, &bytes);
        }
    }
}

fn send(timed: bool, bytes: &Bytes) {
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

// Raw addresses on purpose: the bit loops below are timed to the cycle, and `write` must run
// entirely from RAM. A PAC or cortex-m accessor is a call that may stay out of line, in flash,
// and stretch a pulse.
const DWT_CYCCNT: *const u32 = 0xE000_1004 as *const u32;
const GPIOA_BSRR: *mut u32 = 0x4001_0810 as *mut u32;
const SET: u32 = 1 << 10;
const CLEAR: u32 = 1 << 26;

/// Clocks the bytes out on PA10. Runs from RAM, interrupts off, timed by the cycle counter.
#[unsafe(link_section = ".data.ws2812")]
#[inline(never)]
fn write(bytes: &Bytes) {
    let was_enabled = cortex_m::register::primask::read().is_active();
    cortex_m::interrupt::disable();
    // SAFETY: DWT_CYCCNT is the always-readable DWT cycle counter, and BSRR is GPIOA's
    // write-only set/reset register: a write changes only PA10, which `run` owns as `_pin`.
    // Interrupts are re-enabled only if they were enabled on entry.
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
fn write_untimed(bytes: &Bytes) {
    // SAFETY: as in `write`: the BSRR writes change only PA10, which `run` owns.
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
