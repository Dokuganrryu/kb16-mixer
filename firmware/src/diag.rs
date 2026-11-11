//! Crash log in a spare flash page (0x0801F800), readable over DFU upload after the firmware
//! dies. Entries are 16-bit words, written through `flash.rs`.
//!
//! Always on: `crash()` appends panic / hard-fault details.
//! With the `diag` cargo feature: the page is erased at every boot and `log()` records how far
//! start-up got, which is how the first boot problems on this board were found.

use crate::flash;
use pad_core::flash::LOG_PAGE as LOG;

const ENTRIES: usize = 512;
const BLANK: u16 = 0xFFFF;

fn entry(i: usize) -> u16 {
    // SAFETY: i < ENTRIES, so the address is a half-word-aligned word of the crash-log flash
    // page, which is always mapped and readable.
    unsafe { ((LOG + i * 2) as *const u16).read_volatile() }
}

/// Appends one word after the last used entry. A full log or a failed write drops the word:
/// a crash log must never cause a crash of its own.
fn append(v: u16) {
    cortex_m::interrupt::free(|_| {
        let Some(i) = (0..ENTRIES).find(|&i| entry(i) == BLANK) else { return };
        let _ = flash::program(LOG + i * 2, &[v]);
    });
}

pub fn init() {
    // Start each diag boot clean; in a normal build only clear leftovers from a diag build
    // (those begin with a 0x01xx stage marker), so crash entries accumulate.
    let first = entry(0);
    if cfg!(feature = "diag") && first != BLANK || first & 0xFF00 == 0x0100 {
        let _ = flash::erase(LOG);
    }
}

/// Start-up breadcrumb; compiled out unless the `diag` feature is on.
pub fn log(v: u16) {
    if cfg!(feature = "diag") {
        append(v);
    }
}

pub fn crash(v: u16) {
    append(v);
}

pub fn crash32(v: u32) {
    append(v as u16);
    append((v >> 16) as u16);
}
