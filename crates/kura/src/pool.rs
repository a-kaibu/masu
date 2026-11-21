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
//! Each pool has a brand of its own, a type of which one value ever exists ([`Brand`], made by
//! [`pool!`] with its one pool), and a Lease carries its pool's: one pool's lease does not go to
//! another (review 2026-11-16: `b.give(a.take(..).lease())` put a value `b` never made on its
//! free list; review 2026-11-17: two pools of one brand type still took each other's leases, so
//! the brand is now a value `Pool::new` takes and only `pool!` makes, once).
//!
//! ```compile_fail
//! kura::pool!(mod a: u8 as A);
//! a::POOL.locked().give(kura::pool::Lease(1u8, std::marker::PhantomData));
//! ```
//! ```compile_fail
//! kura::pool!(mod a: u8 as A);
//! a::POOL.locked().give(1u8);
//! ```
//! ```compile_fail
//! // (not Clone: one lease, one give)
//! kura::pool!(mod a: u8 as A);
//! let mut p = a::POOL.locked();
//! let l = p.take(|| 1).lease();
//! p.give(l.clone());
//! ```
//! ```compile_fail
//! // (another pool's lease)
//! kura::pool!(mod a: u8 as A);
//! kura::pool!(mod b: u8 as B);
//! let l = a::POOL.locked().take(|| 42).lease();
//! b::POOL.locked().give(l);
//! ```
//! ```compile_fail
//! // (a second pool of a brand: the brand's one value is in its pool, none made outside)
//! kura::pool!(mod a: u8 as A);
//! let _b = kura::pool::Pool::<u8, a::A>::new(a::A(()));
//! ```
//! ```compile_fail
//! // (a brand of one's own: a unit struct has values for the asking, so it is no Brand)
//! struct Tag;
//! let _a = kura::pool::Pool::<u32, Tag>::new(Tag);
//! ```
//! ```
//! use kome::locks::Lock;
//! kura::pool!(mod a: u8 as A);
//! let mut p = a::POOL.locked();
//! let l = p.take(|| 42).lease();
//! p.give(l);
//! assert_eq!(p.counts().waiting, 1);
//! ```

/// A pool's brand: a type of which exactly one value is ever made, the one its pool holds.
///
/// # Safety
/// No second value of the type is ever made (the type's fields are private to a module that
/// makes the one value only). [`pool!`] makes such types; nothing else should implement it.
pub unsafe trait Brand {}

/// `pool!(mod name: T as Brand)`: a module `name` with the brand type `Brand` (one value ever)
/// and its pool `POOL` (a `Mutex<Pool<T, Brand>>`, the only one that brand's leases go back to).
#[macro_export]
macro_rules! pool {
    ($(#[$m:meta])* $vis:vis mod $name:ident: $t:ty as $brand:ident) => {
        $(#[$m])*
        $vis mod $name {
            #[allow(unused_imports)]
            use super::*;
            /// The brand of this module's [`POOL`]: its one value is there.
            pub struct $brand(());
            // SAFETY: its field is private to this module, which makes one value: POOL's.
            unsafe impl $crate::pool::Brand for $brand {}
            pub static POOL: std::sync::Mutex<$crate::pool::Pool<$t, $brand>> = std::sync::Mutex::new($crate::pool::Pool::new($brand(())));
        }
    };
}

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

/// `B`: the pool's brand (its one value, see [`Brand`]), which its leases carry
pub struct Pool<T, B> {
    free: Vec<Lease<T, B>>,
    made: usize,
    peak: usize,
    _brand: B,
}

/// (tests) what a pool holds: waiting for reuse, made, the most out at once
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Counts {
    pub waiting: usize,
    pub made: usize,
    pub peak: usize,
}

impl<T, B: Brand> Pool<T, B> {
    /// The pool of brand `b` (whose one value it takes: [`pool!`])
    pub const fn new(b: B) -> Self {
        Pool { free: Vec::new(), made: 0, peak: 0, _brand: b }
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
        // (a lease only comes from this pool's take: never more waiting than made)
        debug_assert!(self.free.len() < self.made);
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

    crate::pool!(mod test: u32 as Test);

    proptest! {
        /// Any order of takes and gives: as many made as were ever out at once, the waiting
        /// ones are the made ones not out, and a take with one waiting makes none
        #[test]
        fn makes_only_what_is_out_at_once(ops in proptest::collection::vec(any::<bool>(), 0..200)) {
            let mut p = test::POOL.lock().unwrap_or_else(|e| e.into_inner());
            // (one pool for all the cases: each starts it empty)
            p.free.clear();
            (p.made, p.peak) = (0, 0);
            let mut out: Vec<Lease<u32, test::Test>> = Vec::new();
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
