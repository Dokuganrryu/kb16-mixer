//! The firmware logic as it was before it moved into pad-core (commit 3e6b588), kept as a test
//! oracle. Each test feeds the old code and the new code the same long pseudo-random input and
//! asks for the same output, bit for bit.
//!
//! The old code is copied unchanged except for three things: a stand-in for embassy-time's
//! `Instant` and `Duration` (ticking at 1 MHz, as in the firmware), loop bodies turned into
//! step functions, and globals (the state lock, the event channel, the meter atomics) passed in
//! as arguments.
//!
//! One module differs on purpose: the keyboard report (`pad_core::route::Keyboard`, which left
//! `firmware/src/main.rs` after commit 14c3e6a). The old `release(code)` freed the
//! first slot with that usage, whichever key had pressed it: with `ctrl+c` on one key and `c` on
//! another, a tap of `c` let go of Ctrl and left `c` held, so it auto-repeated. The old code
//! also dropped a modifier-only key and a seventh key for good, and put a usage in the report
//! twice when two keys held it. The new code frees what the released key added. So
//! `old::keyboard` is compared only where its answer was right: up to six keys, each with its
//! own non-zero usage. The cases where it was wrong are pinned in
//! `tests/keyboard.rs`.
//!
//! The LED bytes differ on purpose too. The old lights stepped visibly while audio played:
//! a meter jumped with each host frame (~30 Hz), and 9-bit gamma-2 output left large steps near
//! black. The new lights glide per LED, use gamma 2.4 and dither in quarter steps, dim much
//! further when idle and then go off (see `pad_core::leds`; pinned in `tests/lights.rs`). So
//! `old::leds` is compared only for the meter ballistics, which the screen draws and which did
//! not change.

use kb16_mixer_protocol::{HostMsg, LOGO_NAME_LEN, NEVER, NOTICE};
use pad_core::Time;

mod embassy {
    /// embassy-time's clock at the firmware's 1 MHz tick rate.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Instant {
        pub ticks: u64,
    }

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
    pub struct Duration {
        ticks: u64,
    }

    impl Instant {
        pub const fn from_ticks(ticks: u64) -> Instant {
            Instant { ticks }
        }

        pub fn as_millis(&self) -> u64 {
            self.ticks / 1000
        }

        pub fn saturating_duration_since(&self, earlier: Instant) -> Duration {
            Duration { ticks: self.ticks.saturating_sub(earlier.ticks) }
        }
    }

    impl Duration {
        pub const fn from_millis(ms: u64) -> Duration {
            Duration { ticks: ms * 1000 }
        }

        pub fn as_millis(&self) -> u64 {
            self.ticks / 1000
        }
    }
}

use embassy::{Duration, Instant};

/// What `firmware/src/state.rs` implements for its `Tick`, here for the stand-in clock.
impl Time for Instant {
    fn ms(self) -> u64 {
        self.as_millis()
    }

    fn ms_since(self, earlier: Instant) -> u64 {
        self.saturating_duration_since(earlier).as_millis()
    }
}

/// xorshift64*: small, deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.below(one_in) == 0
    }
}

// ---------------------------------------------------------------------------------------------
// The old code.

mod old {
    /// `firmware/src/main.rs` at commit 14c3e6a.
    pub mod keyboard {
        /// Keys currently typed, as (HID usage, modifier bits).
        pub struct Keyboard {
            pub held: [(u8, u8); 6],
        }

        impl Keyboard {
            pub fn report(&self, extra: u8) -> [u8; 8] {
                let mut r = [0u8; 8];
                let mut n = 2;
                for &(code, mods) in self.held.iter().chain([(extra, 0)].iter()) {
                    if code != 0 && n < 8 {
                        r[0] |= mods;
                        r[n] = code;
                        n += 1;
                    }
                }
                r
            }
            pub fn press(&mut self, code: u8, mods: u8) {
                if let Some(slot) = self.held.iter_mut().find(|s| s.0 == 0) {
                    *slot = (code, mods);
                }
            }
            pub fn release(&mut self, code: u8) {
                if let Some(slot) = self.held.iter_mut().find(|s| s.0 == code) {
                    *slot = (0, 0);
                }
            }
        }
    }

    pub mod keys {
        pub const ROWS: usize = 4;
        pub const COLS: usize = 5;
        const DEBOUNCE_MS: u8 = 5;
        const BOOT_COMBO_MS: u16 = 2000;

        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Event {
            Key { row: u8, col: u8, pressed: bool },
            Turn { knob: u8, cw: bool },
        }

        const STEPS_PER_DETENT: i8 = 4;

        pub struct Knob {
            phase: u8,
            steps: i8,
        }

        fn phase(ab: u8) -> u8 {
            [0, 1, 3, 2][ab as usize & 3]
        }

        impl Knob {
            pub fn new(ab: u8) -> Knob {
                Knob { phase: phase(ab), steps: 0 }
            }

            pub fn update(&mut self, ab: u8) -> Option<bool> {
                let p = phase(ab);
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

        /// The locals of the old `scan` task.
        pub struct Scan {
            pub stable: [[bool; COLS]; ROWS],
            pub count: [[u8; COLS]; ROWS],
            pub knobs: [Knob; 3],
            pub turns: [i8; 3],
            pub combo_ms: u16,
        }

        impl Scan {
            /// One pass of the old scan loop. Returns true where the old code entered the bootloader.
            pub fn step(
                &mut self,
                raw_keys: &[[bool; COLS]; ROWS],
                knob_ab: &[u8; 3],
                try_send: &mut impl FnMut(Event) -> bool,
            ) -> bool {
                let (stable, count, knobs, turns) =
                    (&mut self.stable, &mut self.count, &mut self.knobs, &mut self.turns);
                for row in 0..ROWS {
                    for col in 0..COLS {
                        let raw = raw_keys[row][col];
                        if raw == stable[row][col] {
                            count[row][col] = 0;
                            continue;
                        }
                        count[row][col] = count[row][col].saturating_add(1);
                        if count[row][col] >= DEBOUNCE_MS
                            && try_send(Event::Key { row: row as u8, col: col as u8, pressed: raw })
                        {
                            count[row][col] = 0;
                            stable[row][col] = raw;
                        }
                    }
                }

                for (i, &ab) in knob_ab.iter().enumerate() {
                    if let Some(cw) = knobs[i].update(ab) {
                        turns[i] = (turns[i] + if cw { 1 } else { -1 }).clamp(-8, 8);
                    }
                    if turns[i] != 0 && try_send(Event::Turn { knob: i as u8, cw: turns[i] > 0 }) {
                        turns[i] -= turns[i].signum();
                    }
                }

                if stable[0][0] && stable[0][3] && stable[3][0] && stable[3][3] {
                    self.combo_ms += 1;
                    if self.combo_ms >= BOOT_COMBO_MS {
                        return true;
                    }
                } else {
                    self.combo_ms = 0;
                }
                false
            }
        }
    }

    pub mod name {
        use kb16_mixer_protocol::LOGO_NAME_LEN as LEN;

