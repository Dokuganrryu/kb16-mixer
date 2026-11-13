//! Draws the pad's screens with the firmware's own drawing code and writes them, four times
//! enlarged, as PNG files: the mixer and the readouts in each screen style, the logo and a
//! notice. They go to the folder given as the argument, or to `kb16-oled` in the system's
//! temporary folder.
//!
//! Run from anywhere:  cargo run --manifest-path pad-core/Cargo.toml --example render_oled [folder]
//!
//! Each picture is the whole 128x32 frame buffer as the firmware sends it to the panel, at
//! rest, with the default 3-pixel top offset (the case hides the top rows).

use kb16_mixer_protocol::{LOGO_NAME_LEN, NOTICE};
use pad_core::oled::{Fb, H, Input, SPLASH_MS, Screen, W};
use pad_core::state::{DEFAULT_OLED_TOP, Mixer, Popup, STYLE_CLASSIC, STYLE_LEAN, STYLE_NUMBERS};
use std::path::PathBuf;

const SCALE: usize = 4;
/// A time well after the boot splash, when the logo has wiped in and its streak has passed.
const NOW: u64 = 18_600;

fn name<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    out[..s.len()].copy_from_slice(s.as_bytes());
    out
}

fn mixer(second_page: bool) -> Mixer {
    Mixer {
        n: 4,
        colors: [[255; 3]; 4],
        level: [0; 4],
        volume: [80, 55, 100, 30],
        mutes: 0b0100,
        selected: 0,
        mixer_keys: true,
        mic_muted: false,
        playing: true,
        second_page,
        names: [*b"GAME  ", *b"MUSIC ", *b"CHAT  ", *b"WEB   "],
    }
}

fn input(style: u8, alive: bool, mixer: Mixer, popup: Option<Popup<u64>>) -> Input<u64> {
    Input { alive, mixer, popup, idle: 0, name: name::<LOGO_NAME_LEN>("KB16"), meters: [200, 120, 0, 60], style }
}

fn draw(input: &Input<u64>) -> Fb {
    let mut fb = Fb::new(DEFAULT_OLED_TOP as i32);
    let mut screen = Screen::new(0);
    screen.draw(&mut fb, NOW, NOW + SPLASH_MS, input);
    fb
}

fn popup(volume: u8, muted: bool, text: &str) -> Popup<u64> {
    Popup { at: NOW, volume, muted, name: name::<16>(text) }
}

fn main() -> std::io::Result<()> {
    let out = std::env::args_os().nth(1).map_or_else(|| std::env::temp_dir().join("kb16-oled"), PathBuf::from);
    std::fs::create_dir_all(&out)?;
    let mut screens = vec![
        ("oled-logo".to_string(), input(STYLE_CLASSIC, false, mixer(false), None)),
        (
            "oled-notice".to_string(),
            input(STYLE_CLASSIC, true, mixer(false), Some(popup(NOTICE, false, "Config loaded"))),
        ),
    ];
    for (style, label) in [(STYLE_CLASSIC, "classic"), (STYLE_LEAN, "lean"), (STYLE_NUMBERS, "numbers")] {
        screens.extend([
            (format!("{label}-mixer"), input(style, true, mixer(false), None)),
            (format!("{label}-readout"), input(style, true, mixer(false), Some(popup(64, false, "Music")))),
            (format!("{label}-muted"), input(style, true, mixer(false), Some(popup(64, true, "Music")))),
            (format!("{label}-page2"), input(style, true, mixer(true), None)),
        ]);
    }
    for (file, input) in &screens {
        let path = out.join(format!("{file}.png"));
        std::fs::write(&path, png(&draw(input)))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// The frame buffer as a 1-bit greyscale PNG, `SCALE` times enlarged, lit pixels white.
fn png(fb: &Fb) -> Vec<u8> {
    let (w, h) = (W as usize * SCALE, H as usize * SCALE);
    let lit = |x: usize, y: usize| fb.buf[1 + (y / 8) * W as usize + x] & 1 << (y % 8) != 0;
    let mut raw = Vec::with_capacity(h * (1 + w / 8));
    for y in 0..h {
        raw.push(0); // filter: none
        for byte in 0..w / 8 {
            let mut b = 0u8;
            for bit in 0..8 {
                if lit((byte * 8 + bit) / SCALE, y / SCALE) {
                    b |= 0x80 >> bit;
                }
            }
            raw.push(b);
        }
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[1, 0, 0, 0, 0]); // bit depth 1, greyscale, deflate, no interlace
    let mut file = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut file, b"IHDR", &ihdr);
    chunk(&mut file, b"IDAT", &zlib_stored(&raw));
    chunk(&mut file, b"IEND", &[]);
    file
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// zlib with uncompressed deflate blocks: the pictures are small, and this needs no library.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(0xFFFF).collect();
    for (i, block) in blocks.iter().enumerate() {
        out.push((i + 1 == blocks.len()) as u8);
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&(b << 16 | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { crc >> 1 ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
