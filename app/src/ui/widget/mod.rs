//! Real Win32 controls with the look of `draw`: each control keeps its own keyboard behaviour,
//! focus, tab stop and screen-reader role, and only its painting is ours. A control registered
//! here is subclassed for hover, press and focus feedback; its parent passes its messages
//! through `parent_msg`, which paints the owner-drawn and custom-drawn controls.
//!
//! Every control paints the parent's background (its "scene": the cards, frames, focus ring)
//! under itself first, so rounded corners and sliding indicators meet the right colour.

use super::anim::{self, Curve, Paint};
use super::canvas::{Canvas, R};
use super::draw::{self, ButtonState, Label};
use super::{Font, font, px, radius};
use crate::ui::theme::{Tok, theme};
use crate::win;
use std::cell::{Cell, RefCell};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::Shell::SetWindowSubclass;
use windows::Win32::UI::WindowsAndMessaging::*;

mod input;
mod paint;
mod tip;

pub use input::restyle;
use input::{dress_dropdown, scrollbar_theme, sub};
use paint::*;
pub use tip::tooltips;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Label(Label),
    Button { primary: bool },
    Select,
    Toggle,
    Segment,
    Slider,
    List,
    Edit,
    Status,
}

struct Entry {
    hwnd: HWND,
    kind: Kind,
    /// The frame the parent paints around an edit or a list (parent coordinates).
    frame: Option<R>,
    /// A select's value in force (the row with the check mark).
    value: i32,
    /// A button that is the default one now (Enter presses it).
    default: bool,
    /// An edit whose last value was refused (amber border until the next keystroke).
    error: bool,
}

/// A window's background painter.
pub type Scene = fn(&mut Canvas, HWND);

const HOVER: u32 = 1;
const PRESS: u32 = 2;
const ON: u32 = 3;
const BIG: u32 = 4;
/// A list row's hover: ROW + row.
const ROW: u32 = 0x100;

const HOVER_MS: u32 = 120;
const SUBCLASS: usize = 1;

thread_local! {
    static ENTRIES: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static SCENES: RefCell<Vec<(HWND, Scene)>> = const { RefCell::new(Vec::new()) };
    /// The control under the mouse, and for a list the row.
    static HOT: Cell<(HWND, i32)> = const { Cell::new((HWND(std::ptr::null_mut()), -1)) };
    /// The last input came from the keyboard: what follows from it does not animate.
    static KEYBOARD: Cell<bool> = const { Cell::new(false) };
    /// A toggle being clicked with the mouse: its knob slides.
    static CLICKING: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static CTRL_TAB: Cell<Option<fn(bool)>> = const { Cell::new(None) };
    /// The hidden renderer draws the focus ring even though no key was pressed.
    static SHOW_FOCUS: Cell<bool> = const { Cell::new(false) };
    /// The hidden renderer is printing a window that is not shown.
    static PRINTING: Cell<bool> = const { Cell::new(false) };
    /// A closed select's height above its selection field, once measured.
    static SELECT_FRAME: Cell<i32> = const { Cell::new(6) };
}

pub fn set_select_frame(frame: i32) {
    SELECT_FRAME.set(frame);
}

/// The hidden renderer prints: paint even though the window is not shown.
pub fn printing(on: bool) {
    PRINTING.set(on);
}

/// False while the window of `hwnd` is not shown (it is still being filled, say): a select
/// repaints itself at once on each change, and such paints would be thrown away.
fn worth_painting(hwnd: HWND) -> bool {
    // SAFETY: IsWindowVisible only reads window styles.
    PRINTING.get() || unsafe { IsWindowVisible(hwnd) }.as_bool()
}

fn with<T>(hwnd: HWND, f: impl FnOnce(&mut Entry) -> T) -> Option<T> {
    ENTRIES.with_borrow_mut(|e| e.iter_mut().find(|e| e.hwnd == hwnd).map(f))
}