        const MAGIC: u16 = 0x4E4D;
        pub const WORDS: usize = 2 + LEN / 2 + 1;
        pub const DEFAULT: [u8; LEN] = *b"KB16\0\0\0\0\0\0";
        const MIN_GAP_S: u32 = 60;
        const MAX_WRITES: u8 = 8;

        pub fn valid(name: &[u8; LEN]) -> bool {
            let end = name.iter().position(|&b| b == 0).unwrap_or(LEN);
            let (text, pad) = name.split_at(end);
            pad.iter().all(|&b| b == 0)
                && text.iter().all(|&b| b == b' ' || b.is_ascii_uppercase() || b.is_ascii_digit())
                && text.iter().any(|&b| b != b' ')
        }

        pub fn record(name: &[u8; LEN]) -> [u16; WORDS] {
            let mut w = [0u16; WORDS];
            w[0] = MAGIC;
            w[1] = LEN as u16;
            for (i, pair) in name.chunks(2).enumerate() {
                w[2 + i] = pair[0] as u16 | (pair[1] as u16) << 8;
            }
            w[WORDS - 1] = checksum(&w[..WORDS - 1]);
            w
        }

        fn checksum(words: &[u16]) -> u16 {
            let (mut a, mut b) = (0u16, 0u16);
            for w in words {
                for byte in w.to_le_bytes() {
                    a = (a + byte as u16) % 255;
                    b = (b + a) % 255;
                }
            }
            b << 8 | a
        }

        /// The old `load`, on the words `stored()` read.
        pub fn load(w: [u16; WORDS]) -> [u8; LEN] {
            if w[0] != MAGIC || w[1] != LEN as u16 || w[WORDS - 1] != checksum(&w[..WORDS - 1]) {
                return DEFAULT;
            }
            let mut name = [0u8; LEN];
            for (i, pair) in name.chunks_mut(2).enumerate() {
                pair.copy_from_slice(&w[2 + i].to_le_bytes());
            }
            if valid(&name) { name } else { DEFAULT }
        }

        /// The rate limit of the old `save`: false where it returned before writing.
        pub fn gate(last: u32, writes: u8, now: u32) -> bool {
            !((last != 0 && now.wrapping_sub(last) < MIN_GAP_S) || writes >= MAX_WRITES)
        }
    }

    pub mod boot {
        const MAX_CRASHES: u32 = 3;

        /// The old `boot_guard` on its registers: None = bootloader, Some = the new DR9.
        pub fn guard(crashed: bool, dr9: u32) -> Option<u32> {
            let count = if crashed { dr9 + 1 } else { 0 };
            if count >= MAX_CRASHES {
                return None;
            }
            Some(count)
        }
    }

    pub mod state {
        use super::super::{Duration, Instant};
        use kb16_mixer_protocol::{HostMsg, Key, LOGO_NAME_LEN, NEVER, NOTICE};

        pub const NCH: usize = kb16_mixer_protocol::CHANNELS;
        const HOST_TIMEOUT: Duration = Duration::from_millis(3000);
        pub const DEFAULT_BRIGHTNESS: u8 = 170;
        pub const DEFAULT_OLED_TOP: u8 = 3;
        pub const RIPPLES: usize = 4;
        pub const DEFAULT_IDLE_MIN: u8 = 10;

        #[derive(Clone, Copy)]
        pub struct Popup {
            pub at: Instant,
            pub volume: u8,
            pub muted: bool,
            pub name: [u8; 16],
        }

        #[derive(Clone, Copy)]
        pub struct Mixer {
            pub n: u8,
            pub colors: [[u8; 3]; NCH],
            pub level: [u8; NCH],
            pub volume: [u8; NCH],
            pub mutes: u8,
            pub selected: u8,
            pub mixer_keys: bool,
            pub mic_muted: bool,
            pub playing: bool,
            pub second_page: bool,
            pub names: [[u8; 6]; NCH],
        }

        #[derive(Clone, Copy)]
        pub struct Ripple {
            pub at: Instant,
            pub row: u8,
            pub col: u8,
        }

        pub struct State {
            pub host_seen: Option<Instant>,
            pub mixer: Mixer,
            pub brightness: u8,
            pub oled_top: u8,
            pub screen_off_min: u8,
            pub lights_dim_min: u8,
            pub popup: Option<Popup>,
            pub activity: Instant,
            pub keys: [Key; 16],
            pub ripples: [Option<Ripple>; RIPPLES],
            pub ripple_next: usize,
            pub logo_name: [u8; LOGO_NAME_LEN],
        }

        impl State {
            pub const fn new() -> State {
                State {
                    host_seen: None,
                    mixer: Mixer {
                        n: 0,
                        colors: [[0; 3]; NCH],
                        level: [0; NCH],
                        volume: [0; NCH],
                        mutes: 0,
                        selected: 0,
                        mixer_keys: true,
                        mic_muted: false,
                        playing: false,
                        second_page: false,
                        names: [[b' '; 6]; NCH],
                    },
                    brightness: DEFAULT_BRIGHTNESS,
                    oled_top: DEFAULT_OLED_TOP,
                    screen_off_min: DEFAULT_IDLE_MIN,
                    lights_dim_min: DEFAULT_IDLE_MIN,
                    popup: None,
                    activity: Instant::from_ticks(0),
                    keys: [Key::Host; 16],
                    ripples: [None; RIPPLES],
                    ripple_next: 0,
                    logo_name: super::name::DEFAULT,
                }
            }

            pub fn host_alive(&self, now: Instant) -> bool {
                self.host_seen.is_some_and(|t| now.saturating_duration_since(t) < HOST_TIMEOUT)
            }

            pub fn add_ripple(&mut self, now: Instant, row: u8, col: u8) {
                self.ripples[self.ripple_next] = Some(Ripple { at: now, row, col });
                self.ripple_next = (self.ripple_next + 1) % RIPPLES;
            }
        }

        pub fn past(idle_ms: u64, minutes: u8) -> bool {
            minutes != NEVER && idle_ms > minutes as u64 * 60_000
        }

        #[derive(PartialEq, Eq, Debug)]
        pub enum After {
            Nothing,
            Bootloader,
            SaveName([u8; LOGO_NAME_LEN]),
        }

