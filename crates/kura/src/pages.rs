//! A file's bytes in memory, kept in pages of PAGE bytes instead of one Vec: a big file written
//! a piece at a time (an installer extracting a 457 MB app.asar) never needs one block of twice
//! its size, as a doubling Vec does (#234), and never moves what is already written. A page holds
//! only what was written into it (bytes past its end, up to the file's length, read as zero), so
//! a small file costs its size and a file made long by set_len costs nothing until written.

/// Bytes per page.
pub const PAGE: usize = 1 << 20;
/// PAGE as a file offset
const PAGE64: u64 = 1 << 20;
const _: () = assert!(PAGE64.count_ones() == 1 && PAGE64.trailing_zeros() == PAGE.trailing_zeros());

// The caller has already bounded at by the file length, whose page count fits usize.
fn page_location(at: u64) -> (usize, usize) {
    ((at / PAGE64) as usize, (at % PAGE64) as usize)
}

/// A length the file cannot have: past u64 (an offset near u64::MAX plus a write) or one whose
/// page list the memory does not hold (review 2026-11-16: write's `off + len` overflowed, and
/// set_len relied on its caller having asked room() first).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TooLong;

#[derive(Default, Clone)]
pub struct Pages {
    len: u64,
    /// `ceil(len / PAGE)` pages; page `i` holds at most `min(PAGE, len - i * PAGE)` bytes.
    pages: Vec<Vec<u8>>,
}

impl std::fmt::Debug for Pages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pages({} bytes)", self.len)
    }
}

impl Pages {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_slice(b: &[u8]) -> Self {
        Pages { len: b.len() as u64, pages: b.chunks(PAGE).map(<[u8]>::to_vec).collect() }
    }

    pub fn from_vec(v: Vec<u8>) -> Self {
        if v.len() <= PAGE {
            let len = v.len() as u64;
            return Pages { len, pages: if v.is_empty() { Vec::new() } else { vec![v] } };
        }
        Self::from_slice(&v)
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Copies bytes from `off` into `buf`; how many (fewer at the end of the file).
    pub fn read(&self, off: u64, buf: &mut [u8]) -> usize {
        if off >= self.len {
            return 0;
        }
        let n = buf.len().min(usize::try_from(self.len - off).unwrap_or(usize::MAX));
        let mut done = 0;
        while done < n {
            let at = off + done as u64;
            let (i, o) = page_location(at);
            let k = (PAGE - o).min(n - done);
            let page = &self.pages[i];
            let have = page.len().saturating_sub(o).min(k);
            if have > 0 {
                buf[done..done + have].copy_from_slice(&page[o..o + have]);
            }
            buf[done + have..done + k].fill(0);
            done += k;
        }
        n
    }

    /// Writes `data` at `off`, making the file longer (zeros between) when it ends past it;
    /// nothing is written (Err) when it would end past what a file can be.
    pub fn write(&mut self, off: u64, data: &[u8]) -> Result<(), TooLong> {
        if data.is_empty() {
            return Ok(());
        }
        let end = u64::try_from(data.len()).ok().and_then(|n| off.checked_add(n)).ok_or(TooLong)?;
        if end > self.len {
            self.set_len(end)?;
        }
        let mut done = 0;
        while done < data.len() {
            let at = off + done as u64;
            let (i, o) = page_location(at);
            let k = (PAGE - o).min(data.len() - done);
            let page = &mut self.pages[i];
            if page.len() < o + k {
                let want = o + k;
                if page.capacity() < want {
                    // doubling, but never past a page
                    let cap = want.max(page.capacity() * 2).min(PAGE);
                    page.reserve_exact(cap - page.len());
                }
                page.resize(want, 0);
            }
            page[o..o + k].copy_from_slice(&data[done..done + k]);
            done += k;
        }
        Ok(())
    }

    /// Whether the file can be `end` bytes long: its page list made room for (false when the
    /// memory is not there; a page itself is at most PAGE bytes, its data what is written).
    /// set_len and write ask it themselves; a caller asks first to fail before other work.
    pub fn room(&mut self, end: u64) -> bool {
        let Ok(count) = usize::try_from(end.div_ceil(PAGE as u64)) else { return false };
        count <= self.pages.len() || self.pages.try_reserve(count - self.pages.len()).is_ok()
    }

    /// Cuts or lengthens (with zeros) the file to `n` bytes; unchanged (Err) when its page list
    /// cannot be made.
    pub fn set_len(&mut self, n: u64) -> Result<(), TooLong> {
        if !self.room(n) {
            return Err(TooLong);
        }
        let count = usize::try_from(n.div_ceil(PAGE64)).ok().ok_or(TooLong)?;
        if n < self.len {
            self.pages.truncate(count);
            if let Some(last) = self.pages.last_mut() {
                let keep = (n - (count as u64 - 1) * PAGE as u64) as usize;
                last.truncate(keep);
                if last.capacity() > 2 * keep.max(4096) {
                    last.shrink_to_fit();
                }
            }
        } else {
            self.pages.resize_with(count, Vec::new);
        }
        self.len = n;
        Ok(())
    }

    /// The bytes in order, a piece at a time (zeros for what was never written).
    pub fn for_each(&self, mut f: impl FnMut(&[u8])) {
        let zeros = [0u8; 4096];
        for (i, page) in self.pages.iter().enumerate() {
            let size = ((self.len - i as u64 * PAGE as u64).min(PAGE as u64)) as usize;
            f(page);
            let mut rest = size - page.len();
            while rest > 0 {
                let k = rest.min(zeros.len());
                f(&zeros[..k]);
                rest -= k;
            }
        }
    }

    pub fn to_vec(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.len as usize);
        self.for_each(|b| v.extend_from_slice(b));
        v
    }

