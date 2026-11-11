"""Generates the OLED bitmaps: src/assets.rs (logo font, big font, sine table), src/font.rs
(5x7 font) and assets/preview.png (what they look like on the 128x32 screen, 4x scale).

Run from anywhere:  uv run --with pillow python firmware/gen_assets.py [TEXT]
TEXT (default KB16) only picks the name drawn in the preview; the pad draws whatever name the
PC app sends it, with the logo font.

Fonts, all in assets/fonts:
  OpenSans-ExtraBold.ttf, OpenSans-SemiBold.ttf   SIL Open Font License 1.1 (OFL.txt)
  5x7.bdf                                        X11 misc-fixed, public domain
SSD1306 layout: a column byte holds 8 vertical pixels, LSB on top.
"""
import math
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
FONTS = HERE / "assets" / "fonts"
ASSETS_RS = HERE / "src" / "assets.rs"
FONT_RS = HERE / "src" / "font.rs"
PREVIEW = HERE / "assets" / "preview.png"
LOGO_FONT = "OpenSans-ExtraBold.ttf"
BIG_FONT = "OpenSans-SemiBold.ttf"
SMALL_FONT = "5x7.bdf"
LOGO_MAX = (124, 24)
# What a name on the logo screen may contain (the app checks the same set).
LOGO_CHARS = " 0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"
# Columns between two logo letters, and the width of a space.
LOGO_GAP = 6
LOGO_SPACE = 8
# Logo glyphs are 4 pages tall: capitals fill the top 24 rows, J and Q reach below.
LOGO_PAGES = 4
# Rows the case hides at the top of the panel (the firmware's default `oled_top`).
OLED_TOP = 3


def ttf(name, size):
    return ImageFont.truetype(str(FONTS / name), size)


def to_columns(img, pages):
    """1-bit image -> list of columns, each `pages` bytes (page-major per column)."""
    w, h = img.size
    px = img.load()
    cols = []
    for x in range(w):
        col = []
        for p in range(pages):
            b = 0
            for bit in range(8):
                y = p * 8 + bit
                if y < h and px[x, y]:
                    b |= 1 << bit
            col.append(b)
        cols.append(col)
    return cols


