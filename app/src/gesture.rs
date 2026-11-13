//! Tap, hold and double tap from one button's downs and ups. No clock and no Windows: the
//! caller passes the time of each event, and `poll`s at `deadline` (the mixer loop already
//! waits on a timeout, so this needs no thread or timer of its own).
//!
//! The rules, one each:
//! - A gesture that has no action is never waited for: with neither a hold nor a double tap
//!   set, a press is a tap at once, on the way down.
//! - A hold fires while the button is still down, the moment it has been down for `HOLD`, so
//!   the screen answers under the finger. Its release then does nothing.
//! - With a hold set, a tap is a release before `HOLD`.
//! - With a double tap set, a tap waits `DOUBLE` after its release for a second press, which
//!   fires the double tap on its way down. Only then is a plain tap delayed.
//! - An interruption (the knob turned while it is down) uses up the press: no tap, no hold.

use std::time::{Duration, Instant};

/// The same as a held Focus or Page key: one feel for every hold on the pad.
pub const HOLD: Duration = crate::keys::MOMENTARY;
/// How long a tap waits for a second one, when a double tap is set.
pub const DOUBLE: Duration = Duration::from_millis(280);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gesture {
    Tap,
    Hold,
    Double,
}

/// Which gestures have an action. Only these are waited for.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Wants {
    pub hold: bool,
    pub double: bool,
}

#[derive(Default)]
pub struct Button {
    /// Since when the button is down; None while it is up.
    down: Option<Instant>,
    /// This press has fired or was interrupted: its hold and its release do nothing more.
    spent: bool,
    /// When a tap was let go that still waits for a second press.
    first_tap: Option<Instant>,
}

impl Button {
    pub fn is_down(&self) -> bool {
        self.down.is_some()
    }

    /// Fires what is due by `now`: a tap whose wait for a second one is over, a hold reached.
    pub fn poll(&mut self, now: Instant, wants: Wants, fire: &mut impl FnMut(Gesture)) {
        if self.first_tap.is_some_and(|t| now.saturating_duration_since(t) >= DOUBLE) {
            self.first_tap = None;
            fire(Gesture::Tap);
        }
        if wants.hold && !self.spent && self.down.is_some_and(|t| now.saturating_duration_since(t) >= HOLD) {
            self.spent = true;
            fire(Gesture::Hold);
        }
    }

    pub fn press(&mut self, now: Instant, wants: Wants, fire: &mut impl FnMut(Gesture)) {
        self.poll(now, wants, fire);
        self.down = Some(now);
        self.spent = false;
        if self.first_tap.take().is_some() {
            self.spent = true;
            fire(Gesture::Double);
        } else if !wants.hold && !wants.double {
            self.spent = true;
            fire(Gesture::Tap);
        }
    }

    pub fn release(&mut self, now: Instant, wants: Wants, fire: &mut impl FnMut(Gesture)) {
        self.poll(now, wants, fire);
        if self.down.take().is_none() || self.spent {
            return;
        }
        if wants.double {
            self.first_tap = Some(now);
        } else {
            fire(Gesture::Tap);
        }
    }

    /// Something else happened with the button (the knob turned): a press that is down is used
    /// up, and a finished tap that waits for a second one counts at once.
    pub fn interrupt(&mut self, fire: &mut impl FnMut(Gesture)) {
        if self.down.is_some() {
            self.spent = true;
        }
        if self.first_tap.take().is_some() {
            fire(Gesture::Tap);
        }
    }

