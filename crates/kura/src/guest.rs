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