def render_text(text, font, height, spacing=0, threshold=110):
    widths = [font.getlength(c) for c in text]
    w = int(sum(widths) + spacing * (len(text) - 1)) + 4
    img = Image.new("L", (w, height), 0)
    d = ImageDraw.Draw(img)
    top = font.getbbox("H")[1]
    x = 0.0
    for c, cw in zip(text, widths):
        d.text((x, -top + (height - (font.getbbox("H")[3] - top)) // 2), c, font=font, fill=255)
        x += cw + spacing
    img = img.point(lambda v: 255 if v >= threshold else 0).convert("1")
    bbox = img.getbbox()
    if bbox:
        img = img.crop((bbox[0], 0, bbox[2], height))
    return img


def logo_font():
    """The logo letters at the size that makes KB16 as big as LOGO_MAX allows, capital tops on
    row 0. Each glyph is its ink only; the firmware puts LOGO_GAP columns between them."""
    max_w, max_h = LOGO_MAX
    for size in range(60, 10, -1):
        f = ttf(LOGO_FONT, size)
        l, t, r, b = f.getbbox("KB16")
        if b - t <= max_h and render_text("KB16", f, max_h, spacing=max(1, size // 10)).size[0] <= max_w:
            break
    else:
        raise SystemExit("KB16 does not fit")
    top = f.getbbox("H")[1]
    glyphs = []
    for ch in LOGO_CHARS:
        if ch == " ":
            glyphs.append([[0] * LOGO_PAGES] * LOGO_SPACE)
            continue
        img = Image.new("L", (64, LOGO_PAGES * 8), 0)
        ImageDraw.Draw(img).text((8, -top), ch, font=f, fill=255)
        img = img.point(lambda v: 255 if v >= 110 else 0).convert("1")
        l, _, r, _ = img.getbbox()
        glyphs.append(to_columns(img.crop((l, 0, r, LOGO_PAGES * 8)), LOGO_PAGES))
    return glyphs


def logo_width(text, glyphs):
    return sum(len(glyphs[LOGO_CHARS.index(c)]) + LOGO_GAP for c in text) - LOGO_GAP


def big_font():
    f = ttf(BIG_FONT, 17)
    glyphs = []
    for code in range(32, 127):
        ch = chr(code)
        if ch == " ":
            glyphs.append([[0, 0]] * 4)
            continue
        img = Image.new("L", (24, 16), 0)
        d = ImageDraw.Draw(img)
        top = f.getbbox("H")[1]
        d.text((1, -top + 1), ch, font=f, fill=255)
        img = img.point(lambda v: 255 if v >= 110 else 0).convert("1")
        bbox = img.getbbox()
        if not bbox:
            glyphs.append([[0, 0]] * 3)
            continue
        img = img.crop((bbox[0], 0, bbox[2], 16))
        glyphs.append(to_columns(img, 2))
    return glyphs


def small_font():
    """5x7.bdf, ASCII 32..=126 -> 5 column bytes per glyph. The cell sits one row down
    (bits 1..7) so capitals end on the same row as a 7-row font's would."""
    cell_w, ascent, top_pad = 5, 6, 1
    glyphs, enc, bbx, rows = {}, None, None, None
    for line in (FONTS / SMALL_FONT).read_text(encoding="latin-1").splitlines():
        key, _, rest = line.partition(" ")
        if key == "ENCODING":
            enc = int(rest)
        elif key == "BBX":
            bbx = [int(v) for v in rest.split()]
        elif key == "BITMAP":
            rows = []
        elif key == "ENDCHAR":
            if 32 <= enc <= 126:
                w, h, xoff, yoff = bbx
                cols = [0] * cell_w
                for r, (bits, nbits) in enumerate(rows):
                    for x in range(w):
                        if bits >> (nbits - 1 - x) & 1:
                            cy = ascent - (yoff + h) + r + top_pad
                            cols[xoff + x] |= 1 << cy
                glyphs[enc] = cols
            rows = None
        elif rows is not None:
            rows.append((int(line, 16), 4 * len(line.strip())))
    missing = [c for c in range(32, 127) if c not in glyphs]
    if missing:
        raise SystemExit(f"{SMALL_FONT} lacks {missing}")
    return [glyphs[c] for c in range(32, 127)]


def write_assets(logo, glyphs):
    out = ["//! OLED bitmaps: the logo font, the big font and a sine table.", "//!",
           "//! Generated by gen_assets.py from fonts in assets/fonts - do not edit.", ""]
    offsets, data = [], []
    for g in logo:
        offsets.append(len(data))
        data.extend(g)
    offsets.append(len(data))
    out.append(f"/// Characters the logo font has; glyph i spans LOGO_COLS[LOGO_OFF[i]..LOGO_OFF[i+1]].")
    out.append(f"pub const LOGO_CHARS: &[u8] = b\"{LOGO_CHARS}\";")
    out.append(f"/// Columns between two logo letters.")
    out.append(f"pub const LOGO_GAP: i32 = {LOGO_GAP};")
    out.append(f"pub static LOGO_OFF: [u16; {len(offsets)}] = [{', '.join(map(str, offsets))}];")
    out.append(f"/// {LOGO_PAGES} pages per column, LSB on top; capitals fill the first 3.")
    out.append(f"pub static LOGO_COLS: [[u8; {LOGO_PAGES}]; {len(data)}] = [")
    for i in range(0, len(data), 4):
        out.append("    " + " ".join("[" + ", ".join(f"0x{b:02x}" for b in c) + "]," for c in data[i:i + 4]))
    out.append("];")
    out.append("")
    offsets, data = [], []
    for g in glyphs:
        offsets.append(len(data))
        data.extend(g)
    offsets.append(len(data))
    out.append("/// Big proportional font (16 px), ASCII 32..=126. Glyph i spans BIG_COLS[BIG_OFF[i]..BIG_OFF[i+1]].")
    out.append(f"pub static BIG_OFF: [u16; {len(offsets)}] = [{', '.join(map(str, offsets))}];")
    out.append(f"pub static BIG_COLS: [[u8; 2]; {len(data)}] = [")
    for i in range(0, len(data), 8):
        out.append("    " + " ".join(f"[0x{c[0]:02x}, 0x{c[1]:02x}]," for c in data[i:i + 8]))
    out.append("];")
    sine = [round(127.5 + 127.5 * math.sin(2 * math.pi * i / 256)) for i in range(256)]
    out.append("")
    out.append("/// One sine period, 0..=255 centred on 128.")
    out.append("pub static SIN: [u8; 256] = [")
    for i in range(0, 256, 16):
        out.append("    " + ", ".join(str(v) for v in sine[i:i + 16]) + ",")
    out.append("];")
    ASSETS_RS.write_text("\n".join(out) + "\n", newline="\n")
    return len(data)


def write_small_font(small):
    out = ["//! 5x7 font, ASCII 32..=126. Each glyph is 5 columns, LSB on top, drawn in bits 1..=7.", "//!",
           "//! Generated by gen_assets.py from assets/fonts/5x7.bdf (X11 misc-fixed, public domain) - do not edit.",
           "", "pub static FONT5X7: [[u8; 5]; 95] = ["]
    for code, cols in zip(range(32, 127), small):
        ch = "backslash" if chr(code) == "\\" else repr(chr(code)) if chr(code) == " " else chr(code)
        out.append(f"    [{', '.join(f'0x{b:02X}' for b in cols)}], // {ch}")
    out.append("];")
    FONT_RS.write_text("\n".join(out) + "\n", newline="\n")


class Screen:
    """A 128x32 frame, drawn like the firmware does (everything `OLED_TOP` rows lower)."""

    def __init__(self):
        self.img = Image.new("L", (128, 32), 0)
        self.px = self.img.load()

    def dot(self, x, y):
        y += OLED_TOP
        if 0 <= x < 128 and 0 <= y < 32:
            self.px[x, y] = 255

    def bits(self, x, y, cols, height):
        for dx, col in enumerate(cols):
            for dy in range(height):
                if col >> dy & 1:
                    self.dot(x + dx, y + dy)

    def big(self, x, y, text, big):
        for ch in text:
            g = big[ord(ch) - 32]
            self.bits(x, y, [c[0] | c[1] << 8 for c in g], 16)
            x += len(g) + 2
        return x

    def small(self, x, y, text, small):
        for ch in text:
            self.bits(x, y, small[ord(ch) - 32], 8)
            x += 6


def big_width(text, big):
    return sum(len(big[ord(c) - 32]) + 2 for c in text) - 2


def preview(name, logo, big, small):
    screens = []
    s = Screen()
    x = (128 - logo_width(name, logo)) // 2
    for ch in name:
        g = logo[LOGO_CHARS.index(ch)]
        s.bits(x, 0, [sum(b << (8 * p) for p, b in enumerate(c)) for c in g], LOGO_PAGES * 8)
        x += len(g) + LOGO_GAP
    for x in range(128):
        s.dot(x, 25 + round(127.5 + 127.5 * math.sin(2 * math.pi * x * 4 / 256)) * 3 // 255)
    screens.append(s)
    s = Screen()
    s.big(0, 0, "Chat", big)
    s.big(128 - big_width("42%", big), 0, "42%", big)
    screens.append(s)
    chars = "".join(chr(c) for c in range(33, 127))
    line = ""
    for ch in chars + "\0":
        if ch == "\0" or big_width(line + ch, big) > 126:
            s = Screen()
            s.big(0, 6, line, big)
            screens.append(s)
            line = ""
        line += ch
    s = Screen()
    for i, (name, value) in enumerate([("MIC", "100%"), ("CHAT", "42%"), ("GAME", "7%"), ("MEDIA", "MUTE")]):
        x0 = i * 32
        s.small(x0 + (31 - (len(name) * 6 - 1)) // 2, 1, name, small)
        s.small(x0 + (31 - (len(value) * 6 - 1)) // 2, 22, value, small)
    screens.append(s)
    for part in (chars[:21], chars[21:42], chars[42:63], chars[63:84], chars[84:]):
        s = Screen()
        s.small(0, 8, part, small)
        screens.append(s)

    scale, gap = 4, 6
    sheet = Image.new("RGB", (128 * scale + 2 * gap, len(screens) * (32 * scale + gap) + gap), (40, 40, 48))
    for i, s in enumerate(screens):
        frame = s.img.resize((128 * scale, 32 * scale), Image.NEAREST).convert("RGB")
        hidden = Image.new("RGB", (128 * scale, OLED_TOP * scale), (70, 30, 30))
        frame.paste(Image.blend(frame.crop((0, 0, 128 * scale, OLED_TOP * scale)), hidden, 0.6), (0, 0))
        sheet.paste(frame, (gap, gap + i * (32 * scale + gap)))
    sheet.save(PREVIEW)


def main():
    name = (sys.argv[1] if len(sys.argv) > 1 else "KB16").upper()
    logo = logo_font()
    big = big_font()
    small = small_font()
    n = write_assets(logo, big)
    write_small_font(small)
    preview(name, logo, big, small)
    print(f"logo font {sum(map(len, logo))} columns ({name!r} is {logo_width(name, logo)} wide), "
          f"big font {n} columns -> {ASSETS_RS.name}, {FONT_RS.name}, {PREVIEW.name}")


main()
