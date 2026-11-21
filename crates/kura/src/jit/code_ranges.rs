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
        u64::from(self.addr) + u64::from(self.len.get())
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
        let last = if s.end() > u64::from(u32::MAX) { self.0.iter().next_back() } else { self.0.range(..lo32(s.end())).next_back() };
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
                self.0.insert(lo32(hi), e);
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

/// `v` (below 2^32 where called) as a u32.
fn lo32(v: u64) -> u32 {
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
                        let s = Span::at_least_one(a, n.min(lo32((1 << 32) - u64::from(a))));
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
                prop_assert_eq!(iv.iter().map(|&(a, e)| e - u64::from(a)).sum::<u64>(), bytes.len() as u64);
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
        assert_eq!(r.meets(store), u64::from(store.addr()) < code.end() && u64::from(code.addr()) < store.end());
    }
}
