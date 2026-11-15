//! Golden files for tests: the text a test makes today, kept in the repository and compared on
//! every run, so a rewrite meant to change nothing proves it. One env var for every crate:
//! `MASU_UPDATE_GOLDEN=1` rewrites the files a run compares (say why in the commit). An empty
//! value counts as unset (`MASU_UPDATE_GOLDEN= cargo test` compares).
//!
//! Also the two pieces every golden test copied: [`Fnv`] (FNV-1a 64, a hash that stays the same
//! across Rust versions, unlike std's) and [`Rng`] (xorshift64*, seeded, the same on every run).
use std::path::Path;

/// The variable that rewrites golden files instead of comparing them.
pub const UPDATE: &str = "MASU_UPDATE_GOLDEN";

/// Is this run rewriting golden files (MASU_UPDATE_GOLDEN set and not empty)?
pub fn updating() -> bool {
    std::env::var_os(UPDATE).is_some_and(|v| !v.is_empty())
}

/// What differs between `want` and `got`, for a failure message: the first 20 changed lines
/// (1-based numbers), then the line counts if they differ.
pub fn diff(want: &str, got: &str) -> String {
    let (w, g): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
    let mut out: Vec<String> = Vec::new();
    for i in 0..w.len().max(g.len()) {
        let (a, b) = (w.get(i), g.get(i));
        if a != b {
            out.push(format!("{}:\n- {}\n+ {}", i + 1, a.unwrap_or(&"(none)"), b.unwrap_or(&"(none)")));
            if out.len() == 20 {
                out.push("...".into());
                break;
            }
        }
    }
    if w.len() != g.len() {
        out.push(format!("({} lines in the file, {} now)", w.len(), g.len()));
    }
    if out.is_empty() && want != got {
        out.push("(the same lines: only the line endings or the final newline differ)".into());
    }
    out.join("\n")
}

/// Compares `got` with the file at `path` (rewrites it when [`updating`]); an error names what
/// differs. A missing file is empty, so a new golden fails until it is written once.
pub fn compare(path: impl AsRef<Path>, got: &str) -> Result<(), String> {
    let path = path.as_ref();
    if updating() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, got).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    let want = std::fs::read_to_string(path).unwrap_or_default();
    if want == got {
        return Ok(());
    }
    Err(format!("{} differs ({UPDATE}=1 rewrites it):\n{}", path.display(), diff(&want, got)))
}

/// [`compare`] for a test: panics with the difference.
#[track_caller]
pub fn check(path: impl AsRef<Path>, got: &str) {
    if let Err(e) = compare(path, got) {
        panic!("{e}");
    }
}

/// FNV-1a, 64 bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv::new()
    }
}

impl Fnv {
    pub const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    pub const PRIME: u64 = 0x100_0000_01b3;
    pub const fn new() -> Fnv {
        Fnv(Fnv::OFFSET)
    }
    pub fn bytes(&mut self, b: &[u8]) -> &mut Fnv {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(Fnv::PRIME);
        }
        self
    }
    /// each word's 4 little-endian bytes
    pub fn words(&mut self, v: impl IntoIterator<Item = u32>) -> &mut Fnv {
        for w in v {
            self.bytes(&w.to_le_bytes());
        }
        self
    }
    pub fn get(&self) -> u64 {
        self.0
    }
    /// The hash of `b` from the standard offset.
    pub fn of(b: &[u8]) -> u64 {
        Fnv::new().bytes(b).get()
    }
}

/// Deterministic test inputs: xorshift64* (Vigna), so a seeded sweep is the same on every run.
/// A zero seed would stay zero; [`Rng::new`] turns it into a fixed non-zero one.
#[derive(Clone, Debug)]
pub struct Rng(pub u64);

impl Rng {
    pub const fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9e37_79b9_7f4a_7c15 } else { seed })
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn next_u32(&mut self) -> u32 {
        let [.., a, b, c, d] = self.next().to_le_bytes();
        u32::from_le_bytes([a, b, c, d])
    }
    /// 0..n (0 for n = 0)
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    /// 0..n as an index (0 for n = 0)
    pub fn index(&mut self, n: usize) -> usize {
        let b = self.below(u64::try_from(n).unwrap_or(u64::MAX));
        usize::try_from(b).unwrap_or(0)
    }
    /// One of `v` (an empty `v` is the test's own mistake: it panics).
    #[track_caller]
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        let i = self.index(v.len());
        match v.get(i) {
            Some(&x) => x,
            None => panic!("Rng::pick from an empty slice"),
        }
    }
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next().to_le_bytes()[0]).collect()
    }
    pub fn coin(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

#[cfg(test)]
#[path = "golden_tests.rs"]
mod tests;
