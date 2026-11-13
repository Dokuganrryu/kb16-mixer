//! The key lights' maths: gamma, smoothing, dithering, idle easing, and what a frame shows.

use kb16_mixer_protocol::{Frame, HostMsg, NEVER};
use pad_core::leds::{self, Bytes, DIM_MS, Ease, Input, Lights, dither, gamma, glide, off_after, ungamma};
use pad_core::state::{Mixer, POPUP_MS, State};

const FULL: u32 = 0xFFFF;
const MIN: u64 = 60_000;

#[test]
fn gamma_is_monotone_with_exact_ends_and_follows_2_4() {
    assert_eq!(gamma(0), 0);
    assert_eq!(gamma(FULL), FULL);
    let mut last = 0;
    for p in 0..=FULL {
        let g = gamma(p);
        assert!(g >= last, "gamma dips at {p}");
        last = g;
        let want = (p as f64 / FULL as f64).powf(2.4) * FULL as f64;
        assert!((g as f64 - want).abs() <= want * 0.02 + 8.0, "gamma({p}) = {g}, want {want:.0}");
    }
}

#[test]
fn ungamma_inverts_gamma() {
    for lin in (0..=FULL).step_by(97).chain([FULL]) {
        let p = ungamma(lin);
        assert!(gamma(p) <= lin);
        assert!(p == FULL || gamma(p + 1) > lin, "{lin}");
    }
}

/// Frames of `glide` from `from` to `to`; the trace includes the start.
fn trace(from: u32, to: u32, k: u64) -> Vec<u32> {
    let mut v = vec![from];
    while *v.last().unwrap() != to {
        v.push(glide(*v.last().unwrap(), to, k));
        assert!(v.len() < 100_000, "never lands");
    }
    v
}

#[test]
fn smoothing_is_monotone_never_overshoots_and_lands_exactly() {
    for (from, to) in [(0, FULL << 8), (FULL << 8, 0), (5, 6), (1000, 999), (123_456, 7_654_321)] {
        for k in [8724, 1297, 5243] {
            let t = trace(from, to, k);
            for w in t.windows(2) {
                if to > from {
                    assert!(w[1] > w[0] && w[1] <= to);
                } else {
                    assert!(w[1] < w[0] && w[1] >= to);
                }
            }
            assert_eq!(*t.last().unwrap(), to);
        }
    }
}

#[test]
fn attack_is_quick_and_release_slow() {
    // One time constant covers about 63% of the way: 7 frames (35 ms) up, 50 frames (250 ms) down.
    let up = trace(0, FULL << 8, 8724);
    let share = |v: u32| v as f64 / (FULL << 8) as f64;
    assert!((0.58..0.68).contains(&share(up[7])), "{}", share(up[7]));
    let down = trace(FULL << 8, 0, 1297);
    assert!((0.32..0.42).contains(&share(down[50])), "{}", share(down[50]));
}

#[test]
fn dithering_averages_to_the_light_within_one_step() {
    for lin in (0..=FULL).step_by(13) {
        let mut err = 0;
        let frames = 64;
        let sum: u32 = (0..frames).map(|_| dither(lin, &mut err) as u32).sum();
        let mean = sum as f64 / frames as f64;
        let want = lin as f64 * 255.0 / FULL as f64;
        assert!((mean - want).abs() <= 1.0, "light {lin}: mean {mean}, want {want}");
        // Every frame shows one of the two nearest steps: no flash.
        let mut err = 0;
        for _ in 0..8 {
            let v = dither(lin, &mut err) as f64;
            assert!((v - want).abs() < 1.5, "light {lin}: {v} vs {want}");
        }
    }
}

#[test]
fn dithering_above_sixteen_steps_repeats_within_four_frames() {
    // A pattern that repeats at least every 4 frames (50 Hz at 200 Hz) cannot be seen as flicker.
    for lin in (16 * 257..=FULL).step_by(31) {
        let mut err = 0;
        let a: Vec<u8> = (0..8).map(|_| dither(lin, &mut err)).collect();
        let b: Vec<u8> = (0..8).map(|_| dither(lin, &mut err)).collect();
        assert_eq!(a[..4], a[4..]);
        assert_eq!(a, b);
    }
    // Below 2 steps it rounds, so a near-black LED never blinks between 0 and 1.
    let mut err = 0;
    for _ in 0..10 {
        assert_eq!(dither(300, &mut err), 1);
    }
}

