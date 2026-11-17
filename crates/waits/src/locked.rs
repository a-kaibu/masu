//! x86 LOCK-prefixed operations on guest memory that wasm cannot do as one atomic instruction:
//! an operand that is not aligned to its size (x86 allows it, wasm atomics trap on it) and
//! CMPXCHG16B's 16 bytes (wasm has no 128-bit atomics). One rule for all of them, kura's
//! interpreter (cpu.rs Linear::cmpxchg, cmpxchg16) and sake's Interlocked functions
//! (mem::rmw32_at) alike (review 2026-11-17: each had its own lock, and the straddling case read
//! and wrote the bytes with plain accesses while aligned atomics on the same bytes took no lock):
//!
//! - Every access here is an atomic one on the aligned 8-byte words holding the operand: no
//!   plain read or write of guest memory that another thread's atomic could race with.
//! - An operand inside one aligned word ([`Place::Word`]) is one compare-exchange of that word:
//!   atomic against every other atomic access, whatever its size.
//! - An operand across two words ([`Place::Pair`], and [`Pair::WIDE`] for CMPXCHG16B) takes
//!   [`MULTI`], one lock for all of them: they are atomic among themselves. Each word is then
//!   written with a compare-exchange against the value the operation read, so a single-word
//!   atomic of another thread on either word is never overwritten (the operation undoes its
//!   first word and goes again instead): no update is lost.
//!
//! What it cannot give is the x86's single store of both words: between the two
//! compare-exchanges the first word holds the new bytes while the second holds the old ones, and
//! another thread's single-word access to the first word's part of the operand can see that
//! (it is then ordered after the pair operation, which still completes: fine). Only when single-
//! word operations write the operand's bytes in *both* words while a pair operation is between
//! its two writes can the result differ from some order of the three (loom's
//! `straddling_increment_racing_both_sides`, which documents it: a program racing a split lock
//! against aligned locks on both its halves).
//! (The aligned fast paths stay as they are, lock-free: taking a lock on every Interlocked call
//! for this rare case would cost every program.) `tests/loom.rs` checks every interleaving of
//! these with single-word atomics on both words: no lost update, and the pair operations
//! linearizable among themselves.

#[cfg(loom)]
use loom::sync::atomic::AtomicU64;
#[cfg(not(loom))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::SeqCst;

/// An operand's size in bytes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Width {
    B1,
    B2,
    B4,
    B8,
}

impl Width {
    pub const fn bytes(self) -> u32 {
        match self {
            Width::B1 => 1,
            Width::B2 => 2,
            Width::B4 => 4,
            Width::B8 => 8,
        }
    }
    /// its value bits
    const fn ones(self) -> u64 {
        match self {
            Width::B1 => 0xff,
            Width::B2 => 0xffff,
            Width::B4 => 0xffff_ffff,
            Width::B8 => u64::MAX,
        }
    }
}

/// Where an operand lies in the aligned words around it. Made only by [`place`] (or
/// [`Pair::WIDE`]): an operand inside one word cannot be handed to [`pair`], nor one across two
/// to [`word`].
/// ```compile_fail
/// let _ = waits::locked::Word { sh: 0, ones: 0xff };
/// ```
/// ```compile_fail
/// let _ = waits::locked::Pair { sh: 60, ones: 0xff };
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// inside the aligned word at the address with its low 3 bits cleared
    Word(Word),
    /// across that word and the next
    Pair(Pair),
}

/// An operand inside one aligned 8-byte word: at bit `sh`, `ones` its value bits
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Word {
    sh: u32,
    ones: u64,
}

/// An operand across two aligned words (the low one first): at bit `sh` of the pair
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    sh: u32,
    ones: u128,
}

impl Pair {
    /// CMPXCHG16B's operand: both words whole
    pub const WIDE: Pair = Pair { sh: 0, ones: u128::MAX };
}

/// Where an operand of `w` at `addr` lies (its aligned word: `addr & !7`)
pub fn place(addr: usize, w: Width) -> Place {
    let into = u32::try_from(addr & 7).unwrap_or(0);
    let sh = into.wrapping_mul(8);
    if into.wrapping_add(w.bytes()) <= 8 { Place::Word(Word { sh, ones: w.ones() }) } else { Place::Pair(Pair { sh, ones: u128::from(w.ones()) }) }
}

