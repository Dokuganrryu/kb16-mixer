//! SSD1306 128x32 on I2C2. Frames are drawn into RAM (by `pad_core::oled`) and sent with DMA
//! (async, 400 kHz), only when something changed, so the display never stalls the keys or the
//! LEDs.

use crate::state::{self, NCH, Tick};
use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Async;
use embassy_time::{Duration, Instant, Ticker, Timer};
use pad_core::Time;
use pad_core::oled::{Fb, H, Input, SPLASH_MS, Screen, W, contrast};
use pad_core::state::{DEFAULT_OLED_TOP, past};

const ADDR: u8 = 0x3C;

type Bus = I2c<'static, Async, Master>;

async fn cmd(i2c: &mut Bus, bytes: &[u8]) -> bool {
    let mut b = [0u8; 32];
    b[1..=bytes.len()].copy_from_slice(bytes); // b[0] = 0x00: command stream
    i2c.write(ADDR, &b[..=bytes.len()]).await.is_ok()
}

async fn init(i2c: &mut Bus) -> bool {
    // Standard SSD1306 128x32 initialisation values.
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
    let mut fb = Fb::new(DEFAULT_OLED_TOP as i32);
    let mut sent = [0xFFu8; (W * H / 8) as usize];
    let mut screen = Screen::new(Tick::now());
    let mut asleep = false;
    let mut shown_contrast = 0u8;
    let boot = Instant::now();
    let mut woke = boot;
    let mut was_calm = false;
    let mut skip = false;
    let mut ticker = Ticker::every(Duration::from_millis(33));

    loop {
        ticker.next().await;
        // The idle logo moves slowly; drawing it at half rate halves the bus traffic.
        skip = was_calm && !skip;
        if skip && ready {
            let now = Tick::now();
            let busy = state::with(|s| s.host_alive(now) && now.ms_since(s.activity) < 500);
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
            shown_contrast = 0;
            continue;
        }
        let now = Tick::now();
        let since_boot = now.0.saturating_duration_since(boot).as_millis();
        let (mut input, top, off_after) = state::with(|s| {
            let input = Input {
                alive: s.host_alive(now),
                mixer: s.mixer,
                popup: s.popup,
                idle: now.ms_since(s.activity),
                name: s.logo_name,
                meters: [0; NCH], // read below, after any wait on the bus
            };
            (input, s.oled_top, s.screen_off_min)
        });
        fb.top = top as i32;

        // Sleep after long inactivity; any key, knob or app audio the host hears wakes it.
        let want_sleep = state::suspended() || (since_boot > SPLASH_MS && past(input.idle, off_after));
        if want_sleep != asleep {
            asleep = want_sleep;
            woke = now.0;
            ready = cmd(&mut i2c, &[if asleep { 0xAE } else { 0xAF }]).await;
        }
        if asleep {
            continue;
        }

        input.meters = core::array::from_fn(state::meter);
        let calm = screen.draw(&mut fb, now, since_boot, &input);
        was_calm = calm;

        // Dim the panel on the idle logo to spare it; fade out before sleep, and in on wake.
        let since_wake = now.0.saturating_duration_since(woke).as_millis();
        let want = contrast(if calm { 0x30 } else { 0xCF }, input.idle, off_after, since_wake);
        if want != shown_contrast {
            shown_contrast = want;
            ready &= cmd(&mut i2c, &[0x81, want]).await;
        }

        if fb.buf[1..] != sent[..] {
            ready &= cmd(&mut i2c, &[0x21, 0, 127, 0x22, 0, 3]).await;
            ready &= i2c.write(ADDR, &fb.buf).await.is_ok();
            sent.copy_from_slice(&fb.buf[1..]);
        }
    }
}