pub fn kind(hwnd: HWND) -> Option<Kind> {
    with(hwnd, |e| e.kind)
}

/// Makes `hwnd` paint as `kind`. `frame` is the box the parent paints around an edit or list.
pub fn add(hwnd: HWND, kind: Kind, frame: Option<R>) {
    ENTRIES.with_borrow_mut(|e| e.push(Entry { hwnd, kind, frame, value: -1, default: false, error: false }));
    // SAFETY: `sub` is this module's subclass procedure; the subclass is removed on
    // WM_NCDESTROY, before the window is gone.
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(sub), SUBCLASS, 0);
    }
    match kind {
        Kind::Select => dress_dropdown(hwnd),
        Kind::List => scrollbar_theme(hwnd),
        _ => {}
    }
}

pub fn set_scene(top: HWND, scene: Scene) {
    SCENES.with_borrow_mut(|s| {
        s.retain(|(h, _)| *h != top);
        s.push((top, scene));
    });
}

pub fn drop_scene(top: HWND) {
    SCENES.with_borrow_mut(|s| s.retain(|(h, _)| *h != top));
    anim::forget(top);
}

/// What Ctrl+Tab (false) and Ctrl+Shift+Tab (true) do from any control.
pub fn on_ctrl_tab(f: Option<fn(bool)>) {
    CTRL_TAB.set(f);
}

pub fn show_focus(on: bool) {
    SHOW_FOCUS.set(on);
}

/// True if the last input came from the keyboard.
pub fn keyboard() -> bool {
    KEYBOARD.get()
}

pub fn set_value(hwnd: HWND, at: i32) {
    with(hwnd, |e| e.value = at);
}

pub fn set_error(hwnd: HWND, on: bool) {
    if with(hwnd, |e| std::mem::replace(&mut e.error, on)) == Some(!on) {
        redraw_frame(hwnd);
    }
}

/// Changes how a label looks (a hint that turns into a warning).
pub fn set_look(hwnd: HWND, look: Label) {
    if with(hwnd, |e| std::mem::replace(&mut e.kind, Kind::Label(look)) != Kind::Label(look)) == Some(true) {
        anim::redraw(hwnd, Paint::Own);
    }
}

/// Marks a button as the one Enter presses (a dialog's OK).
pub fn set_default(hwnd: HWND) {
    with(hwnd, |e| e.default = true);
}

/// Where `hwnd` sits in its parent's client area.
pub fn rect_in_parent(hwnd: HWND) -> R {
    let mut r = RECT::default();
    // SAFETY: both calls only read window positions; a stale handle leaves `r` empty.
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
        let mut pts = [POINT { x: r.left, y: r.top }, POINT { x: r.right, y: r.bottom }];
        MapWindowPoints(None, GetParent(hwnd).ok(), &mut pts);
        R::new(pts[0].x, pts[0].y, pts[1].x - pts[0].x, pts[1].y - pts[0].y)
    }
}

fn parent(hwnd: HWND) -> HWND {
    // SAFETY: GetParent only reads the window tree.
    unsafe { GetParent(hwnd).unwrap_or_default() }
}

fn visible(hwnd: HWND) -> bool {
    // SAFETY: reads a window's style; a stale handle reads 0.
    unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 }
}

fn client(hwnd: HWND) -> R {
    let mut r = RECT::default();
    // SAFETY: reads the client size into `r`.
    unsafe {
        let _ = GetClientRect(hwnd, &mut r);
    }
    R::of(r)
}

/// The ring's box and corner radius around a control, or None for an edit (its frame shows
/// the focus instead).
fn ring_of(hwnd: HWND) -> Option<(R, i32)> {
    let (kind, frame) = with(hwnd, |e| (e.kind, e.frame))?;
    let r = frame.unwrap_or_else(|| rect_in_parent(hwnd));
    match kind {
        Kind::Edit | Kind::Label(_) | Kind::Status => None,
        Kind::Toggle => Some((r, radius(6))),
        _ => Some((r, radius(6))),
    }
}

