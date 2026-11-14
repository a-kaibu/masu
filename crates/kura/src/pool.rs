//! A free list of things kept for good (the Cpus and crash-log records of threads that ended,
//! which other threads may still read without a lock): taken again before a new one is made.
//! It counts what it made and the most that were out at once; as a new one is made only when
//! none waits, the two are always equal (#242: the reuse test now checks that, not a bound that
//! other tests' threads running at the same time could pass).

pub struct Pool<T> {
    free: Vec<T>,
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

impl<T> Pool<T> {
    pub const fn new() -> Self {
        Pool { free: Vec::new(), made: 0, peak: 0 }
    }

    /// One that waits, else a new one from `make`
    pub fn take(&mut self, make: impl FnOnce() -> T) -> Reuse<T> {
        let got = match self.free.pop() {
            Some(t) => Reuse::Again(t),
            None => {
                self.made += 1;
                Reuse::New(make())
            }
        };
        self.peak = self.peak.max(self.made - self.free.len());
        got
    }

    pub fn give(&mut self, t: T) {
        self.free.push(t);
    }

    pub fn counts(&self) -> Counts {
        Counts { waiting: self.free.len(), made: self.made, peak: self.peak }
    }
}

/// What `take` gave: one used before (to be made like new) or a new one
pub enum Reuse<T> {
    Again(T),
    New(T),
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
            let mut p = Pool::new();
            let mut out: Vec<u32> = Vec::new();
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
