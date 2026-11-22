// Verus (https://github.com/verus-lang/verus): unbounded proofs, for every input length and
// every Params the real code can make, of the FastCDC cutter of crates/masu/src/cdc.rs. What
// Kani and proptest check for bounded inputs, proved for all:
//
// * a cut is never before `min` bytes of the chunk nor after `max` (every chunk but a file's
//   last is min..=max long, and the last one at most max);
// * without a cut, the chunk so far stays shorter than `max` (so the next bytes cut by then);
// * the cut is inside the bytes given (at least one of them, at most all).
//
// Assumed: what Params::new and Params::with_avg hold (`min < max`; `avg` a power of two, which
// only the masks need, and the masks and the gear table are any values here), nothing more: min
// may be 0, avg anywhere, max up to u64::MAX. `with_avg_holds` proves with_avg's
// sizes hold it, `new` that a fresh cutter is well formed.
//
// Correspondence: `next` below is Cutter::next's body token for token, but for the names
// (self.p.min / self.len here c.min / c.len, `a += b` here `a = a + b`, `1` here `1u64`) and
// the loop's invariant; cdc.rs's test `the_verus_proof_is_of_this_code` compares the two and
// fails on any other difference.
//
//   verus formal/verus/cdc.rs        (formal/verus.sh; CI: formal.yml job "verus")
use vstd::prelude::*;

verus! {

global size_of usize == 8;

pub struct Cutter {
    pub min: u64,
    pub avg: u64,
    pub max: u64,
    pub small: u64,
    pub large: u64,
    /// bytes of the current chunk so far
    pub len: u64,
    pub h: u64,
}

/// Params' contract (cdc.rs Params::new), with the chunk so far short of max
pub open spec fn well_formed(c: &Cutter) -> bool {
    &&& c.min < c.max
    &&& c.len < c.max
}

/// Cutter::new on any Params: a fresh cutter is well formed (len 0 < max, from min < max)
pub fn new(min: u64, avg: u64, max: u64, small: u64, large: u64) -> (r: Cutter)
    requires
        min < max,
    ensures
        well_formed(&r),
        r.min == min && r.avg == avg && r.max == max && r.len == 0,
{
    Cutter { min, avg, max, small, large, len: 0, h: 0 }
}

/// Params::with_avg from any avg its clamp lets through (64 KiB..=2^60): min < max, and max fits
pub proof fn with_avg_holds(avg: u64)
    requires
        0x1_0000 <= avg <= 0x1000_0000_0000_0000,
    ensures
        avg / 4 < avg * 4,
        avg * 4 <= u64::MAX,
{
}

/// Where in `buf` the current chunk ends, if it does (cdc.rs Cutter::next).
pub fn next(c: &mut Cutter, buf: &[u8], gear: &[u64; 256]) -> (r: Option<usize>)
    requires
        well_formed(old(c)),
    ensures
        well_formed(final(c)),
        final(c).min == old(c).min && final(c).max == old(c).max && final(c).avg == old(c).avg,
        match r {
            Some(i) => {
                &&& 1 <= i <= buf.len()
                &&& final(c).len == 0
                &&& old(c).min <= old(c).len + i <= old(c).max
            },
            None => final(c).len == old(c).len + buf.len(),
        },
{
    let mut i = 0;
    // (no cut before min: the hash is not even kept there)
    if c.len < c.min {
        // (what is short of min, or all the bytes: no cast of a u64 that may not fit usize)
        let need = c.min - c.len;
        let skip: usize = if need < buf.len() as u64 { need as usize } else { buf.len() };
        c.len = c.len + skip as u64;
        i = skip;
        if c.len < c.min {
            return None;
        }
    }
    let mut h = c.h;
    while i < buf.len()
        invariant
            c.min < c.max,
            c.min == old(c).min,
            c.avg == old(c).avg,
            c.max == old(c).max,
            c.min <= c.len < c.max,
            i <= buf.len(),
            c.len == old(c).len + i,
        decreases buf.len() - i,
    {
        h = (h << 1u64).wrapping_add(gear[buf[i] as usize]);
        c.len = c.len + 1;
        i = i + 1;
        let m = if c.len < c.avg { c.small } else { c.large };
        if h & m == 0 || c.len >= c.max {
            c.len = 0;
            c.h = 0;
            return Some(i);
        }
    }
    c.h = h;
    None
}

} // verus!

fn main() {}
