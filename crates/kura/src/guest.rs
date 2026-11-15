//! Guest values as host ones and back, without `as`: a guest u32 (address, size, count, index)
//! as a usize, a guest address as a host pointer (the guest's memory is the host's, below 4G),
//! and a host usize or u64 back as its low 32 bits, as `as u32` was.

/// A guest u32 (address, size, count, index) as a host usize: lossless, usize being 32 bits or
/// more on every target.
pub(crate) fn us(v: u32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}

/// Guest address `a` as a host pointer (the guest's memory is the host's, below 4G).
pub(crate) fn gp<T>(a: u32) -> *mut T {
    std::ptr::with_exposed_provenance_mut(us(a))
}

/// Guest address `a` as a host pointer to read.
pub(crate) fn gc<T>(a: u32) -> *const T {
    std::ptr::with_exposed_provenance(us(a))
}

/// A host usize (length, offset, address) as the guest's u32: its low 32 bits, as `as u32` was.
pub(crate) fn lo32(v: usize) -> u32 {
    let [b0, b1, b2, b3, ..] = v.to_le_bytes();
    u32::from_le_bytes([b0, b1, b2, b3])
}

/// A u64 as a u32: its low 32 bits, as `as u32` was.
pub(crate) fn lo32w(v: u64) -> u32 {
    let [b0, b1, b2, b3, ..] = v.to_le_bytes();
    u32::from_le_bytes([b0, b1, b2, b3])
}

#[cfg(test)]
#[path = "guest_tests.rs"]
mod tests;
