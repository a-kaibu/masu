//! The guest bytes translated code was made from, as disjoint intervals: a store hits code
//! when [addr, addr + len) meets one (exactly: the store's whole span, not its first byte; one
//! starting below a block's start and running into it is a hit too, review e18bf44), and the
//! pages a translation drop covers are cut out of them (what is left on other pages stays,
//! still translated).
use std::collections::BTreeMap;

/// A store's (or a code range's) span: `len` bytes from `addr`, never empty; it may end at
/// 4 GB (summed wide).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    addr: u32,
    len: std::num::NonZeroU32,
}

impl Span {
    /// None for an empty span.
    pub(crate) fn new(addr: u32, len: u32) -> Option<Span> {
        std::num::NonZeroU32::new(len).map(|len| Span { addr, len })
    }
    /// `len` bytes from `addr`, 1 for 0 (a code range's: its first byte at least).
    pub(crate) fn at_least_one(addr: u32, len: u32) -> Span {
        Span { addr, len: std::num::NonZeroU32::new(len).unwrap_or(std::num::NonZeroU32::MIN) }
    }
    pub(crate) fn addr(self) -> u32 {
        self.addr
    }
    pub(crate) fn len(self) -> u32 {
        self.len.get()
    }
    /// One past the last byte (up to 2^32).
    pub(crate) fn end(self) -> u64 {
        // (two u32s: no u64 overflow)
        u64::from(self.addr).saturating_add(u64::from(self.len.get()))
    }
}

/// Disjoint, non-adjacent intervals start -> end (end exclusive, up to 2^32).
#[derive(Default, Debug)]
pub(crate) struct CodeRanges(BTreeMap<u32, u64>);

impl CodeRanges {
    pub(crate) const fn new() -> CodeRanges {
        CodeRanges(BTreeMap::new())
    }

    /// Adds `s`, merged with those it meets or touches.
    pub(crate) fn insert(&mut self, s: Span) {
        let (mut lo, mut hi) = (s.addr, s.end());
        // (the one starting at or below lo that reaches it, then each starting up to hi)
        if let Some((&a, &e)) = self.0.range(..=lo).next_back()
            && e >= u64::from(lo)
        {
            lo = a;
            hi = hi.max(e);
        }
        let met: Vec<(u32, u64)> = self.0.range(lo..).take_while(|(a, _)| u64::from(**a) <= hi).map(|(&a, &e)| (a, e)).collect();
        for (a, e) in met {
            self.0.remove(&a);
            hi = hi.max(e);
        }
        self.0.insert(lo, hi);
    }

    /// Whether `s` meets any.
    pub(crate) fn meets(&self, s: Span) -> bool {
        // (the last one starting before s ends: disjoint ones start in order, so it is the one
        // whose end is furthest among those that could reach s)
        let last = if s.end() > u64::from(u32::MAX) { self.0.iter().next_back() } else { self.0.range(..clamp32(s.end())).next_back() };
        last.is_some_and(|(_, &e)| e > u64::from(s.addr))
    }

