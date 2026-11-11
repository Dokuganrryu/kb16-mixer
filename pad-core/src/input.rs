//! Key and knob decoding for the 1 kHz scan in `firmware/src/keys.rs`: debounce, quadrature
//! knobs, the queue of knob detents, and the four-corner bootloader combo.
//!
//! Each `queue` callback is the firmware's non-blocking send into its event channel. It
//! returns false when the channel is full: then nothing counts as seen, and the next scan offers
//! the same event again. A release is thus never lost (a lost release of a shortcut would leave
//! Ctrl or Shift held on the PC).

/// Scans (1 ms each) a key must read the same before the change counts.
pub const DEBOUNCE_MS: u8 = 5;
/// Scans (1 ms each) the four corner keys must be held to restart into the bootloader.
pub const BOOT_COMBO_MS: u16 = 2000;
/// Contact steps per knob detent.
const STEPS_PER_DETENT: i8 = 4;
/// Detents a knob may have waiting, so a long stall cannot replay a whole spin afterwards.
const MAX_QUEUED_TURNS: i8 = 8;

/// One key of the matrix.
#[derive(Clone, Copy, Default)]
pub struct Debounce {
    stable: bool,
    count: u8,
}

impl Debounce {
    pub const fn new() -> Debounce {
        Debounce { stable: false, count: 0 }
    }

    /// The debounced state: true while the key is down.
    pub fn pressed(&self) -> bool {
        self.stable
    }

    /// Takes one scan's reading (true = down). Once a change has held for `DEBOUNCE_MS` scans,
    /// offers the new state to `queue`, and keeps it if `queue` takes it.
    pub fn sample(&mut self, raw: bool, queue: impl FnOnce(bool) -> bool) {
        if raw == self.stable {
            self.count = 0;
            return;
        }
        self.count = self.count.saturating_add(1);
        if self.count >= DEBOUNCE_MS && queue(raw) {
            self.count = 0;
            self.stable = raw;
        }
    }
}

/// One quadrature knob. Its two contacts, read as `a | b << 1`, walk the cycle
/// 0, 1, 3, 2 one way round and the reverse the other way; four steps make a detent.
#[derive(Clone, Copy)]
pub struct Knob {
    phase: u8,
    steps: i8,
}

/// Position of a contact reading in that cycle.
fn phase(ab: u8) -> u8 {
    [0, 1, 3, 2][ab as usize & 3]
}

impl Knob {
    pub fn new(ab: u8) -> Knob {
        Knob { phase: phase(ab), steps: 0 }
    }

    /// Takes a fresh reading. Returns the direction (true = clockwise) once a detent is complete.
    pub fn update(&mut self, ab: u8) -> Option<bool> {
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

/// Detents of one knob not yet queued (+ clockwise).
#[derive(Clone, Copy, Default)]
pub struct Turns(i8);

impl Turns {
    pub const fn new() -> Turns {
        Turns(0)
    }

    pub fn add(&mut self, cw: bool) {
        self.0 = (self.0 + if cw { 1 } else { -1 }).clamp(-MAX_QUEUED_TURNS, MAX_QUEUED_TURNS);
    }

    /// Offers one waiting detent to `queue` (true = clockwise).
    pub fn drain(&mut self, queue: impl FnOnce(bool) -> bool) {
        if self.0 != 0 && queue(self.0 > 0) {
            self.0 -= self.0.signum();
        }
    }
}

/// The four-corner bootloader combo.
#[derive(Clone, Copy, Default)]
pub struct Combo(u16);

impl Combo {
    pub const fn new() -> Combo {
        Combo(0)
    }

    /// One scan. True once all four corners have been held for `BOOT_COMBO_MS` scans in a row.
    pub fn tick(&mut self, all_held: bool) -> bool {
        if all_held {
            self.0 = self.0.saturating_add(1);
            self.0 >= BOOT_COMBO_MS
        } else {
            self.0 = 0;
            false
        }
    }
}