/// The locked operation `f` on the operand `at` of `word`: `f` gets its value and answers the
/// new one (None: nothing written, as a CMPXCHG that fails). Returns the value it saw. `f` may
/// run more than once (each time on what is there then): it must not do more than answer.
pub fn word(word: &AtomicU64, at: Word, mut f: impl FnMut(u64) -> Option<u64>) -> u64 {
    let m = at.ones << at.sh;
    let mut cur = word.load(SeqCst);
    loop {
        let v = (cur & m) >> at.sh;
        let Some(new) = f(v) else { return v };
        match word.compare_exchange_weak(cur, (cur & !m) | ((new & at.ones) << at.sh), SeqCst, SeqCst) {
            Ok(_) => return v,
            Err(c) => cur = c,
        }
    }
}

/// All operations on two words take this one (they are atomic among themselves)
#[cfg(not(loom))]
static MULTI: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(loom)]
loom::lazy_static! {
    static ref MULTI: loom::sync::Mutex<()> = loom::sync::Mutex::new(());
}

/// The low 64 bits
fn low(x: u128) -> u64 {
    u64::try_from(x & u128::from(u64::MAX)).unwrap_or(0)
}

/// `a` changed to what `f` answers for its value, unless None: whether it was (fetch_update,
/// renamed on newer toolchains and not in loom's)
fn update(a: &AtomicU64, f: impl Fn(u64) -> Option<u64>) -> bool {
    let mut cur = a.load(SeqCst);
    loop {
        let Some(new) = f(cur) else { return false };
        match a.compare_exchange_weak(cur, new, SeqCst, SeqCst) {
            Ok(_) => return true,
            Err(c) => cur = c,
        }
    }
}

/// Both words as they were at one moment (the first read again after the second: unchanged, so
/// it held that value when the second was read)
fn both(lo: &AtomicU64, hi: &AtomicU64) -> (u64, u64) {
    loop {
        let l = lo.load(SeqCst);
        let h = hi.load(SeqCst);
        if lo.load(SeqCst) == l {
            return (l, h);
        }
    }
}

/// [`word`] for an operand across `lo` and `hi` (the next aligned word): `f` as there, on a value
/// of up to 128 bits. Atomic among pair operations; never overwrites a single-word atomic of
/// another thread (see the module doc for what such an access may see meanwhile).
pub fn pair(lo: &AtomicU64, hi: &AtomicU64, at: Pair, mut f: impl FnMut(u128) -> Option<u128>) -> u128 {
    // (a poisoned lock guards no data of its own: the words are atomics)
    let _g = MULTI.lock().unwrap_or_else(|e| e.into_inner());
    let m = at.ones << at.sh;
    let (lm, hm) = (low(m), low(m >> 64));
    loop {
        let (l, h) = both(lo, hi);
        let cur = u128::from(l) | u128::from(h) << 64;
        let v = (cur & m) >> at.sh;
        let Some(new) = f(v) else { return v };
        let next = (cur & !m) | ((new & at.ones) << at.sh);
        let (nl, nh) = (low(next), low(next >> 64));
        if lo.compare_exchange(l, nl, SeqCst, SeqCst).is_err() {
            continue;
        }
        // (`hi`'s bytes outside the operand may have changed since: they stay as they are now)
        if update(hi, |c| (c & hm == h & hm).then_some((c & !hm) | (nh & hm))) {
            return v;
        }
        // (the operand's bytes in `hi` changed since they were read: those in `lo` back as they
        // were, where no other thread wrote them meanwhile, and again)
        update(lo, |c| (c & lm == nl & lm).then_some((c & !lm) | (l & lm)));
    }
}

/// CMPXCHG16B on the 16 bytes `lo`, `hi`: `new` stored if they hold `old`; what they held.
pub fn swap16(lo: &AtomicU64, hi: &AtomicU64, old: u128, new: u128) -> u128 {
    pair(lo, hi, Pair::WIDE, |v| (v == old).then_some(new))
}