    /// When `poll` has something to fire next, if anything.
    pub fn deadline(&self, wants: Wants) -> Option<Instant> {
        let tap = self.first_tap.map(|t| t + DOUBLE);
        let hold = self.down.filter(|_| wants.hold && !self.spent).map(|t| t + HOLD);
        match (tap, hold) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Gesture::*;

    const NONE: Wants = Wants { hold: false, double: false };
    const HOLD_ONLY: Wants = Wants { hold: true, double: false };
    const BOTH: Wants = Wants { hold: true, double: true };

    enum Ev {
        Down,
        Up,
        Turn,
        /// The loop wakes up (at a deadline, or for any other reason).
        Poll,
    }
    use Ev::*;

    /// Plays (ms, event) against a fake clock; returns (ms, gesture) for each one fired.
    fn play(wants: Wants, script: &[(u64, Ev)]) -> Vec<(u64, Gesture)> {
        let t0 = Instant::now();
        let mut b = Button::default();
        let mut out = Vec::new();
        for (ms, ev) in script {
            let now = t0 + Duration::from_millis(*ms);
            let mut fire = |g| out.push((*ms, g));
            match ev {
                Down => b.press(now, wants, &mut fire),
                Up => b.release(now, wants, &mut fire),
                Turn => b.interrupt(&mut fire),
                Poll => b.poll(now, wants, &mut fire),
            }
        }
        out
    }

    #[test]
    fn with_no_hold_or_double_a_tap_fires_on_the_way_down() {
        assert_eq!(play(NONE, &[(0, Down)]), [(0, Tap)]);
        assert_eq!(play(NONE, &[(0, Down), (900, Up), (901, Poll)]), [(0, Tap)], "a long press is still one tap");
        assert_eq!(play(NONE, &[(0, Down), (50, Up), (100, Down)]), [(0, Tap), (100, Tap)], "two taps, no double");
    }

    #[test]
    fn a_hold_fires_while_held_and_its_release_does_nothing() {
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (349, Poll)]), []);
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (350, Poll), (2000, Up), (2001, Poll)]), [(350, Hold)]);
        // The loop woke late: the hold still counts, once.
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (600, Up)]), [(600, Hold)]);
    }

    #[test]
    fn with_a_hold_set_a_tap_fires_on_release_without_waiting() {
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (120, Up)]), [(120, Tap)]);
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (120, Up), (200, Down), (300, Up)]), [(120, Tap), (300, Tap)]);
    }

    #[test]
    fn a_double_tap_fires_on_the_second_press() {
        let got = play(BOTH, &[(0, Down), (80, Up), (200, Down), (260, Up), (1000, Poll)]);
        assert_eq!(got, [(200, Double)], "no tap before or after it, no hold");
        let held_second = play(BOTH, &[(0, Down), (80, Up), (200, Down), (900, Poll), (950, Up)]);
        assert_eq!(held_second, [(200, Double)], "holding the second press is not a hold too");
    }

    #[test]
    fn only_a_double_tap_setting_delays_the_tap() {
        assert_eq!(play(BOTH, &[(0, Down), (80, Up), (359, Poll)]), [], "waits for a second press");
        assert_eq!(play(BOTH, &[(0, Down), (80, Up), (360, Poll)]), [(360, Tap)], "280 ms after the release");
        assert_eq!(play(BOTH, &[(0, Down), (80, Up), (400, Down)]), [(400, Tap)], "too late for a double");
        let late_poll = play(BOTH, &[(0, Down), (80, Up), (400, Down), (500, Up), (900, Poll)]);
        assert_eq!(late_poll, [(400, Tap), (900, Tap)], "the second press is a tap of its own");
    }

    #[test]
    fn a_turn_while_down_cancels_the_tap_and_the_hold() {
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (100, Turn), (200, Up)]), []);
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (100, Turn), (500, Poll), (600, Up)]), []);
        assert_eq!(play(BOTH, &[(0, Down), (100, Turn), (200, Up), (900, Poll)]), []);
        // After the hold fired, turning on keeps its normal meaning and the hold stands.
        assert_eq!(play(HOLD_ONLY, &[(0, Down), (400, Poll), (500, Turn), (600, Up)]), [(400, Hold)]);
    }

    #[test]
    fn a_turn_after_a_finished_tap_lets_it_count_at_once() {
        assert_eq!(play(BOTH, &[(0, Down), (80, Up), (150, Turn), (900, Poll)]), [(150, Tap)]);
    }

    #[test]
    fn the_deadline_is_the_next_thing_due() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut b = Button::default();
        let mut sink = |_| {};
        assert_eq!(b.deadline(BOTH), None);
        b.press(t0, BOTH, &mut sink);
        assert_eq!(b.deadline(BOTH), Some(t0 + HOLD));
        assert_eq!(b.deadline(Wants { hold: false, double: true }), None, "no hold set: nothing to wait for");
        b.release(t0 + ms(100), BOTH, &mut sink);
        assert_eq!(b.deadline(BOTH), Some(t0 + ms(100) + DOUBLE));
        b.poll(t0 + ms(100) + DOUBLE, BOTH, &mut sink);
        assert_eq!(b.deadline(BOTH), None);
    }
}
