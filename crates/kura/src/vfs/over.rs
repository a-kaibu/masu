//! A file from the store written to (review of 6852be1, stage 5): the store's file it was, the
//! pages written over it, its length, and how far the store's bytes still show (`base`). A cut
//! takes the store's bytes past it away for good: a file cut and made long again reads zeros
//! there, never the store's bytes again. Before, the first write copied the whole file from the
//! store into memory (copy_up): one byte written into a 1 GiB archive held 1 GiB, fetched it all
//! first, and the saves kept all of it.
//!
//! A page a write covers only in part holds the store's bytes under it first: they are fetched
//! past the VFS lock (`fills`, then `fill`), and the write goes once none is missing.

use super::Hash;
use crate::pages::{Pages, TooLong, PAGE, PAGE64};
use std::collections::BTreeSet;
use std::sync::Arc;

/// A file as the store gave it: its size, its chunks (`None`: all zero) and where each starts
/// (its size last).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Store {
    pub(super) size: u64,
    pub(super) chunks: Arc<Vec<Option<Hash>>>,
    pub(super) starts: Arc<Vec<u64>>,
}

#[derive(Clone)]
pub(super) struct Over {
    store: Store,
    /// the store's bytes show below this (at most the store's size and the file's length)
    base: u64,
    /// what was written, the file's length theirs
    pages: Pages,
    /// the pages below `base` that hold their bytes themselves (the store's under them put in
    /// before they were written)
    own: BTreeSet<u64>,
}

impl std::fmt::Debug for Over {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Over({} bytes, the store's {} below {}, {} pages written)", self.len(), self.store.size, self.base, self.own.len())
    }
}

/// The same file: the same store file, base, length and pages written (what a save says).
impl PartialEq for Over {
    fn eq(&self, o: &Over) -> bool {
        let shape = |x: &Over| {
            let mut v = Vec::new();
            x.put_shape(&mut |b| v.extend_from_slice(b));
            v
        };
        self.store == o.store && shape(self) == shape(o)
    }
}

impl Eq for Over {}

/// The store's bytes `start..end` of page `page`, to put in before a write that covers that
/// page only in part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fill {
    pub(super) page: u64,
    pub(super) start: u64,
    pub(super) end: u64,
}

/// Why a write did not go.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Unwritten {
    /// these store bytes are to be put in first
    Fill(Vec<Fill>),
    /// it would end past what a file can be
    TooLong,
}

fn page_of(at: u64) -> u64 {
    at.checked_div(PAGE64).unwrap_or(0)
}

/// The pages bytes `off..end` lie in (`end` past `off`).
fn touched(off: u64, end: u64) -> std::ops::RangeInclusive<u64> {
    page_of(off)..=page_of(end.saturating_sub(1))
}

impl Over {
    /// The store's file, nothing written yet.
    pub(super) fn new(store: Store) -> Result<Over, TooLong> {
        let mut pages = Pages::new();
        pages.set_len(store.size)?;
        Ok(Over { base: store.size, store, pages, own: BTreeSet::new() })
    }

    pub(super) fn store(&self) -> &Store {
        &self.store
    }

    pub(super) fn len(&self) -> u64 {
        self.pages.len()
    }

    pub(super) fn base(&self) -> u64 {
        self.base
    }

    /// What was written (and the store's bytes put in under it), the file's length its own.
    pub(super) fn pages(&self) -> &Pages {
        &self.pages
    }

