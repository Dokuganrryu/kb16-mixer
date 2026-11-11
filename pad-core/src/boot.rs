//! The rule that parks a crashing firmware in the bootloader.
//!
//! The firmware counts crashed starts in backup register DR9 (`firmware/src/state.rs`). A start
//! after a watchdog reset or a panic adds one; ~10 s of healthy running sets it back to 0.
//! Three crashed starts in a row mean the image is broken, so the pad stays in the bootloader,
//! where the PC can always flash it again.

pub const MAX_CRASHES: u32 = 3;

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
