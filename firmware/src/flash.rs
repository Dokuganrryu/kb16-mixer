//! The only code that erases or programs flash. It writes the two reserved pages (logo name and
//! crash log) and nothing else.
//!
//! The steps and their order (unlock, wait, erase or program, read back, lock again on every
//! path out, also on an error) are in `pad_core::flash`, which also checks every address
//! first. This file only gives it the registers. The panic and hard-fault handlers write the
//! crash log through here, so nothing on this path may panic or wait without a limit.
//!
//! This uses the PAC, not embassy-stm32's flash driver: that driver needs the FLASH peripheral
//! handed over at start-up, and the panic handler must write without it.

use embassy_stm32::pac;
use pad_core::flash::{self, Controller, Error, Flags, check_program};

/// The real flash controller. Private, so only `erase` and `program` below drive it.
struct Regs;

impl Controller for Regs {
    fn locked(&mut self) -> bool {
        pac::FLASH.cr().read().lock()
    }

    fn key(&mut self, key: u32) {
        pac::FLASH.keyr().write_value(key);
    }

    fn lock(&mut self) {
        pac::FLASH.cr().write(|w| w.set_lock(true));
    }

    fn busy(&mut self) -> bool {
        pac::FLASH.sr().read().bsy()
    }

    fn take_flags(&mut self) -> Flags {
        let sr = pac::FLASH.sr().read();
        pac::FLASH.sr().write(|w| {
            w.set_pgerr(true);
            w.set_wrprterr(true);
            w.set_eop(true);
        });
        Flags { write_protected: sr.wrprterr(), program: sr.pgerr() }
    }

    fn start_erase(&mut self, page: usize) {
        pac::FLASH.cr().write(|w| w.set_per(true));
        pac::FLASH.ar().write(|w| w.set_far(page as u32));
        pac::FLASH.cr().write(|w| {
            w.set_per(true);
            w.set_strt(true);
        });
    }

    fn end_erase(&mut self) {
        pac::FLASH.cr().write(|w| w.set_per(false));
    }

    fn program_mode(&mut self, on: bool) {
        pac::FLASH.cr().write(|w| w.set_pg(on));
    }

    fn write(&mut self, addr: usize, value: u16) {
        // Checked again here, so that the store below is sound on its own.
        if check_program(addr, 1).is_ok() {
            // SAFETY: `check_program` put `addr` inside a reserved flash page, half-word
            // aligned, so this store programs (PG set) or is ignored by (PG clear) one
            // half-word of that page and touches no other memory.
            unsafe { (addr as *mut u16).write_volatile(value) };
        }
    }

    fn read(&mut self, addr: usize) -> u16 {
        if check_program(addr, 1).is_err() {
            return 0;
        }
        // SAFETY: `check_program` put `addr` inside a reserved flash page, half-word aligned.
        // Flash is always mapped and readable, and the volatile read stops the compiler from
        // reusing a value read before the write.
        unsafe { (addr as *const u16).read_volatile() }
    }
}

/// Erases the 1 KB reserved page that starts at `page`. The CPU stalls for ~20 ms while it
/// runs, with interrupts off.
pub fn erase(page: usize) -> Result<(), Error> {
    cortex_m::interrupt::free(|_| flash::erase(&mut Regs, page))
}

/// Writes `words` from `addr` on, inside one reserved page. The target must be erased.
pub fn program(addr: usize, words: &[u16]) -> Result<(), Error> {
    cortex_m::interrupt::free(|_| flash::program(&mut Regs, addr, words))
}
