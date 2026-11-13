//! The keyboard report and the routing of key presses and knob turns (`pad_core::route`).

use kb16_mixer_protocol::{Key, PadMsg};
use pad_core::route::{Action, Event, HOLD_MS, KEYMAP, Keyboard, Router};

const CTRL: u8 = 0x01;
const SHIFT: u8 = 0x02;
const ALT: u8 = 0x04;
const A: u8 = 0x04;
const C: u8 = 0x06;

fn report(mods: u8, codes: &[u8]) -> [u8; 8] {
    let mut r = [0; 8];
    r[0] = mods;
    r[2..2 + codes.len()].copy_from_slice(codes);
    r
}

// --- The keyboard report ------------------------------------------------------------------------

#[test]
fn holding_ctrl_c_and_tapping_c_keeps_ctrl_c_held() {
    let mut k = Keyboard::new();
    k.press(0, C, CTRL); // key A: ctrl+c
    k.press(1, C, 0); // key B: c
    assert_eq!(k.report(0), report(CTRL, &[C]));
    k.release(1);
    assert_eq!(k.report(0), report(CTRL, &[C]));
    k.release(0);
    assert_eq!(k.report(0), report(0, &[]));
}

#[test]
fn releasing_the_plain_key_first_or_last_frees_only_what_it_added() {
    // Hold c, then ctrl+c; let go of c first: ctrl+c stays.
    let mut k = Keyboard::new();
    k.press(1, C, 0);
    k.press(0, C, CTRL);
    k.release(1);
    assert_eq!(k.report(0), report(CTRL, &[C]));
    // Let go of ctrl+c while c is held: c stays, Ctrl goes.
    let mut k = Keyboard::new();
    k.press(1, C, 0);
    k.press(0, C, CTRL);
    k.release(0);
    assert_eq!(k.report(0), report(0, &[C]));
}

#[test]
fn a_modifier_stays_while_any_key_that_holds_it_is_down() {
    let mut k = Keyboard::new();
    k.press(0, A, CTRL);
    k.press(1, C, CTRL | SHIFT);
    assert_eq!(k.report(0), report(CTRL | SHIFT, &[A, C]));
    k.release(1);
    assert_eq!(k.report(0), report(CTRL, &[A]));
    k.press(1, C, CTRL | SHIFT);
    k.release(0);
    assert_eq!(k.report(0), report(CTRL | SHIFT, &[C]));
}

#[test]
fn a_modifier_only_key_holds_its_modifiers() {
    let mut k = Keyboard::new();
    k.press(5, 0, ALT);
    assert_eq!(k.report(0), report(ALT, &[]));
    k.press(6, A, 0);
    assert_eq!(k.report(0), report(ALT, &[A]));
    k.release(5);
    assert_eq!(k.report(0), report(0, &[A]));
}

#[test]
fn the_same_usage_on_two_keys_is_in_the_report_once_until_both_are_up() {
    let mut k = Keyboard::new();
    k.press(2, A, 0);
    k.press(3, A, 0);
    assert_eq!(k.report(0), report(0, &[A]));
    k.release(2);
    assert_eq!(k.report(0), report(0, &[A]));
    k.release(3);
    assert_eq!(k.report(0), report(0, &[]));
}

#[test]
fn six_usages_fit_and_a_seventh_waits_without_its_modifiers() {
    let mut k = Keyboard::new();
    for key in 0..6 {
        k.press(key, 0x1E + key, 0);
    }
    assert_eq!(k.report(0), report(0, &[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23]));
    k.press(6, C, CTRL);
    // Ctrl with the other six would be a shortcut that nobody pressed.
    assert_eq!(k.report(0), report(0, &[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23]));
    // A usage that is in the report already still fits.
    k.press(7, 0x20, SHIFT);
    assert_eq!(k.report(0), report(SHIFT, &[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23]));
    k.release(7);
    k.release(2);
    assert_eq!(k.report(0), report(CTRL, &[0x1E, 0x1F, 0x21, 0x22, 0x23, C]));
    // And a tapped key has no room either.
    assert_eq!(k.report(0x4B), k.report(0));
}

