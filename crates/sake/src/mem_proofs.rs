//! Kani proofs of the address-space arithmetic of VirtualAlloc, VirtualProtect, VirtualFree and
//! VirtualQuery (formal/kani.sh). Each proof states what it guarantees, over which inputs and
//! what it assumes; `kani::cover!` shows the cases it is about are reached (masu-lint
//! kani-cover). They call the functions the address space uses, not copies of them.
use super::*;

/// The page index `i`'s bytes [i * PAGE, (i + 1) * PAGE) as wide integers
fn page_bytes(i: u32) -> (u64, u64) {
    (u64::from(i) * u64::from(PAGE), (u64::from(i) + 1) * u64::from(PAGE))
}

/// Guarantees, for every u32 size (no assumption, no bound): `page_size` is the size rounded
/// up to whole pages (0 taken as 1) when that fits below HI, None exactly when it does not.
#[kani::proof]
fn page_size_rounds_up_to_whole_pages() {
    let size: u32 = kani::any();
    let want = u64::from(size.max(1)).div_ceil(u64::from(PAGE)) * u64::from(PAGE);
    match page_size(size) {
        Some(n) => {
            assert_eq!(u64::from(n), want);
            assert_eq!(n % PAGE, 0);
            assert!(n >= size && n - size.max(1) < PAGE);
            assert!(n <= HI);
        }
        None => assert!(want > u64::from(HI)),
    }
    kani::cover!(page_size(size).is_none(), "a size near 4 GB is refused");
    kani::cover!(size == 0 && page_size(size) == Some(PAGE), "0 takes one page");
}

/// Guarantees, for every u32 value and every power of two `a` (assumed: callers pass GRAN or a
/// page): the result is a multiple of `a`, and the smallest one at or above `v` taken modulo
/// 2^32 (so it wraps to 0 past the top, as an address does).
#[kani::proof]
fn align_up_is_the_next_multiple() {
    let (v, a): (u32, u32) = (kani::any(), kani::any());
    kani::assume(a.is_power_of_two());
    let r = align_up(v, a);
    let want = (u64::from(v) + u64::from(a) - 1) / u64::from(a) * u64::from(a);
    assert_eq!(u64::from(r), want % (1 << 32));
    assert_eq!(r % a, 0);
    kani::cover!(want == 1 << 32, "wraps at the top");
    kani::cover!(r > v, "rounds up");
}

/// Guarantees, for every offset, length and allocation size of whole pages below HI (assumed:
/// what an allocation is) with the offset inside it and the length at least 1 (assumed:
/// commit_range passes n.max(1), decommit_range takes n = 0 apart): `first..last` is exactly
/// the pages of the allocation that [off, off + n) touches (checked for every page index by a
/// symbolic one), and it never runs backwards or past the allocation's end (no slice panic, no
/// underflowing span). A second proof drops the assumptions on off and n: it never panics and
/// first <= last <= pages for any input.
#[kani::proof]
fn page_span_is_the_pages_touched() {
    let (off, n, size, i): (u32, u32, u32, u32) = (kani::any(), kani::any(), kani::any(), kani::any());
    kani::assume(size % PAGE == 0 && size > 0 && size <= HI);
    kani::assume(off < size && n >= 1);
    let pages = size / PAGE;
    let (first, last) = page_span(off, n, size);
    assert!(first <= last && last <= pages);
    kani::assume(i < pages);
    let (lo, hi) = page_bytes(i);
    let touched = lo < u64::from(off) + u64::from(n) && u64::from(off) < hi;
    assert_eq!(first <= i && i < last, touched);
    assert_eq!(span(first, last), (last - first) * PAGE);
    kani::cover!(n > u32::MAX - PAGE, "a length near 4 GB (review A-13)");
    kani::cover!(touched && i == pages - 1 && off < PAGE && pages > 1, "a range to the allocation's end");
    kani::cover!(!touched && i > first, "a page after the range");
}

#[kani::proof]
fn page_span_never_runs_backwards() {
    let (off, n, size): (u32, u32, u32) = (kani::any(), kani::any(), kani::any());
    let (first, last) = page_span(off, n, size);
    assert!(first <= last && last <= size / PAGE);
    kani::cover!(off >= size, "an offset past the end");
}

