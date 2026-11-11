//! Volume inside a channel, without Core Audio: how an app's level relates to its channel's,
//! and what a knob is aimed at.
//!
//! An app is heard at `channel level x trim`. The trim (0..=1, default 1) belongs to the app's
//! exe name and is remembered across runs, so an app turned down against the others stays
//! lower whatever the channel knob does, through 0 % and 100 % and back.

use std::time::{Duration, Instant};

/// Trim of an app nobody has turned on its own.
pub const FULL: f32 = 1.0;
/// Below this a channel counts as off: its apps' levels say nothing about their balance.
const OFF: f32 = 0.005;
/// A knob aimed at one app goes back to the whole channel after this long untouched.
pub const AIM_LASTS: Duration = Duration::from_secs(8);

/// The level Windows should show for an app.
pub fn heard(level: f32, trim: f32) -> f32 {
    (level * trim).clamp(0.0, 1.0)
}

/// What putting one app at a new level does to its channel.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placed {
    pub level: f32,
    pub trim: f32,
    /// Factor for every other app's trim in the channel.
    pub others: f32,
}

/// Puts one app at `v` (from the pad or from the Windows mixer, the same rule for both) so
/// that no other app in the channel changes what it sounds like:
/// - at or below the channel level, only the app's trim changes;
/// - above it, the channel level rises to `v` and the other trims shrink by the same factor;
/// - on a channel that was off, the channel level becomes `v` and the other trims stay, so
///   those apps come back at their old balance (an off channel holds no balance to keep).
pub fn place(level: f32, v: f32) -> Placed {
    let v = v.clamp(0.0, 1.0);
    if level < OFF {
        Placed { level: v, trim: FULL, others: 1.0 }
    } else if v <= level {
        Placed { level, trim: v / level, others: 1.0 }
    } else {
        Placed { level: v, trim: FULL, others: level / v }
    }
}

/// What a press made the knob aim at.
#[derive(PartialEq, Debug)]
pub enum Step {
    Channel,
    App(String),
    /// Nothing in the channel is making sound; the knob stays on the whole channel.
    NoApps,
}

/// What one knob controls: its whole channel, or for a while one app in it.
pub struct Aim {
    channel: usize,
    app: Option<String>,
    touched: Instant,
}

impl Default for Aim {
    fn default() -> Aim {
        Aim { channel: usize::MAX, app: None, touched: Instant::now() }
    }
}

impl Aim {
    /// The app the knob is aimed at, if any. `live` is the channel's apps playing now. Falls
    /// back to the whole channel when the knob was left alone for `AIM_LASTS`, when the app
    /// went away, or when the knob now stands for another channel (a "selected" knob).
    pub fn app(&mut self, channel: usize, live: &[String], now: Instant) -> Option<&str> {
        let stale = now.saturating_duration_since(self.touched) >= AIM_LASTS;
        if self.channel != channel || stale || self.app.as_ref().is_some_and(|a| !live.contains(a)) {
            self.channel = channel;
            self.app = None;
        }
        self.app.as_deref()
    }

    /// A press: whole channel, then each live app in `live`'s order, then the whole channel.
    pub fn step(&mut self, channel: usize, live: &[String], now: Instant) -> Step {
        let next = match self.app(channel, live, now) {
            None => live.first(),
            Some(app) => live.iter().skip_while(|a| *a != app).nth(1),
        };
        self.app = next.cloned();
        self.touched = now;
        match next {
            Some(app) => Step::App(app.clone()),
            None if live.is_empty() => Step::NoApps,
            None => Step::Channel,
        }
    }

    /// A turn keeps the aim alive.
    pub fn touch(&mut self, now: Instant) {
        self.touched = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn the_channel_knob_keeps_the_balance_through_the_ends() {
        let (spotify, browser) = (0.5, FULL);
        let mut level: f32 = 0.6;
        for step in [-0.3, -0.3, -0.3, 0.2, 0.9, 0.9, -0.5] {
            level = (level + step).clamp(0.0, 1.0);
            assert!(close(heard(level, spotify), heard(level, browser) / 2.0), "at {level}");
        }
        assert!(close(level, 0.5));
        assert!(close(heard(level, spotify), 0.25));
    }

    #[test]
    fn placing_one_app_leaves_the_others_as_they_sound() {
        // Turned down: only its trim changes.
        assert_eq!(place(0.8, 0.2), Placed { level: 0.8, trim: 0.25, others: 1.0 });
        // Turned above the channel: the channel rises, the others keep their level.
        let p = place(0.5, 0.8);
        assert!(close(p.level, 0.8) && p.trim == FULL);
        assert!(close(heard(p.level, 0.6 * p.others), heard(0.5, 0.6)));
        // A channel that was off holds no balance: the others keep their trims.
        assert_eq!(place(0.0, 0.3), Placed { level: 0.3, trim: FULL, others: 1.0 });
        assert_eq!(place(0.5, 7.0).level, 1.0, "clamped");
        assert_eq!(place(0.5, 0.5), Placed { level: 0.5, trim: FULL, others: 1.0 });
    }

    #[test]
    fn presses_walk_the_channel_then_each_app_then_back() {
        let t0 = Instant::now();
        let live = vec!["chrome.exe".to_string(), "spotify.exe".to_string()];
        let mut aim = Aim::default();
        assert_eq!(aim.step(3, &live, t0), Step::App("chrome.exe".into()));
        assert_eq!(aim.step(3, &live, t0), Step::App("spotify.exe".into()));
        assert_eq!(aim.step(3, &live, t0), Step::Channel);
        assert_eq!(aim.app(3, &live, t0), None);
        assert_eq!(aim.step(3, &live, t0), Step::App("chrome.exe".into()));
        assert_eq!(aim.step(3, &[], t0), Step::NoApps);
        assert_eq!(aim.app(3, &[], t0), None);
    }

    #[test]
    fn an_aim_never_outlives_its_app_or_its_time() {
        let t0 = Instant::now();
        let live = vec!["chrome.exe".to_string(), "spotify.exe".to_string()];
        let mut aim = Aim::default();
        aim.step(3, &live, t0);
        aim.step(3, &live, t0);
        let later = t0 + AIM_LASTS - Duration::from_millis(1);
        assert_eq!(aim.app(3, &live, later), Some("spotify.exe"));
        aim.touch(later);
        assert_eq!(aim.app(3, &live, later + AIM_LASTS - Duration::from_millis(1)), Some("spotify.exe"));
        assert_eq!(aim.app(3, &live, later + AIM_LASTS), None, "untouched for 8 s");

        aim.step(3, &live, t0);
        assert_eq!(aim.app(3, &live[..1], t0), Some("chrome.exe"));
        aim.step(3, &live, t0);
        assert_eq!(aim.app(3, &live[..1], t0), None, "spotify went away");

        aim.step(3, &live, t0);
        assert_eq!(aim.app(2, &live, t0), None, "the knob now stands for another channel");
    }
}
