//! Which flash the firmware may erase and program: the logo-name page and the crash-log page.
//!
//! Everything else is off limits: the bootloader (0x08000000..0x08002000), the firmware image,
//! and the spare last page (0x0801FC00). A wrong address must cost a refused write, never a
//! bricked pad. The firmware's flash driver (`firmware/src/flash.rs`) asks these functions
//! before it touches the flash controller.

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
