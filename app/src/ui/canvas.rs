//! A back buffer to paint into: a 32-bit DIB section whose pixels are written directly for
//! shapes (anti-aliased rounded rectangles and strokes, blended into the pixels already there,
//! so no alpha channel is needed) and through GDI for text. Coordinates are those of the window
//! being painted; the buffer covers only the part being painted.

use super::theme::Rgb;
use std::cell::RefCell;
use windows::Win32::Foundation::{RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;

/// A rectangle as position and size.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct R {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl R {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> R {
        R { x, y, w, h }
    }

    pub fn right(self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(self) -> i32 {
        self.y + self.h
    }

    /// Grown by `d` on every side (shrunk when negative).
    pub fn grow(self, d: i32) -> R {
        R::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }

    pub fn at(self, dx: i32, dy: i32) -> R {
        R::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn rect(self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }

    pub fn of(r: RECT) -> R {
        R::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
    }

    pub fn overlaps(self, o: R) -> bool {
        self.x < o.right() && o.x < self.right() && self.y < o.bottom() && o.y < self.bottom()
    }

    #[cfg(test)]
    pub fn contains(self, o: R) -> bool {
        o.x >= self.x && o.y >= self.y && o.right() <= self.right() && o.bottom() <= self.bottom()
    }
}

/// The share (0 to 255) of each pixel of a rounded corner that lies inside the outer edge and
/// inside the inner edge (the border's), for a radius and a border width, supersampled 8 x 8.
/// Row by row from the corner's outer pixel; the same mask serves all four corners mirrored.
pub fn corner_mask(radius: i32, border: i32) -> Vec<(u8, u8)> {
    const N: i32 = 8;
    let (r, inner) = (radius as f32, (radius - border).max(0) as f32);
    let mut mask = Vec::with_capacity((radius * radius) as usize);
    for j in 0..radius {
        for i in 0..radius {
            let (mut outer_n, mut inner_n) = (0, 0);
            for sy in 0..N {
                for sx in 0..N {
                    let dx = r - (i as f32 + (sx as f32 + 0.5) / N as f32);
                    let dy = r - (j as f32 + (sy as f32 + 0.5) / N as f32);
                    let d2 = dx * dx + dy * dy;
                    outer_n += (d2 <= r * r) as i32;
                    inner_n += (d2 <= inner * inner) as i32;
                }
            }
            let share = |n: i32| (n * 255 / (N * N)) as u8;
            mask.push((share(outer_n), share(inner_n)));
        }
    }
    mask
}

/// A corner mask with the (radius, border) it was made for.
type Mask = ((i32, i32), Vec<(u8, u8)>);

thread_local! {
    /// The corner masks made so far. A window uses a handful of sizes.
    static MASKS: RefCell<Vec<Mask>> = const { RefCell::new(Vec::new()) };
}

/// Forgets the corner masks (the window is closed, or its scale changed).
pub fn forget_masks() {
    MASKS.with_borrow_mut(Vec::clear);
}

fn mix(dst: u32, parts: [(u32, u32); 2], keep: u32) -> u32 {
    let ch = |c: u32, s: u32| (c >> s) & 0xFF;
    let mut out = 0;
    for s in [0, 8, 16] {
        let v = ch(dst, s) * keep + ch(parts[0].0, s) * parts[0].1 + ch(parts[1].0, s) * parts[1].1;
        out |= ((v + 127) / 255).min(255) << s;
    }
    out
}

pub struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    origin: (i32, i32),
    w: i32,
    h: i32,
    /// GDI drew since the pixels were last touched directly: flush before the next write.
    gdi: bool,
}

