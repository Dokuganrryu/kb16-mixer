//! The name on the logo screen, kept in its own flash page (0x0801F400, just below the crash
//! log) so the pad shows it at power-up, before the PC app connects. The record format and the
//! write limits are in `pad_core::name`.

use crate::flash;
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use embassy_time::Instant;
use kb16_mixer_protocol::LOGO_NAME_LEN as LEN;
use pad_core::flash::NAME_PAGE as PAGE;
use pad_core::name::{WORDS, decode, may_write, record, valid};

/// Seconds since boot of the last write, plus one (0 = none yet).
static LAST_WRITE: AtomicU32 = AtomicU32::new(0);
static WRITES: AtomicU8 = AtomicU8::new(0);

fn stored() -> [u16; WORDS] {
    // SAFETY: i < WORDS (8), so every address is a half-word-aligned word near the start of the
    // logo-name flash page, which is always mapped and readable.
    core::array::from_fn(|i| unsafe { ((PAGE + i * 2) as *const u16).read_volatile() })
}

/// The stored name, or the default if there is none that checks out.
pub fn load() -> [u8; LEN] {
    decode(&stored())
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
    if !may_write(last, writes, now) {
        return;
    }
    LAST_WRITE.store(now, Ordering::Relaxed);
    WRITES.store(writes + 1, Ordering::Relaxed);
    // A failed write leaves a record that does not check out, which loads as the default name;
    // the name still shows from RAM until the next start.
    if flash::erase(PAGE).is_ok() {
        let _ = flash::program(PAGE, &rec);
    }
}
