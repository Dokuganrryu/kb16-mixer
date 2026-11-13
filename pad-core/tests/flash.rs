//! The flash write sequence (`pad_core::flash::erase` and `program`) against a fake controller:
//! the order of the register steps, and the lock again on every path out.

use pad_core::flash::{self, Controller, Error, Flags, KEY1, KEY2, LOG_PAGE, NAME_PAGE, PAGE_SIZE};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    Key(u32),
    Lock,
    StartErase(usize),
    EndErase,
    ProgramMode(bool),
    Write(usize, u16),
}

/// When a fault the test asks for starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum When {
    /// Already there when the sequence begins.
    Start,
    /// After the erase starts.
    Erase,
    /// After a half-word is stored.
    Write,
}

#[derive(Default)]
struct Fake {
    locked: bool,
    /// The right keys do not unlock it.
    stays_locked: bool,
    /// A key was written while unlocked: the real controller then stays locked until reset.
    bricked: bool,
    ops: Vec<Op>,
    mem: HashMap<usize, u16>,
    pg: bool,
    keys: Vec<u32>,
    hang: Option<When>,
    busy: bool,
    error: Option<(When, Flags)>,
    pending: Flags,
    /// A half-word that neither erases nor programs.
    stuck: Option<usize>,
}

impl Fake {
    fn locked() -> Fake {
        Fake { locked: true, ..Fake::default() }
    }

    fn at(&mut self, when: When) {
        if self.hang == Some(when) {
            self.busy = true;
        }
        if let Some((w, f)) = self.error
            && w == when
        {
            self.pending = f;
        }
    }

    fn start(mut self) -> Fake {
        self.at(When::Start);
        self
    }

    /// The controller ends locked, with the last step the lock and no other lock before it.
    fn assert_locked_at_the_end(&self, what: &str) {
        assert!(self.locked, "{what}: left unlocked");
        assert!(!self.bricked, "{what}: a key was written while unlocked");
        assert_eq!(self.ops.last(), Some(&Op::Lock), "{what}: {:?}", self.ops);
        assert_eq!(self.ops.iter().filter(|o| **o == Op::Lock).count(), 1, "{what}: {:?}", self.ops);
    }
}

impl Controller for Fake {
    fn locked(&mut self) -> bool {
        self.locked
    }

    fn key(&mut self, key: u32) {
        self.ops.push(Op::Key(key));
        if !self.locked {
            self.bricked = true;
        }
        self.keys.push(key);
        if self.keys.ends_with(&[KEY1, KEY2]) && !self.stays_locked && !self.bricked {
            self.locked = false;
        }
    }

    fn lock(&mut self) {
        self.ops.push(Op::Lock);
        self.locked = true;
    }

    fn busy(&mut self) -> bool {
        self.busy
    }

    fn take_flags(&mut self) -> Flags {
        core::mem::take(&mut self.pending)
    }

    fn start_erase(&mut self, page: usize) {
        self.ops.push(Op::StartErase(page));
        assert!(!self.locked, "erase started while locked");
        for a in (page..page + PAGE_SIZE).step_by(2) {
            if self.stuck != Some(a) {
                self.mem.insert(a, 0xFFFF);
            }
        }
        self.at(When::Erase);
    }

    fn end_erase(&mut self) {
        self.ops.push(Op::EndErase);
    }

    fn program_mode(&mut self, on: bool) {
        self.ops.push(Op::ProgramMode(on));
        self.pg = on;
    }

    fn write(&mut self, addr: usize, value: u16) {
        self.ops.push(Op::Write(addr, value));
        assert!(!self.locked && self.pg, "write outside program mode");
        if self.stuck != Some(addr) {
            let old = self.read(addr);
            self.mem.insert(addr, old & value);
        }
        self.at(When::Write);
    }

    fn read(&mut self, addr: usize) -> u16 {
        // Whatever was there before: not blank.
        *self.mem.get(&addr).unwrap_or(&0x1234)
    }
}

#[test]
fn the_unlock_keys_are_the_ones_the_reference_manual_gives() {
    assert_eq!((KEY1, KEY2), (0x4567_0123, 0xCDEF_89AB));
}

#[test]
fn an_erase_unlocks_erases_checks_and_locks() {
    let mut f = Fake::locked();
    assert_eq!(flash::erase(&mut f, NAME_PAGE), Ok(()));
    assert_eq!(f.ops, [Op::Key(KEY1), Op::Key(KEY2), Op::StartErase(NAME_PAGE), Op::EndErase, Op::Lock]);
    assert!((NAME_PAGE..NAME_PAGE + PAGE_SIZE).step_by(2).all(|a| f.mem[&a] == 0xFFFF));
    f.assert_locked_at_the_end("erase");
}

#[test]
fn a_program_unlocks_writes_each_word_and_locks() {
    let mut f = Fake::locked();
    assert_eq!(flash::erase(&mut f, LOG_PAGE), Ok(()));
    f.ops.clear();
    assert_eq!(flash::program(&mut f, LOG_PAGE + 4, &[0xABCD, 0x0102]), Ok(()));
    assert_eq!(
        f.ops,
        [
            Op::Key(KEY1),
            Op::Key(KEY2),
            Op::ProgramMode(true),
            Op::Write(LOG_PAGE + 4, 0xABCD),
            Op::Write(LOG_PAGE + 6, 0x0102),
            Op::ProgramMode(false),
            Op::Lock,
        ]
    );
    assert_eq!((f.mem[&(LOG_PAGE + 4)], f.mem[&(LOG_PAGE + 6)]), (0xABCD, 0x0102));
    f.assert_locked_at_the_end("program");
}

