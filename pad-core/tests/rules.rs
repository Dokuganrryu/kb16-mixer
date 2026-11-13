//! The firmware rules as the docs state them: the flash guard, the idle rule
//! (`docs/how-it-works.md`, "Idle rule"), the crash counter, the knobs, the keys and the logo
//! name.

use kb16_mixer_protocol::{Frame, HostMsg, NEVER, Popup, Settings};
use pad_core::boot::{self, Boot, after_reset};
use pad_core::flash::{Error, LOG_PAGE, NAME_PAGE, PAGE_SIZE, check_erase, check_program};
use pad_core::input::{BOOT_COMBO_MS, Combo, DEBOUNCE_MS, Debounce, Knob, Turns};
use pad_core::name;
use pad_core::state::{After, State, past};
use pad_core::{Time, leds};

const BOOTLOADER_END: usize = 0x0800_2000;
const SPARE_PAGE: usize = 0x0801_FC00;
const FLASH_END: usize = 0x0802_0000;

// --- Flash guard ------------------------------------------------------------------------------

#[test]
fn only_the_two_reserved_pages_can_be_erased() {
    assert_eq!(check_erase(NAME_PAGE), Ok(()));
    assert_eq!(check_erase(LOG_PAGE), Ok(()));
    for (addr, want) in [
        (NAME_PAGE - 1, Error::Outside),
        (NAME_PAGE + 1, Error::Unaligned),
        (LOG_PAGE - 1, Error::Unaligned),
        (LOG_PAGE + 1, Error::Unaligned),
        (SPARE_PAGE - 1, Error::Unaligned),
        (SPARE_PAGE, Error::Outside),
        (SPARE_PAGE + 1, Error::Outside),
        (0x0800_0000, Error::Outside),
        (BOOTLOADER_END - PAGE_SIZE, Error::Outside),
        (BOOTLOADER_END, Error::Outside),
        (NAME_PAGE - PAGE_SIZE, Error::Outside),
        (0, Error::Outside),
        (usize::MAX, Error::Outside),
    ] {
        assert_eq!(check_erase(addr), Err(want), "{addr:#x}");
    }
}

#[test]
fn no_other_flash_address_can_be_erased() {
    for addr in 0x0800_0000..FLASH_END {
        let ok = check_erase(addr).is_ok();
        assert_eq!(ok, addr == NAME_PAGE || addr == LOG_PAGE, "{addr:#x}");
    }
}

#[test]
fn programming_stays_inside_one_reserved_page() {
    let words = PAGE_SIZE / 2;
    for page in [NAME_PAGE, LOG_PAGE] {
        let end = page + PAGE_SIZE;
        assert_eq!(check_program(page, words), Ok(()));
        assert_eq!(check_program(page, 0), Ok(()));
        assert_eq!(check_program(end - 2, 1), Ok(()));
        // One byte either side of each boundary.
        assert_eq!(check_program(page, words + 1), Err(Error::Outside));
        assert_eq!(check_program(page - 2, 1), if page == LOG_PAGE { Ok(()) } else { Err(Error::Outside) });
        assert_eq!(check_program(page - 1, 1), Err(Error::Outside));
        assert_eq!(check_program(page + 1, 1), Err(Error::Unaligned));
        assert_eq!(check_program(end - 1, 1), Err(Error::Outside));
        assert_eq!(check_program(end - 2, 2), Err(Error::Outside));
    }
    // Not across from the name page into the log page either.
    assert_eq!(check_program(LOG_PAGE - 2, 2), Err(Error::Outside));
    assert_eq!(check_program(SPARE_PAGE, 1), Err(Error::Outside));
    assert_eq!(check_program(SPARE_PAGE - 1, 1), Err(Error::Outside));
    assert_eq!(check_program(0x0800_0000, 1), Err(Error::Outside));
    assert_eq!(check_program(BOOTLOADER_END, 1), Err(Error::Outside));
    // Lengths that overflow the address space.
    assert_eq!(check_program(NAME_PAGE, usize::MAX), Err(Error::Outside));
    assert_eq!(check_program(NAME_PAGE, usize::MAX / 2 + 1), Err(Error::Outside));
    assert_eq!(check_program(usize::MAX, 1), Err(Error::Outside));
}