#[test]
fn the_idle_fade_eases_down_and_a_wake_comes_back_fast() {
    let mut e = Ease::new(FULL, 0u64);
    e.aim(10_000, 1000);
    let mut last = FULL;
    for t in (1000..=1000 + DIM_MS).step_by(5) {
        let v = e.value(t);
        assert!(v <= last);
        last = v;
    }
    assert_eq!(DIM_MS, 700, "the screen's idle fade takes the same time");
    assert_eq!(e.value(1000 + DIM_MS), 10_000, "lands exactly at 700 ms");
    // Gentle start: after 5% of the time, under 2% of the way.
    assert!(FULL - e.value(1035) < (FULL - 10_000) / 50);

    e.aim(FULL, 2000);
    assert!(e.value(2050) > FULL / 2, "a wake is more than half there in 50 ms");
    assert_eq!(e.value(2150), FULL, "and fully in 150 ms");

    // Interrupted halfway down, it starts back up from where it was: no jump.
    e.aim(0, 3000);
    let mid = e.value(4000);
    e.aim(FULL, 4000);
    assert_eq!(e.value(4000), mid);
    assert_eq!(e.value(4150), FULL);
}

#[test]
fn the_lights_go_off_after_three_times_the_dim_time() {
    assert_eq!(off_after(10), 30);
    assert_eq!(off_after(1), 3);
    assert_eq!(off_after(100), 240, "at most four hours");
    assert_eq!(off_after(250), 250, "never before they dim");
    assert_eq!(off_after(NEVER), NEVER);
}

fn mixer() -> Mixer {
    let mut m = State::new(0u64).mixer;
    m.n = 4;
    // The proposed defaults: Mic a touch warm, Chat ice, Game a touch cool, Media neutral.
    m.colors = [[230, 220, 215], [200, 215, 255], [175, 205, 255], [215, 222, 245]];
    m.selected = 255;
    m
}

struct Rig {
    lights: Lights<u64>,
    input: Input<u64>,
    t: u64,
    last: Bytes,
}

impl Rig {
    fn new(alive: bool) -> Rig {
        let s = State::new(0u64);
        Rig {
            lights: Lights::new(0),
            input: Input {
                alive,
                mixer: mixer(),
                brightness: s.brightness,
                ripples: s.ripples,
                frame_seen: true,
                activity: 0,
                dim_after: 10,
                idle_percent: 0,
            },
            t: 0,
            last: [0; 48],
        }
    }

    /// Runs 5 ms frames for `ms`; returns the bytes of every frame.
    fn run(&mut self, ms: u64) -> Vec<Bytes> {
        let mut out = Vec::new();
        for _ in 0..ms / 5 {
            self.t += 5;
            if let Some(b) = self.lights.frame(self.t, &self.input) {
                self.last = b;
            }
            out.push(self.last);
        }
        out
    }

    /// Jumps the clock (as a long quiet stretch would), then settles.
    fn skip_to(&mut self, t: u64) {
        self.t = t;
        self.run(3000);
    }

    fn total(&self) -> u32 {
        self.last.iter().map(|&v| v as u32).sum()
    }
}

/// GRB bytes of one LED as (r, g, b).
fn rgb(b: &Bytes, led: usize) -> (u8, u8, u8) {
    (b[led * 3 + 1], b[led * 3], b[led * 3 + 2])
}

#[test]
fn a_meter_glides_between_host_frames() {
    let mut r = Rig::new(true);
    r.run(1500);
    // The host sends a new level every ~33 ms; the top meter LED of column 2 (led 2) must move
    // on every 5 ms frame in between, in small steps, never backwards while the level rises.
    let mut frames = Vec::new();
    for step in 1..=12u32 {
        r.input.mixer.level[2] = (step * 20) as u8;
        r.input.activity = r.t;
        frames.extend(r.run(35));
    }
    let lum = |b: &Bytes| {
        let (x, y, z) = rgb(b, 2);
        x as u32 + y as u32 + z as u32
    };
    let mut rises = 0;
    for w in frames.windows(2) {
        let (a, b) = (lum(&w[0]), lum(&w[1]));
        // Dithering may move each channel by one step.
        assert!(b + 3 >= a, "a rising meter dropped: {a} -> {b}");
        assert!(b <= a + 24, "a step too big for 5 ms: {a} -> {b}");
        rises += (b > a) as u32;
    }
    assert!(rises > frames.len() as u32 / 3, "moves between host frames ({rises} of {})", frames.len());
}

#[test]
fn quiet_lights_dim_to_six_percent_then_go_off_and_wake_at_once() {
    let mut r = Rig::new(true);
    r.run(2000);
    let awake = r.total();
    r.skip_to(10 * MIN + 1000);
    let dim = r.total();
    assert!(dim > 0, "dimmed lights stay alive");
    assert!(dim * 8 < awake, "dimmed a lot: {dim} vs {awake}");

    r.skip_to(30 * MIN + 1000);
    assert_eq!(r.total(), 0, "off after three times the dim time");

    // A key press: most of the way back within 150 ms, all of it soon after.
    r.input.activity = r.t;
    r.run(150);
    assert!(r.total() * 2 > awake, "{} vs {awake}", r.total());
    r.run(1000);
    assert!(r.total() * 20 > awake * 19, "{} vs {awake}", r.total());
}