        /// The old `handle_report`, with the lock taken by the caller and the clock read passed in.
        pub fn handle_report(s: &mut State, r: &[u8], now: Instant) -> After {
            let Some(msg) = HostMsg::decode(r) else { return After::Nothing };
            if msg == HostMsg::Bootloader {
                return After::Bootloader;
            }
            let mut after = After::Nothing;
            {
                if !s.host_alive(now) {
                    s.mixer.playing = false;
                    s.mixer.second_page = false;
                }
                s.host_seen = Some(now);
                let m = &mut s.mixer;
                match msg {
                    HostMsg::Popup(p) => {
                        let volume = if p.volume == NOTICE { NOTICE } else { p.volume.min(100) };
                        s.popup = Some(Popup { at: now, volume, muted: p.muted, name: p.name });
                    }
                    HostMsg::Settings(c) => {
                        m.n = c.channels.min(NCH as u8);
                        m.colors = c.colors;
                        s.brightness = if c.brightness == 0 { DEFAULT_BRIGHTNESS } else { c.brightness };
                        s.oled_top =
                            if c.oled_top_plus_one == 0 { DEFAULT_OLED_TOP } else { (c.oled_top_plus_one - 1).min(6) };
                        s.screen_off_min = if c.screen_off_min == 0 { DEFAULT_IDLE_MIN } else { c.screen_off_min };
                        s.lights_dim_min = if c.lights_dim_min == 0 { DEFAULT_IDLE_MIN } else { c.lights_dim_min };
                    }
                    HostMsg::Frame(f) => {
                        m.level = f.levels;
                        #[allow(clippy::needless_range_loop)]
                        for i in 0..NCH {
                            m.volume[i] = f.volumes[i].min(100);
                        }
                        m.mutes = f.mutes;
                        m.selected = f.selected;
                        m.mixer_keys = f.mixer_keys;
                        m.mic_muted = f.mic_muted;
                        m.playing = f.playing;
                        m.second_page = f.second_page;
                    }
                    HostMsg::Names(names) => m.names = names,
                    HostMsg::Keys(k) => {
                        let first = k.first as usize;
                        for (i, key) in k.keys.into_iter().enumerate() {
                            if let Some(slot) = s.keys.get_mut(first + i) {
                                *slot = key;
                            }
                        }
                    }
                    HostMsg::LogoName(name) if super::name::valid(&name) => {
                        s.logo_name = name;
                        after = After::SaveName(name);
                    }
                    HostMsg::LogoName(_) | HostMsg::Heartbeat | HostMsg::Bootloader | HostMsg::Unknown(_) => {}
                }
                if s.mixer.playing {
                    s.activity = now;
                }
            }
            after
        }

        /// What the old `input` task did to the state for each key or knob event.
        #[allow(clippy::collapsible_if)]
        pub fn touched(s: &mut State, now: Instant, ev: super::keys::Event) {
            s.activity = now;
            if let super::keys::Event::Key { row, col, pressed: true } = ev {
                if col < 4 {
                    s.add_ripple(now, row, col);
                }
            }
        }
    }

    #[allow(clippy::needless_range_loop)]
    pub mod leds {
        use super::super::Instant;
        use super::state::{Mixer, NCH, RIPPLES, Ripple};
        use pad_core::assets::SIN;

        pub const N: usize = 16;
        const POWER_CAP: u32 = (N as u32) * 255 * 3 * 9 / 20;
        const FADE_MS: u64 = 700;
        const RIPPLE_MS: u64 = 520;
        const IDLE_DIM: u32 = 35;

        const AMBER: [u8; 3] = [255, 70, 12];
        const ICE: [u8; 3] = [185, 212, 255];

        fn wave(phase: u32) -> u32 {
            SIN[(phase & 0xFF) as usize] as u32
        }

        fn isqrt(v: u32) -> u32 {
            let mut r = 0;
            let mut bit = 1 << 14;
            while bit != 0 {
                if (r + bit) * (r + bit) <= v {
                    r += bit;
                }
                bit >>= 1;
            }
            r
        }

        struct Frame {
            px: [[u32; 3]; N],
        }

        impl Frame {
            fn put(&mut self, led: usize, c: [u8; 3], bright: u32) {
                let b = bright.min(255);
                let lin = b * b;
                for k in 0..3 {
                    self.px[led][k] = c[k] as u32 * lin / 255;
                }
            }
        }

        fn bloom(ripples: &[Option<Ripple>; RIPPLES], now: Instant, row: u32, col: u32) -> u32 {
            let mut best = 0;
            for r in ripples.iter().flatten() {
                let age = now.saturating_duration_since(r.at).as_millis();
                if age >= RIPPLE_MS {
                    continue;
                }
                let (dr, dc) = (row as i32 - r.row as i32, col as i32 - r.col as i32);
                let dist = isqrt(((dr * dr + dc * dc) as u32) << 12);
                let front = (age as u32) * 64 * 8 / 1000;
                let off = dist.abs_diff(front);
                if off < 80 {
                    let fade = (RIPPLE_MS - age) as u32 * 255 / RIPPLE_MS as u32;
                    best = best.max((80 - off) * 255 / 80 * fade / 255);
                }
            }
            best
        }

        fn render_mixer(
            f: &mut Frame,
            m: &Mixer,
            disp: &[u32; NCH],
            swell: u32,
            ripples: &[Option<Ripple>; RIPPLES],
            now: Instant,
        ) {
            let ms = now.as_millis();
            for led in 0..N {
                let (row, col) = ((led / 4) as u32, led % 4);
                if col >= m.n as usize {
                    continue;
                }
                let tint = m.colors[col];
                let muted = m.mutes & (1 << col) != 0;
                let sel = col as u8 == m.selected;
                let glow = bloom(ripples, now, row, col as u32) * 90 / 255;

                if muted {
                    let breath = wave((ms / 14) as u32);
                    let b = if row == 3 { 135 + breath * 55 / 255 } else { 62 + breath * 14 / 255 };
                    f.put(led, AMBER, b + glow);
                    continue;
                }
                if row == 3 {
                    f.put(led, tint, if sel { 215 } else { 128 } + glow);
                    continue;
                }
                let floor = if sel { 124 } else { 98 } + swell * 26 / 255;
                let seg = 2 - row;
                let level = disp[col] >> 8;
                let fill = (level * 3).saturating_sub(seg * 255).min(255);
                f.put(led, tint, floor + (255 - floor) * fill / 255 + glow);
            }
        }

        fn render_idle(f: &mut Frame, ripples: &[Option<Ripple>; RIPPLES], now: Instant) {
            let ms = now.as_millis();
            for led in 0..N {
                let (row, col) = ((led / 4) as u32, (led % 4) as u32);
                let swell = wave((ms / 24) as u32 + (row + col) * 22);
                let glow = bloom(ripples, now, row, col) * 90 / 255;
                f.put(led, ICE, 104 + swell * 78 / 255 + glow);
            }
        }

        /// The locals of the old `run` task.
        pub struct Run {
            pub disp: [u32; NCH],
            pub swell: u32,
            pub mode_alive: bool,
            pub mode_since: Instant,
            pub parity: bool,
            pub dim: u32,
            pub dark: bool,
            pub last_px: [[u32; 3]; N],
            pub last_bytes: [u8; N * 3],
            pub still: u32,
            pub unsent: u32,
        }

        /// What one pass read from the state lock.
        pub struct Shared<'a> {
            pub alive: bool,
            pub mixer: &'a Mixer,
            pub brightness: u8,
            pub ripples: &'a [Option<Ripple>; RIPPLES],
            pub activity: Instant,
            pub dim_after: u8,
        }

        impl Run {
            pub fn new(now: Instant) -> Run {
                Run {
                    disp: [0u32; NCH],
                    swell: 0u32,
                    mode_alive: false,
                    mode_since: now,
                    parity: false,
                    dim: 255u32 << 8,
                    dark: false,
                    last_px: [[0u32; 3]; N],
                    last_bytes: [0u8; N * 3],
                    still: 0u32,
                    unsent: 0u32,
                }
            }