#[test]
fn no_other_flash_half_word_can_be_programmed() {
    let reserved = |a: usize| (NAME_PAGE..SPARE_PAGE).contains(&a);
    for addr in 0x0800_0000..FLASH_END {
        let ok = check_program(addr, 1).is_ok();
        assert_eq!(ok, reserved(addr) && addr % 2 == 0, "{addr:#x}");
    }
}

// --- Idle rule --------------------------------------------------------------------------------

const MIN: u64 = 60_000;

fn idle(s: &State<u64>, now: u64) -> u64 {
    now.ms_since(s.activity)
}

fn frame(playing: bool, levels: [u8; 4]) -> HostMsg {
    HostMsg::Frame(Frame {
        levels,
        volumes: [50; 4],
        mutes: 0,
        selected: 0,
        mixer_keys: true,
        mic_muted: false,
        playing,
        second_page: false,
    })
}

fn popup(volume: u8) -> HostMsg {
    HostMsg::Popup(Popup { knob: 0, volume, muted: false, name: *b"GAME\0\0\0\0\0\0\0\0\0\0\0\0" })
}

fn settings(screen_off_min: u8, lights_dim_min: u8) -> HostMsg {
    HostMsg::Settings(Settings {
        channels: 4,
        colors: [[0; 3]; 4],
        brightness: 0,
        oled_top_plus_one: 0,
        screen_off_min,
        lights_dim_min,
        idle_led_percent: 0,
    })
}

#[test]
fn the_screen_goes_off_after_ten_quiet_minutes_by_default() {
    let s = State::new(0u64);
    assert!(!past(idle(&s, 10 * MIN), s.screen_off_min));
    assert!(past(idle(&s, 10 * MIN + 1), s.screen_off_min));
    assert!(past(idle(&s, 10 * MIN + 1), s.lights_dim_min));
}

#[test]
fn a_key_press_and_a_knob_turn_are_activity() {
    let mut s = State::new(0u64);
    s.touched(9 * MIN, Some((1, 2))); // a key
    assert!(!past(idle(&s, 15 * MIN), s.screen_off_min));
    s.touched(20 * MIN, None); // a knob
    assert_eq!(idle(&s, 20 * MIN), 0);
    // A press on one of the 16 keys also starts a light bloom; a knob push (column 4) does not.
    assert_eq!(s.ripples.iter().flatten().count(), 1);
    s.touched(21 * MIN, Some((0, 4)));
    assert_eq!(s.ripples.iter().flatten().count(), 1);
}

#[test]
fn audible_sound_is_activity_for_as_long_as_the_host_says_so() {
    let mut s = State::new(0u64);
    s.handle(frame(true, [0; 4]), 5 * MIN);
    assert_eq!(idle(&s, 5 * MIN), 0);
    // A heartbeat each second keeps it fresh while the flag holds.
    for t in 1..=600 {
        s.handle(HostMsg::Heartbeat, 5 * MIN + t * 1000);
    }
    assert_eq!(idle(&s, 15 * MIN), 0);
    // The sound stops: from then on the pad counts quiet time.
    s.handle(frame(false, [0; 4]), 15 * MIN + 500);
    s.handle(HostMsg::Heartbeat, 16 * MIN);
    assert_eq!(idle(&s, 16 * MIN), MIN); // since the last heartbeat with the flag set
}

#[test]
fn levels_popups_notices_and_settings_are_not_activity() {
    let mut s = State::new(0u64);
    s.handle(frame(false, [255; 4]), MIN); // loud meters, but the host does not call it audible
    s.handle(popup(40), 2 * MIN);
    s.handle(popup(kb16_mixer_protocol::NOTICE), 3 * MIN); // "Config loaded"
    s.handle(settings(0, 0), 4 * MIN);
    s.handle(HostMsg::Names([*b"GAME  "; 4]), 5 * MIN);
    s.handle(HostMsg::Heartbeat, 6 * MIN);
    s.handle(HostMsg::Unknown(0x55), 7 * MIN);
    assert_eq!(s.activity, 0);
    assert!(s.popup.is_some());
    assert!(past(idle(&s, 10 * MIN + 1), s.screen_off_min));
}

#[test]
fn a_host_that_comes_back_starts_from_silence() {
    let mut s = State::new(0u64);
    s.handle(frame(true, [0; 4]), 1000);
    assert!(s.host_alive(3999));
    assert!(!s.host_alive(4000));
    // Gone for more than 3 s: its last "playing" no longer counts.
    s.handle(HostMsg::Heartbeat, 10 * MIN);
    assert!(!s.mixer.playing);
    assert_eq!(s.activity, 1000);
}

