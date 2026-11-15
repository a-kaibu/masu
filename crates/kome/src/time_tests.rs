use super::*;
use proptest::prelude::*;

proptest! {
    /// what fits is the plain sum; what does not is still later than `t` and never panics
    #[test]
    fn after_is_the_sum_or_saturates(secs in any::<u64>(), nanos in 0u32..1_000_000_000) {
        let t = Instant::now();
        let d = Duration::new(secs, nanos);
        let v = after(t, d);
        match t.checked_add(d) {
            Some(s) => prop_assert_eq!(v, s),
            None => prop_assert!(v > t),
        }
    }
}

#[test]
fn zero_is_now() {
    let t = Instant::now();
    assert_eq!(after(t, Duration::ZERO), t);
    assert!(from_now(Duration::MAX) > t);
}