    /// Bytes held in memory (what was written, not the file's length).
    pub fn held(&self) -> usize {
        self.pages.iter().map(Vec::capacity).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[derive(Debug, Clone)]
    enum Op {
        Write(u64, Vec<u8>),
        SetLen(u64),
    }

    /// Offsets around page edges, where the arithmetic can slip.
    fn off() -> impl Strategy<Value = u64> {
        prop_oneof![0u64..4 * PAGE as u64, (0u64..5).prop_flat_map(|p| (p * PAGE as u64).saturating_sub(8)..p * PAGE as u64 + 8)]
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            3 => (off(), prop::collection::vec(any::<u8>(), 0..3000)).prop_map(|(o, d)| Op::Write(o, d)),
            1 => off().prop_map(Op::SetLen),
        ]
    }

    fn apply(model: &mut Vec<u8>, op: &Op) {
        match op {
            Op::Write(o, d) => {
                if d.is_empty() {
                    return;
                }
                let end = *o as usize + d.len();
                if model.len() < end {
                    model.resize(end, 0);
                }
                model[*o as usize..end].copy_from_slice(d);
            }
            Op::SetLen(n) => model.resize(*n as usize, 0),
        }
    }

    #[test]
    fn reads_start_of_sparse_four_gib_file() {
        let mut file = Pages::default();
        file.write(0, b"abc").unwrap();
        file.set_len(1u64 << 32).unwrap();
        let mut bytes = [0; 3];
        assert_eq!(file.read(0, &mut bytes), 3);
        assert_eq!(&bytes, b"abc");
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// Any writes and truncations read back as a plain Vec would hold them, from any offset.
        #[test]
        fn pages_match_a_vec(ops in prop::collection::vec(op(), 1..20), probes in prop::collection::vec((off(), 0usize..5000), 8)) {
            let (mut p, mut m) = (Pages::new(), Vec::new());
            for o in &ops {
                match o {
                    Op::Write(at, d) => p.write(*at, d).unwrap(),
                    Op::SetLen(n) => p.set_len(*n).unwrap(),
                }
                apply(&mut m, o);
                prop_assert_eq!(p.len(), m.len() as u64);
            }
            prop_assert!(p.to_vec() == m, "contents differ");
            for (at, n) in probes {
                let mut got = vec![0xaa; n];
                let k = p.read(at, &mut got);
                let start = (at as usize).min(m.len());
                let want = &m[start..(start + n).min(m.len())];
                prop_assert_eq!(k, want.len());
                prop_assert!(&got[..k] == want);
            }
            prop_assert!(p.held() <= p.pages.len() * PAGE);
        }

        /// A file written in pieces holds the same bytes whatever order the pieces come in.
        #[test]
        fn write_order_does_not_matter(data in prop::collection::vec(any::<u8>(), 1..(3 * PAGE + 77)).no_shrink(), cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..12), order in any::<u64>()) {
            let mut at: Vec<usize> = cuts.iter().map(|c| c.index(data.len())).collect();
            at.extend([0, data.len()]);
            at.sort_unstable();
            at.dedup();
            let mut pieces: Vec<(usize, usize)> = at.windows(2).map(|w| (w[0], w[1])).collect();
            let mut s = order | 1;
            for i in (1..pieces.len()).rev() {
                s ^= s << 13; s ^= s >> 7; s ^= s << 17;
                pieces.swap(i, (s % (i as u64 + 1)) as usize);
            }
            let mut p = Pages::new();
            for (a, b) in pieces {
                p.write(a as u64, &data[a..b]).unwrap();
            }
            prop_assert!(p.to_vec() == data);
        }

