//! The backup-register marks that decide what runs after a reset, and the rule that parks a
//! crashing firmware in the bootloader.
//!
//! The stm32duino bootloader reads backup register DR10 after every reset: `STAY_IN_DFU` keeps
//! it in DFU mode, `START_APP` starts the firmware at once. Backup registers survive a reset but
//! not a power cycle. The firmware keeps DR10 at `START_APP`, so a watchdog reset or a panic
//! restarts the firmware by itself. DR8 marks a panic restart, and DR9 counts crashed starts:
//! a start after a watchdog reset or a panic adds one, ~10 s of healthy running sets it back to
//! 0. Three crashed starts in a row mean the image is broken, so the pad stays in the
//! bootloader, where the PC can always flash it again.
//!
//! The bootloader reads `STAY_IN_DFU` and `START_APP`, so their values can never change.
//! `firmware/src/state.rs` gives these functions the real registers and does the reset.

/// A backup data register by the index that the PAC's `BKP.dr(n)` takes: index n is DRn+1 in
/// the reference manual.
pub type Reg = usize;
/// DR8: `PANIC_MARK` here means that the last reset was a panic or fault restart.
pub const PANIC_REG: Reg = 7;
/// DR9: the count of crashed starts in a row.
pub const CRASHES_REG: Reg = 8;
/// DR10: what the bootloader does after the next reset.
pub const MODE_REG: Reg = 9;

/// DR10 value: the bootloader stays in DFU mode.
pub const STAY_IN_DFU: u16 = 0x424C;
/// DR10 value: the bootloader starts the firmware at once.
pub const START_APP: u16 = 0x424D;
/// DR8 value: the last reset was a panic or fault restart ("PA").
pub const PANIC_MARK: u16 = 0x5041;

pub const MAX_CRASHES: u32 = 3;

/// The backup registers.
pub trait Backup {
    fn read(&mut self, reg: Reg) -> u16;
    fn write(&mut self, reg: Reg, value: u16);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Boot {
    /// Start the firmware, and keep this crash count.
    Run { crashes: u32 },
    /// Stay in the bootloader (and set the count back to 0).
    Bootloader,
}

/// What to do at power-up or after a reset. `crashed`: this start follows a watchdog reset or
/// a panic. `before`: the crash count that the last start left in DR9 (a 16-bit register).
pub fn after_reset(crashed: bool, before: u16) -> Boot {
    let crashes = if crashed { before as u32 + 1 } else { 0 };
    if crashes >= MAX_CRASHES { Boot::Bootloader } else { Boot::Run { crashes } }
}

/// The first thing at boot. `watchdog_reset`: the reset flags say the watchdog fired. Writes
/// the marks for the decision; on `Boot::Bootloader` the caller must reset at once.
pub fn start(regs: &mut impl Backup, watchdog_reset: bool) -> Boot {
    let crashed = watchdog_reset || regs.read(PANIC_REG) == PANIC_MARK;
    regs.write(PANIC_REG, 0);
    let boot = after_reset(crashed, regs.read(CRASHES_REG));
    match boot {
        Boot::Bootloader => {
            regs.write(CRASHES_REG, 0);
            enter_bootloader(regs);
        }
        Boot::Run { crashes } => {
            // Below MAX_CRASHES (3), so it fits the 16-bit register.
            regs.write(CRASHES_REG, crashes as u16);
            regs.write(MODE_REG, START_APP);
        }
    }
    boot
}

/// The firmware has run long enough to count as healthy.
pub fn healthy(regs: &mut impl Backup) {
    regs.write(CRASHES_REG, 0);
}

/// The marks for a restart after a panic or fault; the caller resets next. `start` counts it.
pub fn crash(regs: &mut impl Backup) {
    regs.write(PANIC_REG, PANIC_MARK);
    regs.write(MODE_REG, START_APP);
}

/// The mark that keeps the bootloader in DFU mode; the caller resets next. Every way into the
/// bootloader comes here: the BOOT report, the key held at plug-in, the four-corner combo and
/// the crash park in `start`.
pub fn enter_bootloader(regs: &mut impl Backup) {
    regs.write(MODE_REG, STAY_IN_DFU);
}
