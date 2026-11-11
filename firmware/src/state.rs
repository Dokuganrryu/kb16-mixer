//! State shared between tasks, and the reset / bootloader logic. What the state holds and how a
//! host report changes it is in `pad_core::state`.

use core::cell::RefCell;
use core::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use embassy_stm32::pac;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_time::Instant;
use kb16_mixer_protocol::HostMsg;
use pad_core::boot::{Boot, after_reset};
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
// Resets and the bootloader.
//
// The stm32duino bootloader reads backup register DR10 after every reset:
//   0x424C  stay in DFU          0x424D  start the app at once
// Backup registers survive resets but not a power cycle. We keep DR10 at "start the app" so a
// watchdog reset or panic restarts the firmware by itself, and count those restarts in DR9
// (the rule is `pad_core::boot`). DR8 marks a panic restart.

/// BKP data registers by index: `dr(n)` is DRn+1 in the reference manual.
const DR8: usize = 7;
const DR9: usize = 8;
const DR10: usize = 9;
const STAY_IN_DFU: u16 = 0x424C;
const START_APP: u16 = 0x424D;
const PANIC_MARK: u16 = 0x5041;

fn backup_access() {
    pac::RCC.apb1enr().modify(|w| {
        w.set_pwren(true);
        w.set_bkpen(true);
    });
    pac::PWR.cr().modify(|w| w.set_dbp(true));
}

fn backup_write(n: usize, v: u16) {
    pac::BKP.dr(n).write(|w| w.set_d(v));
}

fn backup_read(n: usize) -> u16 {
    pac::BKP.dr(n).read().d()
}

/// Call first thing at boot. Parks in DFU after repeated crashes; otherwise arms self-restart.
pub fn boot_guard() {
    backup_access();
    let csr = pac::RCC.csr().read();
    let crashed = csr.iwdgrstf() || backup_read(DR8) == PANIC_MARK;
    pac::RCC.csr().modify(|w| w.set_rmvf(true));
    backup_write(DR8, 0);
    match after_reset(crashed, backup_read(DR9)) {
        Boot::Bootloader => {
            backup_write(DR9, 0);
            enter_bootloader();
        }
        // Below MAX_CRASHES (3), so it fits the 16-bit register.
        Boot::Run { crashes } => backup_write(DR9, crashes as u16),
    }
    backup_write(DR10, START_APP);
}

/// The firmware has run long enough to count as healthy.
pub fn mark_healthy() {
    backup_write(DR9, 0);
}

/// Restart the firmware after a panic or fault (counted by `boot_guard`).
pub fn restart_after_crash() -> ! {
    cortex_m::interrupt::disable();
    backup_access();
    backup_write(DR8, PANIC_MARK);
    backup_write(DR10, START_APP);
    cortex_m::peripheral::SCB::sys_reset()
}

/// Restart into the bootloader's DFU mode: write the stay-in-bootloader mark that the
/// stm32duino bootloader reads from BKP_DR10, then reset.
pub fn enter_bootloader() -> ! {
    cortex_m::interrupt::disable();
    backup_access();
    backup_write(DR10, STAY_IN_DFU);
    cortex_m::peripheral::SCB::sys_reset()
}