#[test]
fn an_unlocked_controller_gets_no_keys_and_is_locked_after() {
    // A key written while unlocked would lock the controller until the next reset.
    let mut f = Fake::default();
    assert_eq!(flash::erase(&mut f, NAME_PAGE), Ok(()));
    assert!(!f.ops.iter().any(|o| matches!(o, Op::Key(_))));
    f.assert_locked_at_the_end("erase");
    let mut f = Fake::default();
    assert_eq!(flash::program(&mut f, NAME_PAGE, &[0]), Ok(()));
    f.assert_locked_at_the_end("program");
}

#[test]
fn every_error_path_locks_the_controller_again() {
    let wp = Flags { write_protected: true, program: false };
    let pg = Flags { write_protected: false, program: true };
    let both = Flags { write_protected: true, program: true };
    let cases: [(&str, Fake, Error); 7] = [
        ("keys refused", Fake { stays_locked: true, ..Fake::locked() }, Error::Locked),
        ("busy at start", Fake { hang: Some(When::Start), ..Fake::locked() }, Error::Timeout),
        ("write protected at start", Fake { error: Some((When::Start, wp)), ..Fake::locked() }, Error::WriteProtected),
        ("program error at start", Fake { error: Some((When::Start, pg)), ..Fake::locked() }, Error::Program),
        ("both flags at start", Fake { error: Some((When::Start, both)), ..Fake::locked() }, Error::WriteProtected),
        ("stuck half-word", Fake { stuck: Some(NAME_PAGE + 2), ..Fake::locked() }, Error::Verify),
        ("busy after the step", Fake { hang: Some(When::Erase), ..Fake::locked() }, Error::Timeout),
    ];
    for (what, f, want) in cases {
        let mut f = f.start();
        assert_eq!(flash::erase(&mut f, NAME_PAGE), Err(want), "erase: {what}");
        f.assert_locked_at_the_end(&format!("erase: {what}"));
    }
    let cases: [(&str, Fake, Error); 7] = [
        ("keys refused", Fake { stays_locked: true, ..Fake::locked() }, Error::Locked),
        ("busy at start", Fake { hang: Some(When::Start), ..Fake::locked() }, Error::Timeout),
        ("write protected at start", Fake { error: Some((When::Start, wp)), ..Fake::locked() }, Error::WriteProtected),
        ("program error after a write", Fake { error: Some((When::Write, pg)), ..Fake::locked() }, Error::Program),
        (
            "write protected after a write",
            Fake { error: Some((When::Write, wp)), ..Fake::locked() },
            Error::WriteProtected,
        ),
        ("stuck half-word", Fake { stuck: Some(NAME_PAGE + 2), ..Fake::locked() }, Error::Verify),
        ("busy after a write", Fake { hang: Some(When::Write), ..Fake::locked() }, Error::Timeout),
    ];
    for (what, f, want) in cases {
        let mut f = f.start();
        assert_eq!(flash::program(&mut f, NAME_PAGE, &[0x0000, 0x0000, 0x0000]), Err(want), "program: {what}");
        f.assert_locked_at_the_end(&format!("program: {what}"));
        // Program mode is off again before the lock whenever it was turned on.
        if f.ops.contains(&Op::ProgramMode(true)) {
            assert_eq!(f.ops[f.ops.len() - 2], Op::ProgramMode(false), "program: {what}");
        }
    }
}

#[test]
fn an_erase_turns_the_erase_mode_off_before_it_locks_even_on_an_error() {
    let f = Fake { error: Some((When::Erase, Flags { write_protected: false, program: true })), ..Fake::locked() };
    let mut f = f.start();
    assert_eq!(flash::erase(&mut f, LOG_PAGE), Err(Error::Program));
    assert_eq!(f.ops[f.ops.len() - 2..], [Op::EndErase, Op::Lock]);
}

#[test]
fn a_program_stops_at_the_first_failed_word() {
    let mut f = Fake { stuck: Some(NAME_PAGE + 2), ..Fake::locked() };
    assert_eq!(flash::program(&mut f, NAME_PAGE, &[0, 0, 0]), Err(Error::Verify));
    let writes = f.ops.iter().filter(|o| matches!(o, Op::Write(..))).count();
    assert_eq!(writes, 2);
}

#[test]
fn an_address_outside_the_reserved_pages_never_touches_the_controller() {
    for page in [0x0800_0000, 0x0800_2000, NAME_PAGE + 1, 0x0801_FC00] {
        let mut f = Fake::locked();
        assert!(flash::erase(&mut f, page).is_err(), "{page:#x}");
        assert!(f.ops.is_empty(), "{page:#x}: {:?}", f.ops);
    }
    for (addr, words) in [(0x0800_0000, 1), (NAME_PAGE + 1, 1), (LOG_PAGE + PAGE_SIZE - 2, 2)] {
        let mut f = Fake::locked();
        assert!(flash::program(&mut f, addr, &vec![0; words]).is_err(), "{addr:#x}");
        assert!(f.ops.is_empty(), "{addr:#x}: {:?}", f.ops);
    }
}
