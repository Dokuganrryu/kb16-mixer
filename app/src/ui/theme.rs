//! The colours of the settings window and its dialogs, and where they come from: the Windows
//! app theme (dark or light), or the system colours when high contrast is on. Also the title
//! bar, which DWM paints in the same colours.

use std::cell::Cell;
use windows::Win32::Foundation::{COLORREF, HWND};
use windows::Win32::Graphics::Dwm::{DWMWINDOWATTRIBUTE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_HOTLIGHT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, GetSysColor, SYS_COLOR_INDEX,
};
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, PWSTR, w};

/// A colour as 0xRRGGBB.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u32);

impl Rgb {
    fn channel(self, shift: u32) -> u32 {
        (self.0 >> shift) & 0xFF
    }

    /// The same colour as GDI writes it (0x00BBGGRR).
    pub fn cr(self) -> COLORREF {
        COLORREF(self.channel(16) | self.channel(8) << 8 | self.channel(0) << 16)
    }

    fn from_cr(c: u32) -> Rgb {
        Rgb((c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF)
    }

    /// The colour `t` of the way (0 to 1) from `self` to `to`, in sRGB.
    pub fn lerp(self, to: Rgb, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        let mix = |s: u32| {
            let (a, b) = (self.channel(s) as f32, to.channel(s) as f32);
            ((a + (b - a) * t).round() as u32) << s
        };
        Rgb(mix(16) | mix(8) | mix(0))
    }
}

/// One colour of the palette, by its role (the names of the design spec).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Bg,
    Surface,
    Control,
    Hover,
    Press,
    Seg,
    Ind,
    Line,
    Cborder,
    Text,
    Text2,
    Text3,
    Accent,
    AccentH,
    AccentFg,
    Tint,
    Amber,
    /// The focus ring: the accent, or the hot-light colour in high contrast.
    Ring,
    /// Text on the segmented indicator and on a tint: the text colour, or the highlight text.
    OnInd,
    Button,
    ButtonText,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Look {
    Dark,
    Light,
    Contrast,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Palette([Rgb; 21]);

impl Palette {
    pub fn get(&self, t: Tok) -> Rgb {
        self.0[t as usize]
    }

    /// The design palette: bg, surface, control, hover, press, seg, ind, line, cborder, text,
    /// text2, text3, accent, accent-h, accentfg, tint, amber. The rest follow from those.
    const fn design(c: [u32; 17]) -> Palette {
        Palette([
            Rgb(c[0]),
            Rgb(c[1]),
            Rgb(c[2]),
            Rgb(c[3]),
            Rgb(c[4]),
            Rgb(c[5]),
            Rgb(c[6]),
            Rgb(c[7]),
            Rgb(c[8]),
            Rgb(c[9]),
            Rgb(c[10]),
            Rgb(c[11]),
            Rgb(c[12]),
            Rgb(c[13]),
            Rgb(c[14]),
            Rgb(c[15]),
            Rgb(c[16]),
            Rgb(c[12]),
            Rgb(c[9]),
            Rgb(c[2]),
            Rgb(c[9]),
        ])
    }

    pub const DARK: Palette = Palette::design([
        0x111214, 0x18191c, 0x212226, 0x292a2f, 0x1c1d20, 0x212226, 0x2c2e33, 0x26272b, 0x70747d, 0xecedf0, 0xa4a8b0,
        0x6c7078, 0x9fcfff, 0xb8dcff, 0x0a1622, 0x1d2a38, 0xf0b75e,
    ]);
    pub const LIGHT: Palette = Palette::design([
        0xf3f4f6, 0xffffff, 0xffffff, 0xf2f3f5, 0xe8e9ec, 0xe9eaed, 0xffffff, 0xe2e4e8, 0x80858e, 0x16171a, 0x575b63,
        0xa3a7ae, 0x1d63c8, 0x1a58b2, 0xffffff, 0xe6effb, 0x9a5800,
    ]);

    /// High contrast: only the system colours, so no two fills differ by shade alone.
    fn contrast() -> Palette {
        let sys = |i: SYS_COLOR_INDEX| {
            // SAFETY: GetSysColor only reads a system setting.
            Rgb::from_cr(unsafe { GetSysColor(i) })
        };
        let (window, text, gray) = (sys(COLOR_WINDOW), sys(COLOR_WINDOWTEXT), sys(COLOR_GRAYTEXT));
        let (high, high_text) = (sys(COLOR_HIGHLIGHT), sys(COLOR_HIGHLIGHTTEXT));
        Palette([
            window,
            window,
            window,
            window,
            window,
            window,
            high,
            text,
            text,
            text,
            text,
            gray,
            high,
            high,
            high_text,
            high,
            text,
            sys(COLOR_HOTLIGHT),
            high_text,
            sys(COLOR_BTNFACE),
            sys(COLOR_BTNTEXT),
        ])
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    pub look: Look,
    pub c: Palette,
    /// False: every state change is instant (reduced motion, or high contrast).
    pub motion: bool,
}

impl Theme {
    pub fn of(look: Look, motion: bool) -> Theme {
        let c = match look {
            Look::Dark => Palette::DARK,
            Look::Light => Palette::LIGHT,
            Look::Contrast => Palette::contrast(),
        };
        Theme { look, c, motion: motion && look != Look::Contrast }
    }

    pub fn get(&self, t: Tok) -> Rgb {
        self.c.get(t)
    }
}

thread_local! {
    static THEME: Cell<Theme> = Cell::new(Theme::of(Look::Dark, true));
    /// A look forced by the hidden renderer; None follows Windows.
    static FORCED: Cell<Option<Look>> = const { Cell::new(None) };
}

pub fn theme() -> Theme {
    THEME.get()
}

pub fn force(look: Option<Look>) {
    FORCED.set(look);
}

/// Reads the Windows settings again; true if the theme changed.
pub fn refresh() -> bool {
    let look = FORCED.get().unwrap_or_else(system_look);
    let new = Theme::of(look, FORCED.get().is_none() && animations_on());
    THEME.replace(new) != new
}

fn system_look() -> Look {
    if high_contrast() {
        return Look::Contrast;
    }
    let mut light = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the value is a DWORD (RRF_RT_REG_DWORD), written into `light`, whose size is given.
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut light).cast()),
            Some(&mut size),
        )
    };
    if found.is_ok() && light != 0 { Look::Light } else { Look::Dark }
}

