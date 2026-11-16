//! The pointer-sized out-parameters of the call running on this thread (x64 programs): the
//! addresses of the caller's 8-byte slots, as Microsoft's Win32 metadata gives the function's or
//! the COM method's (sake's outptrs.txt / com_outptrs.txt). A 32-bit write to one of them leaves
//! the upper half as the caller had it (review 2026-11-16: the type of the argument closes that
//! where a handler takes it as sake's api::Out; this finds it wherever the address went: a
//! helper, a COM method's untyped argument, kura's own d3d9 / d3d11 writers, which take the
//! guest's arguments as words).
//!
//! sake's dispatcher enters the slots of each call (`enter`, undone when the guard drops: a
//! callback into the program and its calls nest), and the 32-bit writers ask `wrote32`. Under
//! test or a debug build a write so fails; a game goes on, and the log says so once per slot's
//! call (MASU_OUTS_STRICT=1: fails the run).
use std::cell::Cell;

/// At most this many out-parameters of one call are watched (the metadata's most: 7, the
/// D3D11 device creators').
pub const MAX: usize = 8;

/// The slots of one call: who (a trap address the reporter names) and where.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Slots {
    who: u32,
    n: u8,
    at: [u32; MAX],
}

impl Slots {
    /// The slots at `at` (NULL ones skipped: nothing is written there; past MAX not watched)
    /// of the call `who`.
    pub fn new(who: u32, at: impl IntoIterator<Item = u32>) -> Slots {
        let mut s = Slots { who, n: 0, at: [0; MAX] };
        for a in at.into_iter().filter(|&a| a != 0) {
            if let Some(x) = s.at.get_mut(usize::from(s.n)) {
                *x = a;
                s.n = s.n.saturating_add(1);
            }
        }
        s
    }
    #[inline]
    fn holds(&self, a: u32) -> bool {
        self.at.iter().take(usize::from(self.n)).any(|&x| x == a)
    }
}

thread_local! {
    static CUR: Cell<Slots> = const { Cell::new(Slots { who: 0, n: 0, at: [0; MAX] }) };
    /// CUR's n (a write asks it first: most calls have no such slot)
    static ANY: Cell<bool> = const { Cell::new(false) };
}

/// The slots watched while it lives; the ones before come back when it drops.
#[must_use]
pub struct Guard {
    was: Slots,
}

impl Drop for Guard {
    fn drop(&mut self) {
        CUR.with(|c| c.set(self.was));
        ANY.with(|c| c.set(self.was.n != 0));
    }
}

/// Watch `s` for the call now running on this thread (a 32-bit write to such a slot is short
/// of it on x64, and the same mistake on x86, where sake's tests run).
pub fn enter(s: Slots) -> Guard {
    ANY.with(|c| c.set(s.n != 0));
    Guard { was: CUR.with(|c| c.replace(s)) }
}

/// A 32-bit value is being written at `a`: reported when that is one of the running call's
/// pointer-sized out-parameters.
#[inline]
pub fn wrote32(a: u32) {
    if !ANY.with(Cell::get) {
        return;
    }
    let s = CUR.with(Cell::get);
    if s.holds(a) {
        short_write(s.who, a);
    }
}

/// MASU_OUTS_STRICT=1 (tests/host-tests.sh: the programs' runs on the release builds): a
/// report fails the run as it does a test.
pub fn strict() -> bool {
    static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var("MASU_OUTS_STRICT").as_deref() == Ok("1"))
}

static FAIL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A dependent's tests (sake's, under `cargo test --release` too, where this crate's
/// cfg!(test) and debug_assertions are off): a report fails like a test's.
pub fn fail_reports() {
    FAIL.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Names a call for the report (sake's: its DLL!function or interface::method).
pub type Namer = fn(u32) -> String;
static NAMER: std::sync::OnceLock<Namer> = std::sync::OnceLock::new();

/// The reporter's names (set once, by sake).
pub fn set_namer(f: Namer) {
    NAMER.get_or_init(|| f);
}

#[cold]
fn short_write(who: u32, a: u32) {
    let name = NAMER.get().map_or_else(|| format!("{who:#x}"), |f| f(who));
    let what = format!("{name}: a 32-bit write to its pointer-sized out-parameter at {a:#x} (the upper half stays the caller's): mem::putp / wp / api::Out");
    assert!(!cfg!(test) && !cfg!(debug_assertions) && !strict() && !FAIL.load(std::sync::atomic::Ordering::Relaxed), "{what}");
    static SAID: std::sync::Mutex<Option<std::collections::HashSet<(u32, u32)>>> = std::sync::Mutex::new(None);
    let mut g = SAID.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if g.get_or_insert_with(Default::default).insert((who, a)) {
        eprintln!("kura: {what}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn caught(a: u32) -> bool {
        std::panic::catch_unwind(|| wrote32(a)).is_err()
    }

    proptest! {
        /// Any slots, nested any way: a write is reported exactly when it is at one of the
        /// innermost call's slots, and leaving a call watches the outer one's again.
        #[test]
        fn reported_exactly_at_the_running_calls_slots(calls in proptest::collection::vec(proptest::collection::vec(1u32..64, 0..=MAX), 1..5), probe in proptest::collection::vec(0u32..64, 1..20)) {
            let mut guards = vec![];
            let mut stack: Vec<Slots> = vec![];
            for (k, at) in calls.iter().enumerate() {
                let s = Slots::new(kome::num::u32_of(k), at.iter().map(|x| x * 8));
                stack.push(s);
                guards.push(enter(s));
                for &p in &probe {
                    let a = p * 8;
                    let want = at.iter().any(|&x| x * 8 == a);
                    prop_assert_eq!(caught(a), want, "call {} write {:#x}", k, a);
                }
            }
            while let Some(g) = guards.pop() {
                drop(g);
                stack.pop();
                let now = stack.last().copied().unwrap_or_default();
                prop_assert_eq!(CUR.with(Cell::get), now);
            }
            prop_assert!(!caught(8));
        }
    }

    /// The upper dword of a slot is no slot: writing the two halves as words reports the lower.
    #[test]
    fn only_the_slot_itself() {
        let _g = enter(Slots::new(1, [0x1000, 0, 0x2000]));
        // (NULL skipped: two slots)
        assert_eq!(CUR.with(Cell::get).n, 2);
        assert!(caught(0x1000));
        assert!(!caught(0x1004));
        assert!(caught(0x2000));
        assert!(!caught(0));
    }
}
