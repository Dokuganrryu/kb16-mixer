//! The keys this app handles (Talk, Move, Page, Focus, Mute, Open), one event at a time in the
//! order the pad sent them. Several events can arrive at once; a release that came before a
//! press must also be handled before it, or a quick release-and-press of Talk leaves the mic
//! muted while Talk is held.
//!
//! A mute, open or empty key can also have a hold and a double tap (see `gesture.rs`); a key
//! without them acts on its press at once, as it always did.

use crate::config::{Config, KeyAction, PressAction};
use crate::gesture::{Button, Gesture};
use std::time::{Duration, Instant};

/// A Focus or Page key held this long is momentary: it ends on release instead of latching.
pub const MOMENTARY: Duration = Duration::from_millis(350);

/// What the keys act on: the mixer, and what the pad's screen says about it.
pub trait Hands {
    fn focus(&self) -> Option<usize>;
    /// `announce`: say so on the pad's screen.
    fn set_focus(&mut self, focus: Option<usize>, announce: bool);
    fn mic_muted(&self) -> bool;
    fn set_mic_muted(&mut self, muted: bool, announce: bool);
    /// Mute or unmute the channel of this column.
    fn mute(&mut self, col: usize);
    /// Move was held and a key in this column pressed: move the app in front there.
    fn move_to(&mut self, col: usize);
    /// Move was pressed: show which channel the app in front plays in.
    fn show_where(&mut self);
    fn open(&mut self, target: &str);
}

#[derive(Default)]
pub struct Keys {
    /// Key page in use (0 = [keys]).
    page: usize,
    /// The page key that is down: since when, and the page to go back to if it turns out to be
    /// held rather than tapped.
    page_key: Option<(Instant, usize)>,
    /// The Focus key that is down: its column, since when, and whether it was already latched.
    focus_key: Option<(usize, Instant, bool)>,
    /// The Talk key that is down: the mic mute state to go back to on release.
    talk_restore: Option<bool>,
    move_armed: bool,
    /// What each key meant when it went down, so its release does the matching thing even
    /// after the page changed underneath it.
    held: [[Option<KeyAction>; 4]; 4],
    /// Keys with a hold or a double tap, and the page each was pressed on.
    buttons: [[(Button, usize); 4]; 4],
}

impl Keys {
    pub fn page(&self) -> usize {
        self.page
    }

    pub fn press(&mut self, row: usize, col: usize, now: Instant, cfg: &Config, hands: &mut impl Hands) {
        if row >= 4 || col >= 4 {
            return;
        }
        let n = cfg.channels.len();
        let action = cfg.pages[self.page][row][col].clone();
        self.held[row][col] = Some(action.clone());
        // Move is held like a shift key: the next key picks the column.
        if self.move_armed && action != KeyAction::Move && col < n {
            return hands.move_to(col);
        }
        let wants = cfg.key_wants(self.page, row, col);
        if wants != Default::default() && crate::config::takes_gestures(&action) {
            let page = self.page;
            self.buttons[row][col].1 = page;
            let mut fired = Vec::new();
            self.buttons[row][col].0.press(now, wants, &mut |g| fired.push(g));
            return fired.into_iter().for_each(|g| act(g, page, row, col, cfg, hands));
        }
        match action {
            KeyAction::Mute if col < n => hands.mute(col),
            _ if is_talk(&action, col, cfg) => {
                if self.talk_restore.is_none() {
                    let muted = hands.mic_muted();
                    self.talk_restore = Some(muted);
                    hands.set_mic_muted(!muted, true);
                }
            }
            KeyAction::Focus if col < n => {
                let latched = hands.focus() == Some(col);
                hands.set_focus(Some(col), !latched);
                self.focus_key = Some((col, now, latched));
            }
            KeyAction::Page => {
                self.page_key = Some((now, self.page));
                self.page = 1 - self.page;
            }
            KeyAction::Open(target) => hands.open(&target),
            KeyAction::Move => {
                self.move_armed = true;
                hands.show_where();
            }
            _ => {}
        }
    }

