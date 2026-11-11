//! Short state changes (hover, press, a sliding indicator, a toggle, the "Saved" chip) as
//! numbers that move from one value to another over a few frames. One thread timer of 16 ms
//! runs only while something moves; with nothing moving there is no timer and no wake-up.

use super::theme::theme;
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{RDW_ALLCHILDREN, RDW_INVALIDATE, RedrawWindow};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

/// The easing curves of the design, as CSS cubic-bezier control points.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Curve {
    /// Strong ease-out (0.23, 1, 0.32, 1): entering, pressing, things that answer the user.
    Out,
    /// Strong ease-in-out (0.77, 0, 0.175, 1): something moving across the screen.
    InOut,
    /// CSS `ease` (0.25, 0.1, 0.25, 1): colour changes.
    Ease,
}

impl Curve {
    fn points(self) -> (f32, f32, f32, f32) {
        match self {
            Curve::Out => (0.23, 1.0, 0.32, 1.0),
            Curve::InOut => (0.77, 0.0, 0.175, 1.0),
            Curve::Ease => (0.25, 0.1, 0.25, 1.0),
        }
    }

    /// The eased progress at time `t` (0 to 1): solves the bezier's x for `t` by Newton steps,
    /// with bisection when the slope is too flat, then returns its y.
    pub fn at(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let (x1, y1, x2, y2) = self.points();
        let bez = |a: f32, b: f32, s: f32| 3.0 * a * (1.0 - s).powi(2) * s + 3.0 * b * (1.0 - s) * s * s + s * s * s;
        let slope = |a: f32, b: f32, s: f32| {
            3.0 * a * (1.0 - s).powi(2) + 6.0 * (b - a) * (1.0 - s) * s + 3.0 * (1.0 - b) * s * s
        };
        let mut s = t;
        for _ in 0..8 {
            let d = slope(x1, x2, s);
            if d.abs() < 1e-4 {
                break;
            }
            s = (s - (bez(x1, x2, s) - t) / d).clamp(0.0, 1.0);
        }
        if (bez(x1, x2, s) - t).abs() > 1e-3 {
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..24 {
                s = (lo + hi) / 2.0;
                if bez(x1, x2, s) < t { lo = s } else { hi = s }
            }
        }
        bez(y1, y2, s)
    }
}

/// One moving value.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    pub from: f32,
    pub to: f32,
    pub start: u64,
    pub ms: u32,
    pub curve: Curve,
}

impl Motion {
    pub fn value(&self, now: u64) -> f32 {
        if self.ms == 0 {
            return self.to;
        }
        let t = (now.saturating_sub(self.start)) as f32 / self.ms as f32;
        self.from + (self.to - self.from) * self.curve.at(t)
    }

    pub fn done(&self, now: u64) -> bool {
        now.saturating_sub(self.start) >= self.ms as u64
    }
}

/// What to repaint as a value moves.
#[derive(Clone, Copy, Debug)]
pub enum Paint {
    /// The window the value belongs to.
    Own,
    /// An area of another window (its parent, say), with the children over it.
    Area(HWND, RECT),
}

/// What moves: a part of a window.
#[derive(Clone, Copy)]
struct Slot {
    hwnd: HWND,
    part: u32,
    paint: Paint,
    motion: Motion,
}

/// More than enough for hover in and out, a press, two indicators, a toggle and the chip.
const SLOTS: usize = 16;

thread_local! {
    static RUNNING: RefCell<[Option<Slot>; SLOTS]> = const { RefCell::new([None; SLOTS]) };
    static TIMER: Cell<usize> = const { Cell::new(0) };
    static CLOCK: std::time::Instant = std::time::Instant::now();
}

/// Milliseconds on this thread's clock.
fn now() -> u64 {
    CLOCK.with(|c| c.elapsed().as_millis() as u64)
}

pub fn redraw(hwnd: HWND, paint: Paint) {
    let (target, area) = match paint {
        Paint::Own => (hwnd, None),
        Paint::Area(h, r) => (h, Some(r)),
    };
    // SAFETY: RedrawWindow only marks the window (and children over the area) for repainting;
    // a stale handle makes it fail harmlessly.
    unsafe {
        let _ =
            RedrawWindow(Some(target), area.as_ref().map(|r| r as *const RECT), None, RDW_INVALIDATE | RDW_ALLCHILDREN);
    }
}

/// The value of `part` of `hwnd`: where it is while it moves, else `rest`.
pub fn value(hwnd: HWND, part: u32, rest: f32) -> f32 {
    let t = now();
    RUNNING.with_borrow(|r| {
        r.iter().flatten().find(|s| s.hwnd == hwnd && s.part == part).map_or(rest, |s| s.motion.value(t))
    })
}

/// True while `part` of `hwnd` moves.
#[cfg(test)]
pub fn moving(hwnd: HWND, part: u32) -> bool {
    RUNNING.with_borrow(|r| r.iter().flatten().any(|s| s.hwnd == hwnd && s.part == part))
}

