//! The doorbells adec, idec and video ring for the page: a word the guest bumps after each new
//! request, which the page's worker waits on (Atomics.wait) to answer at once instead of polling.
use std::sync::atomic::{AtomicI32, Ordering};

/// Bumps `bell` and wakes every waiter on it (on the web; elsewhere no page waits).
pub(crate) fn ring(bell: &AtomicI32) {
    bell.fetch_add(1, Ordering::SeqCst);
    #[cfg(target_os = "emscripten")]
    // SAFETY: emscripten_futex_wake only reads the address, and `bell` is a live atomic word.
    unsafe {
        unsafe extern "C" {
            fn emscripten_futex_wake(addr: *const std::ffi::c_void, count: i32) -> i32;
        }
        emscripten_futex_wake(bell.as_ptr() as *const std::ffi::c_void, i32::MAX);
    }
}