    pub fn release(&mut self, row: usize, col: usize, now: Instant, cfg: &Config, hands: &mut impl Hands) {
        if row >= 4 || col >= 4 {
            return;
        }
        let Some(action) = self.held[row][col].take() else { return };
        let (button, page) = &mut self.buttons[row][col];
        if button.is_down() {
            let (page, wants) = (*page, cfg.key_wants(*page, row, col));
            let mut fired = Vec::new();
            button.release(now, wants, &mut |g| fired.push(g));
            return fired.into_iter().for_each(|g| act(g, page, row, col, cfg, hands));
        }
        match action {
            KeyAction::Move => self.move_armed = false,
            KeyAction::Page => {
                // Same rule as Focus: a tap switches and stays, a hold lasts while held.
                if let Some((since, back)) = self.page_key.take()
                    && now.saturating_duration_since(since) >= MOMENTARY
                {
                    self.page = back;
                }
            }
            _ if is_talk(&action, col, cfg) => {
                if let Some(muted) = self.talk_restore.take() {
                    hands.set_mic_muted(muted, true);
                }
            }
            KeyAction::Focus => {
                if let Some((_, since, latched)) = self.focus_key.filter(|(c, ..)| *c == col) {
                    self.focus_key = None;
                    // A tap latches (or unlatches); a hold lasts only while held.
                    if latched || now.saturating_duration_since(since) >= MOMENTARY {
                        hands.set_focus(None, true);
                    }
                }
            }
            _ => {}
        }
    }

    /// Fires the holds and the waiting taps that are due by `now`.
    pub fn poll(&mut self, now: Instant, cfg: &Config, hands: &mut impl Hands) {
        for row in 0..4 {
            for col in 0..4 {
                let (button, page) = &mut self.buttons[row][col];
                let page = *page;
                let mut fired = Vec::new();
                button.poll(now, cfg.key_wants(page, row, col), &mut |g| fired.push(g));
                fired.into_iter().for_each(|g| act(g, page, row, col, cfg, hands));
            }
        }
    }

    /// When `poll` has something to fire next.
    pub fn deadline(&self, cfg: &Config) -> Option<Instant> {
        let keys =
            self.buttons.iter().enumerate().flat_map(|(r, row)| row.iter().enumerate().map(move |(c, b)| (r, c, b)));
        keys.filter_map(|(r, c, (button, page))| button.deadline(cfg.key_wants(*page, r, c))).min()
    }

    /// Undoes everything temporary (the pad went away, a pause, the app closing): Focus off,
    /// the mic as it was before Talk, page 1, nothing held.
    pub fn let_go(&mut self, hands: &mut impl Hands) {
        hands.set_focus(None, false);
        if let Some(muted) = self.talk_restore {
            hands.set_mic_muted(muted, false);
        }
        *self = Keys::default();
    }

    /// A new config: the pad starts again on page 1, and no Focus key counts as down.
    pub fn reload(&mut self) {
        self.page = 0;
        self.page_key = None;
        self.focus_key = None;
        self.buttons = Default::default();
    }
}

/// What a key's tap, hold or double tap does, by the config of the page it was pressed on.
fn act(gesture: Gesture, page: usize, row: usize, col: usize, cfg: &Config, hands: &mut impl Hands) {
    let [holds, doubles] = &cfg.key_gestures[page];
    let action = match gesture {
        Gesture::Tap => match &cfg.pages[page][row][col] {
            KeyAction::Mute => PressAction::Mute,
            KeyAction::Open(target) => PressAction::Open(target.clone()),
            _ => PressAction::Nothing,
        },
        Gesture::Hold => holds[row][col].clone(),
        Gesture::Double => doubles[row][col].clone(),
    };
    match action {
        PressAction::Mute if col < cfg.channels.len() => hands.mute(col),
        PressAction::Open(target) => hands.open(&target),
        _ => {}
    }
}