impl Canvas {
    /// A buffer for the part `area` of a window.
    pub fn new(area: R) -> Option<Canvas> {
        let (w, h) = (area.w.max(1), area.h.max(1));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: the header describes a top-down 32-bit bitmap; CreateDIBSection writes the
        // pixel pointer into `bits`. The DC and bitmap are freed in Drop, in reverse order.
        unsafe {
            let dc = CreateCompatibleDC(None);
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, bitmap.into());
            let _ = SetViewportOrgEx(dc, -area.x, -area.y, None);
            SetBkMode(dc, TRANSPARENT);
            Some(Canvas { dc, bitmap, old, bits: bits.cast(), origin: (area.x, area.y), w, h, gdi: false })
        }
    }

    /// The buffer's DC, for drawing into it with GDI or WM_PRINT.
    pub fn dc(&mut self) -> HDC {
        self.gdi = true;
        self.dc
    }

    /// The area this canvas covers.
    pub fn area(&self) -> R {
        R::new(self.origin.0, self.origin.1, self.w, self.h)
    }

    /// Copies the buffer to `target` at `at` (the target's own coordinates).
    pub fn present(&self, target: HDC, at: (i32, i32)) {
        // SAFETY: both DCs are live; the source rectangle is the whole buffer, in its logical
        // coordinates (those of the window).
        unsafe {
            let _ = BitBlt(target, at.0, at.1, self.w, self.h, Some(self.dc), self.origin.0, self.origin.1, SRCCOPY);
        }
    }

    /// The pixels, as 0x00RRGGBB, row by row.
    pub fn pixels(&mut self) -> &mut [u32] {
        if self.gdi {
            // SAFETY: GdiFlush takes no arguments; it finishes the GDI drawing queued so far.
            unsafe {
                let _ = GdiFlush();
            }
            self.gdi = false;
        }
        // SAFETY: the DIB section holds exactly w * h pixels and lives as long as `self`.
        unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
    }

    /// The part of `r` inside the buffer, in buffer coordinates.
    fn clip(&self, r: R) -> Option<(i32, i32, i32, i32)> {
        let x0 = (r.x - self.origin.0).max(0);
        let y0 = (r.y - self.origin.1).max(0);
        let x1 = (r.right() - self.origin.0).min(self.w);
        let y1 = (r.bottom() - self.origin.1).min(self.h);
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }

    pub fn fill(&mut self, r: R, c: Rgb) {
        let Some((x0, y0, x1, y1)) = self.clip(r) else { return };
        let w = self.w;
        let px = self.pixels();
        for y in y0..y1 {
            px[(y * w + x0) as usize..(y * w + x1) as usize].fill(c.0);
        }
    }

    /// Blends one pixel (window coordinates): `line` at share `a_line`, `fill` at `a_fill`
    /// (0 to 255 each), the rest stays.
    fn blend(&mut self, x: i32, y: i32, line: (Rgb, u32), fill: (Rgb, u32)) {
        let (x, y) = (x - self.origin.0, y - self.origin.1);
        if x < 0 || y < 0 || x >= self.w || y >= self.h {
            return;
        }
        let at = (y * self.w + x) as usize;
        let px = self.pixels();
        let keep = 255 - line.1 - fill.1;
        px[at] = mix(px[at], [(line.0.0, line.1), (fill.0.0, fill.1)], keep);
    }

    /// A rectangle with corners of `radius`, filled with `fill`, with a border of `border`
    /// pixels in `line`. Without a fill only the border is drawn and the inside stays.
    pub fn round(&mut self, r: R, radius: i32, border: i32, fill: Option<Rgb>, line: Option<Rgb>) {
        if r.w <= 0 || r.h <= 0 || !r.overlaps(self.area()) {
            return;
        }
        let Some(line) = line.or(fill) else { return };
        let rad = radius.clamp(0, r.w.min(r.h) / 2);
        let bw = if Some(line) == fill { 0 } else { border.clamp(0, r.w.min(r.h) / 2) };
        let (x, y, right, bottom) = (r.x, r.y, r.right(), r.bottom());
        let edge = rad.max(bw);
        if bw > 0 {
            self.fill(R::new(x + rad, y, r.w - 2 * rad, bw), line);
            self.fill(R::new(x + rad, bottom - bw, r.w - 2 * rad, bw), line);
            self.fill(R::new(x, y + rad, bw, r.h - 2 * rad), line);
            self.fill(R::new(right - bw, y + rad, bw, r.h - 2 * rad), line);
        }
        if let Some(f) = fill {
            self.fill(R::new(x + bw, y + edge, r.w - 2 * bw, r.h - 2 * edge), f);
            self.fill(R::new(x + rad, y + bw, r.w - 2 * rad, edge - bw), f);
            self.fill(R::new(x + rad, bottom - edge, r.w - 2 * rad, edge - bw), f);
        }
        if rad == 0 {
            return;
        }
        MASKS.with_borrow_mut(|masks| {
            if !masks.iter().any(|(k, _)| *k == (rad, bw)) {
                masks.push(((rad, bw), corner_mask(rad, bw)));
            }
        });
        MASKS.with_borrow(|masks| {
            let Some((_, mask)) = masks.iter().find(|(k, _)| *k == (rad, bw)) else { return };
            for j in 0..rad {
                for i in 0..rad {
                    let (o, n) = mask[(j * rad + i) as usize];
                    let on_line = (line, (o - n) as u32);
                    // Without a fill the inside is left as it is.
                    let inside = (fill.unwrap_or(line), if fill.is_some() { n as u32 } else { 0 });
                    self.blend(x + i, y + j, on_line, inside);
                    self.blend(right - 1 - i, y + j, on_line, inside);
                    self.blend(x + i, bottom - 1 - j, on_line, inside);
                    self.blend(right - 1 - i, bottom - 1 - j, on_line, inside);
                }
            }
        });
    }

    /// A line through `points` (window coordinates, as fractions of a pixel), `width` wide with
    /// round ends, anti-aliased.
    pub fn stroke(&mut self, points: &[(f32, f32)], width: f32, c: Rgb) {
        let half = width / 2.0;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for &(px, py) in points {
            (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
        }
        let distance = |qx: f32, qy: f32| {
            points
                .windows(2)
                .map(|s| {
                    let ((ax, ay), (bx, by)) = (s[0], s[1]);
                    let (dx, dy) = (bx - ax, by - ay);
                    let t = (((qx - ax) * dx + (qy - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
                    ((qx - ax - t * dx).powi(2) + (qy - ay - t * dy).powi(2)).sqrt()
                })
                .fold(f32::MAX, f32::min)
        };
        for y in (y0 - half - 1.0).floor() as i32..=(y1 + half + 1.0).ceil() as i32 {
            for x in (x0 - half - 1.0).floor() as i32..=(x1 + half + 1.0).ceil() as i32 {
                let cover = (half + 0.5 - distance(x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.blend(x, y, (c, (cover * 255.0) as u32), (c, 0));
                }
            }
        }
    }

    /// Text in `r` with DrawText `flags`.
    pub fn text(&mut self, r: R, text: &str, font: HFONT, c: Rgb, flags: DRAW_TEXT_FORMAT) {
        if !r.overlaps(self.area()) || text.is_empty() {
            return;
        }
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let mut rect = r.rect();
        // SAFETY: the DC is ours; the font is a live font of the window; DrawTextW reads the
        // text and may write into it only with DT_MODIFYSTRING, which is never passed.
        unsafe {
            let old = SelectObject(self.dc, font.into());
            SetTextColor(self.dc, c.cr());
            DrawTextW(self.dc, &mut wide, &mut rect, flags);
            SelectObject(self.dc, old);
        }
        self.gdi = true;
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: the bitmap is unselected before both are freed; nothing else holds them.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// The size `text` takes in `font` on one line, or the height it takes wrapped at `width`.
pub fn measure(text: &str, font: HFONT, width: Option<i32>) -> SIZE {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = RECT { right: width.unwrap_or(0), ..Default::default() };
    let flags = DT_CALCRECT | DT_NOPREFIX | if width.is_some() { DT_WORDBREAK } else { DT_SINGLELINE };
    // SAFETY: a memory DC of our own, with the font put back before it is freed.
    unsafe {
        let dc = CreateCompatibleDC(None);
        let old = SelectObject(dc, font.into());
        DrawTextW(dc, &mut wide, &mut rect, flags);
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
    }
    SIZE { cx: rect.right, cy: rect.bottom }
}

/// The height of one line of `font` (TEXTMETRIC's tmHeight).
pub fn line_height(font: HFONT) -> i32 {
    let mut tm = TEXTMETRICW::default();
    // SAFETY: as in `measure`.
    unsafe {
        let dc = CreateCompatibleDC(None);
        let old = SelectObject(dc, font.into());
        let _ = GetTextMetricsW(dc, &mut tm);
        SelectObject(dc, old);
        let _ = DeleteDC(dc);
    }
    tm.tmHeight
}

/// Makes a font from the system's message font face, `px` high and of `weight`.
pub fn font(face: &[u16; 32], px: i32, weight: i32) -> HFONT {
    let lf = LOGFONTW {
        lfHeight: -px,
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        lfFaceName: *face,
        ..Default::default()
    };
    // SAFETY: the struct is complete and outlives the call; the caller deletes the font.
    unsafe { CreateFontIndirectW(&lf) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_mask_covers_what_the_circle_covers() {
        let m = corner_mask(6, 1);
        assert_eq!(m.len(), 36);
        assert_eq!(m[0].0, 0, "the outermost corner pixel is outside the circle");
        assert_eq!(m[35], (255, 255), "the pixel next to the centre is inside both edges");
        let outer: u32 = m.iter().map(|p| p.0 as u32).sum();
        let area = outer as f64 / 255.0;
        let want = 36.0 - (36.0 - std::f64::consts::PI * 9.0);
        assert!((area - want).abs() < 0.6, "a quarter disc of radius 6: {area:.2} vs {want:.2}");
        assert!(m.iter().all(|p| p.1 <= p.0), "the inner edge lies inside the outer one");
        let ring: u32 = m.iter().map(|p| (p.0 - p.1) as u32).sum();
        let ring = ring as f64 / 255.0;
        let want = std::f64::consts::PI / 4.0 * (36.0 - 25.0);
        assert!((ring - want).abs() < 0.6, "a 1 px quarter ring: {ring:.2} vs {want:.2}");
        assert!(corner_mask(0, 1).is_empty());
    }

    #[test]
    fn pixels_mix_by_their_shares() {
        assert_eq!(mix(0x000000, [(0xFFFFFF, 255), (0, 0)], 0), 0xFFFFFF);
        assert_eq!(mix(0x000000, [(0xFFFFFF, 128), (0, 0)], 127), 0x808080);
        assert_eq!(mix(0x102030, [(0, 0), (0, 0)], 255), 0x102030, "nothing drawn, nothing changed");
        assert_eq!(mix(0xFF0000, [(0x0000FF, 100), (0x00FF00, 55)], 100), 0x0064_3764);
    }

    #[test]
    fn rectangles_grow_and_clip() {
        let r = R::new(10, 20, 30, 40);
        assert_eq!(r.grow(2), R::new(8, 18, 34, 44));
        assert_eq!((r.right(), r.bottom()), (40, 60));
        assert!(r.overlaps(R::new(39, 59, 5, 5)) && !r.overlaps(R::new(40, 20, 5, 5)));
        assert!(r.contains(R::new(10, 20, 30, 40)) && !r.contains(r.grow(1)));
        assert_eq!(R::of(r.rect()), r);
    }
}
