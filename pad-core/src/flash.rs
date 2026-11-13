//! Which flash the firmware may erase and program (the logo-name page and the crash-log page),
//! and the steps that do it.
//!
//! Everything else is off limits: the bootloader (0x08000000..0x08002000), the firmware image,
//! and the spare last page (0x0801FC00). A wrong address must cost a refused write, never a
//! bricked pad. `erase` and `program` check the address before they touch the controller, and
//! lock it again on every path out. The firmware (`firmware/src/flash.rs`) gives them the
//! registers through `Controller`.

/// Flash page size of the APM32F103CB.
pub const PAGE_SIZE: usize = 1024;
/// The logo name (`firmware/src/name.rs`).
pub const NAME_PAGE: usize = 0x0801_F400;
/// The crash log (`firmware/src/diag.rs`).
pub const LOG_PAGE: usize = 0x0801_F800;
const RESERVED: [usize; 2] = [NAME_PAGE, LOG_PAGE];

/// Why a flash erase or program did not happen, or did not come out right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// The range is not inside one of the two reserved pages.
    Outside,
    /// A program address that is not on a half-word boundary, or an erase address that is not
    /// the first byte of a page.
    Unaligned,
    /// The flash controller stayed busy longer than any erase or program can take.
    Timeout,
    /// The unlock keys did not unlock the flash controller.
    Locked,
    /// The controller reported a write to protected flash (FLASH_SR.WRPRTERR).
    WriteProtected,
    /// The controller reported a program error, such as a target that was not erased
    /// (FLASH_SR.PGERR).
    Program,
    /// The flash does not read back what was written.
    Verify,
}

/// The reserved page that holds `addr`, if any.
fn page_of(addr: usize) -> Option<usize> {
    RESERVED.into_iter().find(|&p| (p..p + PAGE_SIZE).contains(&addr))
}

/// May the page that starts at `page` be erased?
pub fn check_erase(page: usize) -> Result<(), Error> {
    match page_of(page) {
        None => Err(Error::Outside),
        Some(p) if p != page => Err(Error::Unaligned),
        Some(_) => Ok(()),
    }
}

/// May `words` half-words be programmed from `addr` on? The whole range must lie inside one
/// reserved page.
pub fn check_program(addr: usize, words: usize) -> Result<(), Error> {
    let page = page_of(addr).ok_or(Error::Outside)?;
    let end = words.checked_mul(2).and_then(|n| addr.checked_add(n)).ok_or(Error::Outside)?;
    if end > page + PAGE_SIZE {
        return Err(Error::Outside);
    }
    if !addr.is_multiple_of(2) {
        return Err(Error::Unaligned);
    }
    Ok(())
}

/// Unlock sequence for FLASH_KEYR (RM0008).
pub const KEY1: u32 = 0x4567_0123;
pub const KEY2: u32 = 0xCDEF_89AB;
/// Longest page erase, in microseconds: tERASE max in the STM32F103xB datasheet (DS5319). The
/// APM32F103 clone is taken to be as slow; its own figure was not checked.
pub const ERASE_MAX_US: u32 = 40_000;
/// Fastest CPU clock, in MHz (HCLK at 72 MHz once the PLL runs).
pub const CPU_MAX_MHZ: u32 = 72;
/// Fewest CPU cycles one BSY poll can take on the Cortex-M3: a 2-cycle load of FLASH_SR and a
/// 2-cycle test and taken branch, with no flash wait state counted.
pub const POLL_MIN_CYCLES: u32 = 4;
/// BSY polls before giving up: twice the longest erase even when every poll is as short as it
/// can be at full speed. In practice the CPU stalls on its own flash fetches while the controller
/// works, so one poll lasts most of the operation. At the 8 MHz reset clock (the panic handler
/// can run there) the same count lasts 0.72 s, below the watchdog's 1.33 s at the fastest LSI.
pub const WAIT_POLLS: u32 = 2 * ERASE_MAX_US * CPU_MAX_MHZ / POLL_MIN_CYCLES;