#[test]
fn idle_minutes_come_from_the_settings() {
    let mut s = State::new(0u64);
    s.handle(settings(1, NEVER), 0);
    assert!(past(MIN + 1, s.screen_off_min));
    assert!(!past(u64::MAX, s.lights_dim_min));
    // 0 means the pad default.
    s.handle(settings(0, 0), 0);
    assert_eq!((s.screen_off_min, s.lights_dim_min), (10, 10));
}

/// Light output (sum of all bytes) at `to`, for 5 ms frames from 0 with the last activity
/// given by `activity(t)`. The idle swell keeps moving, so compare only at the same instant.
fn light_at(to: u64, activity: impl Fn(u64) -> u64) -> u32 {
    let s = State::new(0u64);
    let mut lights = leds::Lights::new(0u64);
    let mut last = [0u8; 48];
    for t in (0..=to).step_by(5) {
        let input = leds::Input {
            alive: false,
            mixer: s.mixer,
            brightness: s.brightness,
            ripples: s.ripples,
            activity: activity(t),
            dim_after: s.lights_dim_min,
        };
        if let Some(b) = lights.frame(t, &input) {
            last = b;
        }
    }
    last.iter().map(|&v| v as u32).sum()
}

#[test]
fn quiet_lights_dim_and_a_touch_brings_them_back() {
    let awake = |to| light_at(to, |t| t);
    // Ten quiet minutes, then a slow fade down.
    let quiet = 11 * MIN;
    assert!(light_at(quiet, |_| 0) * 4 < awake(quiet));
    // A touch at 11 minutes: back up within half a second.
    let back = quiet + 500;
    let touched = light_at(back, |t| if t < quiet { 0 } else { quiet });
    assert!(touched * 20 > awake(back) * 19, "{touched} vs {}", awake(back));
}

#[test]
fn a_bootloader_request_changes_nothing_else() {
    let mut s = State::new(0u64);
    assert_eq!(s.handle(HostMsg::Bootloader, 5000), After::Bootloader);
    assert_eq!(s.host_seen, None);
}

// --- Crash counter ----------------------------------------------------------------------------

#[test]
fn three_crashed_starts_in_a_row_park_the_pad() {
    assert_eq!(after_reset(true, 0), Boot::Run { crashes: 1 });
    assert_eq!(after_reset(true, 1), Boot::Run { crashes: 2 });
    assert_eq!(after_reset(true, 2), Boot::Bootloader);
    // A clean start (power-up, a reset from the bootloader) forgets earlier crashes.
    assert_eq!(after_reset(false, 2), Boot::Run { crashes: 0 });
    assert_eq!(after_reset(false, u16::MAX), Boot::Run { crashes: 0 });
    assert_eq!(after_reset(true, u16::MAX), Boot::Bootloader);
}

// --- Bootloader marks -------------------------------------------------------------------------

/// The ten backup data registers, and every write in order as (DR number, value).
#[derive(Default)]
struct Regs {
    dr: [u16; 10],
    writes: Vec<(usize, u16)>,
}

impl boot::Backup for Regs {
    fn read(&mut self, reg: usize) -> u16 {
        self.dr[reg]
    }

    fn write(&mut self, reg: usize, value: u16) {
        self.dr[reg] = value;
        // The reference manual's names: index n is DRn+1.
        self.writes.push((reg + 1, value));
    }
}

/// Registers as a start finds them: DR8 and DR9 as given, DR10 as the bootloader left it.
fn regs(dr8: u16, dr9: u16) -> Regs {
    let mut r = Regs::default();
    r.dr[7] = dr8;
    r.dr[8] = dr9;
    r.dr[9] = 0x424D;
    r
}

#[test]
fn the_marks_are_the_values_the_bootloader_reads() {
    // The stm32duino bootloader reads DR10 after every reset. These numbers are its contract.
    assert_eq!(boot::STAY_IN_DFU, 0x424C);
    assert_eq!(boot::START_APP, 0x424D);
    assert_eq!(boot::PANIC_MARK, 0x5041);
    assert_eq!((boot::PANIC_REG, boot::CRASHES_REG, boot::MODE_REG), (7, 8, 9));
}

#[test]
fn every_way_into_the_bootloader_writes_stay_in_dfu_to_dr10() {
    let mut r = regs(0, 0);
    boot::enter_bootloader(&mut r);
    assert_eq!(r.writes, [(10, 0x424C)]);
}

