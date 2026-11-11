//! The only code that erases or programs flash. It writes the two reserved pages (logo name and
//! crash log) and nothing else: `pad_core::flash` checks every address first.
//!
//! Each step waits for BSY with a bound, checks and clears the error flags, and reads the
//! result back. The controller is locked again on every path out, also on an error. The panic
//! and hard-fault handlers write the crash log through here, so nothing in this file may panic
//! or wait without a limit.
//!
//! This uses the PAC, not embassy-stm32's flash driver: that driver needs the FLASH peripheral
//! handed over at start-up, and the panic handler must write without it.

use embassy_stm32::pac;
use pad_core::flash::{Error, PAGE_SIZE, check_erase, check_program};

/// Unlock sequence for FLASH_KEYR (RM0008).
const KEY1: u32 = 0x4567_0123;
const KEY2: u32 = 0xCDEF_89AB;
/// BSY polls before giving up. The CPU stalls on flash reads while the controller works, so
/// one poll lasts most of the operation; a page erase takes 40 ms at most. This bound is far
/// above any real wait at both 8 and 72 MHz and well below the ~2 s watchdog.
const WAIT_POLLS: u32 = 1_000_000;

fn wait_idle() -> Result<(), Error> {
    for _ in 0..WAIT_POLLS {
        if !pac::FLASH.sr().read().bsy() {
            return Ok(());
        }
    }
    Err(Error::Timeout)
}

/// Reads and clears the error and end-of-operation flags (each clears by writing 1).
/// The read-back after each step, not EOP, decides success: EOP is cleared but not required,
/// because its behaviour on this APM32 clone was never checked.
fn take_flags() -> Result<(), Error> {
    let sr = pac::FLASH.sr().read();
    pac::FLASH.sr().write(|w| {
        w.set_pgerr(true);
        w.set_wrprterr(true);
        w.set_eop(true);
    });
    if sr.wrprterr() {
        Err(Error::WriteProtected)
    } else if sr.pgerr() {
        Err(Error::Program)
    } else {
        Ok(())
    }
}

/// Runs `op` with the controller unlocked and interrupts off, then locks it again whatever
/// `op` returned.
fn unlocked(op: impl FnOnce() -> Result<(), Error>) -> Result<(), Error> {
    cortex_m::interrupt::free(|_| {
        let cr = pac::FLASH.cr();
        // Writing the keys while unlocked would lock the controller until the next reset.
        if cr.read().lock() {
            pac::FLASH.keyr().write_value(KEY1);
            pac::FLASH.keyr().write_value(KEY2);
        }
        let result = if cr.read().lock() {
            Err(Error::Locked)
        } else {
            wait_idle().and_then(|()| take_flags()).and_then(|()| op())
        };
        cr.write(|w| w.set_lock(true));
        result
    })
}

fn read(addr: usize) -> u16 {
    // SAFETY: callers pass an address that `check_erase` or `check_program` accepted: a
    // half-word-aligned address inside a reserved flash page. Flash is always mapped and
    // readable, and the volatile read stops the compiler from reusing a value read before the
    // write.
    unsafe { (addr as *const u16).read_volatile() }
}

/// Erases the 1 KB reserved page that starts at `page`. The CPU stalls for ~20 ms while it runs.
pub fn erase(page: usize) -> Result<(), Error> {
    check_erase(page)?;
    unlocked(|| {
        pac::FLASH.cr().write(|w| w.set_per(true));
        pac::FLASH.ar().write(|w| w.set_far(page as u32));
        pac::FLASH.cr().write(|w| {
            w.set_per(true);
            w.set_strt(true);
        });
        let done = wait_idle().and_then(|()| take_flags());
        pac::FLASH.cr().write(|w| w.set_per(false));
        done?;
        let blank = (page..page + PAGE_SIZE).step_by(2).all(|a| read(a) == 0xFFFF);
        if blank { Ok(()) } else { Err(Error::Verify) }
    })
}

/// Writes `words` from `addr` on, inside one reserved page. The target must be erased.
pub fn program(addr: usize, words: &[u16]) -> Result<(), Error> {
    check_program(addr, words.len())?;
    unlocked(|| {
        pac::FLASH.cr().write(|w| w.set_pg(true));
        let mut done = Ok(());
        for (i, &w) in words.iter().enumerate() {
            let a = addr + i * 2;
            // SAFETY: `check_program` put the whole range inside a reserved page, half-word
            // aligned, and PG is set, so this store programs one half-word of that page and
            // touches no other memory.
            unsafe { (a as *mut u16).write_volatile(w) };
            done = wait_idle().and_then(|()| take_flags());
            if done.is_ok() && read(a) != w {
                done = Err(Error::Verify);
            }
            if done.is_err() {
                break;
            }
        }
        pac::FLASH.cr().write(|w| w.set_pg(false));
        done
    })
}
