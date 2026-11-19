//! Guest values as host ones and back, without `as`: a guest u32 (address, size, count, index)
//! as a usize, a guest address as a host pointer (the guest's memory is the host's, below 4G),
//! and a host usize or u64 back as its low 32 bits, as `as u32` was.


use kome::num::*;
/// Guest address `a` as a host pointer (the guest's memory is the host's, below 4G).
pub(crate) fn gp<T>(a: u32) -> *mut T {
    std::ptr::with_exposed_provenance_mut(us(a))
}

/// Guest address `a` as a host pointer to read.
pub(crate) fn gc<T>(a: u32) -> *const T {
    std::ptr::with_exposed_provenance(us(a))
}

#[cfg(test)]
#[path = "guest_tests.rs"]
mod tests;

pub const NULL_TOP: u32 = 0x1_0000;
pub const GUEST_TOP: u32 = 0x8000_0000;
/// A guest address: a value, x86 address arithmetic only. Its arithmetic wraps at 4 GiB as the
/// x86's does (an offset past the top comes round to the bottom: what the guest's own code
/// computes); a length or a count is not one (its arithmetic is checked where it is done). Made
/// from a u32 by `ga(a)` (or `.into()`), given back by `.get()`. It does not touch memory: the
/// accessors here do (`r32(a)`, `w32(a, v)`), each through a checked [`Range`]
/// (review 2026-11-15).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Ga(u32);

/// Guest address `a`
#[inline(always)]
pub const fn ga(a: u32) -> Ga {
    Ga(a)
}

impl Ga {
    #[inline(always)]
    pub const fn get(self) -> u32 {
        self.0
    }
    /// Element `i` of an array of `size`-byte elements here (wrapping, as the x86 computes it)
    #[inline(always)]
    pub const fn at(self, i: u32, size: u32) -> Ga {
        Ga(self.0.wrapping_add(i.wrapping_mul(size)))
    }
    /// How far past `base` this is (wrapping)
    #[inline(always)]
    pub const fn since(self, base: Ga) -> u32 {
        self.0.wrapping_sub(base.0)
    }
    /// The end of `n` bytes from here, unwrapped (a range check: past GUEST_TOP or 4 GiB shows)
    #[inline(always)]
    pub fn end(self, n: u32) -> u64 {
        // (two u32s: the sum is below 2^33, never wraps)
        u64::from(self.0).wrapping_add(u64::from(n))
    }
}
impl std::ops::Add<u32> for Ga {
    type Output = Ga;
    #[inline(always)]
    fn add(self, o: u32) -> Ga {
        Ga(self.0.wrapping_add(o))
    }
}
impl std::ops::Sub<u32> for Ga {
    type Output = Ga;
    #[inline(always)]
    fn sub(self, o: u32) -> Ga {
        Ga(self.0.wrapping_sub(o))
    }
}
impl std::ops::AddAssign<u32> for Ga {
    #[inline(always)]
    fn add_assign(&mut self, o: u32) {
        self.0 = self.0.wrapping_add(o);
    }
}
impl From<u32> for Ga {
    #[inline(always)]
    fn from(a: u32) -> Ga {
        Ga(a)
    }
}
impl From<Ga> for u32 {
    #[inline(always)]
    fn from(a: Ga) -> u32 {
        a.0
    }
}


/// A numerical guest range. Page permissions are checked at the operation.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range { addr: Ga, len: u32 }
pub fn range(addr: Ga, len: u32) -> Option<Range> {
    (len == 0 || (addr.get() >= NULL_TOP && addr.end(len) <= u64::from(GUEST_TOP))).then_some(Range { addr, len })
}
impl Range {
    pub fn addr(self) -> u32 { self.addr.get() }
    pub fn len(self) -> u32 { self.len }
    pub fn is_empty(self) -> bool { self.len == 0 }
}
