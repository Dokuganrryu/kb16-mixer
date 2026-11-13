//! Where every control of the window goes (the numbers of the design, in pixels at 100 %), and
//! which of them the open tab shows. The cards and frames around them are painted by `scene`.

use super::checks::CHANNEL_NAME_LEN;
use super::choices::{BRIGHTNESS_PERCENT, KNOB_COLUMNS, KNOB_NAMES, ROW_NAMES};
use super::controls::*;
use super::ids::*;
use super::{HIDDEN, STATUS_IDLE, TAB, TABS, Tab};
use crate::ui::canvas::R;
use crate::ui::theme::Tok;
use crate::ui::{Align, Font, Label};
use crate::win::send;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::WS_TABSTOP;
use windows::core::w;

/// Client area at 96 dpi.
pub const WIDTH: i32 = 712;
pub const HEIGHT: i32 = 424;
/// The page under the header: tab contents are laid out from here down.
const PAGE: i32 = 48;
/// The footer starts here, with a line.
pub const FOOTER: i32 = 368;

pub const TAB_TRACK: R = R::new(16, 8, 270, 32);
pub const TAB_ITEM: (i32, i32) = (88, 26);
/// The page switch of the Keys tab sits in the header beside the tab strip, which leaves the
/// page room for the key grid and the knob grid.
pub const PAGE_TRACK: R = R::new(TAB_TRACK.x + TAB_TRACK.w + 8, 10, 134, 28);
pub const PAGE_ITEM: (i32, i32) = (64, 22);
pub const KEY_CARD: R = R::new(16, PAGE + 8, 680, 160);
const KEY_COL: i32 = 112;
const KEY_PITCH: i32 = 140;
/// The knob grid under the key grid: a title row, then one row per knob, in the same columns.
const KNOB_GRID: i32 = PAGE + 200;

/// Channel card `i`.
pub fn channel_card(i: i32) -> R {
    R::new(16 + i * 172, PAGE + 8, 164, 264)
}

/// The two cards at the left hold seven rows between them, so their bottom margins are 4 px.
pub const SCREEN_CARD: R = R::new(16, PAGE + 8, 376, 164);
pub const SOUND_CARD: R = R::new(16, PAGE + 180, 376, 132);
pub const LIGHTS_CARD: R = R::new(400, PAGE + 8, 296, 156);
pub const SYSTEM_CARD: R = R::new(400, PAGE + 172, 296, 140);
pub const CHIP: (i32, i32, i32) = (528, 384, 24);

const fn look(font: Font, color: Tok, align: Align, wrap: bool) -> Label {
    Label { font, color, align, wrap, quiet: None }
}

const HINT: Label = look(Font::Small, Tok::Text2, Align::Left, true);
const ROW: Label = look(Font::Body, Tok::Text2, Align::Left, false);
const FORM: Label = look(Font::Body, Tok::Text, Align::Left, false);
const TITLE: Label = look(Font::SmallStrong, Tok::Text, Align::Left, false);
const COLUMN: Label = Label { quiet: Some("(no channel)"), ..look(Font::SmallStrong, Tok::Text, Align::Center, false) };
const VALUE: Label = look(Font::Body, Tok::Text, Align::Right, false);

fn text(hwnd: HWND, id: i32, s: &str, r: R, l: Label) {
    label(hwnd, id, s, at(r.x, r.y, r.w, r.h), l);
}

/// A card's title and its rows: a label at the left, then what `control` makes at `x`.
fn title(hwnd: HWND, card: R, name: &str) {
    text(hwnd, -1, name, R::new(card.x + 12, card.y + 12, 200, 16), TITLE);
}

fn row(hwnd: HWND, card: R, y: i32, name: &str, width: i32) {
    text(hwnd, -1, name, R::new(card.x + 12, card.y + y, width, 28), FORM);
}

fn hint(hwnd: HWND, card: R, y: i32, s: &str) {
    text(hwnd, -1, s, R::new(card.x + 12, card.y + y, card.w - 24, 16), HINT);
}

