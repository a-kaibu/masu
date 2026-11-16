//! A free list of things kept for good (the Cpus and crash-log records of threads that ended,
//! which other threads may still read without a lock): taken again before a new one is made.
//! It counts what it made and the most that were out at once; as a new one is made only when
//! none waits, the two are always equal (#242: the reuse test now checks that, not a bound that
//! other tests' threads running at the same time could pass).
//!
//! What `take` hands out is a [`Lease`], and only a Lease goes back (`give`): a value the pool
//! never lent cannot be put on its free list (review 2026-11-15: `give(1u8)` on a new pool
//! made `made - free.len()` underflow). A Lease is made only here:
//!
//!
//! Each pool has a brand of its own, a type named for it, and a Lease carries its pool's: one
//! pool's lease does not go to another (review 2026-11-16: `b.give(a.take(..).lease())` put a
//! value `b` never made on its free list).
//!
//! ```compile_fail
//! struct A;
//! let mut p = kura::pool::Pool::<u8, A>::new();
//! p.give(kura::pool::Lease(1u8, std::marker::PhantomData));
//! ```
//! ```compile_fail
//! struct A;
//! let mut p = kura::pool::Pool::<u8, A>::new();
//! p.give(1u8);
//! ```
//! ```compile_fail
//! // (not Clone: one lease, one give)
//! struct A;
//! let mut p = kura::pool::Pool::<u8, A>::new();
//! let l = p.take(|| 1).lease();
//! p.give(l.clone());
//! ```
//! ```compile_fail
//! // (another pool's lease)
//! struct A;
//! struct B;
//! let mut a = kura::pool::Pool::<u8, A>::new();
//! let mut b = kura::pool::Pool::<u8, B>::new();
//! let l = a.take(|| 42).lease();
//! b.give(l);
//! ```
//! ```
//! struct A;
//! let mut a = kura::pool::Pool::<u8, A>::new();
//! let l = a.take(|| 42).lease();
//! a.give(l);
//! assert_eq!(a.counts().waiting, 1);
//! ```

/// A value out of the [`Pool`] branded `B`, to be given back to it. Not Clone, not made outside
/// this module.
#[must_use = "a lease not given back stays out of its pool for good"]
pub struct Lease<T, B>(T, std::marker::PhantomData<fn() -> B>);

// (by hand: a brand needs no Debug or PartialEq of its own)
impl<T: std::fmt::Debug, B> std::fmt::Debug for Lease<T, B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Lease").field(&self.0).finish()
    }
}
impl<T: PartialEq, B> PartialEq for Lease<T, B> {
    fn eq(&self, o: &Self) -> bool {
        self.0 == o.0
    }
}

impl<T, B> std::ops::Deref for Lease<T, B> {
    type Target = T;
    #[inline]
    fn deref(&self) -> &T {
        &self.0
    }
}

/// `B`: the pool's brand (a type of its own, named for it), which its leases carry
pub struct Pool<T, B> {
    free: Vec<Lease<T, B>>,
    made: usize,
    peak: usize,
}

/// (tests) what a pool holds: waiting for reuse, made, the most out at once
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counts {
    pub waiting: usize,
    pub made: usize,
    pub peak: usize,
}

impl<T, B> Pool<T, B> {
    pub const fn new() -> Self {
        Pool { free: Vec::new(), made: 0, peak: 0 }
    }

    /// One that waits, else a new one from `make`
    pub fn take(&mut self, make: impl FnOnce() -> T) -> Reuse<T, B> {
        let got = match self.free.pop() {
            Some(t) => Reuse::Again(t),
            None => {
                self.made = self.made.saturating_add(1);
                Reuse::New(Lease(make(), std::marker::PhantomData))
            }
        };
        // (every lease on the free list was made here: free.len() <= made)
        self.peak = self.peak.max(self.made.saturating_sub(self.free.len()));
        got
    }

    pub fn give(&mut self, t: Lease<T, B>) {
        self.free.push(t);
    }

    pub fn counts(&self) -> Counts {
        Counts { waiting: self.free.len(), made: self.made, peak: self.peak }
    }
}

/// What `take` gave: one used before (to be made like new) or a new one
#[must_use]
pub enum Reuse<T, B> {
    Again(Lease<T, B>),
    New(Lease<T, B>),
}

impl<T, B> Reuse<T, B> {
    /// The lease, new or not
    pub fn lease(self) -> Lease<T, B> {
        match self {
            Reuse::Again(l) | Reuse::New(l) => l,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Any order of takes and gives: as many made as were ever out at once, the waiting
        /// ones are the made ones not out, and a take with one waiting makes none
        #[test]
        fn makes_only_what_is_out_at_once(ops in proptest::collection::vec(any::<bool>(), 0..200)) {
            struct Test;
            let mut p = Pool::<u32, Test>::new();
            let mut out: Vec<Lease<u32, Test>> = Vec::new();
            let mut next = 0u32;
            for take in ops {
                if take || out.is_empty() {
                    let before = p.counts();
                    let t = match p.take(|| { next += 1; next }) {
                        Reuse::Again(t) => { prop_assert!(before.waiting > 0); prop_assert_eq!(p.counts().made, before.made); t }
                        Reuse::New(t) => { prop_assert_eq!(before.waiting, 0); t }
                    };
                    prop_assert!(!out.contains(&t));
                    out.push(t);
                } else if let Some(t) = out.pop() {
                    p.give(t);
                }
                let c = p.counts();
                prop_assert_eq!(c.made, c.peak);
                prop_assert_eq!(c.waiting + out.len(), c.made);
            }
        }
    }
}
