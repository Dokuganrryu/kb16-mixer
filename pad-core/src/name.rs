//! The name on the logo screen: what may be shown, and how it is kept in its flash page.
//!
//! Record, 16-bit words: MAGIC, length, the name (LEN bytes, two per word), checksum. Anything
//! that does not check out (a blank page, a torn write, a foreign layout) reads as `DEFAULT`.

use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use kb16_mixer_protocol::LOGO_NAME_LEN as LEN;

const MAGIC: u16 = 0x4E4D; // "NM"
/// Words in one record.
pub const WORDS: usize = 2 + LEN / 2 + 1;
pub const DEFAULT: [u8; LEN] = *b"KB16\0\0\0\0\0\0";
/// Flash wears out: should a host ever send a different name every few seconds, only one
/// write per this long goes through (the app resends every 30 s, so the latest still lands).
const MIN_GAP_S: u32 = 60;
/// And at most this many per power-up, so even a host that keeps changing the name for days
/// costs a handful of erases. Past it the new name still shows, from RAM, until the next start.
pub const MAX_WRITES: u8 = 8;

/// Capitals, digits and spaces, zero-padded, with at least one letter or digit: what the logo
/// font can draw.
pub fn valid(name: &[u8; LEN]) -> bool {
    let end = name.iter().position(|&b| b == 0).unwrap_or(LEN);
    let (text, pad) = name.split_at(end);
    pad.iter().all(|&b| b == 0)
        && text.iter().all(|&b| b == b' ' || b.is_ascii_uppercase() || b.is_ascii_digit())
        && text.iter().any(|&b| b != b' ')
}

/// The flash record for `name`.
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

/// The name in a stored record, or `DEFAULT` if the record does not check out.
pub fn decode(w: &[u16; WORDS]) -> [u8; LEN] {
    if w[0] != MAGIC || w[1] != LEN as u16 || w[WORDS - 1] != checksum(&w[..WORDS - 1]) {
        return DEFAULT;
    }
    let mut name = [0u8; LEN];
    for (i, pair) in name.chunks_mut(2).enumerate() {
        pair.copy_from_slice(&w[2 + i].to_le_bytes());
    }
    if valid(&name) { name } else { DEFAULT }
}

/// May a new name be written now? `last` and `now` are seconds since boot plus one (`last` = 0:
/// no write yet); `writes` counts the writes since boot.
pub fn may_write(last: u32, writes: u8, now: u32) -> bool {
    // Wrapping, as in the release build that first shipped this rule.
    !((last != 0 && now.wrapping_sub(last) < MIN_GAP_S) || writes >= MAX_WRITES)
}

/// The write limit's memory since boot. The firmware keeps one in a `static` and asks it before
/// every write, so the limit holds wherever the write comes from.
pub struct Writes {
    /// Seconds since boot of the last write, plus one (0 = none yet).
    last: AtomicU32,
    count: AtomicU8,
}

impl Default for Writes {
    fn default() -> Writes {
        Writes::new()
    }
}

impl Writes {
    pub const fn new() -> Writes {
        Writes { last: AtomicU32::new(0), count: AtomicU8::new(0) }
    }

    /// The record to write for `name` at `now` (seconds since boot plus one), or `None` when
    /// the name is not valid, is the one `stored` already, or the limit says no. A yes counts
    /// as a write, whether or not the flash write then works: a failing page must not be
    /// erased again and again.
    pub fn admit(&self, name: &[u8; LEN], stored: &[u16; WORDS], now: u32) -> Option<[u16; WORDS]> {
        let rec = record(name);
        let (last, count) = (self.last.load(Ordering::Relaxed), self.count.load(Ordering::Relaxed));
        if !valid(name) || *stored == rec || !may_write(last, count, now) {
            return None;
        }
        self.last.store(now, Ordering::Relaxed);
        self.count.store(count + 1, Ordering::Relaxed);
        Some(rec)
    }
}
