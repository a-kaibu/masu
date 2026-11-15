//! Deadlines without `Instant + Duration`, which panics when the sum is past what the clock can
//! hold (a guest's "wait 2^64 ns"): the deadline saturates to the latest one that fits instead.
use std::time::{Duration, Instant};

/// `t + d`, or the latest instant after `t` that the clock can hold (halving `d` until it fits)
#[must_use]
pub fn after(t: Instant, d: Duration) -> Instant {
    let mut d = d;
    loop {
        if let Some(v) = t.checked_add(d) {
            return v;
        }
        if d.is_zero() {
            return t;
        }
        d = d.checked_div(2).unwrap_or_default();
    }
}

/// `after(Instant::now(), d)`
#[must_use]
pub fn from_now(d: Duration) -> Instant {
    after(Instant::now(), d)
}

#[cfg(test)]
#[path = "time_tests.rs"]
mod tests;