#[test]
fn a_crash_restart_marks_dr8_and_starts_the_app_again() {
    let mut r = regs(0, 1);
    boot::crash(&mut r);
    assert_eq!(r.writes, [(8, 0x5041), (10, 0x424D)]);
}

#[test]
fn a_clean_start_clears_the_marks_and_arms_the_restart() {
    let mut r = regs(0, 2);
    assert_eq!(boot::start(&mut r, false), Boot::Run { crashes: 0 });
    assert_eq!(r.writes, [(8, 0), (9, 0), (10, 0x424D)]);
}

#[test]
fn a_panic_mark_or_a_watchdog_reset_counts_a_crash() {
    let mut r = regs(0x5041, 0);
    assert_eq!(boot::start(&mut r, false), Boot::Run { crashes: 1 });
    assert_eq!(r.writes, [(8, 0), (9, 1), (10, 0x424D)]);
    let mut r = regs(0, 1);
    assert_eq!(boot::start(&mut r, true), Boot::Run { crashes: 2 });
    assert_eq!(r.writes, [(8, 0), (9, 2), (10, 0x424D)]);
    // Another value in DR8 is not a panic mark.
    let mut r = regs(0x5040, 1);
    assert_eq!(boot::start(&mut r, false), Boot::Run { crashes: 0 });
}

#[test]
fn the_third_crashed_start_parks_the_pad_in_dfu() {
    let mut r = regs(0x5041, 2);
    assert_eq!(boot::start(&mut r, false), Boot::Bootloader);
    assert_eq!(r.writes, [(8, 0), (9, 0), (10, 0x424C)]);
}

#[test]
fn healthy_running_clears_the_crash_count() {
    let mut r = regs(0, 2);
    boot::healthy(&mut r);
    assert_eq!(r.writes, [(9, 0)]);
}

#[test]
fn three_crashes_in_a_row_park_the_pad_and_the_next_start_runs() {
    let mut r = regs(0, 0);
    for want in [Boot::Run { crashes: 1 }, Boot::Run { crashes: 2 }, Boot::Bootloader] {
        boot::crash(&mut r);
        assert_eq!(boot::start(&mut r, false), want);
    }
    assert_eq!(r.dr[9], 0x424C);
    // The PC flashes a fixed image; the bootloader then starts it with a clean count.
    r.dr[9] = 0x424D;
    assert_eq!(boot::start(&mut r, false), Boot::Run { crashes: 0 });
}

// --- Knobs and keys ---------------------------------------------------------------------------

#[test]
fn a_knob_reports_one_detent_per_four_contact_steps() {
    let cw = [1, 3, 2, 0];
    let mut k = Knob::new(0);
    let out: Vec<_> = cw.iter().map(|&ab| k.update(ab)).collect();
    assert_eq!(out, [None, None, None, Some(true)]);
    let out: Vec<_> = [2, 3, 1, 0].iter().map(|&ab| k.update(ab)).collect();
    assert_eq!(out, [None, None, None, Some(false)]);
    // Contact bounce back and forth, and a missed reading, make no detent.
    for ab in [1, 0, 1, 0, 3, 0, 3, 0] {
        assert_eq!(k.update(ab), None);
    }
}

#[test]
fn a_spin_during_a_full_queue_is_cut_to_eight_detents() {
    let mut t = Turns::new();
    for _ in 0..20 {
        t.add(true);
    }
    let mut sent = 0;
    for _ in 0..20 {
        t.drain(|cw| {
            assert!(cw);
            sent += 1;
            true
        });
    }
    assert_eq!(sent, 8);
}

#[test]
fn a_key_change_counts_after_five_steady_scans() {
    let mut k = Debounce::new();
    let mut events = Vec::new();
    // Bounce: never five in a row.
    for raw in [true, true, false, true, true, true, true, false] {
        k.sample(raw, |p| {
            events.push(p);
            true
        });
    }
    assert!(events.is_empty());
    for _ in 0..DEBOUNCE_MS {
        k.sample(true, |p| {
            events.push(p);
            true
        });
    }
    assert_eq!(events, [true]);
    assert!(k.pressed());
}

#[test]
fn a_release_is_kept_until_the_queue_takes_it() {
    let mut k = Debounce::new();
    for _ in 0..DEBOUNCE_MS {
        k.sample(true, |_| true);
    }
    for _ in 0..50 {
        k.sample(false, |_| false); // queue full
    }
    assert!(k.pressed());
    let mut got = None;
    k.sample(false, |p| {
        got = Some(p);
        true
    });
    assert_eq!(got, Some(false));
    assert!(!k.pressed());
}