#[test]
fn the_idle_percent_sets_how_dim() {
    let dim_total = |pct| {
        let mut r = Rig::new(true);
        r.input.idle_percent = pct;
        r.run(500);
        r.skip_to(10 * MIN + 1000);
        r.total()
    };
    let (default, six, forty) = (dim_total(0), dim_total(6), dim_total(40));
    assert_eq!(default, six);
    assert!(forty > six * 3, "{forty} vs {six}");
}

#[test]
fn the_dim_fade_never_jumps() {
    let mut r = Rig::new(false);
    r.run(2000);
    r.t = 10 * MIN - 100;
    let frames = r.run(1500);
    for w in frames.windows(2) {
        for (i, (a, b)) in w[0].iter().zip(w[1].iter()).enumerate() {
            assert!(a.abs_diff(*b) <= 3, "byte {i}: {a} -> {b}");
        }
    }
}

#[test]
fn muted_keys_turn_amber_smoothly_and_the_rest_stay_ice() {
    let mut r = Rig::new(true);
    r.run(2000);
    let before = rgb(&r.last, 12);
    assert!(before.1 as u32 * 5 > before.0 as u32 * 4, "a near-white tint before: {before:?}");
    r.input.mixer.mutes = 1; // column 0
    let frames = r.run(600);
    // Mute key (led 12): amber, red well above green, almost no blue.
    let (red, green, blue) = rgb(&r.last, 12);
    assert!(red > green * 2 && green >= blue && blue <= red / 8, "{:?}", (red, green, blue));
    // The other columns did not change colour.
    let other = rgb(&r.last, 13);
    assert!(other.2 >= other.0, "{other:?}");
    // The change took several frames (no snap), and no channel jumped by much in one frame.
    let mut changed = 0;
    for w in frames.windows(2) {
        let (a, b) = (rgb(&w[0], 12), rgb(&w[1], 12));
        changed += (a != b) as u32;
        assert!(a.0.abs_diff(b.0) <= 30 && a.2.abs_diff(b.2) <= 30, "{a:?} -> {b:?}");
    }
    assert!(changed >= 10, "{changed}");
}

#[test]
fn sixteen_white_lights_stay_inside_the_power_cap() {
    let mut r = Rig::new(true);
    r.input.brightness = 255;
    r.input.mixer.level = [255; 4];
    r.input.mixer.selected = 1;
    r.input.mixer.colors = [[255; 3]; 4];
    r.run(2000);
    // About 45% of all channels at full.
    assert!(r.total() <= 16 * 3 * 255 * 9 / 20 + 48, "{}", r.total());
    assert!(r.total() > 16 * 3 * 255 * 2 / 5, "the cap does not starve the picture: {}", r.total());
}

#[test]
fn the_default_idle_percent_is_light_not_brightness() {
    assert_eq!(leds::DEFAULT_IDLE_PERCENT, 6);
    // 6% of the light is about a third of the perceived brightness.
    let p = leds::idle_level(0);
    assert!((FULL * 30 / 100..FULL * 33 / 100).contains(&p), "{p}");
}

#[test]
fn the_dim_starts_at_the_idle_limit_and_lands_after_the_fade_time() {
    let mut r = Rig::new(true);
    r.input.dim_after = 1;
    r.run(MIN);
    let awake = r.total();
    // Up to the limit nothing has moved; the screen's fade starts from the same moment.
    assert_eq!(r.t, MIN);
    r.run(DIM_MS / 2);
    let mid = r.total();
    r.run(DIM_MS / 2);
    let landed = r.total();
    r.run(2000);
    assert!(mid < awake && mid > landed, "{awake} > {mid} > {landed}");
    assert!(landed.abs_diff(r.total()) <= 4, "landed at the fade time: {landed} vs {}", r.total());
}

/// A host frame with these volumes, as the app sends it.
fn host_frame(volumes: [u8; 4], second_page: bool) -> HostMsg {
    HostMsg::Frame(Frame {
        levels: [0; 4],
        volumes,
        mutes: 0,
        selected: 255,
        mixer_keys: true,
        mic_muted: false,
        playing: false,
        second_page,
    })
}

/// One host report through the pad's state, then into the lights' input as the firmware does.
fn feed(r: &mut Rig, s: &mut State<u64>, msg: HostMsg) {
    s.handle(msg, r.t);
    r.input.alive = s.host_alive(r.t);
    r.input.mixer.volume = s.mixer.volume;
    r.input.mixer.second_page = s.mixer.second_page;
    r.input.frame_seen = s.frame_seen;
}