#[cfg(all(test, not(loom)))]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The 24 bytes `b` as three words, an operand of `w` at byte `at` (< 8) done through
    /// place/word/pair; the bytes after
    fn run(b: [u8; 24], at: usize, w: Width, f: impl FnMut(u128) -> Option<u128>) -> (u128, [u8; 24]) {
        let words: Vec<AtomicU64> = b.chunks_exact(8).map(|c| AtomicU64::new(u64::from_le_bytes(c.try_into().unwrap_or([0; 8])))).collect();
        let mut f = f;
        let v = match place(at, w) {
            Place::Word(o) => u128::from(word(&words[0], o, |v| f(u128::from(v)).map(low))),
            Place::Pair(o) => pair(&words[0], &words[1], o, f),
        };
        let mut out = [0u8; 24];
        for (c, x) in out.chunks_exact_mut(8).zip(&words) {
            c.copy_from_slice(&x.load(SeqCst).to_le_bytes());
        }
        (v, out)
    }

    fn widths() -> impl Strategy<Value = Width> {
        prop_oneof![Just(Width::B1), Just(Width::B2), Just(Width::B4), Just(Width::B8)]
    }

    proptest! {
        /// Any operand at any offset of a word (inside it or across two): the value it sees is
        /// the little-endian bytes there, it writes exactly those bytes, and every other byte
        /// stays (a plain model of the bytes; differential against the old per-case code).
        #[test]
        fn operand_bytes_as_a_plain_model(b in any::<[u8; 24]>(), at in 0usize..8, w in widths(), add in any::<u64>(), fail in any::<bool>()) {
            let n = usize::try_from(w.bytes()).unwrap_or(8);
            let mut want = b;
            let mut seen = [0u8; 8];
            seen[..n].copy_from_slice(&b[at..at + n]);
            let seen = u64::from_le_bytes(seen);
            let new = seen.wrapping_add(add);
            if !fail {
                want[at..at + n].copy_from_slice(&new.to_le_bytes()[..n]);
            }
            let (v, got) = run(b, at, w, |v| (!fail).then_some(v.wrapping_add(u128::from(add))));
            prop_assert_eq!(v, u128::from(seen));
            prop_assert_eq!(got, want);
            prop_assert_eq!(matches!(place(at, w), Place::Pair(_)), at + n > 8);
        }

        /// CMPXCHG16B: stored only when both halves match, the old value always returned
        #[test]
        fn swap16_is_compare_and_swap(cur in any::<u128>(), old in any::<u128>(), new in any::<u128>(), same in any::<bool>()) {
            let old = if same { cur } else { old };
            let (lo, hi) = (AtomicU64::new(low(cur)), AtomicU64::new(low(cur >> 64)));
            prop_assert_eq!(swap16(&lo, &hi, old, new), cur);
            let now = u128::from(lo.load(SeqCst)) | u128::from(hi.load(SeqCst)) << 64;
            prop_assert_eq!(now, if old == cur { new } else { cur });
        }
    }

    /// Threads adding through straddling and in-word operands and through plain 64-bit atomics on
    /// the same words (as the aligned fast paths and the JIT's atomics do): no update lost.
    #[test]
    fn no_update_lost_against_single_word_atomics() {
        static W: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
        const N: u64 = 20_000;
        std::thread::scope(|s| {
            // a dword at byte 6 (bytes 6..10 across both words), counted up by one
            for _ in 0..2 {
                s.spawn(|| {
                    let Place::Pair(p) = place(6, Width::B4) else { unreachable!() };
                    for _ in 0..N {
                        pair(&W[0], &W[1], p, |v| Some(v + 1));
                    }
                });
            }
            // the low word's bytes 0..4 and the high word's bytes 4..8, by 64-bit atomics
            s.spawn(|| (0..N).for_each(|_| { W[0].fetch_add(1, SeqCst); }));
            s.spawn(|| (0..N).for_each(|_| { W[1].fetch_add(1 << 32, SeqCst); }));
        });
        let all = u128::from(W[0].load(SeqCst)) | u128::from(W[1].load(SeqCst)) << 64;
        assert_eq!((all >> 48) & 0xffff_ffff, u128::from(2 * N));
        assert_eq!(all & 0xffff_ffff, u128::from(N));
        assert_eq!(all >> 96, u128::from(N));
    }
}