#[test]
fn keys_go_up_in_any_order() {
    let codes = [0x1E, 0x1F, 0x20, 0x21];
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]] {
        let mut k = Keyboard::new();
        for (key, &code) in codes.iter().enumerate() {
            k.press(key as u8, code, 1 << key);
        }
        let mut left: Vec<usize> = (0..4).collect();
        for &up in &order {
            k.release(up as u8);
            left.retain(|&key| key != up);
            let codes: Vec<u8> = left.iter().map(|&key| codes[key]).collect();
            let mods = left.iter().fold(0, |m, &key| m | 1 << key);
            assert_eq!(k.report(0), report(mods, &codes), "order {order:?}, after {up}");
        }
    }
}

#[test]
fn a_tapped_key_comes_on_top_of_the_held_keys() {
    let mut k = Keyboard::new();
    k.press(0, A, CTRL);
    assert_eq!(k.report(0x4B), report(CTRL, &[A, 0x4B]));
    assert_eq!(k.report(A), report(CTRL, &[A]));
    assert_eq!(k.report(0), report(CTRL, &[A]));
}

#[test]
fn a_release_without_a_press_and_clear_change_nothing_else() {
    let mut k = Keyboard::new();
    k.release(4);
    assert_eq!(k.report(0), report(0, &[]));
    k.press(0, A, CTRL);
    k.release(4);
    assert_eq!(k.report(0), report(CTRL, &[A]));
    k.clear();
    assert_eq!(k.report(0), report(0, &[]));
    k.release(0);
    assert_eq!(k.report(0), report(0, &[]));
}

// --- Routing ------------------------------------------------------------------------------------

fn key(row: u8, col: u8, pressed: bool) -> Event {
    Event::Key { row, col, pressed }
}

const HOST: [Key; 16] = [Key::Host; 16];

#[test]
fn with_the_app_knobs_and_host_keys_go_to_the_app() {
    let mut r = Router::<u64>::new();
    assert_eq!(
        r.route(Event::Turn { knob: 2, cw: true }, true, true, &HOST, 0),
        Action::Host(PadMsg::Turn { knob: 2, delta: 1 })
    );
    assert_eq!(
        r.route(Event::Turn { knob: 0, cw: false }, true, true, &HOST, 0),
        Action::Host(PadMsg::Turn { knob: 0, delta: -1 })
    );
    assert_eq!(r.route(key(1, 4, true), true, true, &HOST, 0), Action::Host(PadMsg::Press { knob: 1 }));
    assert_eq!(r.route(key(1, 4, false), true, true, &HOST, 0), Action::Nothing);
    assert_eq!(r.route(key(2, 3, true), true, true, &HOST, 0), Action::HostDown { row: 2, col: 3 });
    assert_eq!(r.route(key(2, 3, false), true, true, &HOST, 0), Action::HostUp { row: 2, col: 3 });
}

#[test]
fn without_the_app_the_pad_is_a_keypad_with_media_knobs() {
    let mut r = Router::<u64>::new();
    let turns = [
        (0, true, Action::TapMedia(0xB5)),
        (0, false, Action::TapMedia(0xB6)),
        (1, true, Action::TapKey(0x4B)),
        (1, false, Action::TapKey(0x4E)),
        (2, true, Action::TapMedia(0xE9)),
        (2, false, Action::TapMedia(0xEA)),
    ];
    for (knob, cw, want) in turns {
        assert_eq!(r.route(Event::Turn { knob, cw }, false, true, &HOST, 0), want);
    }
    let presses = [Action::TapMedia(0xCD), Action::TapKey(0x4A), Action::TapMedia(0xE2)];
    for (row, want) in presses.into_iter().enumerate() {
        assert_eq!(r.route(key(row as u8, 4, true), false, true, &HOST, 0), want);
    }
    // Every matrix key types the fallback keymap.
    for i in 0..16u8 {
        let (row, col) = (i / 4, i % 4);
        assert_eq!(r.route(key(row, col, true), false, true, &HOST, 0), Action::Keyboard);
        assert_eq!(r.keyboard.report(0), report(0, &[KEYMAP[row as usize][col as usize]]));
        assert_eq!(r.route(key(row, col, false), false, true, &HOST, 0), Action::Keyboard);
        assert_eq!(r.keyboard.report(0), report(0, &[]));
    }
}