/// Every channel byte of every LED within `tol` of the other frame's.
fn near(a: &Bytes, b: &Bytes, tol: u8) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| x.abs_diff(*y) <= tol)
}

fn lum(b: &Bytes, led: usize) -> u32 {
    let (x, y, z) = rgb(b, led);
    x as u32 + y as u32 + z as u32
}

#[test]
fn a_volume_change_shows_the_fader_on_its_column_for_the_readout_time() {
    let mut s = State::new(0u64);
    let mut r = Rig::new(false);
    feed(&mut r, &mut s, host_frame([50; 4], false));
    r.run(1500);
    let rest = r.last;

    feed(&mut r, &mut s, host_frame([50, 50, 100, 50], false));
    let moved = r.t;
    r.run(300);
    // Column 2's three meter LEDs (2, 6, 10) fill to full; the sound is silent.
    for led in [2, 6, 10] {
        assert!(lum(&r.last, led) > lum(&rest, led) * 3, "led {led}: {} vs {}", lum(&r.last, led), lum(&rest, led));
    }
    // The other columns and the mute row do not move.
    for led in [0, 1, 3, 4, 5, 7, 8, 9, 11, 12, 13, 14, 15] {
        assert!(lum(&r.last, led).abs_diff(lum(&rest, led)) <= 3, "led {led} moved");
    }
    // It holds for the readout time, then glides back down to the meter.
    r.run(moved + POPUP_MS - 20 - r.t);
    assert!(lum(&r.last, 2) > lum(&rest, 2) * 3, "still the fader just before the readout ends");
    let after = r.run(100);
    assert!(lum(&after[after.len() - 1], 2) < lum(&after[0], 2), "the release starts");
    for w in after.windows(2) {
        assert!(lum(&w[0], 2) <= lum(&w[1], 2) + 12, "glides, no jump");
    }
    r.run(2000);
    assert!(near(&r.last, &rest, 2), "back to the meter");
}

#[test]
fn the_first_frame_from_a_new_host_shows_no_fader() {
    let mut s = State::new(0u64);
    let mut r = Rig::new(false);
    // The app sends its settings before its first frame: the host is there, its volumes not yet.
    feed(&mut r, &mut s, HostMsg::Heartbeat);
    r.run(1500);
    let before = r.last;
    feed(&mut r, &mut s, host_frame([70, 80, 55, 30], false));
    r.run(300);
    assert!(near(&r.last, &before, 2), "the pad's boot volumes are not a change");

    // The host goes away for longer than its timeout and comes back with other volumes.
    r.run(4000);
    feed(&mut r, &mut s, HostMsg::Heartbeat);
    r.run(1500);
    let back = r.last;
    feed(&mut r, &mut s, host_frame([10, 20, 30, 40], false));
    r.run(300);
    assert!(near(&r.last, &back, 2), "the last host's volumes are not a change");

    // The next change is one.
    feed(&mut r, &mut s, host_frame([10, 20, 90, 40], false));
    r.run(300);
    assert!(lum(&r.last, 2) > lum(&back, 2) * 2);
}

#[test]
fn page_two_rests_the_meter_rows_and_keeps_the_mute_row() {
    let mut s = State::new(0u64);
    let mut r = Rig::new(false);
    feed(&mut r, &mut s, host_frame([50; 4], false));
    r.input.mixer.mutes = 0b0010;
    r.input.mixer.selected = 3;
    r.input.mixer.level = [200, 200, 120, 40];
    r.run(1500);
    let one = r.last;

    feed(&mut r, &mut s, host_frame([50; 4], true));
    r.run(1500);
    let two = r.last;
    // The meter rows of the unmuted channels rest at one even ice level, whatever plays and
    // whichever has focus.
    let rest = rgb(&two, 0);
    for led in [0, 2, 3, 4, 6, 7, 8, 10, 11] {
        let (a, b, c) = rgb(&two, led);
        assert!(a.abs_diff(rest.0) <= 1 && b.abs_diff(rest.1) <= 1 && c.abs_diff(rest.2) <= 1, "led {led}");
        assert!(lum(&two, led) < lum(&one, led), "led {led} rests below its meter");
    }
    assert!(rest.2 > rest.0, "ice: blue above red");
    // The mute row and the muted channel stay as on page 1.
    for led in [12, 13, 14, 15] {
        assert!(lum(&two, led).abs_diff(lum(&one, led)) <= 30, "mute row led {led}");
    }
    let (red, _, blue) = rgb(&two, 5);
    assert!(red > blue, "the muted column stays amber");

    // A turned knob on page 2 leaves the rest alone.
    feed(&mut r, &mut s, host_frame([50, 50, 100, 50], true));
    r.run(300);
    assert!(lum(&r.last, 2).abs_diff(lum(&two, 2)) <= 3);
}