            /// One pass of the old loop after the tick. `wake` is the clock read for the fade-in,
            /// `now` the one after it. Returns the bytes sent, if any, and the meters stored.
            pub fn step(
                &mut self,
                suspended: bool,
                wake: Instant,
                now: Instant,
                s: &Shared,
                meter: &mut [u16; NCH],
            ) -> Option<[u8; N * 3]> {
                if suspended {
                    if !self.dark {
                        self.dark = true;
                        return Some([0; N * 3]);
                    }
                    return None;
                }
                if self.dark {
                    self.dark = false;
                    self.mode_since = wake;
                }
                let (alive, mixer, brightness, ripples, activity, dim_after) =
                    (s.alive, *s.mixer, s.brightness, *s.ripples, s.activity, s.dim_after);
                let idle = super::state::past(now.saturating_duration_since(activity).as_millis(), dim_after);
                let dim_target = if idle { IDLE_DIM << 8 } else { 255 << 8 };
                if self.dim < dim_target {
                    self.dim = (self.dim + 600).min(dim_target);
                } else {
                    self.dim -= (self.dim - dim_target).min(40);
                }
                if alive != self.mode_alive {
                    self.mode_alive = alive;
                    self.mode_since = now;
                }

                let disp = &mut self.disp;
                let mut loudest = 0;
                for i in 0..NCH {
                    let target = if alive { (mixer.level[i] as u32) << 8 } else { 0 };
                    if target > disp[i] {
                        disp[i] += (target - disp[i]) / 3 + 1;
                    } else {
                        disp[i] -= (disp[i] - target) / 16;
                    }
                    meter[i] = disp[i].min(0xFFFF) as u16;
                    if mixer.mutes & (1 << i) == 0 {
                        loudest = loudest.max(disp[i]);
                    }
                }
                if loudest > self.swell {
                    self.swell += (loudest - self.swell) / 12 + 1;
                } else {
                    self.swell -= (self.swell - loudest) / 48;
                }

                let mut f = Frame { px: [[0; 3]; N] };
                if alive {
                    render_mixer(&mut f, &mixer, &self.disp, self.swell >> 8, &ripples, now);
                } else {
                    render_idle(&mut f, &ripples, now);
                }

                let env =
                    (now.saturating_duration_since(self.mode_since).as_millis().min(FADE_MS) * 255 / FADE_MS) as u32;
                let master = brightness as u32 * env / 255 * (self.dim >> 8) / 255;

                let mut scaled = [[0u32; 3]; N];
                for i in 0..N {
                    for k in 0..3 {
                        scaled[i][k] = f.px[i][k] * master / 255;
                    }
                }
                self.still = if scaled == self.last_px { self.still + 1 } else { 0 };
                self.last_px = scaled;
                let moving = self.still < 20;

                self.parity = !self.parity;
                let mut out = [[0u8; 3]; N];
                let mut total = 0u32;
                for i in 0..N {
                    for k in 0..3 {
                        let v9 = (scaled[i][k] + 64) >> 7;
                        let half = if moving { v9 & 1 & self.parity as u32 } else { v9 & 1 };
                        let v = ((v9 >> 1) + half).min(255);
                        out[i][k] = v as u8;
                        total += v;
                    }
                }
                if total > POWER_CAP {
                    for px in out.iter_mut() {
                        for c in px.iter_mut() {
                            *c = (*c as u32 * POWER_CAP / total) as u8;
                        }
                    }
                }
                let mut bytes = [0u8; N * 3];
                for (i, p) in out.iter().enumerate() {
                    bytes[i * 3] = p[1];
                    bytes[i * 3 + 1] = p[0];
                    bytes[i * 3 + 2] = p[2];
                }
                self.unsent += 1;
                if bytes != self.last_bytes || self.unsent >= 100 {
                    self.last_bytes = bytes;
                    self.unsent = 0;
                    return Some(bytes);
                }
                None
            }
        }
    }

    #[allow(clippy::needless_range_loop, clippy::manual_div_ceil)]
    pub mod oled {
        use super::super::Instant;
        use super::state::{Mixer, NCH, Popup};
        use pad_core::assets::{BIG_COLS, BIG_OFF, LOGO_CHARS, LOGO_COLS, LOGO_GAP, LOGO_OFF, SIN};
        use pad_core::font::FONT5X7;

        pub const W: i32 = 128;
        const H: i32 = 32;
        const ROWS: i32 = 29;
        const SPLASH_MS: u64 = 1800;
        const POPUP_MS: u64 = 1300;
        const SCREENSAVER_MS: u64 = 45_000;

        pub struct Fb {
            pub buf: [u8; 1 + (W * H / 8) as usize],
            pub top: i32,
        }

        impl Fb {
            fn clear(&mut self) {
                self.buf[1..].fill(0);
            }

            fn get(&self, x: i32, y: i32) -> bool {
                let y = y + self.top;
                (0..W).contains(&x) && (0..H).contains(&y) && self.buf[1 + (y / 8 * W + x) as usize] & 1 << (y % 8) != 0
            }

            fn px(&mut self, x: i32, y: i32, on: bool) {
                let y = y + self.top;
                if !(0..W).contains(&x) || !(0..H).contains(&y) {
                    return;
                }
                let i = 1 + (y / 8 * W + x) as usize;
                let bit = 1u8 << (y % 8);
                if on {
                    self.buf[i] |= bit;
                } else {
                    self.buf[i] &= !bit;
                }
            }

            fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, on: bool) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        self.px(x, y, on);
                    }
                }
            }

            fn pill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
                self.rect(x0, y0, x1, y1, true);
                for (x, y) in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
                    self.px(x, y, false);
                }
            }

            fn text(&mut self, mut x: i32, y: i32, s: &[u8], on: bool) -> i32 {
                for &ch in s {
                    let g = &FONT5X7[(ch.clamp(32, 126) - 32) as usize];
                    for (dx, col) in g.iter().enumerate() {
                        for dy in 0..8 {
                            if col & (1 << dy) != 0 {
                                self.px(x + dx as i32, y + dy, on);
                            }
                        }
                    }
                    x += 6;
                }
                x
            }

            fn centred(&mut self, x0: i32, y: i32, s: &[u8], on: bool) {
                self.text(x0 + (31 - (s.len() as i32 * 6 - 1)) / 2, y, s, on);
            }

            fn big_width(s: &[u8]) -> i32 {
                s.iter().map(|&c| glyph(c).len() as i32 + 2).sum::<i32>() - 2
            }

            fn big(&mut self, mut x: i32, y: i32, s: &[u8]) {
                for &ch in s {
                    for (dx, col) in glyph(ch).iter().enumerate() {
                        let bits = col[0] as u16 | (col[1] as u16) << 8;
                        for dy in 0..16 {
                            if bits & (1 << dy) != 0 {
                                self.px(x + dx as i32, y + dy, true);
                            }
                        }
                    }
                    x += glyph(ch).len() as i32 + 2;
                }
            }
        }

        fn logo_glyph(c: u8) -> &'static [[u8; 4]] {
            match LOGO_CHARS.iter().position(|&l| l == c) {
                Some(i) => &LOGO_COLS[LOGO_OFF[i] as usize..LOGO_OFF[i + 1] as usize],
                None => &[],
            }
        }

        fn logo_width(s: &[u8]) -> i32 {
            s.iter().map(|&c| logo_glyph(c).len() as i32 + LOGO_GAP).sum::<i32>() - LOGO_GAP
        }

        fn glyph(c: u8) -> &'static [[u8; 2]] {
            let i = (c.clamp(32, 126) - 32) as usize;
            &BIG_COLS[BIG_OFF[i] as usize..BIG_OFF[i + 1] as usize]
        }

        fn trimmed(s: &[u8]) -> &[u8] {
            let end = s.iter().rposition(|&c| c != 0 && c != b' ').map_or(0, |i| i + 1);
            &s[..end]
        }

        fn percent(v: u8, out: &mut [u8; 4]) -> &[u8] {
            let v = v.min(100);
            let mut n = 0;
            if v >= 100 {
                out[n] = b'1';
                n += 1;
            }
            if v >= 10 {
                out[n] = b'0' + (v / 10) % 10;
                n += 1;
            }
            out[n] = b'0' + v % 10;
            out[n + 1] = b'%';
            &out[..n + 2]
        }

        fn draw_logo(fb: &mut Fb, ms: u64, reveal: i32, name: &[u8]) {
            let name = trimmed(name);
            if logo_width(name) <= W {
                let mut x = (W - logo_width(name)) / 2;
                for &ch in name {
                    for col in logo_glyph(ch) {
                        let bits = u32::from_le_bytes(*col);
                        for y in 0..32 {
                            if bits & 1 << y != 0 {
                                fb.px(x, y, true);
                            }
                        }
                        x += 1;
                    }
                    x += LOGO_GAP;
                }
            } else {
                let mut n = name.len();
                while n > 0 && Fb::big_width(&name[..n]) > W {
                    n -= 1;
                }
                fb.big((W - Fb::big_width(&name[..n])) / 2, 4, &name[..n]);
            }
            let streak = (ms % 5200) as i32 / 5 - 30;
            for y in 0..H {
                for x in 0..W {
                    let d = x - streak - y / 2;
                    if ((0..4).contains(&d) || d == 6) && fb.get(x, y) {
                        fb.px(x, y, false);
                    }
                }
            }
            let low: [bool; W as usize] = core::array::from_fn(|x| (24..30).any(|y| fb.get(x as i32, y)));
            let t = (ms / 18) as u32;
            for x in 0..W {
                if (x - 1..=x + 1).any(|cx| low.get(cx as usize).copied().unwrap_or(false)) {
                    continue;
                }
                let a = SIN[((x as u32 * 4 + t) & 0xFF) as usize] as i32;
                fb.px(x, 25 + a * 3 / 255, true);
            }
            if reveal < W - 1 {
                fb.rect(reveal.max(-1) + 1, 0, W - 1, H - 1, false);
            }
        }

        /// `meter` stands in for `state::meter`.
        fn draw_mixer(fb: &mut Fb, m: &Mixer, peaks: &[u8; NCH], meter: &[u8; NCH]) {
            for c in 0..m.n as usize {
                let x0 = c as i32 * 32;
                let muted = m.mutes & (1 << c) != 0;
                let name = trimmed(&m.names[c]);
                let name = &name[..name.len().min(kb16_mixer_protocol::NAME_SHOWN)];
                if c as u8 == m.selected {
                    fb.pill(x0, 0, x0 + 30, 8);
                    fb.centred(x0, 1, name, false);
                } else {
                    fb.centred(x0, 1, name, true);
                }

                let lit = ((meter[c] as i32 * 7 + 254) / 255).min(7);
                let peak = ((peaks[c] as i32 * 7 + 254) / 255).min(7);
                for s in 0..7 {
                    let sx = x0 + 2 + s * 4;
                    if !muted && s < lit {
                        fb.rect(sx, 11, sx + 2, 14, true);
                    } else if !muted && s == peak - 1 {
                        fb.rect(sx, 11, sx + 2, 11, true);
                        fb.rect(sx, 14, sx + 2, 14, true);
                    } else {
                        fb.px(sx + 1, 14, true);
                    }
                }

                for x in (x0 + 2..=x0 + 28).step_by(2) {
                    fb.px(x, 18, true);
                }
                let w = m.volume[c] as i32 * 27 / 100;
                if w > 0 {
                    fb.rect(x0 + 2, 17, x0 + 1 + w, 19, true);
                }

                if muted {
                    fb.pill(x0 + 2, 21, x0 + 28, ROWS - 1);
                    fb.centred(x0, 22, b"MUTE", false);
                } else {
                    let mut tmp = [0u8; 4];
                    let p = percent(m.volume[c], &mut tmp);
                    fb.centred(x0, 22, p, true);
                }
            }
            if m.mic_muted {
                fb.rect(0, 9, W - 2, 9, true);
            }
            if m.second_page {
                fb.rect(W - 1, 2, W - 1, 6, true);
            }
        }

        fn draw_notice(fb: &mut Fb, text: &[u8]) {
            let mut n = text.len();
            while n > 0 && Fb::big_width(&text[..n]) > W {
                n -= 1;
            }
            let text = &text[..n];
            fb.big((W - Fb::big_width(text)) / 2, (ROWS - 16) / 2, text);
        }

        fn draw_popup(fb: &mut Fb, p: &Popup, shown_vol: u32) {
            if p.volume == kb16_mixer_protocol::NOTICE {
                return draw_notice(fb, trimmed(&p.name));
            }
            let mut tmp = [0u8; 4];
            let right: &[u8] = if p.muted { b"MUTED" } else { percent(p.volume, &mut tmp) };
            let rw = Fb::big_width(right);
            fb.big(W - rw, 0, right);

            let name = trimmed(&p.name);
            let mut n = name.len();
            while n > 0 && Fb::big_width(&name[..n]) > W - rw - 6 {
                n -= 1;
            }
            fb.big(0, 0, &name[..n]);

            fb.rect(1, 19, W - 2, 19, true);
            fb.rect(1, 28, W - 2, 28, true);
            fb.rect(0, 20, 0, 27, true);
            fb.rect(W - 1, 20, W - 1, 27, true);
            let w = (shown_vol * 122 / 1000) as i32;
            for x in 3..3 + w {
                for y in 21..=26 {
                    if !p.muted || (x + y) % 3 == 0 {
                        fb.px(x, y, true);
                    }
                }
            }
        }

        /// The locals of the old `run` task that the drawing uses.
        pub struct Run {
            pub fb: Fb,
            pub peaks: [u8; NCH],
            pub peak_at: [Instant; NCH],
            pub shown_vol: u32,
            pub popup_open: bool,
        }

        impl Run {
            pub fn new(now: Instant, top: i32) -> Run {
                let mut fb = Fb { buf: [0; 1 + (W * H / 8) as usize], top };
                fb.buf[0] = 0x40;
                Run { fb, peaks: [0u8; NCH], peak_at: [now; NCH], shown_vol: 0, popup_open: false }
            }

            /// The old loop from the peak update to `was_calm`. Returns `calm`.
            #[allow(clippy::too_many_arguments)]
            pub fn step(
                &mut self,
                now: Instant,
                since_boot: u64,
                alive: bool,
                mixer: &Mixer,
                popup: Option<Popup>,
                idle: u64,
                name: &[u8],
                meter: &[u8; NCH],
            ) -> bool {
                let ms = now.as_millis();
                let (fb, peaks, peak_at) = (&mut self.fb, &mut self.peaks, &mut self.peak_at);
                for c in 0..NCH {
                    let l = meter[c];
                    if l >= peaks[c] {
                        peaks[c] = l;
                        peak_at[c] = now;
                    } else if now.saturating_duration_since(peak_at[c]).as_millis() > 900 {
                        peaks[c] = peaks[c].saturating_sub(6);
                    }
                }

                fb.clear();
                let popup = popup.filter(|p| alive && now.saturating_duration_since(p.at).as_millis() < POPUP_MS);
                let calm = if since_boot < SPLASH_MS {
                    draw_logo(fb, ms, (since_boot * 160 / 900) as i32, name);
                    false
                } else if let Some(p) = popup {
                    let target = p.volume.min(100) as u32 * 10;
                    self.shown_vol = if !self.popup_open {
                        target
                    } else if target > self.shown_vol {
                        self.shown_vol + (target - self.shown_vol + 2) / 3
                    } else {
                        self.shown_vol - (self.shown_vol - target + 2) / 3
                    };
                    draw_popup(fb, &p, self.shown_vol);
                    false
                } else if alive && idle < SCREENSAVER_MS {
                    draw_mixer(fb, mixer, peaks, meter);
                    false
                } else {
                    draw_logo(fb, ms, W, name);
                    true
                };
                self.popup_open = popup.is_some();
                calm
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Old and new, side by side.

use pad_core::state::State;

/// Every field of the old and the new state agrees.
fn same_state(o: &old::state::State, n: &State<Instant>) {
    assert_eq!(o.host_seen, n.host_seen);
    same_mixer(&o.mixer, &n.mixer);
    assert_eq!(o.brightness, n.brightness);
    assert_eq!(o.oled_top, n.oled_top);
    assert_eq!(o.screen_off_min, n.screen_off_min);
    assert_eq!(o.lights_dim_min, n.lights_dim_min);
    match (o.popup, n.popup) {
        (None, None) => {}
        (Some(a), Some(b)) => assert_eq!((a.at, a.volume, a.muted, a.name), (b.at, b.volume, b.muted, b.name)),
        _ => panic!("popup differs"),
    }
    assert_eq!(o.activity, n.activity);
    assert_eq!(o.keys, n.keys);
    for (a, b) in o.ripples.iter().zip(n.ripples.iter()) {
        assert_eq!(a.map(|r| (r.at, r.row, r.col)), b.map(|r| (r.at, r.row, r.col)));
    }
    assert_eq!(o.logo_name, n.logo_name);
}

fn same_mixer(a: &old::state::Mixer, b: &pad_core::state::Mixer) {
    assert_eq!(
        (a.n, a.colors, a.level, a.volume, a.mutes, a.selected),
        (b.n, b.colors, b.level, b.volume, b.mutes, b.selected)
    );
    assert_eq!(
        (a.mixer_keys, a.mic_muted, a.playing, a.second_page, a.names),
        (b.mixer_keys, b.mic_muted, b.playing, b.second_page, b.names)
    );
}

fn new_mixer(m: &old::state::Mixer) -> pad_core::state::Mixer {
    pad_core::state::Mixer {
        n: m.n,
        colors: m.colors,
        level: m.level,
        volume: m.volume,
        mutes: m.mutes,
        selected: m.selected,
        mixer_keys: m.mixer_keys,
        mic_muted: m.mic_muted,
        playing: m.playing,
        second_page: m.second_page,
        names: m.names,
    }
}

/// A report the host might send: mostly well-formed messages with random contents, some noise.
fn report(rng: &mut Rng) -> Vec<u8> {
    let mut r: Vec<u8> = (0..32).map(|_| rng.byte()).collect();
    r[0] = if rng.chance(20) { rng.byte() } else { kb16_mixer_protocol::MAGIC };
    let ids = [0x00, 0x10, 0x20, 0x21, 0x21, 0x21, 0x22, 0x23, 0x24, 0x7F, 0x55];
    r[1] = ids[rng.below(ids.len() as u64) as usize];
    match r[1] {
        // Popups: often a notice, often a volume past 100.
        0x10 if rng.chance(3) => r[3] = NOTICE,
        // Settings: the "pad default" zero and the "never" value turn up often.
        0x20 => {
            for v in &mut r[15..19] {
                if rng.chance(3) {
                    *v = 0;
                } else if rng.chance(3) {
                    *v = NEVER;
                }
            }
        }
        0x24 => {
            let name = random_name(rng);
            r[2..2 + LOGO_NAME_LEN].copy_from_slice(&name);
        }
        0x7F if rng.chance(2) => r[2..6].copy_from_slice(b"BOOT"),
        _ => {}
    }
    let len = if rng.chance(10) { rng.below(33) as usize } else { 32 };
    r.truncate(len);
    r
}

/// A logo name that is valid about half the time.
fn random_name(rng: &mut Rng) -> [u8; LOGO_NAME_LEN] {
    const OK: &[u8] = b" 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut name = [0u8; LOGO_NAME_LEN];
    let len = rng.below(LOGO_NAME_LEN as u64 + 1) as usize;
    for b in name.iter_mut().take(len) {
        *b = OK[rng.below(OK.len() as u64) as usize];
    }
    if rng.chance(4) {
        name[rng.below(LOGO_NAME_LEN as u64) as usize] = rng.byte();
    }
    name
}

/// Moves the clock on by a random step, from a few microseconds to minutes, so millisecond
/// rounding and every timeout boundary come up.
fn tick(rng: &mut Rng, now: &mut Instant) {
    now.ticks += match rng.below(10) {
        0 => rng.below(1000),
        1..=6 => rng.below(40_000),
        7 => rng.below(4_000_000),
        8 => rng.below(700_000_000),
        _ => 2_999_000 + rng.below(2000),
    };
}

#[test]
fn host_reports_change_the_state_as_before() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    for _run in 0..200 {
        let mut o = old::state::State::new();
        let mut n = State::new(Instant::from_ticks(0));
        let mut now = Instant::from_ticks(0);
        for _ in 0..1000 {
            tick(&mut rng, &mut now);
            if rng.chance(4) {
                let ev = if rng.chance(2) {
                    old::keys::Event::Key { row: rng.below(4) as u8, col: rng.below(5) as u8, pressed: rng.chance(2) }
                } else {
                    old::keys::Event::Turn { knob: rng.below(3) as u8, cw: rng.chance(2) }
                };
                old::state::touched(&mut o, now, ev);
                let press = match ev {
                    old::keys::Event::Key { row, col, pressed: true } => Some((row, col)),
                    _ => None,
                };
                n.touched(now, press);
            } else {
                let r = report(&mut rng);
                let a = old::state::handle_report(&mut o, &r, now);
                // As `firmware/src/state.rs` does it.
                let b = match HostMsg::decode(&r) {
                    None => pad_core::state::After::Nothing,
                    Some(msg) => n.handle(msg, now),
                };
                let a = match a {
                    old::state::After::Nothing => pad_core::state::After::Nothing,
                    old::state::After::Bootloader => pad_core::state::After::Bootloader,
                    old::state::After::SaveName(name) => pad_core::state::After::SaveName(name),
                };
                assert_eq!(a, b);
            }
            same_state(&o, &n);
            assert_eq!(o.host_alive(now), n.host_alive(now));
            let idle = now.ms_since(n.activity);
            for minutes in [0, 1, 10, 254, NEVER] {
                assert_eq!(old::state::past(idle, minutes), pad_core::state::past(idle, minutes));
            }
        }
    }
}

#[test]
fn the_scan_queues_the_same_events() {
    use old::keys::{COLS, Event, ROWS};
    use pad_core::input::{Combo, Debounce, Knob, Turns};
    let mut rng = Rng(0xFEED_FACE_CAFE_BEEF);
    for _run in 0..100 {
        let start: [u8; 3] = [rng.byte() & 3, rng.byte() & 3, rng.byte() & 3];
        let mut o = old::keys::Scan {
            stable: [[false; COLS]; ROWS],
            count: [[0; COLS]; ROWS],
            knobs: start.map(old::keys::Knob::new),
            turns: [0; 3],
            combo_ms: 0,
        };
        let mut keys = [[Debounce::new(); COLS]; ROWS];
        let mut knobs = start.map(Knob::new);
        let mut turns = [Turns::new(); 3];
        let mut combo = Combo::new();

        let mut raw = [[false; COLS]; ROWS];
        let mut ab = start;
        // Hold the corners now and then, long enough to reach the combo.
        let mut corners = 0u32;
        let mut spin = 0usize;
        let mut full = 0u32;
        for _ in 0..20_000 {
            if corners > 0 {
                corners -= 1;
                for (r, c) in [(0, 0), (0, 3), (3, 0), (3, 3)] {
                    raw[r][c] = !rng.chance(500);
                }
            } else if rng.chance(5000) {
                corners = 2100;
            }
            for row in raw.iter_mut() {
                for k in row.iter_mut() {
                    if rng.chance(30) {
                        *k = !*k; // a bouncing or changing contact
                    }
                }
            }
            if rng.chance(300) {
                spin = [0, 1, 3][rng.below(3) as usize];
            }
            for a in ab.iter_mut() {
                // A fast spin one way, or mostly one step either way and sometimes a missed reading.
                let step = if spin != 0 && !rng.chance(10) {
                    spin
                } else if rng.chance(3) {
                    [1usize, 3, 3, 1, 2][rng.below(5) as usize]
                } else {
                    0
                };
                let p = [0u8, 1, 3, 2].iter().position(|&x| x == *a).unwrap_or(0);
                *a = [0u8, 1, 3, 2][(p + step) % 4];
            }
            // The queue is full now and then, sometimes for a long stretch; both sides see the
            // same answers.
            if rng.chance(200) {
                full = 50 + rng.below(200) as u32;
            }
            full = full.saturating_sub(1);
            let answers: Vec<bool> = (0..32).map(|_| full == 0 && !rng.chance(8)).collect();

            let mut old_events = Vec::new();
            let mut i = 0;
            let old_boot = o.step(&raw, &ab, &mut |ev: Event| {
                let ok = answers[i % answers.len()];
                i += 1;
                if ok {
                    old_events.push(ev);
                }
                ok
            });

            let mut new_events = Vec::new();
            let mut j = 0;
            let mut queue = |ev: Event| {
                let ok = answers[j % answers.len()];
                j += 1;
                if ok {
                    new_events.push(ev);
                }
                ok
            };
            // As `firmware/src/keys.rs` does it.
            for (row, keys) in keys.iter_mut().enumerate() {
                for (col, key) in keys.iter_mut().enumerate() {
                    key.sample(raw[row][col], |pressed| queue(Event::Key { row: row as u8, col: col as u8, pressed }));
                }
            }
            for (i, &ab) in ab.iter().enumerate() {
                if let Some(cw) = knobs[i].update(ab) {
                    turns[i].add(cw);
                }
                turns[i].drain(|cw| queue(Event::Turn { knob: i as u8, cw }));
            }
            let corners_held = [keys[0][0], keys[0][3], keys[3][0], keys[3][3]];
            let new_boot = combo.tick(corners_held.iter().all(Debounce::pressed));

            assert_eq!(old_events, new_events);
            assert_eq!(old_boot, new_boot);
            if old_boot {
                break; // the old code reset the chip here
            }
        }
    }
}

#[test]
fn the_logo_name_record_is_unchanged() {
    use pad_core::name;
    let mut rng = Rng(0x5EED_0000_0000_0001);
    for _ in 0..200_000 {
        let n = random_name(&mut rng);
        assert_eq!(old::name::valid(&n), name::valid(&n));
        assert_eq!(old::name::record(&n), name::record(&n));
        let mut w = old::name::record(&n);
        if rng.chance(2) {
            // A torn or foreign record.
            let i = rng.below(old::name::WORDS as u64) as usize;
            w[i] = rng.next() as u16;
        }
        assert_eq!(old::name::load(w), name::decode(&w));
        let random: [u16; old::name::WORDS] = core::array::from_fn(|_| rng.next() as u16);
        assert_eq!(old::name::load(random), name::decode(&random));
    }
    for last in (0..300).chain([u32::MAX - 1, u32::MAX]) {
        for writes in 0..=10 {
            for now in (0..300).chain([u32::MAX]) {
                assert_eq!(old::name::gate(last, writes, now), name::may_write(last, writes, now));
            }
        }
    }
}

#[test]
fn the_crash_count_rule_is_unchanged() {
    use pad_core::boot::{Boot, after_reset};
    for crashed in [false, true] {
        for dr9 in 0..=u16::MAX {
            let new = match after_reset(crashed, dr9) {
                Boot::Run { crashes } => Some(crashes),
                Boot::Bootloader => None,
            };
            assert_eq!(old::boot::guard(crashed, dr9 as u32), new);
        }
    }
}

/// A random but plausible mixer, as the host would set it.
fn random_mixer(rng: &mut Rng) -> old::state::Mixer {
    let names = [*b"GAME  ", *b"MUSIC ", *b"CHAT  ", *b"SYSTEM", [0; 6], *b"W     "];
    old::state::Mixer {
        n: rng.below(5) as u8,
        colors: core::array::from_fn(|_| [rng.byte(), rng.byte(), rng.byte()]),
        level: core::array::from_fn(|_| if rng.chance(4) { 0 } else { rng.byte() }),
        volume: core::array::from_fn(|_| rng.below(101) as u8),
        mutes: rng.byte() & 0xF,
        selected: if rng.chance(4) { 255 } else { rng.below(4) as u8 },
        mixer_keys: rng.chance(2),
        mic_muted: rng.chance(3),
        playing: rng.chance(2),
        second_page: rng.chance(3),
        names: core::array::from_fn(|_| names[rng.below(names.len() as u64) as usize]),
    }
}

#[test]
fn the_lights_keep_the_same_meters() {
    use old::state::{RIPPLES, Ripple};
    use pad_core::leds::{Input, Lights};
    let mut rng = Rng(0x00C0_FFEE_0000_1234);
    for _run in 0..60 {
        let mut now = Instant::from_ticks(rng.below(10_000_000));
        let mut o = old::leds::Run::new(now);
        let mut n = Lights::new(now);
        let mut mixer = random_mixer(&mut rng);
        let mut ripples: [Option<Ripple>; RIPPLES] = [None; RIPPLES];
        let mut activity = now;
        let mut alive = false;
        let mut suspended = false;
        let mut brightness = 170;
        let mut dim_after = 10;
        let mut meter = [0u16; 4];
        // Busy and quiet stretches, so the lights both dim fully and come back.
        let mut quiet = false;
        for frame in 0..3000 {
            if rng.chance(400) {
                quiet = !quiet;
            }
            // Frames are 5 ms apart; a jump now and then crosses the idle limit.
            now.ticks += 5000 + rng.below(50);
            if rng.chance(500) {
                now.ticks += rng.below(1_200_000_000);
            }
            if rng.chance(200) {
                alive = !alive;
            }
            if rng.chance(800) {
                suspended = !suspended;
            }
            if rng.chance(50) {
                mixer = random_mixer(&mut rng);
            }
            if rng.chance(4) {
                for l in mixer.level.iter_mut() {
                    *l = l.saturating_add(rng.byte() / 8).saturating_sub(rng.byte() / 8);
                }
            }
            if rng.chance(300) {
                brightness = rng.byte();
            }
            if rng.chance(300) {
                dim_after = [0, 1, 10, NEVER][rng.below(4) as usize];
            }
            if !quiet && rng.chance(40) {
                activity = now;
                ripples[frame % RIPPLES] = Some(Ripple { at: now, row: rng.below(4) as u8, col: rng.below(4) as u8 });
            }
            let wake = now;
            now.ticks += rng.below(30); // the second clock read, a little later

            let shared = old::leds::Shared { alive, mixer: &mixer, brightness, ripples: &ripples, activity, dim_after };
            o.step(suspended, wake, now, &shared, &mut meter);

            // As `firmware/src/leds.rs` does it.
            if suspended {
                n.suspended();
            } else {
                n.resume(wake);
                let input = Input {
                    alive,
                    mixer: new_mixer(&mixer),
                    brightness,
                    ripples: ripples.map(|r| r.map(|r| pad_core::state::Ripple { at: r.at, row: r.row, col: r.col })),
                    activity,
                    dim_after,
                    idle_percent: 0,
                };
                n.frame(now, &input);
                assert_eq!(meter, n.meters(), "frame {frame}");
            }
        }
    }
}

#[test]
fn the_screen_draws_the_same_pixels() {
    use pad_core::oled::{Fb, Input, Screen};
    let mut rng = Rng(0xD15C_0DE5_1234_5678);
    let names = [*b"KB16\0\0\0\0\0\0", *b"DESK PC\0\0\0", *b"WWWWWWWWWW", *b"J Q\0\0\0\0\0\0\0", [0; 10]];
    let popups = [*b"GAME\0\0\0\0\0\0\0\0\0\0\0\0", *b"Config loaded\0\0\0", *b"A VERY LONG NAME", [b' '; 16]];
    for _run in 0..40 {
        let mut now = Instant::from_ticks(rng.below(5_000_000));
        let boot = now;
        let top = rng.below(7) as i32;
        let mut o = old::oled::Run::new(now, top);
        let mut fb = Fb::new(top);
        let mut screen = Screen::new(now);
        let mut mixer = random_mixer(&mut rng);
        let mut popup: Option<old::state::Popup> = None;
        let mut alive = rng.chance(2);
        let mut activity = now;
        let name = names[rng.below(names.len() as u64) as usize];
        for _frame in 0..1500 {
            // The 33 ms tick, or any step up to it, so every timeout boundary comes up.
            now.ticks += if rng.chance(2) { 33_000 + rng.below(100) } else { rng.below(33_000) };
            if rng.chance(300) {
                now.ticks += rng.below(60_000_000);
            }
            if rng.chance(100) {
                alive = !alive;
            }
            if rng.chance(30) {
                mixer = random_mixer(&mut rng);
            }
            if rng.chance(20) {
                activity = now;
            }
            if rng.chance(25) {
                let volume = match rng.below(4) {
                    0 => NOTICE,
                    1 => rng.byte(),
                    _ => rng.below(101) as u8,
                };
                popup = Some(old::state::Popup {
                    at: now,
                    volume,
                    muted: rng.chance(3),
                    name: popups[rng.below(popups.len() as u64) as usize],
                });
            }
            let meter: [u8; 4] = core::array::from_fn(|_| rng.byte());
            let since_boot = now.saturating_duration_since(boot).as_millis();
            let mut idle = now.saturating_duration_since(activity).as_millis();
            if rng.chance(20) {
                idle = 44_995 + rng.below(10); // around the screensaver limit
            }

            let a = o.step(now, since_boot, alive, &mixer, popup, idle, &name, &meter);
            let input = Input {
                alive,
                mixer: new_mixer(&mixer),
                popup: popup.map(|p| pad_core::state::Popup {
                    at: p.at,
                    volume: p.volume,
                    muted: p.muted,
                    name: p.name,
                }),
                idle,
                name,
                meters: meter,
            };
            let b = screen.draw(&mut fb, now, since_boot, &input);
            assert_eq!(a, b);
            assert_eq!(o.fb.buf, fb.buf);
        }
    }
}

#[test]
fn the_keyboard_report_holds_the_same_keys_where_the_old_one_was_right() {
    use pad_core::route::Keyboard;
    let mut rng = Rng(0x5EED_0000_0000_0006);
    for _ in 0..2_000 {
        let mut old = old::keyboard::Keyboard { held: [(0, 0); 6] };
        let mut new = Keyboard::new();
        // Each matrix key's usage (distinct, non-zero) and modifiers, and whether it is down.
        let mut codes = [0u8; 16];
        for (i, c) in codes.iter_mut().enumerate() {
            *c = 4 + i as u8 * 8 + rng.below(8) as u8;
        }
        let mods: [u8; 16] = core::array::from_fn(|_| if rng.chance(3) { rng.byte() } else { 0 });
        let mut down = [false; 16];
        for _ in 0..200 {
            let key = rng.below(16) as usize;
            if down[key] {
                down[key] = false;
                old.release(codes[key]);
                new.release(key as u8);
            } else if down.iter().filter(|&&d| d).count() < 6 {
                down[key] = true;
                old.press(codes[key], mods[key]);
                new.press(key as u8, codes[key], mods[key]);
            }
            // A tapped usage that a held key has already was in the old report twice.
            let extra = if rng.chance(4) && !codes.contains(&0x4B) { 0x4B } else { 0 };
            let (o, n) = (old.report(extra), new.report(extra));
            // The same modifiers and the same usages; the old code reused a freed slot, so the
            // order of the usages can differ, which the PC ignores.
            assert_eq!((o[0], o[1]), (n[0], n[1]));
            let (mut o, mut n) = (o[2..].to_vec(), n[2..].to_vec());
            o.sort();
            n.sort();
            assert_eq!(o, n);
        }
    }
}