    /// Cuts [lo, hi) out (hi up to 2^32).
    pub(crate) fn remove(&mut self, lo: u64, hi: u64) {
        if lo >= hi {
            return;
        }
        let cut: Vec<(u32, u64)> = self.0.iter().filter(|&(&a, &e)| u64::from(a) < hi && e > lo).map(|(&a, &e)| (a, e)).collect();
        for (a, e) in cut {
            self.0.remove(&a);
            if u64::from(a) < lo {
                self.0.insert(a, lo);
            }
            if e > hi {
                self.0.insert(clamp32(hi), e);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }

    #[cfg(test)]
    fn intervals(&self) -> Vec<(u32, u64)> {
        self.0.iter().map(|(&a, &e)| (a, e)).collect()
    }
}

/// `v` as a u32, saturating at u32::MAX (not the low bits: 2^32 must stay the top, not 0).
fn clamp32(v: u64) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The model: a byte set over a small address space at the top and bottom of 4 GB.
    #[derive(Clone, Debug)]
    enum Op {
        Insert(u32, u32),
        Remove(u64, u64),
        Meets(u32, u32),
    }

    const AT: [u32; 2] = [0, u32::MAX - 255];

    fn op() -> impl Strategy<Value = Op> {
        let addr = (0usize..2, 0u32..256).prop_map(|(w, o)| AT[w].wrapping_add(o));
        prop_oneof![
            (addr.clone(), 1u32..40).prop_map(|(a, n)| Op::Insert(a, n)),
            (addr.clone(), 0u32..80).prop_map(|(a, n)| Op::Remove(u64::from(a), u64::from(a) + u64::from(n))),
            (addr, 1u32..40).prop_map(|(a, n)| Op::Meets(a, n)),
        ]
    }

    proptest! {
        /// Against a set of bytes: a span meets the intervals exactly when it shares a byte
        /// with what was inserted and not cut out since; the intervals stay disjoint, sorted
        /// and apart.
        #[test]
        fn as_a_set_of_bytes(ops in proptest::collection::vec(op(), 1..60)) {
            let mut r = CodeRanges::new();
            let mut bytes = std::collections::BTreeSet::<u64>::new();
            for op in ops {
                match op {
                    Op::Insert(a, n) => {
                        // (code ends at 4 GB at the latest)
                        let s = Span::at_least_one(a, n.min(clamp32((1 << 32) - u64::from(a))));
                        r.insert(s);
                        bytes.extend(u64::from(a)..s.end());
                    }
                    Op::Remove(lo, hi) => {
                        r.remove(lo, hi.min(1 << 32));
                        bytes.retain(|&b| b < lo || b >= hi);
                    }
                    Op::Meets(a, n) => {
                        let s = Span::new(a, n).unwrap();
                        let want = (u64::from(a)..s.end()).any(|b| bytes.contains(&b));
                        prop_assert_eq!(r.meets(s), want, "{:?}", r.intervals());
                    }
                }
                let iv = r.intervals();
                for w in iv.windows(2) {
                    prop_assert!(w[0].1 < u64::from(w[1].0), "disjoint and apart: {:?}", iv);
                }
                for &(a, e) in &iv {
                    prop_assert!(u64::from(a) < e);
                    prop_assert!((u64::from(a)..e).all(|b| bytes.contains(&b)), "{:?}", iv);
                }
                prop_assert_eq!(iv.iter().map(|&(a, e)| e - u64::from(a)).sum::<u64>(), u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            }
        }
    }

    #[test]
    fn a_store_running_into_code_from_below_hits_it() {
        let mut r = CodeRanges::new();
        r.insert(Span::new(0x40_1000, 0x20).unwrap());
        assert!(r.meets(Span::new(0x40_0ffe, 4).unwrap()));
        assert!(!r.meets(Span::new(0x40_0ffc, 4).unwrap()));
        assert!(r.meets(Span::new(0x40_101f, 1).unwrap()));
        assert!(!r.meets(Span::new(0x40_1020, 8).unwrap()));
        // (a long function's range: a store far past its start, more than a page in)
        r.insert(Span::new(0x50_0000, 0x3000).unwrap());
        assert!(r.meets(Span::new(0x50_2ff0, 4).unwrap()));
        r.remove(0x50_1000, 0x50_2000);
        assert!(!r.meets(Span::new(0x50_1800, 4).unwrap()));
        assert!(r.meets(Span::new(0x50_2ff0, 4).unwrap()) && r.meets(Span::new(0x50_0ff0, 4).unwrap()));
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// For any span and any one code range: meets says whether they share a byte.
    #[kani::proof]
    #[kani::unwind(4)]
    fn meets_is_intersection() {
        let (a, n, b, m): (u32, u32, u32, u32) = (kani::any(), kani::any(), kani::any(), kani::any());
        let (Some(code), Some(store)) = (Span::new(a, n), Span::new(b, m)) else { return };
        let mut r = CodeRanges::new();
        r.insert(code);
        let shared = u64::from(store.addr()) < code.end() && u64::from(code.addr()) < store.end();
        assert_eq!(r.meets(store), shared);
        kani::cover!(shared && store.addr() < code.addr(), "a store running into the code from below");
        kani::cover!(!shared, "a store missing it");
    }

    /// One operation of a sequence: insert a span, or cut [lo, hi) out.
    #[derive(Clone, Copy)]
    enum Op {
        Insert(Span),
        Remove(u64, u64),
    }

    fn any_op() -> Op {
        if kani::any() {
            let (a, n): (u32, u32) = (kani::any(), kani::any());
            // (code ends at 4 GB at the latest: what translate() records)
            kani::assume(n >= 1 && u64::from(a) + u64::from(n) <= 1 << 32);
            Op::Insert(Span::at_least_one(a, n))
        } else {
            let (lo, hi): (u64, u64) = (kani::any(), kani::any());
            kani::assume(hi <= 1 << 32);
            Op::Remove(lo, hi)
        }
    }

    /// The intervals the operations make, from none
    fn run(ops: &[Op]) -> CodeRanges {
        let mut r = CodeRanges::new();
        for op in ops {
            match *op {
                Op::Insert(s) => r.insert(s),
                Op::Remove(lo, hi) => r.remove(lo, hi),
            }
        }
        r
    }

    /// The byte set the operations leave: whether byte `x` is in it (the last op covering it
    /// decides; none: not in it).
    fn holds(ops: &[Op], x: u64) -> bool {
        ops.iter().fold(false, |held, op| match *op {
            Op::Insert(s) => held || (u64::from(s.addr()) <= x && x < s.end()),
            Op::Remove(lo, hi) => held && !(lo <= x && x < hi),
        })
    }

    /// Guarantees, for a span inserted and then any insert or remove of any bounds (assumed: an
    /// inserted span ends by 4 GB, a cut's end too, as their callers give them): the intervals
    /// stay sorted, non-empty, disjoint and apart (no two touch), and for every byte (a
    /// symbolic one) `meets` agrees with the byte set: insert is the
    /// union, remove the difference. The bound is the 2 operations (a cut splitting the interval,
    /// a second one beside it, merged with it or inside it); 3 symbolic operations over the
    /// BTreeMap took CBMC past 9 GB and 40 minutes. Longer sequences: the proptest
    /// `as_a_set_of_bytes` runs 60 over a small address space at both ends of 4 GB.
    #[kani::proof]
    #[kani::unwind(6)]
    #[kani::solver(cadical)]
    fn ops_on_a_range_are_a_set_of_bytes() {
        let (a0, n0): (u32, u32) = (kani::any(), kani::any());
        kani::assume(n0 >= 1 && u64::from(a0) + u64::from(n0) <= 1 << 32);
        let ops = [Op::Insert(Span::at_least_one(a0, n0)), any_op()];
        let r = run(&ops);
        let mut prev: Option<u64> = None;
        for (&a, &e) in &r.0 {
            assert!(u64::from(a) < e && e <= 1 << 32);
            if let Some(p) = prev {
                assert!(p < u64::from(a), "disjoint and apart");
            }
            prev = Some(e);
        }
        let x: u32 = kani::any();
        assert_eq!(r.meets(Span::at_least_one(x, 1)), holds(&ops, u64::from(x)));
        kani::cover!(matches!(ops, [_, Op::Remove(..)]) && r.0.len() == 2, "a cut splitting one");
        kani::cover!(matches!(ops, [_, Op::Insert(_)]) && r.0.len() == 2, "two apart");
        kani::cover!(matches!(ops, [_, Op::Insert(s)] if s.addr() > a0) && r.0.len() == 1, "two merged");
    }

    /// The same sequences, for a span of any length (symbolic): it meets the intervals exactly
    /// when one of its bytes is in the set the operations leave.
    #[kani::proof]
    #[kani::unwind(6)]
    #[kani::solver(cadical)]
    fn ops_on_a_range_meet_as_the_bytes_do() {
        let (a0, n0): (u32, u32) = (kani::any(), kani::any());
        kani::assume(n0 >= 1 && u64::from(a0) + u64::from(n0) <= 1 << 32);
        let ops = [Op::Insert(Span::at_least_one(a0, n0)), any_op()];
        let r = run(&ops);
        // (a longer span: it meets them when a byte of it is held; its bytes checked through
        // the interval it would share, the first held byte at or after its start)
        let (b, m): (u32, u32) = (kani::any(), kani::any());
        if let Some(s) = Span::new(b, m) {
            let y: u64 = kani::any();
            kani::assume(u64::from(b) <= y && y < s.end());
            if holds(&ops, y) {
                assert!(r.meets(s));
            }
            if r.meets(s) {
                // some interval shares a byte with s: that byte is held
                let shared = r.0.iter().find(|&(&a, &e)| u64::from(a) < s.end() && e > u64::from(b)).map(|(&a, _)| u64::from(a).max(u64::from(b)));
                assert!(shared.is_some_and(|z| holds(&ops, z)));
            }
        }
        kani::cover!(Span::new(b, m).is_some_and(|s| r.meets(s) && !holds(&ops, u64::from(b))), "a span meeting them past its first byte");
    }
}
