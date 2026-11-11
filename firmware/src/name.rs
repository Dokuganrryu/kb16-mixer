//! The name on the logo screen, kept in its own flash page (0x0801F400, just below the crash
//! log) so the pad shows it at power-up, before the PC app connects.
//!
//! Record, 16-bit words: MAGIC, length, the name (LEN bytes, two per word), checksum. Anything
//! that does not check out (a blank page, a torn write, a foreign layout) reads as `DEFAULT`.

use crate::diag;
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use embassy_time::Instant;
use kb16_mixer_protocol::LOGO_NAME_LEN as LEN;

const PAGE: usize = 0x0801_F400;
const MAGIC: u16 = 0x4E4D; // "NM"
const WORDS: usize = 2 + LEN / 2 + 1;
pub const DEFAULT: [u8; LEN] = *b"KB16\0\0\0\0\0\0";
/// Flash wears out: should a host ever send a different name every few seconds, only one
/// write per this long goes through (the app resends every 30 s, so the latest still lands).
const MIN_GAP_S: u32 = 60;
/// And at most this many per power-up, so even a host that keeps changing the name for days
/// costs a handful of erases. Past it the new name still shows, from RAM, until the next start.
const MAX_WRITES: u8 = 8;

/// Seconds since boot of the last write, plus one (0 = none yet).
static LAST_WRITE: AtomicU32 = AtomicU32::new(0);
static WRITES: AtomicU8 = AtomicU8::new(0);

/// Capitals, digits and spaces, zero-padded, with at least one letter or digit: what the logo
/// font can draw.
pub fn valid(name: &[u8; LEN]) -> bool {
    let end = name.iter().position(|&b| b == 0).unwrap_or(LEN);
    let (text, pad) = name.split_at(end);
    pad.iter().all(|&b| b == 0)
        && text.iter().all(|&b| b == b' ' || b.is_ascii_uppercase() || b.is_ascii_digit())
        && text.iter().any(|&b| b != b' ')
}

fn record(name: &[u8; LEN]) -> [u16; WORDS] {
    let mut w = [0u16; WORDS];
    w[0] = MAGIC;
    w[1] = LEN as u16;
    for (i, pair) in name.chunks(2).enumerate() {
        w[2 + i] = pair[0] as u16 | (pair[1] as u16) << 8;
    }
    w[WORDS - 1] = checksum(&w[..WORDS - 1]);
    w
}

/// Fletcher-16: catches a swapped or half-written word, which a plain sum would not.
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

fn stored() -> [u16; WORDS] {
    core::array::from_fn(|i| unsafe { ((PAGE + i * 2) as *const u16).read_volatile() })
}

/// The stored name, or `DEFAULT` if there is none that checks out.
pub fn load() -> [u8; LEN] {
    let w = stored();
    if w[0] != MAGIC || w[1] != LEN as u16 || w[WORDS - 1] != checksum(&w[..WORDS - 1]) {
        return DEFAULT;
    }
    let mut name = [0u8; LEN];
    for (i, pair) in name.chunks_mut(2).enumerate() {
        pair.copy_from_slice(&w[2 + i].to_le_bytes());
    }
    if valid(&name) { name } else { DEFAULT }
}

/// Keeps `name` for the next power-up. Writes only when it differs from what is stored.
pub fn save(name: &[u8; LEN]) {
    let rec = record(name);
    if !valid(name) || stored() == rec {
        return;
    }
    let now = Instant::now().as_secs() as u32 + 1;
    let last = LAST_WRITE.load(Ordering::Relaxed);
    let writes = WRITES.load(Ordering::Relaxed);
    if (last != 0 && now - last < MIN_GAP_S) || writes >= MAX_WRITES {
        return;
    }
    LAST_WRITE.store(now, Ordering::Relaxed);
    WRITES.store(writes + 1, Ordering::Relaxed);
    diag::erase_page(PAGE);
    diag::program(PAGE, &rec);
}