pub fn build(hwnd: HWND) {
    let on = |tab: Tab| BUILDING.set(TABS.iter().position(|t| t.0 == tab));

    // ---- The tab strip: real radio buttons, so the arrows move between them.
    BUILDING.set(None);
    for (i, (_, name)) in TABS.iter().enumerate() {
        let x = TAB_TRACK.x + 3 + i as i32 * TAB_ITEM.0;
        segment(hwnd, name, at(x, TAB_TRACK.y + 3, TAB_ITEM.0, TAB_ITEM.1), ID_TAB + i as i32, i == 0);
    }
    set_checked(ID_TAB + TAB.get() as i32, true);

    // ---- Keys
    on(Tab::Keys);
    for p in 0..2 {
        let r = at(PAGE_TRACK.x + 3 + p * PAGE_ITEM.0, PAGE_TRACK.y + 3, PAGE_ITEM.0, PAGE_ITEM.1);
        segment(hwnd, &format!("Page {}", p + 1), r, ID_PAGE + p, p == 0);
    }
    let k = KEY_CARD;
    for c in 0..4 {
        text(hwnd, ID_COLUMN + c, "", R::new(k.x + KEY_COL + c * KEY_PITCH, k.y + 8, 132, 16), COLUMN);
    }
    for r in 0..4 {
        text(hwnd, -1, ROW_NAMES[r as usize], R::new(k.x + 12, k.y + 28 + r * 32, 96, 28), ROW);
        for c in 0..4 {
            combo(hwnd, ID_KEY + r * 4 + c, k.x + KEY_COL + c * KEY_PITCH, k.y + 28 + r * 32, 132);
        }
    }
    let typing =
        "The pad starts on page 1. Keys that type (media keys, F13 to F24, shortcuts) are typed by the pad itself.";
    text(hwnd, -1, typing, R::new(16, PAGE + 176, 680, 16), HINT);
    for (c, head) in KNOB_COLUMNS.iter().enumerate() {
        text(hwnd, -1, head, R::new(k.x + KEY_COL + c as i32 * KEY_PITCH, KNOB_GRID, 132, 16), TITLE);
    }
    for i in 0..3 {
        let y = KNOB_GRID + 20 + i * 32;
        text(hwnd, -1, KNOB_NAMES[i as usize], R::new(k.x + 12, y, 96, 28), ROW);
        combo(hwnd, ID_KNOB + i, k.x + KEY_COL, y, 132);
        for p in 0..3 {
            combo(hwnd, ID_PRESS + i * 3 + p, k.x + KEY_COL + (p + 1) * KEY_PITCH, y, 132);
        }
    }

    // ---- Channels
    on(Tab::Channels);
    for i in 0..4 {
        let card = channel_card(i);
        let (x, y) = (card.x + 8, card.y);
        let frame = at(x, y + 12, 148, 28);
        text(hwnd, ID_CH_LABEL + i * 2, "Name", R::new(x + 10, y + 12, 40, 28), ROW);
        let name = edit(hwnd, ID_CH_NAME + i, frame, 52, 0);
        send(name, EM_LIMITTEXT, CHANNEL_NAME_LEN, 0);
        text(hwnd, ID_CH_LABEL + i * 2 + 1, "Plays", R::new(x, y + 48, 148, 16), TITLE);
        combo(hwnd, ID_CH_KIND + i, x, y + 68, 148);
        text(hwnd, ID_CH_ABOUT + i, "", R::new(x, y + 104, 148, 32), HINT);
        list(hwnd, ID_CH_LIST + i, at(x, y + 140, 148, 76));
        button(hwnd, "Add...", x, y + 224, 72, ID_CH_ADD + i);
        button(hwnd, "Remove", x + 76, y + 224, 72, ID_CH_REMOVE + i);
    }
    let sliders = "Windows has one volume slider per program. All tabs of one browser move together.";
    text(hwnd, -1, sliders, R::new(16, PAGE + 280, 680, 16), HINT);
    let moved = "(moved) = put there with the pad's Move key, which wins over the lists. Remove undoes it.";
    text(hwnd, -1, moved, R::new(16, PAGE + 296, 680, 16), HINT);

    // ---- Pad
    on(Tab::Pad);
    let counted = "Counted while nothing plays or is touched.";
    let s = SCREEN_CARD;
    title(hwnd, s, "Screen");
    row(hwnd, s, 36, "Name on the screen", 184);
    let name = edit(hwnd, ID_NAME, at(s.x + 204, s.y + 36, 160, 28), 10, ES_UPPERCASE);
    send(name, EM_LIMITTEXT, kb16_mixer_protocol::LOGO_NAME_LEN, 0);
    row(hwnd, s, 68, "Screen style", 184);
    combo(hwnd, ID_SCREEN_STYLE, s.x + 204, s.y + 68, 160);
    row(hwnd, s, 100, "Screen shift", 184);
    combo(hwnd, ID_SHIFT, s.x + 204, s.y + 100, 160);
    // The card has no room left for the idle hint; the Lights card's hint says it.
    row(hwnd, s, 132, "Screen off after", 184);
    combo(hwnd, ID_SCREEN_OFF, s.x + 204, s.y + 132, 160);

    let k = SOUND_CARD;
    title(hwnd, k, "Sound");
    row(hwnd, k, 36, "Knob step", 184);
    combo(hwnd, ID_STEP, k.x + 204, k.y + 36, 160);
    row(hwnd, k, 68, "Focus turns the others down to", 184);
    combo(hwnd, ID_DUCK, k.x + 204, k.y + 68, 160);
    // Device names are long, and the row's label is short: the box takes the room.
    row(hwnd, k, 100, "Output device", 104);
    combo(hwnd, ID_OUTPUT, k.x + 124, k.y + 100, 240);
    send(child(ID_OUTPUT), CB_SETDROPPEDWIDTH, scale(320) as usize, 0);

    let l = LIGHTS_CARD;
    title(hwnd, l, "Lights");
    row(hwnd, l, 36, "Light brightness", 112);
    let style = WS_TABSTOP.0 | TBS_NOTICKS | TBS_FIXEDLENGTH;
    let bar = control(hwnd, w!("msctls_trackbar32"), "", style, l.x + 124, l.y + 36, 116, 28, ID_BRIGHTNESS);
    send(bar, TBM_SETTHUMBLENGTH, scale(18) as usize, 0);
    send(bar, TBM_SETRANGE, 1, (BRIGHTNESS_PERCENT.1 << 16) | BRIGHTNESS_PERCENT.0);
    crate::ui::widget::add(bar, crate::ui::widget::Kind::Slider, None);
    text(hwnd, ID_BRIGHTNESS_VALUE, "", R::new(l.x + 248, l.y + 36, 36, 28), VALUE);
    row(hwnd, l, 68, "Dim lights after", 112);
    combo(hwnd, ID_LIGHTS_DIM, l.x + 132, l.y + 68, 152);
    row(hwnd, l, 100, "Dim lights to", 112);
    combo(hwnd, ID_IDLE_LIGHTS, l.x + 132, l.y + 100, 152);
    hint(hwnd, l, 130, counted);

    let y = SYSTEM_CARD;
    title(hwnd, y, "System");
    toggle(hwnd, "Start with Windows", y.x + 12, y.y + 36, ID_AUTOSTART);
    button(hwnd, "Open config file", y.x + 12, y.y + 72, 120, ID_OPEN_FILE);
    button(hwnd, "Open settings folder", y.x + 140, y.y + 72, 144, ID_OPEN_FOLDER);
    text(hwnd, ID_VERSION, "", R::new(y.x + 12, y.y + 108, 272, 16), HINT);

    // ---- The footer.
    BUILDING.set(None);
    let status = control_at(hwnd, w!("STATIC"), STATUS_IDLE, SS_OWNERDRAW, at(16, FOOTER + 1, 500, 55), ID_STATUS);
    crate::ui::widget::add(status, crate::ui::widget::Kind::Status, None);
    button(hwnd, "Close", 608, FOOTER + 14, 88, ID_CLOSE);

    // The whole text of a select or the status line, when it does not fit.
    let mut tools: Vec<HWND> = CONTROLS.with_borrow(|c| {
        c.iter()
            .filter(|k| crate::ui::widget::kind(k.0) == Some(crate::ui::widget::Kind::Select))
            .map(|k| k.0)
            .collect()
    });
    tools.push(status);
    crate::ui::widget::tooltips(hwnd, &tools);
}

