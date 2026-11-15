//! Shared by golden_interp.rs and golden_jit.rs: seeded random x86 programs, a sparse guest
//! memory that remembers which bytes were written, and a stable hash (FNV-1a, not std's
//! hasher, whose output may change between Rust versions).
//!
//! The goldens pin what koji does today, bit for bit, so a rewrite that is meant to change
//! nothing (newtypes, exhaustive matches, typed conversions) proves it: the interpreter's
//! state after every step and the JIT's wasm bytes must stay the same. A change that is meant
//! to change behaviour rewrites them (`MASU_UPDATE_GOLDEN=1`) and says why in its commit.
#![allow(dead_code)]

use koji::{Fault, Memory};
use std::collections::BTreeSet;

/// Guest memory below this faults (a null page area big enough for small pointers).
pub const LOW: u32 = 0x1_0000;
pub const CODE: u32 = 0x1_0000;
pub const DATA: u32 = 0x1_8000;
pub const STACK_TOP: u32 = 0x2_f000;
pub const TOP: u32 = 0x3_0000;
pub const PROG_LEN: usize = 48;
/// Programs per golden line (a failure names the group, MASU_GOLDEN_GROUP=<n> prints its cases)
pub const GROUP: usize = 32;

/// Index of guest address `a` (>= LOW) in Mem::b.
pub fn off(a: u32) -> usize {
    usize::try_from(a - LOW).unwrap()
}

/// `n` as a u64 (a program number).
pub fn n64(n: usize) -> u64 {
    u64::try_from(n).unwrap()
}

pub struct Fnv(pub u64);
impl Fnv {
    pub fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= u64::from(x);
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    pub fn u8(&mut self, v: u8) {
        self.bytes(&[v]);
    }
    pub fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    pub fn u128(&mut self, v: u128) {
        self.bytes(&v.to_le_bytes());
    }
}

/// splitmix64
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn byte(&mut self) -> u8 {
        self.next().to_le_bytes()[0]
    }
}

/// Instruction leads the random programs are made of: escape bytes, prefixes and opcode
/// groups that would be rare in uniform bytes (each followed by random ModRM/SIB/disp/imm).
const LEADS: &[&[u8]] = &[
    &[], &[], &[], &[0x0f], &[0x0f], &[0x66, 0x0f], &[0xf3, 0x0f], &[0xf2, 0x0f], &[0x0f, 0x38], &[0x66, 0x0f, 0x38],
    &[0x66, 0x0f, 0x3a], &[0xd8], &[0xd9], &[0xda], &[0xdb], &[0xdc], &[0xdd], &[0xde], &[0xdf], &[0xc5], &[0xc4],
    &[0x66], &[0xf3], &[0xf2], &[0xf0], &[0x48], &[0x4c], &[0x41], &[0x64], &[0x65],
];

/// Program `n` of the set `seed`: instructions of a random lead and 1..7 random bytes, then a
/// RET; PROG_LEN bytes with the tail filled with INT3.
pub fn program(seed: u64, n: usize) -> Vec<u8> {
    let mut r = Rng(seed ^ n64(n).wrapping_mul(0x2545_f491_4f6c_dd1d));
    let mut v = Vec::with_capacity(PROG_LEN);
    while v.len() < PROG_LEN - 9 {
        let lead = LEADS[usize::try_from(r.below(n64(LEADS.len()))).unwrap()];
        v.extend_from_slice(lead);
        for _ in 0..1 + r.below(7) {
            v.push(r.byte());
        }
    }
    v.truncate(PROG_LEN - 1);
    v.push(0xc3);
    v.resize(PROG_LEN, 0xcc);
    v
}

/// Random register values, most of them pointing into the data area (so memory operands
/// land in mapped memory more often than not).
pub fn regs(seed: u64, n: usize) -> [u64; 16] {
    let mut r = Rng(seed.rotate_left(17) ^ n64(n));
    let mut out = [0u64; 16];
    for x in &mut out {
        *x = match r.below(4) {
            0 => r.next(),
            1 => r.below(256),
            _ => u64::from(DATA) + r.below(0x4000),
        };
    }
    out
}

/// [LOW, TOP) mapped; every byte written is remembered (the final state is what is hashed).
pub struct Mem {
    pub b: Vec<u8>,
    pub dirty: BTreeSet<u32>,
}
impl Mem {
    pub fn new(code: &[u8], at: u32) -> Self {
        let mut b = vec![0u8; off(TOP)];
        // (the data area: a fixed pattern, so loads read something other than zeros)
        let mut r = Rng(0x5eed);
        for x in &mut b[off(DATA)..off(STACK_TOP)] {
            *x = r.byte();
        }
        let o = off(at);
        b[o..o + code.len()].copy_from_slice(code);
        Mem { b, dirty: BTreeSet::new() }
    }
    pub fn hash(&self, h: &mut Fnv) {
        for &a in &self.dirty {
            h.u32(a);
            h.u8(self.b[off(a)]);
        }
    }
}
impl Memory for Mem {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        if (LOW..TOP).contains(&a) { Ok(self.b[off(a)]) } else { Err(Fault { addr: a, write: false }) }
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        if (LOW..TOP).contains(&a) {
            self.b[off(a)] = v;
            self.dirty.insert(a);
            Ok(())
        } else {
            Err(Fault { addr: a, write: true })
        }
    }
}

/// Checks `lines` ("<name> <hash>") against crates/koji/testdata/golden/<file>, or writes them
/// with MASU_UPDATE_GOLDEN=1.
pub fn check(file: &str, lines: &[String]) {
    let path = format!("{}/testdata/golden/{file}", env!("CARGO_MANIFEST_DIR"));
    if let Err(e) = kome::golden::compare(&path, &(lines.join("\n") + "\n")) {
        panic!("{e}\n(a change meant to change nothing changed this; MASU_GOLDEN_GROUP=<n> prints a group's cases)");
    }
}