#[test]
fn the_app_can_leave_the_keys_to_the_keypad() {
    let mut r = Router::<u64>::new();
    assert_eq!(r.route(key(0, 0, true), true, false, &HOST, 0), Action::Keyboard);
    assert_eq!(r.keyboard.report(0), report(0, &[0x1E]));
}

#[test]
fn a_release_does_what_the_press_did_whatever_changed_since() {
    let mut map = HOST;
    map[5] = Key::Keyboard { code: C, mods: CTRL };
    let mut r = Router::<u64>::new();
    assert_eq!(r.route(key(1, 1, true), true, true, &map, 0), Action::Keyboard);
    // The app goes away while the key is held: the release still lets go of ctrl+c.
    assert_eq!(r.route(key(1, 1, false), false, true, &HOST, 10), Action::Keyboard);
    assert_eq!(r.keyboard.report(0), report(0, &[]));
    // A host key that the app took is released to the app, even with the app gone.
    assert_eq!(r.route(key(0, 0, true), true, true, &HOST, 20), Action::HostDown { row: 0, col: 0 });
    assert_eq!(r.route(key(0, 0, false), false, true, &HOST, 30), Action::HostUp { row: 0, col: 0 });
    // A second release is nothing.
    assert_eq!(r.route(key(0, 0, false), false, true, &HOST, 40), Action::Nothing);
}

#[test]
fn the_ctrl_c_and_c_keys_route_to_the_right_report() {
    let mut map = HOST;
    map[0] = Key::Keyboard { code: C, mods: CTRL };
    map[1] = Key::Keyboard { code: C, mods: 0 };
    let mut r = Router::<u64>::new();
    r.route(key(0, 0, true), true, true, &map, 0);
    r.route(key(0, 1, true), true, true, &map, 1);
    r.route(key(0, 1, false), true, true, &map, 2);
    assert_eq!(r.keyboard.report(0), report(CTRL, &[C]));
}

#[test]
fn a_media_key_with_a_hold_function_sends_on_release() {
    let mut map = HOST;
    map[3] = Key::Media { tap: 0xB5, hold: 0xB6 };
    map[4] = Key::Media { tap: 0xCD, hold: 0 };
    let mut r = Router::<u64>::new();
    assert_eq!(r.route(key(0, 3, true), true, true, &map, 1000), Action::Nothing);
    assert_eq!(r.route(key(0, 3, false), true, true, &map, 1000 + HOLD_MS - 1), Action::TapMedia(0xB5));
    r.route(key(0, 3, true), true, true, &map, 5000);
    assert_eq!(r.route(key(0, 3, false), true, true, &map, 5000 + HOLD_MS), Action::TapMedia(0xB6));
    // Without a hold function it sends at once, and the release is nothing.
    assert_eq!(r.route(key(1, 0, true), true, true, &map, 0), Action::TapMedia(0xCD));
    assert_eq!(r.route(key(1, 0, false), true, true, &map, 9000), Action::Nothing);
    assert_eq!(HOLD_MS, 400);
}

#[test]
fn a_nothing_key_and_a_key_outside_the_matrix_do_nothing() {
    let mut map = HOST;
    map[0] = Key::Nothing;
    let mut r = Router::<u64>::new();
    assert_eq!(r.route(key(0, 0, true), true, true, &map, 0), Action::Nothing);
    assert_eq!(r.route(key(0, 0, false), true, true, &map, 0), Action::Nothing);
    for (row, col) in [(4, 0), (0, 5), (255, 255)] {
        assert_eq!(r.route(key(row, col, true), false, true, &HOST, 0), Action::Nothing);
        assert_eq!(r.route(key(row, col, false), false, true, &HOST, 0), Action::Nothing);
    }
    assert_eq!(r.keyboard.report(0), report(0, &[]));
}
