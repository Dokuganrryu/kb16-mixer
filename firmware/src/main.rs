//! KB16 mixer firmware for the DOIO KB16-01 rev2 (APM32F103CB).
//!
//! Tasks: USB device, 1 kHz key/knob scan, input routing, host reports, 200 Hz LEDs, 30 Hz OLED.
//! With the PC app running, keys and knobs drive the mixer over raw HID (usage page 0xFF60);
//! without it, the pad is a plain keyboard with media knobs.
//!
//! Recovery: hold the top-left key while plugging in (or all four corner keys for 2 s, or
//! `kb16-mixer.exe --bootloader`) to restart into the bootloader. A panic or fault restarts the
//! firmware; after three crashed starts in a row it stays in the bootloader instead.

#![no_std]
#![no_main]
#![deny(clippy::undocumented_unsafe_blocks)]

mod diag;
mod flash;
mod keys;
mod leds;
mod name;
mod oled;
mod state;

use core::sync::atomic::{AtomicBool, Ordering};
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Flex, Input, Level, Output, Pull, Speed};
use embassy_stm32::i2c::{self, I2c};
use embassy_stm32::peripherals;
use embassy_stm32::time::Hertz;
use embassy_stm32::usb::{self, Driver};
use embassy_stm32::{Config, bind_interrupts, dma, pac};
use embassy_time::{Duration, Timer, with_timeout};
use embassy_usb::Builder;
use embassy_usb::class::hid::{self, HidBootProtocol, HidReaderWriter, HidSubclass, HidWriter};
use pad_core::route::{Action, Event, OwedUps, Router};
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USB_LP_CAN1_RX0 => usb::InterruptHandler<peripherals::USB>;
    I2C2_EV => i2c::EventInterruptHandler<peripherals::I2C2>;
    I2C2_ER => i2c::ErrorInterruptHandler<peripherals::I2C2>;
    DMA1_CHANNEL4 => dma::InterruptHandler<peripherals::DMA1_CH4>;
    DMA1_CHANNEL5 => dma::InterruptHandler<peripherals::DMA1_CH5>;
});

type UsbDriver = Driver<'static, peripherals::USB>;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    diag::crash(0xDEAD);
    if let Some(loc) = info.location() {
        diag::crash(loc.line() as u16);
        let f = loc.file().as_bytes();
        let tail = &f[f.len().saturating_sub(8)..];
        for pair in tail.chunks(2) {
            diag::crash(pair[0] as u16 | (*pair.get(1).unwrap_or(&0) as u16) << 8);
        }
    }
    state::restart_after_crash()
}

// cortex-m-rt requires the hard-fault handler to be an `unsafe fn`.
#[cortex_m_rt::exception]
unsafe fn HardFault(ef: &cortex_m_rt::ExceptionFrame) -> ! {
    diag::crash(0xFA17);
    diag::crash32(ef.pc());
    diag::crash32(ef.lr());
    // SAFETY: SCB::PTR points at the always-mapped System Control Block; reading the fault
    // status (CFSR) and bus-fault address (BFAR) registers has no side effects.
    let scb = unsafe { &*cortex_m::peripheral::SCB::PTR };
    diag::crash32(scb.cfsr.read());
    diag::crash32(scb.bfar.read());
    state::restart_after_crash()
}

/// Bootmagic, before clocks or anything else can go wrong: if the top-left key
/// (row 0 = PB3, col 0 = PB14) is held at power-up, go to the bootloader.
fn bootmagic() {
    use pac::gpio::vals::{CnfIn, CnfOut, Mode};
    pac::RCC.apb2enr().modify(|w| {
        w.set_gpioben(true);
        w.set_afioen(true);
    });
    // PB3 is JTDO after reset; free it (SWD stays on).
    pac::AFIO.mapr().modify(|w| w.set_swj_cfg(pac::afio::vals::SwjCfg::JTAG_DISABLE));
    let b = pac::GPIOB;
    b.cr(0).modify(|w| {
        w.set_mode(3, Mode::OUTPUT2MHZ);
        w.set_cnf_out(3, CnfOut::PUSH_PULL);
    });
    b.bsrr().write(|w| w.set_br(3, true));
    b.cr(1).modify(|w| {
        w.set_mode(14 - 8, Mode::INPUT);
        w.set_cnf_in(14 - 8, CnfIn::PULL);
    });
    b.bsrr().write(|w| w.set_bs(14, true)); // pull-up
    cortex_m::asm::delay(80_000); // ~10 ms at 8 MHz
    let held = b.idr().read().idr(14).to_bits() == 0;
    if held {
        state::enter_bootloader();
    }
}