#[test]
fn the_corner_combo_takes_two_seconds_without_a_break() {
    let mut c = Combo::new();
    for _ in 1..BOOT_COMBO_MS {
        assert!(!c.tick(true));
    }
    assert!(!c.tick(false));
    for _ in 1..BOOT_COMBO_MS {
        assert!(!c.tick(true));
    }
    assert!(c.tick(true));
}

// --- Logo name --------------------------------------------------------------------------------

#[test]
fn logo_names_are_capitals_digits_and_spaces() {
    assert!(name::valid(&name::DEFAULT));
    assert!(name::valid(b"DESK PC\0\0\0"));
    assert!(name::valid(b"ABCDEFGHIJ"));
    assert!(!name::valid(b"kb16\0\0\0\0\0\0"));
    assert!(!name::valid(b"   \0\0\0\0\0\0\0"));
    assert!(!name::valid(&[0; 10]));
    assert!(!name::valid(b"KB\0X\0\0\0\0\0\0")); // text after the padding
    assert!(!name::valid(b"KB-16\0\0\0\0\0"));
}

#[test]
fn a_stored_name_reads_back_and_damage_reads_as_the_default() {
    let rec = name::record(b"DESK PC\0\0\0");
    assert_eq!(&name::decode(&rec), b"DESK PC\0\0\0");
    assert_eq!(name::decode(&[0xFFFF; name::WORDS]), name::DEFAULT); // blank page
    for i in 0..name::WORDS {
        for bit in 0..16 {
            let mut torn = rec;
            torn[i] ^= 1 << bit;
            assert_eq!(name::decode(&torn), name::DEFAULT, "word {i} bit {bit}");
        }
    }
    let mut swapped = rec;
    swapped.swap(2, 3);
    assert_eq!(name::decode(&swapped), name::DEFAULT);
}

#[test]
fn name_writes_are_rate_limited() {
    assert!(name::may_write(0, 0, 1));
    assert!(!name::may_write(10, 1, 69));
    assert!(name::may_write(10, 1, 70));
    assert!(name::may_write(10, name::MAX_WRITES - 1, 1000));
    assert!(!name::may_write(10, name::MAX_WRITES, 1000));
}

/// What the firmware's `name::save` does: one `Writes` per power-up, asked before each write.
#[test]
fn a_name_is_written_at_most_once_a_minute_and_eight_times_per_power_up() {
    let names: Vec<[u8; 10]> = (0..20u8)
        .map(|i| {
            let mut n = *b"DESK PC 00";
            n[8] += i / 10;
            n[9] += i % 10;
            n
        })
        .collect();
    let writes = name::Writes::new();
    let mut stored = [0xFFFF; name::WORDS];
    let mut written = vec![];
    // A new name every 30 s for 20 minutes, as a host that keeps changing it would send.
    for (i, n) in names.iter().enumerate() {
        let now = 1 + i as u32 * 30;
        if let Some(rec) = writes.admit(n, &stored, now) {
            assert_eq!(rec, name::record(n));
            stored = rec;
            written.push(now);
        }
    }
    // One write per minute (the one 30 s later waits), and the ninth never comes.
    assert_eq!(written, [1, 61, 121, 181, 241, 301, 361, 421]);
    assert_eq!(name::MAX_WRITES, 8);
    assert_eq!(writes.admit(&names[0], &stored, 100_000), None);
}

#[test]
fn a_name_that_is_stored_already_or_not_valid_costs_no_write() {
    let writes = name::Writes::new();
    let desk = *b"DESK PC\0\0\0";
    let stored = name::record(&desk);
    // Neither counts: the limit is untouched, so a new name goes through at once.
    assert_eq!(writes.admit(&desk, &stored, 1), None);
    assert_eq!(writes.admit(b"desk pc\0\0\0", &stored, 2), None);
    assert_eq!(writes.admit(&[0; 10], &stored, 3), None);
    assert!(writes.admit(&name::DEFAULT, &stored, 4).is_some());
    // And the minute starts from that write.
    assert_eq!(writes.admit(&desk, &name::record(&name::DEFAULT), 63), None);
    assert!(writes.admit(&desk, &name::record(&name::DEFAULT), 64).is_some());
}