/// FLASH_SR error flags, as read before they were cleared.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Flags {
    pub write_protected: bool,
    pub program: bool,
}

/// The flash controller registers that `erase` and `program` drive, one method per register
/// access. The firmware implements it on the real controller (`firmware/src/flash.rs`); the
/// tests implement it with a fake, so the order of the steps, and the lock on every path out,
/// are checked on the PC.
pub trait Controller {
    /// FLASH_CR.LOCK.
    fn locked(&mut self) -> bool;
    /// Writes one unlock key to FLASH_KEYR.
    fn key(&mut self, key: u32);
    /// Sets FLASH_CR.LOCK.
    fn lock(&mut self);
    /// FLASH_SR.BSY.
    fn busy(&mut self) -> bool;
    /// Reads FLASH_SR, clears PGERR, WRPRTERR and EOP (each clears by writing 1), and returns
    /// the error flags as read.
    fn take_flags(&mut self) -> Flags;
    /// Sets FLASH_CR.PER, puts `page` in FLASH_AR, then sets PER and STRT.
    fn start_erase(&mut self, page: usize);
    /// Writes FLASH_CR with PER clear.
    fn end_erase(&mut self);
    /// Writes FLASH_CR with PG set (`true`) or clear.
    fn program_mode(&mut self, on: bool);
    /// Stores one half-word at `addr`; with PG set, that programs it.
    fn write(&mut self, addr: usize, value: u16);
    /// Reads one half-word of flash.
    fn read(&mut self, addr: usize) -> u16;
}

fn wait_idle(c: &mut impl Controller) -> Result<(), Error> {
    for _ in 0..WAIT_POLLS {
        if !c.busy() {
            return Ok(());
        }
    }
    Err(Error::Timeout)
}

/// The read-back after each step, not EOP, decides success: EOP is cleared but not required,
/// because its behaviour on this APM32 clone was never checked.
fn take_flags(c: &mut impl Controller) -> Result<(), Error> {
    let f = c.take_flags();
    if f.write_protected {
        Err(Error::WriteProtected)
    } else if f.program {
        Err(Error::Program)
    } else {
        Ok(())
    }
}

/// Runs `op` with the controller unlocked, then locks it again whatever `op` returned.
fn unlocked<C: Controller>(c: &mut C, op: impl FnOnce(&mut C) -> Result<(), Error>) -> Result<(), Error> {
    // Writing the keys while unlocked would lock the controller until the next reset.
    if c.locked() {
        c.key(KEY1);
        c.key(KEY2);
    }
    let result =
        if c.locked() { Err(Error::Locked) } else { wait_idle(c).and_then(|()| take_flags(c)).and_then(|()| op(c)) };
    c.lock();
    result
}

/// Erases the reserved page that starts at `page` and checks that it reads blank.
pub fn erase(c: &mut impl Controller, page: usize) -> Result<(), Error> {
    check_erase(page)?;
    unlocked(c, |c| {
        c.start_erase(page);
        let done = wait_idle(c).and_then(|()| take_flags(c));
        c.end_erase();
        done?;
        let blank = (page..page + PAGE_SIZE).step_by(2).all(|a| c.read(a) == 0xFFFF);
        if blank { Ok(()) } else { Err(Error::Verify) }
    })
}

/// Writes `words` from `addr` on, inside one reserved page, and reads each one back. The
/// target must be erased.
pub fn program(c: &mut impl Controller, addr: usize, words: &[u16]) -> Result<(), Error> {
    check_program(addr, words.len())?;
    unlocked(c, |c| {
        c.program_mode(true);
        let mut done = Ok(());
        for (i, &w) in words.iter().enumerate() {
            let a = addr + i * 2;
            c.write(a, w);
            done = wait_idle(c).and_then(|()| take_flags(c));
            if done.is_ok() && c.read(a) != w {
                done = Err(Error::Verify);
            }
            if done.is_err() {
                break;
            }
        }
        c.program_mode(false);
        done
    })
}