/// Remove sits where Add is when a channel has no Add button.
pub fn place_remove(i: i32, add_shown: bool) {
    move_x(ID_CH_REMOVE + i, channel_card(i).x + if add_shown { 84 } else { 8 });
}

/// Shows the controls of the open tab, minus those hidden by what they show.
pub fn show_tab() {
    let tab = TAB.get();
    let hidden = HIDDEN.with_borrow(Clone::clone);
    CONTROLS.with_borrow(|controls| {
        for &(h, id, on) in controls {
            show(h, on.is_none_or(|t| t == tab) && !hidden.contains(&id));
        }
    });
    if let Some(hwnd) = super::window() {
        crate::ui::anim::redraw(hwnd, crate::ui::anim::Paint::Own);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every box the layout places, by tab (None: the header and footer), at 96 dpi.
    fn boxes() -> Vec<(Option<usize>, &'static str, R)> {
        let mut b = vec![(None, "tabs", TAB_TRACK), (None, "close", R::new(608, FOOTER + 14, 88, 28))];
        b.push((None, "status", R::new(16, FOOTER + 1, 500, 55)));
        b.push((None, "page switch", PAGE_TRACK));
        b.push((None, "pad status", R::new(WIDTH - 276, 8, 260, 32)));
        b.push((Some(0), "key card", KEY_CARD));
        b.push((Some(0), "key hint", R::new(16, PAGE + 176, 680, 16)));
        for k in 0..3 {
            for c in 0..4 {
                let x = KEY_CARD.x + KEY_COL + c * KEY_PITCH;
                b.push((Some(0), "knob select", R::new(x, KNOB_GRID + 20 + k * 32, 132, 28)));
            }
        }
        for i in 0..4 {
            b.push((Some(1), "channel card", channel_card(i)));
        }
        for card in [SCREEN_CARD, SOUND_CARD, LIGHTS_CARD, SYSTEM_CARD] {
            b.push((Some(2), "pad card", card));
        }
        b
    }

    #[test]
    fn the_layout_fits_the_window_without_overlaps_at_every_scale() {
        for dpi in [96, 120, 144, 192] {
            let s = |v: i32| v * dpi / 96;
            let window = R::new(0, 0, s(WIDTH), s(HEIGHT));
            let page = R::new(0, s(PAGE), s(WIDTH), s(FOOTER - PAGE));
            let scaled: Vec<_> =
                boxes().into_iter().map(|(t, n, r)| (t, n, R::new(s(r.x), s(r.y), s(r.w), s(r.h)))).collect();
            for (tab, name, r) in &scaled {
                assert!(window.contains(*r), "{name} at {dpi} dpi leaves the window: {r:?}");
                if tab.is_some() {
                    assert!(page.contains(*r), "{name} at {dpi} dpi leaves the page: {r:?}");
                }
            }
            for (i, (ta, na, a)) in scaled.iter().enumerate() {
                for (tb, nb, b) in &scaled[i + 1..] {
                    let same_view = ta.is_none() || tb.is_none() || ta == tb;
                    let nested = a.contains(*b) || b.contains(*a);
                    assert!(!same_view || nested || !a.overlaps(*b), "{na} and {nb} overlap at {dpi} dpi");
                }
            }
            let footer_line = s(FOOTER);
            assert!(scaled.iter().all(|(t, _, r)| t.is_none() || r.bottom() <= footer_line), "{dpi}");
        }
    }

    #[test]
    fn the_key_grid_fits_its_card() {
        let last = KEY_CARD.x + KEY_COL + 3 * KEY_PITCH + 132;
        assert!(last <= KEY_CARD.right() - 4, "the last column ends at {last}");
        assert_eq!(KEY_CARD.y + 28 + 3 * 32 + 28 + 8, KEY_CARD.bottom(), "four rows and the card's margin");
        assert_eq!(TAB_TRACK.w, 3 * TAB_ITEM.0 + 6, "three items, a 2 px pad and a 1 px border on each side");
        assert_eq!(PAGE_TRACK.w, 2 * PAGE_ITEM.0 + 6);
        assert!(channel_card(3).right() <= WIDTH - 16);
    }

    #[test]
    fn the_pad_cards_fit_their_rows() {
        assert_eq!(SCREEN_CARD.y + 132 + 28 + 4, SCREEN_CARD.bottom(), "four rows and a 4 px margin");
        assert_eq!(SOUND_CARD.y + 100 + 28 + 4, SOUND_CARD.bottom(), "three rows and a 4 px margin");
        assert_eq!(SOUND_CARD.y, SCREEN_CARD.bottom() + 8);
        assert_eq!(SOUND_CARD.bottom(), SYSTEM_CARD.bottom(), "both columns end together");
    }
}
