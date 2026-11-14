//! SSD1306 128x32 on I2C2. Frames are drawn into RAM (by `pad_core::oled`) and sent with DMA
//! (async, 400 kHz), only the columns that changed, so the display never stalls the keys or the
//! LEDs.

use crate::state::{self, NCH, Tick};
use embassy_futures::select::{Either, select};
use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Async;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Ticker, Timer};
use pad_core::Time;
use pad_core::leds::DIM_MS;
use pad_core::oled::{Fb, H, Input, SPLASH_MS, Screen, W, contrast, dirty_spans};
use pad_core::state::{DEFAULT_OLED_TOP, past};

const ADDR: u8 = 0x3C;
/// The frame without the I2C control byte, as the panel holds it.
const FRAME: usize = (W * H / 8) as usize;
/// Bytes on the bus for one window: the address command (I2C address, control byte, six
/// command bytes), then the I2C address and control byte before the data.
const WINDOW: usize = 8 + 2;
/// A popup draws at once, but never sooner than this after the last draw.
const MIN_GAP: Duration = Duration::from_millis(8);
/// The contrast `init` sets (0x81, 0xCF), and the one a busy screen shows.
const INIT_CONTRAST: u8 = 0xCF;

/// Signalled when a popup arrives, so its readout draws now and not at the next tick.
pub static DRAW_NOW: Signal<CriticalSectionRawMutex, ()> = Signal::new();

type Bus = I2c<'static, Async, Master>;

async fn cmd(i2c: &mut Bus, bytes: &[u8]) -> bool {
    let mut b = [0u8; 32];
    b[1..=bytes.len()].copy_from_slice(bytes); // b[0] = 0x00: command stream
    i2c.write(ADDR, &b[..=bytes.len()]).await.is_ok()
}

/// Leaves the panel off: the loop turns it on after the first frame is in its RAM, so the
/// power-up contents never show.
async fn init(i2c: &mut Bus) -> bool {
    // Standard SSD1306 128x32 initialisation values.
    cmd(
        i2c,
        &[
            0xAE, 0xD5, 0x80, 0xA8, 0x1F, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xD3, 0x00, 0xDA, 0x02, 0x81, 0xCF,
            0xD9, 0xF1, 0xDB, 0x20, 0xA4, 0xA6, 0x2E,
        ],
    )
    .await
}

/// Sends what differs between `fb` and `sent` (the panel's RAM), or the whole frame when `full`
/// or when the windows would cost more bytes than the frame. False if the bus failed; then
/// `sent` is unchanged and the caller must send the full frame next time.
async fn send(i2c: &mut Bus, fb: &mut Fb, sent: &mut [u8; FRAME], full: bool) -> bool {
    const PAGES: u8 = (H / 8) as u8;
    // Windows as (first column, last column, first page, last page). One `cmd` and one `write`
    // below serve them all: each await site inlines the whole I2C state machine.
    let mut windows = [(0u8, 0u8, 0u8, 0u8); 2 * PAGES as usize];
    let mut n = 0;
    let mut bytes = 0;
    if !full {
        for page in 0..PAGES {
            for (c0, c1) in dirty_spans(page, &fb.buf[1..], sent).into_iter().flatten() {
                windows[n] = (c0, c1, page, page);
                n += 1;
                bytes += WINDOW + (c1 - c0) as usize + 1;
            }
        }
    }
    if full || bytes > WINDOW + FRAME {
        windows[0] = (0, (W - 1) as u8, 0, PAGES - 1);
        n = 1;
    }
    for &(c0, c1, p0, p1) in &windows[..n] {
        if !cmd(i2c, &[0x21, c0, c1, 0x22, p0, p1]).await {
            return false;
        }
        // The data goes out straight from `fb.buf` in one transfer, which needs the 0x40 control
        // byte just before it: borrow the byte before the window for it (byte 0 already is),
        // then put it back. `write` holds the shared borrow until the DMA is done, and stops the
        // DMA if dropped, so the byte cannot change under the transfer.
        let at = p0 as usize * W as usize + c0 as usize;
        let end = 1 + p1 as usize * W as usize + c1 as usize;
        let data = fb.buf[0];
        let kept = core::mem::replace(&mut fb.buf[at], data);
        let ok = i2c.write(ADDR, &fb.buf[at..=end]).await.is_ok();
        fb.buf[at] = kept;
        if !ok {
            return false;
        }
    }
    sent.copy_from_slice(&fb.buf[1..]);
    true
}

#[embassy_executor::task]
pub async fn run(mut i2c: Bus) {
    Timer::after_millis(100).await; // panel power-up
    crate::diag::log(0x4000);
    let mut ready = init(&mut i2c).await;
    crate::diag::log(0x4100 | ready as u16);
    let mut fb = Fb::new(DEFAULT_OLED_TOP as i32);
    let mut sent = [0u8; FRAME];
    // The panel RAM holds noise after init.
    let mut full = true;
    let mut panel_on = false;
    let mut screen = Screen::new(Tick::now());
    // What `init` sets.
    let mut shown_contrast = INIT_CONTRAST;
    let boot = Instant::now();
    let mut woke = boot;
    let mut drawn = boot;
    let mut was_calm = false;
    let mut skip = false;
    let mut ticker = Ticker::every(Duration::from_millis(33));

    loop {
        let popup = matches!(select(ticker.next(), DRAW_NOW.wait()).await, Either::Second(()));
        // The idle logo moves slowly; drawing it at half rate halves the bus traffic.
        skip = was_calm && !skip && !popup;
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
            panel_on = false;
            full = true;
            shown_contrast = INIT_CONTRAST;
            continue;
        }
        Timer::at(drawn + MIN_GAP).await;
        drawn = Instant::now();
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
                style: s.screen_style,
            };
            (input, s.oled_top, s.screen_off_min)
        });
        fb.top = top as i32;

        // Sleep once the fade after the idle limit is over; any key, knob or app audio the host
        // hears wakes it.
        let fade_done = past(input.idle.saturating_sub(DIM_MS), off_after);
        let on = !state::suspended() && !(since_boot > SPLASH_MS && fade_done);
        if on {
            input.meters = core::array::from_fn(state::meter);
            let calm = screen.draw(&mut fb, now, since_boot, &input);
            was_calm = calm;

            // Dim the panel on the idle logo to spare it, fade it out after the idle limit, and
            // fade it in after it turns on (it turns on below, after this frame, at contrast 0).
            let since_wake = if panel_on { now.0.saturating_duration_since(woke).as_millis() } else { 0 };
            let want = contrast(if calm { 0x30 } else { INIT_CONTRAST }, input.idle, off_after, since_wake);
            if want != shown_contrast {
                shown_contrast = want;
                ready &= cmd(&mut i2c, &[0x81, want]).await;
            }
            ready = ready && send(&mut i2c, &mut fb, &mut sent, full).await;
            full = !ready;
        }
        // Display on only after the frame: a wake shows the new frame, never the one from before.
        if ready && on != panel_on {
            panel_on = on;
            woke = Instant::now();
            // A wake sends the whole frame again, so a column the panel lost while off heals.
            full |= !on;
            ready = cmd(&mut i2c, &[if on { 0xAF } else { 0xAE }]).await;
        }
    }
}