/// Independent watchdog, ~2 s. A hang resets the chip and the firmware starts again;
/// `state::boot_guard` parks it in DFU if that keeps happening.
///
/// PAC writes, not embassy-stm32's watchdog driver: the driver works out its own prescaler and
/// reload, and these exact values are the timing that was tested on the pad.
fn start_watchdog() {
    use pac::iwdg::vals::{Key, Pr};
    let kr = pac::IWDG.kr();
    kr.write(|w| w.set_key(Key::ENABLE)); // unlock PR/RLR
    pac::IWDG.pr().write(|w| w.set_pr(Pr::DIVIDE_BY32)); // LSI / 32
    pac::IWDG.rlr().write(|w| w.set_rl(2500));
    kr.write(|w| w.set_key(Key::RESET));
    kr.write(|w| w.set_key(Key::START));
}

fn feed_watchdog() {
    pac::IWDG.kr().write(|w| w.set_key(pac::iwdg::vals::Key::RESET));
}

#[embassy_executor::task]
async fn watchdog() {
    let mut ticks = 0u32;
    loop {
        feed_watchdog();
        Timer::after_millis(250).await;
        ticks += 1;
        if ticks == 40 {
            state::mark_healthy(); // 10 s without trouble
        }
    }
}

const KBD_DESC: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xA1, 0x01, 0x05, 0x07, 0x19, 0xE0, 0x29, 0xE7, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95,
    0x08, 0x81, 0x02, 0x95, 0x01, 0x75, 0x08, 0x81, 0x01, 0x95, 0x06, 0x75, 0x08, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x05,
    0x07, 0x19, 0x00, 0x2A, 0xFF, 0x00, 0x81, 0x00, 0xC0,
];
const CONSUMER_DESC: &[u8] = &[
    0x05, 0x0C, 0x09, 0x01, 0xA1, 0x01, 0x15, 0x00, 0x26, 0xFF, 0x03, 0x19, 0x00, 0x2A, 0xFF, 0x03, 0x75, 0x10, 0x95,
    0x01, 0x81, 0x00, 0xC0,
];
/// Same vendor interface QMK/VIA use, so the PC app finds the pad the same way.
const RAW_DESC: &[u8] = &[
    0x06, 0x60, 0xFF, 0x09, 0x61, 0xA1, 0x01, 0x09, 0x62, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95, 0x20, 0x75, 0x08, 0x81,
    0x02, 0x09, 0x63, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95, 0x20, 0x75, 0x08, 0x91, 0x02, 0xC0,
];

/// Set when the bus is suspended, reset or switched off: whatever the PC thought was held, the
/// pad lets go of too, and tells it so once the bus is back.
static LET_GO: AtomicBool = AtomicBool::new(false);

/// Tracks USB suspend so the pad goes dark with the PC.
struct BusState;

impl embassy_usb::Handler for BusState {
    fn enabled(&mut self, enabled: bool) {
        if !enabled {
            LET_GO.store(true, Ordering::Relaxed);
        }
    }

    fn suspended(&mut self, suspended: bool) {
        state::SUSPENDED.store(suspended, Ordering::Relaxed);
        if suspended {
            LET_GO.store(true, Ordering::Relaxed);
        }
    }

    fn reset(&mut self) {
        state::SUSPENDED.store(false, Ordering::Relaxed);
        LET_GO.store(true, Ordering::Relaxed);
    }
}

#[embassy_executor::task]
async fn usb_task(mut dev: embassy_usb::UsbDevice<'static, UsbDriver>) {
    diag::log(0x5000);
    dev.run().await
}

#[embassy_executor::task]
async fn host_rx(mut rx: hid::HidReader<'static, UsbDriver, 32>) {
    let mut buf = [0u8; 32];
    let mut first_report = false;
    loop {
        match rx.read(&mut buf).await {
            Ok(n) => {
                if !first_report {
                    first_report = true;
                    diag::log(0x5100);
                }
                let popup = matches!(buf[..n], [kb16_mixer_protocol::MAGIC, kb16_mixer_protocol::id::POPUP, ..]);
                match state::handle_report(&buf[..n]) {
                    state::After::Nothing => {}
                    state::After::Bootloader => {
                        Timer::after_millis(20).await;
                        state::enter_bootloader();
                    }
                    state::After::SaveName(n) => name::save(&n),
                }
                if popup {
                    oled::DRAW_NOW.signal(());
                }
            }
            Err(_) => Timer::after_millis(50).await,
        }
    }
}

/// How long one report may wait for the bus.
const SEND: Duration = Duration::from_millis(50);
/// How often a report the PC is owed is tried again, and `LET_GO` looked at.
const RETRY: Duration = Duration::from_millis(100);

/// Sends one report; false if the bus did not take it in time.
async fn put<const N: usize>(w: &mut HidWriter<'static, UsbDriver, N>, report: &[u8]) -> bool {
    matches!(with_timeout(SEND, w.write(report)).await, Ok(Ok(())))
}