    /// Bytes held in memory: what was written, never the store's file.
    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.pages.held()
    }

    /// Nothing of the store's file shows any more (cut to nothing): it is the pages alone.
    pub(super) fn store_gone(&self) -> bool {
        self.base == 0
    }

    pub(super) fn into_pages(self) -> Pages {
        self.pages
    }

    /// The store's bytes page `i` shows (none: it holds its own, or the store's end is before it).
    fn shows(&self, i: u64) -> Option<(u64, u64)> {
        let start = i.saturating_mul(PAGE64);
        (start < self.base && !self.own.contains(&i)).then(|| (start, start.saturating_add(PAGE64).min(self.base)))
    }

    /// Copies into `buf` what memory holds of the bytes from `off`: how many there are (fewer at
    /// the end of the file), and the pieces of `buf` to be read from the store, in order.
    pub(super) fn read(&self, off: u64, buf: &mut [u8]) -> (usize, Vec<(usize, usize)>) {
        let n = self.pages.read(off, buf);
        let mut gaps: Vec<(usize, usize)> = Vec::new();
        let Some(end) = u64::try_from(n).ok().and_then(|k| off.checked_add(k)).filter(|&e| e > off) else { return (n, gaps) };
        for i in touched(off, end) {
            let Some((s, e)) = self.shows(i) else { continue };
            let (s, e) = (s.max(off), e.min(end));
            let (Ok(a), Ok(b)) = (usize::try_from(s.saturating_sub(off)), usize::try_from(e.saturating_sub(off))) else { continue };
            if a >= b {
                continue;
            }
            match gaps.last_mut() {
                Some(g) if g.1 == a => g.1 = b,
                _ => gaps.push((a, b)),
            }
        }
        (n, gaps)
    }

    /// The store's bytes to put in before `len` bytes are written at `off`.
    pub(super) fn fills(&self, off: u64, len: u64) -> Vec<Fill> {
        let Some(end) = off.checked_add(len).filter(|&e| e > off) else { return Vec::new() };
        touched(off, end)
            .filter_map(|i| {
                let (s, e) = self.shows(i)?;
                (off > s || end < e).then_some(Fill { page: i, start: s, end: e })
            })
            .collect()
    }

    /// Puts in the store's bytes `f` asked for (`bytes`, from `f.start`), unless the page holds
    /// its own since or a cut took them away since (then only what is left of them).
    pub(super) fn fill(&mut self, f: Fill, bytes: &[u8]) -> Result<(), TooLong> {
        let Some((s, e)) = self.shows(f.page) else { return Ok(()) };
        // (a cut only lowers the base: what shows now is at most what was fetched, and only that
        // much of it goes in)
        if s != f.start {
            return Ok(());
        }
        let Some(b) = usize::try_from(e.saturating_sub(s)).ok().and_then(|k| bytes.get(..k)) else { return Ok(()) };
        self.pages.write(s, b)?;
        self.own.insert(f.page);
        Ok(())
    }

    /// Writes `data` at `off` (the file made longer when it ends past it), once the store's
    /// bytes of the pages it covers in part are in.
    pub(super) fn write(&mut self, off: u64, data: &[u8]) -> Result<(), Unwritten> {
        let len = u64::try_from(data.len()).ok().ok_or(Unwritten::TooLong)?;
        let fills = self.fills(off, len);
        if !fills.is_empty() {
            return Err(Unwritten::Fill(fills));
        }
        self.pages.write(off, data).map_err(|TooLong| Unwritten::TooLong)?;
        if let Some(end) = off.checked_add(len).filter(|&e| e > off) {
            let below = self.base.div_ceil(PAGE64);
            self.own.extend(touched(off, end).take_while(|&i| i < below));
        }
        Ok(())
    }

    /// Cuts or lengthens (with zeros) the file to `n` bytes; the store's bytes past a cut never
    /// show again.
    pub(super) fn set_len(&mut self, n: u64) -> Result<(), TooLong> {
        self.pages.set_len(n)?;
        self.base = self.base.min(n);
        let below = self.base.div_ceil(PAGE64);
        self.own.retain(|&i| i < below);
        Ok(())
    }

    /// The pages written, as saved: base (u64), length (u64), their count (u32), then each: its
    /// number (u64), whether it holds its own bytes below the base (u8: 1, the store's under it
    /// put in; 0, past the base), the bytes it holds (u32 length, bytes; zeros after them to the
    /// page's end).
    pub(super) fn put_shape(&self, out: &mut dyn FnMut(&[u8])) {
        let written = || (0..self.len().div_ceil(PAGE64)).filter_map(|i| {
            let held = usize::try_from(i).ok().and_then(|k| self.pages.page(k)).unwrap_or(&[]);
            let own = self.own.contains(&i);
            (own || !held.is_empty()).then_some((i, own, held))
        });
        out(&self.base.to_le_bytes());
        out(&self.len().to_le_bytes());
        out(&u32::try_from(written().count()).unwrap_or(u32::MAX).to_le_bytes());
        for (i, own, held) in written() {
            out(&i.to_le_bytes());
            out(&[u8::from(own)]);
            out(&u32::try_from(held.len()).unwrap_or(u32::MAX).to_le_bytes());
            out(held);
        }
    }

    /// The byte count `put_shape` writes.
    pub(super) fn shape_len(&self) -> usize {
        let mut n = 0usize;
        self.put_shape(&mut |b| n = n.saturating_add(b.len()));
        n
    }

    /// The pages' numbers that hold their own bytes (what another process is told).
    pub(super) fn own(&self) -> &BTreeSet<u64> {
        &self.own
    }

    /// An overlay of `store` as `put_shape` wrote it, and what follows it in `b` (None: not one;
    /// a base past the store or the length, a page past the base's, out of order, or holding
    /// more than a page or the file).
    pub(super) fn get_shape(store: Store, b: &[u8]) -> Option<(Over, &[u8])> {
        let (base, b) = b.split_first_chunk::<8>()?;
        let (len, b) = b.split_first_chunk::<8>()?;
        let (n, mut b) = b.split_first_chunk::<4>()?;
        let (base, len, n) = (u64::from_le_bytes(*base), u64::from_le_bytes(*len), usize::try_from(u32::from_le_bytes(*n)).ok()?);
        if base > store.size || base > len || n > b.len().checked_div(13).unwrap_or(0) {
            return None;
        }
        let mut over = Over { base, store, pages: Pages::new(), own: BTreeSet::new() };
        over.pages.set_len(len).ok()?;
        let below = base.div_ceil(PAGE64);
        let mut last: Option<u64> = None;
        for _ in 0..n {
            let (i, rest) = b.split_first_chunk::<8>()?;
            let (own, rest) = rest.split_first()?;
            let (k, rest) = rest.split_first_chunk::<4>()?;
            let (i, k) = (u64::from_le_bytes(*i), usize::try_from(u32::from_le_bytes(*k)).ok()?);
            let (held, rest) = rest.split_at_checked(k)?;
            let at = i.checked_mul(PAGE64)?;
            let fits = at.checked_add(u64::try_from(k).ok()?).is_some_and(|e| e <= len);
            let own = match own {
                0 => false,
                1 if i < below => true,
                _ => return None,
            };
            if k > PAGE || !fits || last.is_some_and(|l| l >= i) {
                return None;
            }
            over.pages.write(at, held).ok()?;
            if own {
                over.own.insert(i);
            }
            last = Some(i);
            b = rest;
        }
        Some((over, b))
    }

    /// Takes in another process's length, base and pages holding their own (vfs/shared.rs);
    /// the bytes it wrote go to `pages_mut` after.
    pub(super) fn reshape(&mut self, len: u64, base: u64, own: BTreeSet<u64>) -> Result<(), TooLong> {
        self.pages.set_len(len)?;
        self.base = base.min(len).min(self.store.size);
        let below = self.base.div_ceil(PAGE64);
        self.own = own.into_iter().filter(|&i| i < below).collect();
        Ok(())
    }

    pub(super) fn pages_mut(&mut self) -> &mut Pages {
        &mut self.pages
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Clone, Debug)]
    enum Op {
        Write(u64, Vec<u8>),
        SetLen(u64),
        /// a write whose store bytes were fetched, then a cut came before it went
        CutWhileFetching(u64, Vec<u8>, u64),
    }

    /// Offsets around page edges.
    fn off() -> impl Strategy<Value = u64> {
        prop_oneof![0u64..4 * PAGE64, (0u64..5).prop_flat_map(|p| (p * PAGE64).saturating_sub(8)..p * PAGE64 + 8)]
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            4 => (off(), prop::collection::vec(any::<u8>(), 0..3000)).prop_map(|(o, d)| Op::Write(o, d)),
            2 => off().prop_map(Op::SetLen),
            1 => (off(), prop::collection::vec(any::<u8>(), 1..3000), off()).prop_map(|(o, d, n)| Op::CutWhileFetching(o, d, n)),
        ]
    }

    fn store(size: u64) -> Store {
        Store { size, chunks: Arc::new(Vec::new()), starts: Arc::new(vec![0, size]) }
    }

    /// A read as vfs.rs does it: memory's bytes, then the store's pieces.
    fn read_all(o: &Over, lower: &[u8], off: u64, n: usize) -> Vec<u8> {
        let mut buf = vec![0xaa; n];
        let (k, gaps) = o.read(off, &mut buf);
        for (a, b) in gaps {
            let at = usize::try_from(off).unwrap() + a;
            buf[a..b].copy_from_slice(&lower[at..at + (b - a)]);
        }
        buf.truncate(k);
        buf
    }

    /// The fills a write asks for, fetched (from `lower`) and put in.
    fn fetch(o: &mut Over, lower: &[u8], fills: &[Fill]) {
        for f in fills {
            let (s, e) = (usize::try_from(f.start).unwrap(), usize::try_from(f.end).unwrap());
            o.fill(*f, &lower[s..e]).unwrap();
        }
    }

    fn write(o: &mut Over, lower: &[u8], at: u64, d: &[u8]) {
        for _ in 0..3 {
            match o.write(at, d) {
                Ok(()) => return,
                Err(Unwritten::Fill(f)) => fetch(o, lower, &f),
                Err(Unwritten::TooLong) => panic!("too long"),
            }
        }
        panic!("a write still asks for fills after they were put in");
    }

    fn apply(m: &mut Vec<u8>, at: u64, d: &[u8]) {
        if d.is_empty() {
            return;
        }
        let (at, end) = (usize::try_from(at).unwrap(), usize::try_from(at).unwrap() + d.len());
        if m.len() < end {
            m.resize(end, 0);
        }
        m[at..end].copy_from_slice(d);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Writes and cuts over a store file read back as a plain Vec copied from it would hold
        /// them, a cut and grown file reading zeros where the store's bytes were; memory holds
        /// only the pages written, never the store's file; and the saved shape comes back the same.
        #[test]
        fn an_overlay_reads_as_a_copy(size in prop_oneof![0u64..3 * PAGE64 + 100, Just(2 * PAGE64)], seed in any::<u8>(), ops in prop::collection::vec(op(), 1..16), probes in prop::collection::vec((off(), 0usize..5000), 6)) {
            let lower: Vec<u8> = (0..size).map(|i| (i.to_le_bytes()[0]).wrapping_mul(31).wrapping_add(seed) | 1).collect();
            let mut o = Over::new(store(size)).unwrap();
            let mut m = lower.clone();
            let mut written = BTreeSet::new();
            for op in &ops {
                match op {
                    Op::Write(at, d) => {
                        write(&mut o, &lower, *at, d);
                        apply(&mut m, *at, d);
                        if !d.is_empty() {
                            written.extend(touched(*at, *at + u64::try_from(d.len()).unwrap()));
                        }
                    }
                    Op::SetLen(n) => {
                        o.set_len(*n).unwrap();
                        m.resize(usize::try_from(*n).unwrap(), 0);
                    }
                    Op::CutWhileFetching(at, d, n) => {
                        let fills = o.fills(*at, u64::try_from(d.len()).unwrap());
                        o.set_len(*n).unwrap();
                        m.resize(usize::try_from(*n).unwrap(), 0);
                        // (the store's bytes fetched before the cut: none past it may show)
                        fetch(&mut o, &lower, &fills);
                        write(&mut o, &lower, *at, d);
                        apply(&mut m, *at, d);
                        written.extend(touched(*at, *at + u64::try_from(d.len()).unwrap()));
                    }
                }
                prop_assert_eq!(o.len(), u64::try_from(m.len()).unwrap());
                prop_assert!(o.base() <= o.len() && o.base() <= size);
            }
            prop_assert!(read_all(&o, &lower, 0, m.len()) == m, "contents differ");
            for (at, n) in probes {
                let start = usize::try_from(at).unwrap().min(m.len());
                prop_assert!(read_all(&o, &lower, at, n) == m[start..(start + n).min(m.len())], "a read at {} of {} differs", at, n);
            }
            prop_assert!(o.held() <= written.len() * PAGE, "held {} for {} pages written", o.held(), written.len());
            let mut shape = Vec::new();
            o.put_shape(&mut |b| shape.extend_from_slice(b));
            prop_assert_eq!(shape.len(), o.shape_len());
            let (back, rest) = Over::get_shape(store(size), &shape).unwrap();
            prop_assert!(rest.is_empty());
            prop_assert!(read_all(&back, &lower, 0, m.len()) == m, "the saved shape reads otherwise");
            // (every cut in the header and the first page's, then some through the bytes)
            for k in (0..shape.len().min(64)).chain((64..shape.len()).step_by((shape.len() / 64).max(1))).chain(shape.len().checked_sub(1)) {
                if let Some((b, _)) = Over::get_shape(store(size), &shape[..k]) {
                    prop_assert!(false, "a shape cut short at {} read as one of {} bytes", k, b.len());
                }
            }
        }
    }

    /// A byte written into a big store file holds a page, not the file (copy_up held all of it).
    #[test]
    fn one_byte_into_a_big_file_holds_a_page() {
        let size = 1u64 << 30;
        let mut o = Over::new(store(size)).unwrap();
        let f = o.fills(size / 2, 1);
        assert_eq!(f.len(), 1);
        o.fill(f[0], &vec![7; PAGE]).unwrap();
        o.write(size / 2, b"x").unwrap();
        assert!(o.held() <= PAGE, "held {}", o.held());
        assert!(o.shape_len() <= PAGE + 64);
    }

    /// A cut to nothing and a write after: nothing of the store's file shows.
    #[test]
    fn cut_to_nothing_and_grown_reads_zeros() {
        let lower = vec![5u8; 100];
        let mut o = Over::new(store(100)).unwrap();
        o.set_len(0).unwrap();
        assert!(o.store_gone());
        o.set_len(100).unwrap();
        o.write(10, b"ab").unwrap();
        let mut want = vec![0u8; 100];
        want[10..12].copy_from_slice(b"ab");
        assert_eq!(read_all(&o, &lower, 0, 100), want);
    }
}