/// Guarantees, for up to 2 holes of any bounds in the order `holes` makes them (assumed:
/// lo < hi, each hole after the last, as a walk of the allocations gives), any size and any
/// power of two alignment from GRAN up (assumed: reserve_aligned's callers): the result is
/// aligned and inside one hole with all `size` bytes, and no lower aligned address (a symbolic
/// one) has them in any hole; None only when no aligned address does.
#[kani::proof]
#[kani::unwind(4)]
fn aligned_fit_is_the_lowest_aligned_fit() {
    let h: [(u32, u32); 2] = kani::any();
    let len: usize = kani::any();
    kani::assume(len <= 2);
    kani::assume(h[0].0 < h[0].1 && h[1].0 < h[1].1 && h[0].1 <= h[1].0);
    let (size, align, c): (u32, u32, u32) = (kani::any(), kani::any(), kani::any());
    kani::assume(align.is_power_of_two() && align >= GRAN);
    let holes = &h[..len];
    let fits = |b: u32| b % align == 0 && holes.iter().any(|&(lo, hi)| lo <= b && u64::from(b) + u64::from(size) <= u64::from(hi));
    let got = aligned_fit(holes, size, align);
    if let Some(b) = got {
        assert!(fits(b));
    }
    if fits(c) {
        assert!(got.is_some_and(|b| b <= c));
    }
    kani::cover!(len == 2 && got.is_some_and(|b| b >= h[1].0), "the first hole too small, the second taken");
    kani::cover!(len == 2 && got.is_none(), "no hole holds it");
    kani::cover!(got.is_some_and(|b| b > h[0].0 && len >= 1), "a hole's start rounded up");
}

/// Guarantees, for every u32 size: a block size comes back exactly for 2^k - PAGE with
/// 2^k >= 32 MiB, and then a reservation of `size` bytes at any page aligned address (a
/// symbolic one, below 4 GiB) holds a whole block aligned to its size: what a program reserving
/// twice the block less a page expects (Unity 6's allocators).
#[kani::proof]
fn over_reserved_block_fits_aligned() {
    let (size, base): (u32, u32) = (kani::any(), kani::any());
    let s = u64::from(size) + u64::from(PAGE);
    let got = over_reserved_block(size);
    assert_eq!(got.is_some(), s.is_power_of_two() && s >= 32 << 20 && s <= 1 << 31);
    if let Some(b) = got {
        kani::assume(base % PAGE == 0);
        let b = u64::from(b);
        let at = u64::from(base).div_ceil(b) * b;
        assert!(at + b <= u64::from(base) + u64::from(size));
        kani::cover!(base % (b as u32) == PAGE, "the worst base: one page past a boundary");
    }
    kani::cover!(got.is_some(), "a block size");
}

/// Guarantees, for every address and u32 length: `range` takes exactly the empty ranges and
/// those whose every byte (a symbolic index) lies in [NULL_TOP, GUEST_TOP) without wrapping;
/// `range_of` takes a host length the same way and refuses one past 4 GiB.
#[kani::proof]
fn range_is_the_guest_bytes() {
    let (a, n, k): (u32, u32, u32) = (kani::any(), kani::any(), kani::any());
    let r = range(a, n);
    let inside = n == 0 || (a >= kura::guest::NULL_TOP && u64::from(a) + u64::from(n) <= u64::from(GUEST_TOP));
    assert_eq!(r.is_some(), inside);
    if let Some(r) = r {
        assert!(r.addr() == a && r.len() == n);
        if k < n {
            let b = u64::from(a) + u64::from(k);
            assert!(b >= u64::from(kura::guest::NULL_TOP) && b < u64::from(GUEST_TOP));
        }
    }
    let wide: u64 = kani::any();
    let host = usize::try_from(wide);
    if let Ok(h) = host {
        let got = range_of(a, h);
        match u32::try_from(h) {
            Ok(n32) => assert_eq!(got, range(a, n32)),
            Err(_) => assert!(got.is_none()),
        }
        kani::cover!(u32::try_from(h).is_err(), "a host length past 4 GiB");
    }
    kani::cover!(n > 0 && r.is_some(), "a range taken");
    kani::cover!(n > 0 && a >= kura::guest::NULL_TOP && r.is_none(), "one running past GUEST_TOP");
}
