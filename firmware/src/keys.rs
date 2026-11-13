//! 4x5 key matrix (diodes COL2ROW: rows are driven low, columns read with pull-ups)
//! and three quadrature knobs, scanned at 1 kHz. The decoding is in `pad_core::input`.

use crate::state;
use embassy_stm32::gpio::{Flex, Input, Pull, Speed};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker};
use pad_core::input::{Combo, Debounce, Knob, Turns};
use pad_core::route::Event;

pub const ROWS: usize = 4;
pub const COLS: usize = 5;

pub static EVENTS: Channel<CriticalSectionRawMutex, Event, 64> = Channel::new();

pub struct Scanner {
    pub rows: [Flex<'static>; ROWS],
    pub cols: [Input<'static>; COLS],
    pub knobs: [(Input<'static>, Input<'static>); 3],
}

#[embassy_executor::task]
pub async fn scan(mut s: Scanner) {
    for r in s.rows.iter_mut() {
        r.set_as_input(Pull::Up);
    }
    let mut keys = [[Debounce::new(); COLS]; ROWS];
    let read = |(a, b): &(Input<'static>, Input<'static>)| a.is_high() as u8 | (b.is_high() as u8) << 1;
    let mut knobs = [Knob::new(read(&s.knobs[0])), Knob::new(read(&s.knobs[1])), Knob::new(read(&s.knobs[2]))];
    let mut turns = [Turns::new(); 3];
    let mut combo = Combo::new();
    let mut ticker = Ticker::every(Duration::from_millis(1));

    loop {
        for (row, keys) in keys.iter_mut().enumerate() {
            s.rows[row].set_low();
            s.rows[row].set_as_output(Speed::Low);
            cortex_m::asm::delay(72); // ~1 us for the line to settle
            for (col, key) in keys.iter_mut().enumerate() {
                key.sample(s.cols[col].is_low(), |pressed| {
                    EVENTS.try_send(Event::Key { row: row as u8, col: col as u8, pressed }).is_ok()
                });
            }
            s.rows[row].set_as_input(Pull::Up);
            cortex_m::asm::delay(72);
        }

        for (i, pins) in s.knobs.iter().enumerate() {
            if let Some(cw) = knobs[i].update(read(pins)) {
                turns[i].add(cw);
            }
            turns[i].drain(|cw| EVENTS.try_send(Event::Turn { knob: i as u8, cw }).is_ok());
        }

        // Hold all four corner keys for 2 s: restart into the bootloader.
        let corners = [keys[0][0], keys[0][3], keys[3][0], keys[3][3]];
        if combo.tick(corners.iter().all(Debounce::pressed)) {
            state::enter_bootloader();
        }

        ticker.next().await;
    }
}