/// Talk, or a Focus key sitting in the microphone's column.
fn is_talk(action: &KeyAction, col: usize, cfg: &Config) -> bool {
    *action == KeyAction::Talk || (*action == KeyAction::Focus && crate::audio::mic_channel(&cfg.channels) == Some(col))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[derive(Default)]
    struct Fake {
        focus: Option<usize>,
        mic_muted: bool,
        did: Vec<String>,
    }

    impl Hands for Fake {
        fn focus(&self) -> Option<usize> {
            self.focus
        }
        fn set_focus(&mut self, focus: Option<usize>, _: bool) {
            self.focus = focus;
        }
        fn mic_muted(&self) -> bool {
            self.mic_muted
        }
        fn set_mic_muted(&mut self, muted: bool, _: bool) {
            self.mic_muted = muted;
        }
        fn mute(&mut self, col: usize) {
            self.did.push(format!("mute {col}"));
        }
        fn move_to(&mut self, col: usize) {
            self.did.push(format!("move to {col}"));
        }
        fn show_where(&mut self) {
            self.did.push("show where".into());
        }
        fn open(&mut self, target: &str) {
            self.did.push(format!("open {target}"));
        }
    }

    enum Ev {
        Down(usize, usize),
        Up(usize, usize),
    }
    use Ev::*;

    /// Plays a script of (ms since start, event), as one batch would arrive.
    fn play(keys: &mut Keys, cfg: &Config, hands: &mut Fake, script: &[(u64, Ev)]) {
        let t0 = Instant::now();
        for (ms, ev) in script {
            let now = t0 + Duration::from_millis(*ms);
            match *ev {
                Down(r, c) => keys.press(r, c, now, cfg, hands),
                Up(r, c) => keys.release(r, c, now, cfg, hands),
            }
        }
    }

    // On the default page 1: page (0,0), move (1,0), talk (2,0), focus Chat (2,1), mute (3,c).
    const PAGE: (usize, usize) = (0, 0);
    const MOVE: (usize, usize) = (1, 0);
    const TALK: (usize, usize) = (2, 0);
    const FOCUS_CHAT: (usize, usize) = (2, 1);

    #[test]
    fn talk_let_go_and_pressed_again_at_once_keeps_the_mic_live() {
        let cfg = config::defaults();
        let (mut keys, mut pad) = (Keys::default(), Fake { mic_muted: true, ..Fake::default() });
        play(&mut keys, &cfg, &mut pad, &[(0, Down(TALK.0, TALK.1))]);
        assert!(!pad.mic_muted, "push to talk");
        play(&mut keys, &cfg, &mut pad, &[(0, Up(TALK.0, TALK.1)), (0, Down(TALK.0, TALK.1))]);
        assert!(!pad.mic_muted, "held again: still live");
        play(&mut keys, &cfg, &mut pad, &[(0, Up(TALK.0, TALK.1))]);
        assert!(pad.mic_muted, "let go: muted again");
    }

    #[test]
    fn a_key_after_move_was_let_go_does_its_own_thing() {
        let cfg = config::defaults();
        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play(&mut keys, &cfg, &mut pad, &[(0, Down(MOVE.0, MOVE.1))]);
        play(&mut keys, &cfg, &mut pad, &[(50, Up(MOVE.0, MOVE.1)), (60, Down(3, 2))]);
        assert_eq!(pad.did, ["show where", "mute 2"], "the mute key mutes, the app stays where it is");
        play(&mut keys, &cfg, &mut pad, &[(0, Down(MOVE.0, MOVE.1)), (10, Down(3, 2))]);
        assert_eq!(pad.did[2..], ["show where".to_string(), "move to 2".to_string()], "while held it moves");
    }

    #[test]
    fn a_key_after_a_held_page_key_was_let_go_comes_from_page_one() {
        let mut cfg = config::defaults();
        cfg.pages[1][3][1] = KeyAction::Open("page-two".into());
        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play(&mut keys, &cfg, &mut pad, &[(0, Down(PAGE.0, PAGE.1))]);
        assert_eq!(keys.page(), 1);
        play(&mut keys, &cfg, &mut pad, &[(400, Up(PAGE.0, PAGE.1)), (410, Down(3, 1))]);
        assert_eq!(keys.page(), 0, "a hold lasts while held");
        assert_eq!(pad.did, ["mute 1"], "page 1's key, not page 2's");
        // A tap switches and stays.
        play(&mut keys, &cfg, &mut pad, &[(0, Down(PAGE.0, PAGE.1)), (100, Up(PAGE.0, PAGE.1)), (110, Down(3, 1))]);
        assert_eq!((keys.page(), pad.did.last().unwrap().as_str()), (1, "open page-two"));
    }

    #[test]
    fn focus_latches_on_a_tap_and_lasts_while_held_otherwise() {
        let cfg = config::defaults();
        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        let (r, c) = FOCUS_CHAT;
        play(&mut keys, &cfg, &mut pad, &[(0, Down(r, c)), (100, Up(r, c))]);
        assert_eq!(pad.focus, Some(1), "tapped: latched");
        play(&mut keys, &cfg, &mut pad, &[(0, Down(r, c)), (100, Up(r, c))]);
        assert_eq!(pad.focus, None, "tapped again: off");
        play(&mut keys, &cfg, &mut pad, &[(0, Down(r, c)), (500, Up(r, c))]);
        assert_eq!(pad.focus, None, "held: only while held");
    }

    /// The default config with a hold (open) and a double tap (open) on the Chat mute key, and
    /// the Media mute key turned into an empty key with only a hold (mute).
    fn with_gestures() -> Config {
        let mut cfg = config::defaults();
        cfg.key_gestures[0][0][3][1] = PressAction::Open("held".into());
        cfg.key_gestures[0][1][3][1] = PressAction::Open("twice".into());
        cfg.pages[0][3][3] = KeyAction::Nothing;
        cfg.key_gestures[0][0][3][3] = PressAction::Mute;
        cfg
    }

    /// Plays a script, polling at each step the way the mixer loop wakes at a deadline.
    fn play_polled(keys: &mut Keys, cfg: &Config, hands: &mut Fake, t0: Instant, script: &[(u64, Option<Ev>)]) {
        for (ms, ev) in script {
            let now = t0 + Duration::from_millis(*ms);
            match ev {
                Some(Down(r, c)) => keys.press(*r, *c, now, cfg, hands),
                Some(Up(r, c)) => keys.release(*r, *c, now, cfg, hands),
                None => keys.poll(now, cfg, hands),
            }
        }
    }

    #[test]
    fn a_key_without_a_hold_or_double_tap_still_acts_on_the_press() {
        let (cfg, t0) = (with_gestures(), Instant::now());
        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(0, Some(Down(3, 2)))]);
        assert_eq!(pad.did, ["mute 2"], "at once, no waiting");
        assert_eq!(keys.deadline(&cfg), None);
    }

    #[test]
    fn a_key_with_a_double_tap_waits_for_it_and_a_hold_fires_while_held() {
        let (cfg, t0) = (with_gestures(), Instant::now());
        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(0, Some(Down(3, 1))), (80, Some(Up(3, 1))), (300, None)]);
        assert!(pad.did.is_empty(), "the tap waits 280 ms for a second one");
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(360, None)]);
        assert_eq!(pad.did, ["mute 1"], "then it mutes");

        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play_polled(
            &mut keys,
            &cfg,
            &mut pad,
            t0,
            &[(0, Some(Down(3, 1))), (80, Some(Up(3, 1))), (150, Some(Down(3, 1)))],
        );
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(200, Some(Up(3, 1))), (1000, None)]);
        assert_eq!(pad.did, ["open twice"], "a double tap and no mute");

        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(0, Some(Down(3, 1))), (349, None)]);
        assert!(pad.did.is_empty());
        assert!(keys.deadline(&cfg).is_some(), "the loop must wake for the hold");
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(350, None)]);
        assert_eq!(pad.did, ["open held"], "fires at 350 ms, before the release");
    }

    #[test]
    fn an_empty_key_with_a_hold_does_nothing_on_a_tap() {
        let (cfg, t0) = (with_gestures(), Instant::now());
        let (mut keys, mut pad) = (Keys::default(), Fake::default());
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(0, Some(Down(3, 3))), (100, Some(Up(3, 3)))]);
        assert!(pad.did.is_empty());
        play_polled(&mut keys, &cfg, &mut pad, t0, &[(1000, Some(Down(3, 3))), (1400, None), (1500, Some(Up(3, 3)))]);
        assert_eq!(pad.did, ["mute 3"]);
    }

    #[test]
    fn letting_go_leaves_page_one_with_nothing_held() {
        let cfg = config::defaults();
        let (mut keys, mut pad) = (Keys::default(), Fake { mic_muted: true, ..Fake::default() });
        let script = [(0, Down(TALK.0, TALK.1)), (0, Down(FOCUS_CHAT.0, FOCUS_CHAT.1)), (0, Down(PAGE.0, PAGE.1))];
        play(&mut keys, &cfg, &mut pad, &script);
        assert_eq!((pad.mic_muted, pad.focus, keys.page()), (false, Some(1), 1));
        keys.let_go(&mut pad);
        assert_eq!((pad.mic_muted, pad.focus, keys.page()), (true, None, 0));
        // The releases that come after (the keys were still down) change nothing.
        let script = [(900, Up(TALK.0, TALK.1)), (900, Up(FOCUS_CHAT.0, FOCUS_CHAT.1)), (900, Up(PAGE.0, PAGE.1))];
        play(&mut keys, &cfg, &mut pad, &script);
        assert_eq!((pad.mic_muted, pad.focus, keys.page()), (true, None, 0));
    }
}