/// Repaints the parent around `hwnd`: its ring, its frame, the children under them.
fn redraw_frame(hwnd: HWND) {
    let r = with(hwnd, |e| e.frame).flatten().unwrap_or_else(|| rect_in_parent(hwnd));
    anim::redraw(hwnd, Paint::Area(parent(hwnd), r.grow(px(4) + 2).rect()));
}

use win::focus;
use win::{VK_CONTROL, VK_END, VK_HOME, VK_SHIFT, VK_TAB};

fn has(state: ODS_FLAGS, flag: ODS_FLAGS) -> bool {
    state.0 & flag.0 != 0
}

/// Paints the focus ring around the focused control of `top`, if keyboard use shows it.
pub fn paint_ring(cv: &mut Canvas, top: HWND) {
    let f = focus();
    if f.is_invalid() || parent(f) != top || !visible(f) {
        return;
    }
    let hidden = win::send(top, WM_QUERYUISTATE, 0, 0) as u32 & UISF_HIDEFOCUS != 0;
    if hidden && !SHOW_FOCUS.get() {
        return;
    }
    if let Some((r, rad)) = ring_of(f) {
        draw::ring(cv, r, rad);
    }
}

/// Paints the frames of the visible edits and lists of `top`.
pub fn paint_frames(cv: &mut Canvas, top: HWND) {
    let area = cv.area();
    let f = focus();
    let hot = HOT.get().0;
    ENTRIES.with_borrow(|entries| {
        for e in entries.iter().filter(|e| e.frame.is_some_and(|r| r.grow(2).overlaps(area))) {
            if parent(e.hwnd) != top || !visible(e.hwnd) {
                continue;
            }
            let Some(r) = e.frame else { continue };
            match e.kind {
                Kind::Edit => {
                    let hover = anim::value(e.hwnd, HOVER, (hot == e.hwnd) as i32 as f32);
                    draw::field(cv, r, hover, f == e.hwnd, e.error);
                }
                _ => draw::list_frame(cv, r),
            }
        }
    });
}

/// Paints `top`'s scene into `cv`.
fn scene(cv: &mut Canvas, top: HWND) {
    match SCENES.with_borrow(|s| s.iter().find(|(h, _)| *h == top).map(|(_, f)| *f)) {
        Some(paint) => paint(cv, top),
        None => cv.fill(cv.area(), theme().get(Tok::Bg)),
    }
}

/// A canvas over control `hwnd`, with its parent's scene already painted.
fn backdrop(hwnd: HWND) -> Option<(Canvas, R)> {
    let r = rect_in_parent(hwnd);
    let mut cv = Canvas::new(r)?;
    scene(&mut cv, parent(hwnd));
    Some((cv, r))
}

/// Paints a control of our own class: its parent's scene, then `paint`, into `dc`.
pub fn paint_over(hwnd: HWND, dc: HDC, paint: impl FnOnce(&mut Canvas, R)) {
    if let Some((mut cv, r)) = backdrop(hwnd) {
        paint(&mut cv, r);
        cv.present(dc, (0, 0));
    }
}

fn hover_of(hwnd: HWND) -> f32 {
    anim::value(hwnd, HOVER, (HOT.get().0 == hwnd) as i32 as f32)
}