#[embassy_executor::task]
async fn input(
    mut kbd: HidWriter<'static, UsbDriver, 8>,
    mut media: HidWriter<'static, UsbDriver, 2>,
    mut raw: hid::HidWriter<'static, UsbDriver, 32>,
) {
    let mut router = Router::new();
    // Reports the PC is owed because the bus did not take them (not ready, suspended): the
    // keyboard state, a media key's release, and the key and knob releases for the PC app.
    // Until they get through, the PC would think those keys are still held.
    let mut kbd_owed = false;
    let mut media_owed = false;
    let mut ups_owed = OwedUps::new();
    loop {
        if LET_GO.swap(false, Ordering::Relaxed) {
            router.keyboard.clear();
            kbd_owed = true;
            media_owed = true;
            // The bus went away: the PC app saw the pad go and let go of its keys itself.
            ups_owed.clear();
        }
        if !state::suspended() {
            if kbd_owed {
                kbd_owed = !put(&mut kbd, &router.keyboard.report(0)).await;
            }
            if media_owed {
                media_owed = !put(&mut media, &[0, 0]).await;
            }
            for msg in ups_owed.pending() {
                let delivered = put(&mut raw, &msg.encode()).await;
                ups_owed.sent(msg, delivered);
            }
        }
        let Ok(ev) = with_timeout(RETRY, keys::EVENTS.receive()).await else { continue };
        let now = state::Tick::now();
        let (alive, mixer_keys, keymap) = state::with(|s| {
            let press = match ev {
                Event::Key { row, col, pressed: true } => Some((row, col)),
                _ => None,
            };
            s.touched(now, press);
            (s.host_alive(now), s.mixer.mixer_keys, s.keys)
        });
        match router.route(ev, alive, mixer_keys, &keymap, now) {
            Action::Nothing => {}
            Action::Host(msg) => {
                let delivered = put(&mut raw, &msg.encode()).await;
                ups_owed.sent(msg, delivered);
            }
            Action::Keyboard => kbd_owed = !put(&mut kbd, &router.keyboard.report(0)).await,
            Action::TapKey(code) => {
                put(&mut kbd, &router.keyboard.report(code)).await;
                kbd_owed = !put(&mut kbd, &router.keyboard.report(0)).await;
            }
            Action::TapMedia(usage) => {
                put(&mut media, &usage.to_le_bytes()).await;
                media_owed = !put(&mut media, &[0, 0]).await;
            }
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    diag::init();
    diag::log(0x0100);
    state::boot_guard();
    state::with(|s| s.logo_name = name::load());
    diag::log(0x0200);
    bootmagic();
    diag::log(0x0300);
    start_watchdog();
    diag::log(0x0400);
    // The bootloader's NVIC state survives the jump: drop any IRQ it left enabled or pending.
    // SAFETY: NVIC::PTR points at the always-mapped NVIC. Interrupts are not used yet (no
    // driver is set up), so clearing every enable and pending bit cannot cut off a handler that
    // something relies on.
    unsafe {
        let nvic = &*cortex_m::peripheral::NVIC::PTR;
        for i in 0..8 {
            nvic.icer[i].write(0xFFFF_FFFF);
            nvic.icpr[i].write(0xFFFF_FFFF);
        }
    }
    diag::log(0x0500);

    let mut config = Config::default();
    {
        use embassy_stm32::rcc::*;
        config.rcc.hse = Some(Hse { freq: Hertz(8_000_000), mode: HseMode::Oscillator });
        config.rcc.pll = Some(Pll { src: PllSource::HSE, prediv: PllPreDiv::DIV1, mul: PllMul::MUL9 });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
    }
    let p = embassy_stm32::init(config);
    diag::log(0x0600);
    spawner.spawn(watchdog().expect("watchdog"));
    diag::log(0x0700);
    pac::AFIO.mapr().modify(|w| w.set_swj_cfg(pac::afio::vals::SwjCfg::JTAG_DISABLE));

    // SAFETY: nothing in this firmware takes the core peripherals as an owned value, and the
    // only registers touched here, the DCB trace enable and the DWT cycle-counter enable, are
    // written nowhere else.
    let mut core = unsafe { cortex_m::Peripherals::steal() };
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();
    diag::log(0x0800);

    // LEDs first so the pad lights up even before USB is ready.
    let led_pin = Output::new(p.PA10, Level::Low, Speed::VeryHigh);
    spawner.spawn(leds::run(led_pin).expect("leds"));
    diag::log(0x0900);

    let mut i2c_cfg = i2c::Config::default();
    i2c_cfg.frequency = Hertz(400_000);
    let i2c = I2c::new(p.I2C2, p.PB10, p.PB11, p.DMA1_CH4, p.DMA1_CH5, Irqs, i2c_cfg);
    diag::log(0x0A00);
    spawner.spawn(oled::run(i2c).expect("oled"));
    diag::log(0x0B00);

    let scanner = keys::Scanner {
        rows: [Flex::new(p.PB3), Flex::new(p.PB4), Flex::new(p.PB9), Flex::new(p.PB8)],
        cols: [
            Input::new(p.PB14, Pull::Up),
            Input::new(p.PB13, Pull::Up),
            Input::new(p.PB12, Pull::Up),
            Input::new(p.PB0, Pull::Up),
            Input::new(p.PA7, Pull::Up),
        ],
        knobs: [
            (Input::new(p.PB5, Pull::Up), Input::new(p.PB6, Pull::Up)),
            (Input::new(p.PA1, Pull::Up), Input::new(p.PA2, Pull::Up)),
            (Input::new(p.PA3, Pull::Up), Input::new(p.PA4, Pull::Up)),
        ],
    };
    spawner.spawn(keys::scan(scanner).expect("scan"));
    diag::log(0x0C00);

    // The board has a fixed pull-up on D+: hold D+ low briefly so the PC sees a fresh device.
    {
        // SAFETY: this copy of PA12 is dropped at the end of the block, before the USB driver
        // below takes the real one, so the two never drive the pin at the same time.
        let mut dp = Output::new(unsafe { p.PA12.clone_unchecked() }, Level::Low, Speed::Low);
        dp.set_low();
        Timer::after_millis(10).await;
    }
    diag::log(0x0D00);
    let driver = Driver::new(p.USB, Irqs, p.PA12, p.PA11);
    diag::log(0x0E00);

    let mut ucfg = embassy_usb::Config::new(kb16_mixer_protocol::VID, kb16_mixer_protocol::PID);
    ucfg.manufacturer = Some("kb16-mixer");
    ucfg.product = Some(kb16_mixer_protocol::PRODUCT);
    ucfg.serial_number = Some("KB16-MIXER");
    // The firmware version, so the PC app can show it from the device list.
    ucfg.device_release = const { kb16_mixer_protocol::device_release(env!("CARGO_PKG_VERSION")) };
    ucfg.max_power = 250;
    ucfg.max_packet_size_0 = 64;
    ucfg.composite_with_iads = false;
    ucfg.device_class = 0;
    ucfg.device_sub_class = 0;
    ucfg.device_protocol = 0;

    static CONFIG_DESC: StaticCell<[u8; 256]> = StaticCell::new();
    static BOS_DESC: StaticCell<[u8; 64]> = StaticCell::new();
    static MSOS_DESC: StaticCell<[u8; 64]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    static KBD_STATE: StaticCell<hid::State> = StaticCell::new();
    static RAW_STATE: StaticCell<hid::State> = StaticCell::new();
    static MEDIA_STATE: StaticCell<hid::State> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        ucfg,
        CONFIG_DESC.init([0; 256]),
        BOS_DESC.init([0; 64]),
        MSOS_DESC.init([0; 64]),
        CONTROL_BUF.init([0; 128]),
    );
    static BUS_STATE: StaticCell<BusState> = StaticCell::new();
    builder.handler(BUS_STATE.init(BusState));
    let kbd = HidWriter::<_, 8>::new(
        &mut builder,
        KBD_STATE.init(hid::State::new()),
        hid::Config {
            report_descriptor: KBD_DESC,
            request_handler: None,
            poll_ms: 1,
            max_packet_size: 8,
            hid_subclass: HidSubclass::Boot,
            hid_boot_protocol: HidBootProtocol::Keyboard,
        },
    );
    let raw = HidReaderWriter::<_, 32, 32>::new(
        &mut builder,
        RAW_STATE.init(hid::State::new()),
        hid::Config {
            report_descriptor: RAW_DESC,
            request_handler: None,
            poll_ms: 1,
            max_packet_size: 32,
            hid_subclass: HidSubclass::No,
            hid_boot_protocol: HidBootProtocol::None,
        },
    );
    let media = HidWriter::<_, 2>::new(
        &mut builder,
        MEDIA_STATE.init(hid::State::new()),
        hid::Config {
            report_descriptor: CONSUMER_DESC,
            request_handler: None,
            poll_ms: 4,
            max_packet_size: 8,
            hid_subclass: HidSubclass::No,
            hid_boot_protocol: HidBootProtocol::None,
        },
    );
    let dev = builder.build();
    diag::log(0x0F00);
    let (raw_rx, raw_tx) = raw.split();

    spawner.spawn(usb_task(dev).expect("usb"));
    spawner.spawn(host_rx(raw_rx).expect("rx"));
    spawner.spawn(input(kbd, media, raw_tx).expect("input"));
    diag::log(0x1000);
}