/// HIGHCONTRASTW, declared here: the `windows` crate keeps it in its large accessibility module.
#[repr(C)]
struct HighContrast {
    size: u32,
    flags: u32,
    scheme: PWSTR,
}

fn high_contrast() -> bool {
    const HCF_HIGHCONTRASTON: u32 = 1;
    let mut hc = HighContrast { size: size_of::<HighContrast>() as u32, flags: 0, scheme: PWSTR::null() };
    // SAFETY: the struct carries its own size and outlives the call.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.size,
            Some((&raw mut hc).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && hc.flags & HCF_HIGHCONTRASTON != 0
}

/// Settings > Accessibility > Visual effects > Animation effects.
fn animations_on() -> bool {
    let mut on = BOOL(1);
    // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL into `on`.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut on).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || on.as_bool()
}

fn dwm_set<T>(hwnd: HWND, attribute: i32, value: &T) -> bool {
    // SAFETY: `value` is the documented type of the attribute and lives until the call returns.
    unsafe {
        DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(attribute), (value as *const T).cast(), size_of::<T>() as u32)
            .is_ok()
    }
}

const WINDOW_CORNER_PREFERENCE: i32 = 33;
const BORDER_COLOR: i32 = 34;
const COLOR_DEFAULT: u32 = 0xFFFF_FFFF;

fn colour_attr(tok: Tok) -> u32 {
    let t = theme();
    if t.look == Look::Contrast { COLOR_DEFAULT } else { t.get(tok).cr().0 }
}

/// The title bar and frame in the theme's colours. Windows 10 ignores the colour attributes and
/// keeps its own dark or light caption.
pub fn chrome(hwnd: HWND) {
    const USE_IMMERSIVE_DARK_MODE: i32 = 20;
    const USE_IMMERSIVE_DARK_MODE_BEFORE_20H1: i32 = 19;
    const CAPTION_COLOR: i32 = 35;
    const TEXT_COLOR: i32 = 36;
    const DWMWCP_DEFAULT: i32 = 0;
    let dark = BOOL::from(theme().look == Look::Dark);
    if !dwm_set(hwnd, USE_IMMERSIVE_DARK_MODE, &dark) {
        dwm_set(hwnd, USE_IMMERSIVE_DARK_MODE_BEFORE_20H1, &dark);
    }
    dwm_set(hwnd, WINDOW_CORNER_PREFERENCE, &DWMWCP_DEFAULT);
    dwm_set(hwnd, CAPTION_COLOR, &colour_attr(Tok::Bg));
    dwm_set(hwnd, BORDER_COLOR, &colour_attr(Tok::Line));
    dwm_set(hwnd, TEXT_COLOR, &colour_attr(Tok::Text));
}