        /// Snapshots and exports go through from_slice / for_each: the same bytes come back.
        #[test]
        fn from_slice_round_trips(data in prop::collection::vec(any::<u8>(), 0..(2 * PAGE + 9)).no_shrink()) {
            prop_assert!(Pages::from_slice(&data).to_vec() == data);
            prop_assert!(Pages::from_vec(data.clone()).to_vec() == data);
        }
    }

    /// A 600 MB file written 64 KiB at a time (as nsis7z writes app.asar) never holds more than
    /// its own size plus one page, where a doubling Vec reserves up to twice its size in one block.
    #[test]
    fn room_says_when_the_page_list_cannot_be_made() {
        let mut p = Pages::from_slice(b"abc");
        assert!(p.room(10 << 30));
        assert!(!p.room(isize::MAX as u64 - 8));
        assert!(!p.room(u64::MAX));
        assert_eq!(p.len(), 3);
        let mut b = [0u8; 3];
        assert_eq!(p.read(0, &mut b), 3);
        assert_eq!(&b, b"abc");
    }

    #[test]
    fn a_big_file_written_in_pieces_holds_about_its_size() {
        let size = 600usize << 20;
        let piece = vec![7u8; 64 << 10];
        let mut p = Pages::new();
        let mut at = 0;
        while at < size {
            p.write(at as u64, &piece).unwrap();
            at += piece.len();
        }
        assert_eq!(p.len(), size as u64);
        assert!(p.held() <= size + PAGE, "held {} for {}", p.held(), size);
        let mut b = [0u8; 3];
        assert_eq!(p.read(size as u64 - 2, &mut b), 2);
        assert_eq!(&b[..2], &[7, 7]);
    }

    /// set_len past the end costs nothing until written, and reads as zero.
    #[test]
    fn a_long_empty_file_holds_nothing() {
        let mut p = Pages::new();
        p.set_len(10 << 30).unwrap();
        assert_eq!(p.held(), 0);
        let mut b = [1u8; 16];
        assert_eq!(p.read((5 << 30) + 3, &mut b), 16);
        assert_eq!(b, [0; 16]);
        p.write(1 << 30, b"x").unwrap();
        assert!(p.held() <= 4096);
    }

    proptest! {
        /// Review 2026-11-16: any offset and length, the ones near u64::MAX too: an Err and the
        /// file as it was, or the write as a Vec would hold it; never a panic (overflow checks).
        #[test]
        fn huge_offsets_and_lengths_are_errors(off in prop_oneof![any::<u64>(), (u64::MAX - 4096)..=u64::MAX, Just(u64::MAX)], n in 0u64..64, len in prop_oneof![any::<u64>(), Just(u64::MAX), Just(u64::MAX >> 1)]) {
            let mut p = Pages::from_slice(b"abc");
            let before = p.to_vec();
            let fits = off.checked_add(n).is_some_and(|e| e <= 4 * PAGE as u64);
            let r = p.write(off, &vec![9u8; usize::try_from(n).unwrap()]);
            if n == 0 {
                prop_assert_eq!(r, Ok(()));
            } else if off.checked_add(n).is_none() {
                prop_assert_eq!(r, Err(TooLong));
                prop_assert!(p.to_vec() == before);
            } else if fits {
                prop_assert_eq!(r, Ok(()));
            }
            if r.is_err() {
                prop_assert_eq!(p.len(), 3);
            }
            let had = p.len();
            match p.set_len(len) {
                Ok(()) => prop_assert_eq!(p.len(), len),
                Err(TooLong) => prop_assert_eq!(p.len(), had),
            }
        }
    }

    /// A small file costs about its size, not a page.
    #[test]
    fn a_small_file_holds_its_size() {
        let mut p = Pages::new();
        for i in 0..100u64 {
            p.write(i * 10, b"0123456789").unwrap();
        }
        assert!(p.held() < 4096, "held {}", p.held());
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// Reads and writes stay inside their pages and the page the offset names: for any offset
    /// and length, the (page, offset in page, length in page) walk covers the range exactly once.
    #[kani::proof]
    #[kani::unwind(4)]
    fn page_walk_covers_the_range() {
        let off: u64 = kani::any();
        let n: usize = kani::any();
        kani::assume(n <= 2 * PAGE && off < (1u64 << 40));
        let mut done = 0usize;
        let mut steps = 0;
        while done < n {
            let at = off + done as u64;
            let (i, o) = page_location(at);
            let k = (PAGE - o).min(n - done);
            assert!(k > 0 && o + k <= PAGE);
            assert_eq!(i as u64 * PAGE as u64 + o as u64, at);
            done += k;
            steps += 1;
            assert!(steps <= 3);
        }
        assert_eq!(done, n);
    }

    /// set_len keeps the invariant the reads rely on: ceil(len / PAGE) pages, the last one no
    /// longer than what is left of the file.
    #[kani::proof]
    fn set_len_page_count() {
        let n: u64 = kani::any();
        kani::assume(n < (1u64 << 40));
        let count = n.div_ceil(PAGE as u64);
        if n > 0 {
            let keep = n - (count - 1) * PAGE as u64;
            assert!(keep >= 1 && keep <= PAGE as u64);
        } else {
            assert_eq!(count, 0);
        }
    }
}