/// The parent's share of the work: painting its scene and its owner-drawn and custom-drawn
/// children, and the colours of the edits and lists. None for messages it leaves alone.
pub fn parent_msg(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    let t = theme();
    // SAFETY: Windows sends these messages with their documented parameters: WM_DRAWITEM and
    // WM_MEASUREITEM point to their structs, WM_NOTIFY to an NMHDR (an NMCUSTOMDRAW for
    // NM_CUSTOMDRAW), and the WM_CTLCOLOR* wParam is a live DC.
    unsafe {
        match msg {
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let r = R::of(ps.rcPaint);
                if let Some(mut cv) = Canvas::new(r) {
                    scene(&mut cv, hwnd);
                    cv.present(dc, (r.x, r.y));
                }
                let _ = EndPaint(hwnd, &ps);
                Some(LRESULT(0))
            }
            WM_PRINTCLIENT => {
                if let Some(mut cv) = Canvas::new(client(hwnd)) {
                    scene(&mut cv, hwnd);
                    cv.present(HDC(wparam.0 as *mut _), (0, 0));
                }
                Some(LRESULT(0))
            }
            WM_DRAWITEM => {
                let d = &*(lparam.0 as *const DRAWITEMSTRUCT);
                if d.CtlType == ODT_LISTBOX || d.CtlType == ODT_COMBOBOX && !has(d.itemState, ODS_COMBOBOXEDIT) {
                    paint_row(d);
                } else {
                    paint_control(d.hwndItem, d.hDC, Some(d.itemState));
                }
                Some(LRESULT(1))
            }
            WM_MEASUREITEM => {
                let m = &mut *(lparam.0 as *mut MEASUREITEMSTRUCT);
                m.itemHeight = match (m.CtlType, m.itemID) {
                    (ODT_LISTBOX, _) => px(20),
                    (_, u32::MAX) => px(28) - SELECT_FRAME.get(),
                    _ => px(28),
                } as u32;
                Some(LRESULT(1))
            }
            WM_NOTIFY => {
                let hdr = &*(lparam.0 as *const NMHDR);
                if hdr.code == TTN_GETDISPINFOW {
                    tip::text(&mut *(lparam.0 as *mut NMTTDISPINFOW));
                    return Some(LRESULT(0));
                }
                if hdr.code != NM_CUSTOMDRAW || kind(hdr.hwndFrom).is_none() {
                    return None;
                }
                let cd = &*(lparam.0 as *const NMCUSTOMDRAW);
                // Buttons ask first at the erase stage, the slider at the paint stage: all of the
                // control is painted at the first ask, and Windows paints nothing of it.
                if cd.dwDrawStage == CDDS_PREERASE || cd.dwDrawStage == CDDS_PREPAINT {
                    paint_control(hdr.hwndFrom, cd.hdc, None);
                }
                Some(LRESULT(CDRF_SKIPDEFAULT as isize))
            }
            WM_CTLCOLOREDIT => {
                let from = HWND(lparam.0 as *mut _);
                let hover = hover_of(from);
                let fill = draw::field_fill(&t, hover, focus() == from);
                Some(dc_brush(HDC(wparam.0 as *mut _), fill.cr(), Some(t.get(Tok::Text).cr())))
            }
            WM_CTLCOLORLISTBOX => Some(dc_brush(HDC(wparam.0 as *mut _), t.get(Tok::Control).cr(), None)),
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
                Some(dc_brush(HDC(wparam.0 as *mut _), t.get(Tok::Surface).cr(), Some(t.get(Tok::Text).cr())))
            }
            WM_CTLCOLORDLG => Some(dc_brush(HDC(wparam.0 as *mut _), t.get(Tok::Bg).cr(), None)),
            WM_UPDATEUISTATE => {
                let f = focus();
                if !f.is_invalid() && parent(f) == hwnd {
                    redraw_frame(f);
                }
                None
            }
            _ => None,
        }
    }
}

/// The stock DC brush in `fill` (no brush is made), with the text colours for a control.
fn dc_brush(dc: HDC, fill: COLORREF, ink: Option<COLORREF>) -> LRESULT {
    // SAFETY: `dc` is the live DC of the WM_CTLCOLOR* message; the stock brush is never freed.
    unsafe {
        SetDCBrushColor(dc, fill);
        SetBkColor(dc, fill);
        if let Some(ink) = ink {
            SetTextColor(dc, ink);
        }
        LRESULT(GetStockObject(DC_BRUSH).0 as isize)
    }
}

/// The height of one line of body text, for placing an edit in its frame.
pub fn body_line() -> i32 {
    super::canvas::line_height(font(Font::Body))
}
