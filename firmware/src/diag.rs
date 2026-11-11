//! Crash log in a spare flash page (0x0801F800), readable over DFU upload after the firmware
//! dies. Entries are 16-bit words. The flash routines here also keep the logo name (name.rs).
//!
//! Always on: `crash()` appends panic / hard-fault details.
//! With the `diag` cargo feature: the page is erased at every boot and `log()` records how far
//! start-up got, which is how the first boot problems on this board were found.

const LOG: usize = 0x0801_F800;
const ENTRIES: usize = 512;
const BLANK: u16 = 0xFFFF;
const FLASH_KEYR: *mut u32 = 0x4002_2004 as *mut u32;
const FLASH_SR: *mut u32 = 0x4002_200C as *mut u32;
const FLASH_CR: *mut u32 = 0x4002_2010 as *mut u32;
const FLASH_AR: *mut u32 = 0x4002_2014 as *mut u32;
const CR_PG: u32 = 1 << 0;
const CR_PER: u32 = 1 << 1;
const CR_STRT: u32 = 1 << 6;
const CR_LOCK: u32 = 1 << 7;
const SR_BSY: u32 = 1 << 0;

fn slot(i: usize) -> *mut u16 {
    (LOG + i * 2) as *mut u16
}

/// Waits for the flash controller and clears its status flags.
unsafe fn settle() {
    unsafe {
        while FLASH_SR.read_volatile() & SR_BSY != 0 {}
        FLASH_SR.write_volatile(0x34);
    }
}

unsafe fn unlock() {
    unsafe {
        if FLASH_CR.read_volatile() & CR_LOCK != 0 {
            FLASH_KEYR.write_volatile(0x4567_0123);
            FLASH_KEYR.write_volatile(0xCDEF_89AB);
        }
    }
}

/// Erases the 1 KB flash page at `page`. The CPU stalls for ~20 ms while it runs.
pub fn erase_page(page: usize) {
    cortex_m::interrupt::free(|_| unsafe {
        unlock();
        settle();
        FLASH_CR.write_volatile(CR_PER);
        FLASH_AR.write_volatile(page as u32);
        FLASH_CR.write_volatile(CR_PER | CR_STRT);
        settle();
        FLASH_CR.write_volatile(0);
    });
}

/// Writes `words` from `addr` on, which must be erased.
pub fn program(addr: usize, words: &[u16]) {
    cortex_m::interrupt::free(|_| unsafe {
        unlock();
        settle();
        FLASH_CR.write_volatile(CR_PG);
        for (i, &w) in words.iter().enumerate() {
            ((addr + i * 2) as *mut u16).write_volatile(w);
            settle();
        }
        FLASH_CR.write_volatile(0);
    });
}

/// Appends one word after the last used entry.
fn append(v: u16) {
    cortex_m::interrupt::free(|_| {
        let Some(i) = (0..ENTRIES).find(|&i| unsafe { slot(i).read_volatile() } == BLANK) else { return };
        program(LOG + i * 2, &[v]);
    });
}

pub fn init() {
    // Start each diag boot clean; in a normal build only clear leftovers from a diag build
    // (those begin with a 0x01xx stage marker), so crash entries accumulate.
    let first = unsafe { slot(0).read_volatile() };
    if cfg!(feature = "diag") && first != BLANK || first & 0xFF00 == 0x0100 {
        erase_page(LOG);
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