/// Moves `part` of `hwnd` to `to` over `ms`. It starts from where it is if it already moves
/// (no jump back), else from `from`. Instant (one repaint) when motion is off or `ms` is 0.
pub fn go(hwnd: HWND, part: u32, from: f32, to: f32, ms: u32, curve: Curve, paint: Paint) {
    let t = now();
    let ms = if theme().motion { ms } else { 0 };
    RUNNING.with_borrow_mut(|r| {
        let at = r.iter().position(|s| s.is_some_and(|s| s.hwnd == hwnd && s.part == part));
        let from = at.and_then(|i| r[i]).map_or(from, |s| s.motion.value(t));
        let slot = at.or_else(|| r.iter().position(Option::is_none));
        match slot {
            Some(i) if ms > 0 && from != to => {
                r[i] = Some(Slot { hwnd, part, paint, motion: Motion { from, to, start: t, ms, curve } });
            }
            Some(i) if at == Some(i) => r[i] = None,
            _ => {}
        }
    });
    redraw(hwnd, paint);
    run_timer();
}

/// Starts or stops the shared timer, as there is something moving or not.
fn run_timer() {
    let busy = RUNNING.with_borrow(|r| r.iter().any(Option::is_some));
    let id = TIMER.get();
    // SAFETY: a thread timer (no window) with `tick` as its callback; KillTimer is given the id
    // SetTimer returned.
    unsafe {
        if busy && id == 0 {
            TIMER.set(SetTimer(None, 0, 16, Some(tick)));
        } else if !busy && id != 0 {
            let _ = KillTimer(None, id);
            TIMER.set(0);
        }
    }
}

unsafe extern "system" fn tick(_: HWND, _: u32, _: usize, _: u32) {
    let t = now();
    let mut repaint = [None; SLOTS];
    RUNNING.with_borrow_mut(|r| {
        for (slot, out) in r.iter_mut().zip(repaint.iter_mut()) {
            if let Some(s) = slot {
                *out = Some((s.hwnd, s.paint));
                if s.motion.done(t) {
                    *slot = None;
                }
            }
        }
    });
    for (hwnd, paint) in repaint.into_iter().flatten() {
        redraw(hwnd, paint);
    }
    run_timer();
}

/// Drops everything that moves in `hwnd` (it is being destroyed).
pub fn forget(hwnd: HWND) {
    RUNNING.with_borrow_mut(|r| r.iter_mut().filter(|s| s.is_some_and(|s| s.hwnd == hwnd)).for_each(|s| *s = None));
    run_timer();
}

/// True while the shared timer runs.
#[cfg(test)]
pub fn timer_running() -> bool {
    TIMER.get() != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_curves_start_and_end_where_they_should() {
        for c in [Curve::Out, Curve::InOut, Curve::Ease] {
            assert!(c.at(0.0).abs() < 1e-3, "{c:?} starts at 0");
            assert!((c.at(1.0) - 1.0).abs() < 1e-3, "{c:?} ends at 1");
            let mut last = 0.0;
            for i in 1..=50 {
                let v = c.at(i as f32 / 50.0);
                assert!(v >= last - 1e-4, "{c:?} never goes back");
                last = v;
            }
        }
        assert!(Curve::Out.at(0.2) > 0.6, "ease-out answers at once: {}", Curve::Out.at(0.2));
        assert!(Curve::InOut.at(0.2) < 0.1, "ease-in-out starts slowly: {}", Curve::InOut.at(0.2));
        assert!((0.3..0.7).contains(&Curve::InOut.at(0.5)), "and is near half way at half time");
        assert!((Curve::Ease.at(0.5) - 0.8024).abs() < 0.01, "CSS ease at 50 %: {}", Curve::Ease.at(0.5));
    }

    #[test]
    fn a_motion_runs_on_the_clock() {
        let m = Motion { from: 0.0, to: 88.0, start: 1000, ms: 220, curve: Curve::InOut };
        assert_eq!(m.value(1000), 0.0);
        assert!(m.value(1110) > 30.0 && m.value(1110) < 58.0);
        assert_eq!(m.value(1220), 88.0);
        assert_eq!(m.value(5000), 88.0, "it stays at its end");
        assert!(!m.done(1219) && m.done(1220));
        assert_eq!(m.value(900), 0.0, "a clock that goes back changes nothing");
        let instant = Motion { ms: 0, ..m };
        assert_eq!(instant.value(1000), 88.0);
    }

    #[test]
    fn the_timer_runs_only_while_something_moves() {
        let hwnd = HWND(std::ptr::dangling_mut());
        assert!(!timer_running());
        go(hwnd, 1, 0.0, 1.0, 120, Curve::Ease, Paint::Own);
        assert_eq!(timer_running(), theme().motion);
        assert!(value(hwnd, 1, 0.0) < 1.0 || !theme().motion);
        std::thread::sleep(std::time::Duration::from_millis(30));
        go(hwnd, 1, 0.0, 0.0, 120, Curve::Ease, Paint::Own);
        assert!(moving(hwnd, 1) == theme().motion, "a new target turns it round from where it is");
        forget(hwnd);
        assert!(!timer_running() && !moving(hwnd, 1));
        assert_eq!(value(hwnd, 1, 0.5), 0.5, "at rest it is where its state says");
    }
}
