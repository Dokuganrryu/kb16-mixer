//! State shared between tasks, and the reset / bootloader logic. What the state holds and how a
//! host report changes it is in `pad_core::state`.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use embassy_stm32::pac;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;
use kb16_mixer_protocol::HostMsg;
use pad_core::boot::{self, Boot};
pub use pad_core::state::{After, NCH};

/// The firmware clock, as `pad_core` reads it.
#[derive(Clone, Copy)]
pub struct Tick(pub Instant);

impl Tick {
    pub fn now() -> Tick {
        Tick(Instant::now())
    }
}

/// embassy-time counts at 1 MHz here, a whole multiple of 1 kHz, so comparing whole
/// milliseconds gives the same answer as comparing `Duration`s.
impl pad_core::Time for Tick {
    fn ms(self) -> u64 {
        self.0.as_millis()
    }

    fn ms_since(self, earlier: Tick) -> u64 {
        self.0.saturating_duration_since(earlier.0).as_millis()
    }
}

pub type State = pad_core::state::State<Tick>;

/// Set while the USB bus is suspended (PC asleep or off): lights and screen go dark.
pub static SUSPENDED: AtomicBool = AtomicBool::new(false);

pub fn suspended() -> bool {
    SUSPENDED.load(Ordering::Relaxed)
}

static STATE: Mutex<CriticalSectionRawMutex, RefCell<State>> =
    Mutex::new(RefCell::new(State::new(Tick(Instant::from_ticks(0)))));

/// Meter levels after ballistics (8.8 fixed point), written by the LED task, read by the OLED task.
pub static METER: [AtomicU16; NCH] = [const { AtomicU16::new(0) }; NCH];

pub fn meter(i: usize) -> u8 {
    (METER[i].load(Ordering::Relaxed) >> 8) as u8
}

pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.lock(|s| f(&mut s.borrow_mut()))
}

/// Applies one report from the host.
pub fn handle_report(r: &[u8]) -> After {
    let Some(msg) = HostMsg::decode(r) else { return After::Nothing };
    let now = Tick::now();
    with(|s| s.handle(msg, now))
}

// ---------------------------------------------------------------------------
// Resets and the bootloader. The marks and their order are in `pad_core::boot`; this part gives
// it the backup registers and does the resets.

/// The BKP data registers, through the PAC.
struct Bkp;

impl Bkp {
    /// Turns on the backup domain clock and write access. Every write path calls this first.
    fn open() -> Bkp {
        pac::RCC.apb1enr().modify(|w| {
            w.set_pwren(true);
            w.set_bkpen(true);
        });
        pac::PWR.cr().modify(|w| w.set_dbp(true));
        Bkp
    }
}

impl boot::Backup for Bkp {
    fn read(&mut self, reg: boot::Reg) -> u16 {
        pac::BKP.dr(reg).read().d()
    }

    fn write(&mut self, reg: boot::Reg, value: u16) {
        pac::BKP.dr(reg).write(|w| w.set_d(value));
    }
}

/// Call first thing at boot. Parks in DFU after repeated crashes; otherwise arms self-restart.
pub fn boot_guard() {
    let mut regs = Bkp::open();
    let watchdog_reset = pac::RCC.csr().read().iwdgrstf();
    pac::RCC.csr().modify(|w| w.set_rmvf(true));
    if boot::start(&mut regs, watchdog_reset) == Boot::Bootloader {
        reset();
    }
}

/// The firmware has run long enough to count as healthy.
pub fn mark_healthy() {
    boot::healthy(&mut Bkp::open());
}

/// Restart the firmware after a panic or fault (counted by `boot_guard`).
pub fn restart_after_crash() -> ! {
    cortex_m::interrupt::disable();
    boot::crash(&mut Bkp::open());
    reset()
}

/// Restart into the bootloader's DFU mode.
pub fn enter_bootloader() -> ! {
    cortex_m::interrupt::disable();
    boot::enter_bootloader(&mut Bkp::open());
    reset()
}

/// Interrupts off first, so no handler runs between the marks and the reset.
fn reset() -> ! {
    cortex_m::interrupt::disable();
    cortex_m::peripheral::SCB::sys_reset()
}