/// A dropdown list: small rounded corners and a control-coloured border from DWM. False on
/// Windows 10, which has neither; the list then draws its own square border.
pub fn popup_chrome(list: HWND) -> bool {
    const DWMWCP_ROUNDSMALL: i32 = 3;
    dwm_set(list, WINDOW_CORNER_PREFERENCE, &DWMWCP_ROUNDSMALL)
        && dwm_set(list, BORDER_COLOR, &colour_attr(Tok::Cborder))
}

/// WCAG 2 contrast ratio of two colours.
#[cfg(test)]
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let lum = |c: Rgb| {
        let lin = |s: u32| {
            let v = c.channel(s) as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(16) + 0.7152 * lin(8) + 0.0722 * lin(0)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    use Tok::*;

    #[test]
    fn every_pair_meets_the_contrast_of_the_spec() {
        // (foreground, background, dark ratio, light ratio), from the spec's table.
        let pairs = [
            (Text, Bg, 16.01, 16.29),
            (Text, Surface, 15.02, 17.92),
            (Text, Control, 13.57, 17.92),
            (Text, Hover, 12.23, 16.14),
            (Text, Press, 14.40, 14.76),
            (Text, Tint, 12.45, 15.45),
            (Text, Ind, 11.61, 17.92),
            (Text2, Bg, 7.86, 6.19),
            (Text2, Surface, 7.37, 6.81),
            (Text2, Control, 6.66, 6.81),
            (Text2, Hover, 6.01, 6.14),
            (AccentFg, Accent, 11.16, 5.72),
            (AccentFg, AccentH, 12.79, 6.81),
            (Amber, Bg, 10.39, 5.06),
            (Amber, Surface, 9.74, 5.57),
            (Accent, Bg, 11.46, 5.20),
            (Accent, Surface, 10.75, 5.72),
            (Accent, Control, 9.72, 5.72),
            (Accent, Tint, 8.92, 4.93),
            (Cborder, Bg, 4.00, 3.37),
            (Cborder, Surface, 3.75, 3.71),
            (Cborder, Control, 3.39, 3.71),
            (Cborder, Hover, 3.06, 3.34),
        ];
        for (fg, bg, dark, light) in pairs {
            for (p, want) in [(Palette::DARK, dark), (Palette::LIGHT, light)] {
                let got = contrast(p.get(fg), p.get(bg));
                assert!((got - want).abs() < 0.01, "{fg:?} on {bg:?}: {got:.2}, the spec says {want}");
                // Text needs 4.5:1; borders and the focus ring (accent as a shape) need 3:1.
                let floor = if matches!(fg, Cborder | Accent) { 3.0 } else { 4.5 };
                assert!(got >= floor, "{fg:?} on {bg:?}: {got:.2}");
            }
        }
        assert_eq!(Palette::DARK.get(Ring), Palette::DARK.get(Accent), "the ring is the accent outside high contrast");
    }

    #[test]
    fn colours_mix_in_srgb_and_reach_gdi_in_its_order() {
        assert_eq!(Rgb(0x000000).lerp(Rgb(0xFFFFFF), 0.5), Rgb(0x808080));
        assert_eq!(Rgb(0x112233).lerp(Rgb(0x445566), 0.0), Rgb(0x112233));
        assert_eq!(Rgb(0x112233).lerp(Rgb(0x445566), 1.0), Rgb(0x445566));
        assert_eq!(Rgb(0x112233).lerp(Rgb(0x445566), 7.0), Rgb(0x445566), "clamped");
        assert_eq!(Rgb(0x123456).cr().0, 0x563412);
        assert_eq!(Rgb::from_cr(0x563412), Rgb(0x123456));
        assert!(!Theme::of(Look::Contrast, true).motion, "high contrast never moves");
        assert!(!Theme::of(Look::Dark, false).motion);
    }
}
