//! 4x5 key matrix (diodes COL2ROW: rows are driven low, columns read with pull-ups)
//! and three quadrature knobs, scanned at 1 kHz.

use crate::state;
use embassy_stm32::gpio::{Flex, Input, Pull, Speed};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker};

pub const ROWS: usize = 4;
pub const COLS: usize = 5;
const DEBOUNCE_MS: u8 = 5;
const BOOT_COMBO_MS: u16 = 2000;

#[derive(Clone, Copy)]
pub enum Event {
    Key { row: u8, col: u8, pressed: bool },
    Turn { knob: u8, cw: bool },
}

pub static EVENTS: Channel<CriticalSectionRawMutex, Event, 64> = Channel::new();

/// Contact steps per knob detent.
const STEPS_PER_DETENT: i8 = 4;

/// One quadrature knob. Its two contacts, read as `a | b << 1`, walk the cycle
/// 0, 1, 3, 2 one way round and the reverse the other way; four steps make a detent.
struct Knob {
    phase: u8,
    steps: i8,
}

/// Position of a contact reading in that cycle.
fn phase(ab: u8) -> u8 {
    [0, 1, 3, 2][ab as usize & 3]
}

impl Knob {
    fn new(ab: u8) -> Knob {
        Knob { phase: phase(ab), steps: 0 }
    }

    /// Takes a fresh reading. Returns the direction (true = clockwise) once a detent is complete.
    fn update(&mut self, ab: u8) -> Option<bool> {
        let p = phase(ab);
        // Two positions apart means a reading was missed: the direction is unknown, so no step.
        self.steps += match (p + 4 - self.phase) % 4 {
            1 => 1,
            3 => -1,
            _ => 0,
        };
        self.phase = p;
        let turned = if self.steps >= STEPS_PER_DETENT {
            Some(true)
        } else if self.steps <= -STEPS_PER_DETENT {
            Some(false)
        } else {
            None
        };
        self.steps %= STEPS_PER_DETENT;
        turned
    }
}

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
    let mut stable = [[false; COLS]; ROWS];
    let mut count = [[0u8; COLS]; ROWS];
    let read = |(a, b): &(Input<'static>, Input<'static>)| a.is_high() as u8 | (b.is_high() as u8) << 1;
    let mut knobs = [Knob::new(read(&s.knobs[0])), Knob::new(read(&s.knobs[1])), Knob::new(read(&s.knobs[2]))];
    // Detents not yet queued, per knob (+ clockwise).
    let mut turns = [0i8; 3];
    let mut combo_ms = 0u16;
    let mut ticker = Ticker::every(Duration::from_millis(1));

    loop {
        for row in 0..ROWS {
            s.rows[row].set_low();
            s.rows[row].set_as_output(Speed::Low);
            cortex_m::asm::delay(72); // ~1 us for the line to settle
            for col in 0..COLS {
                let raw = s.cols[col].is_low();
                if raw == stable[row][col] {
                    count[row][col] = 0;
                    continue;
                }
                count[row][col] = count[row][col].saturating_add(1);
                // A change counts as seen only once it is queued: with the queue full it is
                // tried again on the next scan, so a release is never lost (a lost release of a
                // shortcut would leave Ctrl or Shift held on the PC).
                if count[row][col] >= DEBOUNCE_MS
                    && EVENTS.try_send(Event::Key { row: row as u8, col: col as u8, pressed: raw }).is_ok()
                {
                    count[row][col] = 0;
                    stable[row][col] = raw;
                }
            }
            s.rows[row].set_as_input(Pull::Up);
            cortex_m::asm::delay(72);
        }

        for (i, pins) in s.knobs.iter().enumerate() {
            if let Some(cw) = knobs[i].update(read(pins)) {
                // Bounded, so a long stall cannot replay a whole spin afterwards.
                turns[i] = (turns[i] + if cw { 1 } else { -1 }).clamp(-8, 8);
            }
            if turns[i] != 0 && EVENTS.try_send(Event::Turn { knob: i as u8, cw: turns[i] > 0 }).is_ok() {
                turns[i] -= turns[i].signum();
            }
        }

        // Hold all four corner keys for 2 s: restart into the bootloader.
        if stable[0][0] && stable[0][3] && stable[3][0] && stable[3][3] {
            combo_ms += 1;
            if combo_ms >= BOOT_COMBO_MS {
                state::enter_bootloader();
            }
        } else {
            combo_ms = 0;
        }

        ticker.next().await;
    }
}
