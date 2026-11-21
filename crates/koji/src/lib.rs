//! x86 user-mode CPU core (interpreter and x86->wasm JIT) for running Windows code with sake.
//!
//! The guest address space is flat 32-bit; segment registers other than FS/GS
//! (Windows' TEB) are ignored. The CPU never owns memory: everything goes
//! through [`Memory`], so the same core runs natively in tests (a Vec) and in
//! the browser (the wasm linear memory itself, guest address = wasm address).

#![no_std]
use kome::num::*;
extern crate alloc;

pub(crate) mod interp;
pub mod flags;
pub mod lazy;
pub mod jit;
pub mod f80;
pub mod fpu;
pub mod round;

pub use iced_x86;
pub use fpu::Fpu;

/// The MXCSR exception flags of an SSE operation (for tests against the host CPU).
#[doc(hidden)]
pub use interp::sse::{flags32, flags64, narrow_flags, FOp};
/// RDTSC's clock: the embedder's (sake: wall-clock ticks), else [`Cpu::tsc`]. A program timing
/// itself with RDTSC (十六夜の花嫁's game clock: TSC ticks over a second of timeGetTime, then
/// TSC / ticks-per-ms) stood still on a constant one.
static TSC_SOURCE: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Sets RDTSC's clock (a function of no arguments that never goes back).
pub fn set_tsc_source(f: fn() -> u64) {
    TSC_SOURCE.store(f as usize, core::sync::atomic::Ordering::Relaxed);
}

/// What RDTSC reads on `cpu`.
pub fn read_tsc(cpu: &Cpu) -> u64 {
    match TSC_SOURCE.load(core::sync::atomic::Ordering::Relaxed) {
        0 => cpu.tsc,
        // SAFETY: only set_tsc_source stores here, from a `fn() -> u64`
        p => {
            let f = unsafe { core::mem::transmute::<usize, fn() -> u64>(p) };
            f()
        }
    }
}

pub use interp::{cpuid, cpuid_mode, CPUID_AVX, REP_STEP_MAX, step, step_cached, step_flow, DecodeCache};

impl Cpu {
    /// EFLAGS with the lazy status flags worked out.
    #[inline]
    pub fn flags(&self) -> u32 {
        if self.lazy.op == 0 { self.eflags } else { (self.eflags & !flags::STATUS) | self.lazy.status() }
    }

    /// A 64-bit general register.
    #[inline]
    pub fn gpr(&self, r: impl Into<Reg64>) -> u64 {
        match r.into().split() {
            Ok(g) => u64::from(self.regs[g]) | u64::from(self.hi[g]) << 32,
            Err(k) => self.r8[k],
        }
    }

    #[inline]
    pub fn set_gpr(&mut self, r: impl Into<Reg64>, v: u64) {
        match r.into().split() {
            Ok(g) => {
                self.regs[g] = lo32(v);
                self.hi[g] = lo32(v >> 32);
            }
            Err(k) => self.r8[k] = v,
        }
    }

    /// XMM register `i` (0..16; 16 is the interpreter's scratch, `vex_tmp`).
    #[inline]
    pub fn xmm(&self, i: usize) -> u128 {
        if i < 8 { self.xmm[i] } else if i < 16 { self.xmm8[i - 8] } else { self.vex_tmp }
    }

    #[inline]
    pub fn set_xmm(&mut self, i: usize, v: u128) {
        if i < 8 { self.xmm[i] = v } else if i < 16 { self.xmm8[i - 8] = v } else { self.vex_tmp = v }
    }

    /// YMM register `i` (0..16, 16: the scratch): [low half (the XMM register), upper half].
    #[inline]
    pub fn ymm(&self, i: usize) -> [u128; 2] {
        [self.xmm(i), self.ymmh[i]]
    }

    #[inline]
    pub fn set_ymm(&mut self, i: usize, v: [u128; 2]) {
        self.set_xmm(i, v[0]);
        self.ymmh[i] = v[1];
    }

    /// Makes `eflags` exact again.
    #[inline]
    pub fn materialize(&mut self) {
        if self.lazy.op != 0 {
            self.eflags = self.flags();
            self.lazy.op = 0;
        }
    }
}

/// A memory access the guest may not make (unmapped / protected page).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub addr: u32,
    pub write: bool,
}

/// The width of an integer access or operand: 1, 2, 4 or 8 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    W8,
    W16,
    W32,
    W64,
}

impl Width {
    pub const fn bits(self) -> u32 {
        match self {
            Width::W8 => 8,
            Width::W16 => 16,
            Width::W32 => 32,
            Width::W64 => 64,
        }
    }

    pub const fn bytes(self) -> u32 {
        self.bits() / 8
    }

    pub const fn bytes64(self) -> u64 {
        // (const: From is not)
        match self {
            Width::W8 => 1,
            Width::W16 => 2,
            Width::W32 => 4,
            Width::W64 => 8,
        }
    }

    /// None for any other number of bits (an x87 or vector operand, a far pointer).
    pub const fn of_bits(bits: u32) -> Option<Width> {
        match bits {
            8 => Some(Width::W8),
            16 => Some(Width::W16),
            32 => Some(Width::W32),
            64 => Some(Width::W64),
            _ => None,
        }
    }
}

/// Guest memory. Little-endian, unaligned accesses allowed.
pub trait Memory {
    fn read8(&mut self, addr: u32) -> Result<u8, Fault>;
    fn write8(&mut self, addr: u32, v: u8) -> Result<(), Fault>;

    #[inline]
    fn read16(&mut self, addr: u32) -> Result<u16, Fault> {
        Ok(u16::from(self.read8(addr)?) | u16::from(self.read8(addr.wrapping_add(1))?) << 8)
    }
    #[inline]
    fn read32(&mut self, addr: u32) -> Result<u32, Fault> {
        Ok(u32::from(self.read16(addr)?) | u32::from(self.read16(addr.wrapping_add(2))?) << 16)
    }
    #[inline]
    fn read64(&mut self, addr: u32) -> Result<u64, Fault> {
        Ok(u64::from(self.read32(addr)?) | u64::from(self.read32(addr.wrapping_add(4))?) << 32)
    }
    #[inline]
    fn write16(&mut self, addr: u32, v: u16) -> Result<(), Fault> {
        self.write8(addr, lo8(v))?;
        self.write8(addr.wrapping_add(1), lo8(v >> 8))
    }
    #[inline]
    fn write32(&mut self, addr: u32, v: u32) -> Result<(), Fault> {
        self.write16(addr, lo16(v))?;
        self.write16(addr.wrapping_add(2), lo16(v >> 16))
    }
    #[inline]
    fn write64(&mut self, addr: u32, v: u64) -> Result<(), Fault> {
        self.write32(addr, lo32(v))?;
        self.write32(addr.wrapping_add(4), lo32(v >> 32))
    }
    /// Compare-and-swap of a `w` access at `addr`, atomic with respect to other threads'
    /// accesses: stores `new` if the memory holds `old`; returns what it held.
    fn cmpxchg(&mut self, addr: u32, w: Width, old: u64, new: u64) -> Result<u64, Fault> {
        let cur = match w {
            Width::W8 => u64::from(self.read8(addr)?),
            Width::W16 => u64::from(self.read16(addr)?),
            Width::W32 => u64::from(self.read32(addr)?),
            Width::W64 => self.read64(addr)?,
        };
        if cur == old {
            let b = new.to_le_bytes();
            match w {
                Width::W8 => self.write8(addr, b[0])?,
                Width::W16 => self.write16(addr, u16::from_le_bytes([b[0], b[1]]))?,
                Width::W32 => self.write32(addr, u32::from_le_bytes([b[0], b[1], b[2], b[3]]))?,
                Width::W64 => self.write64(addr, new)?,
            }
        }
        Ok(cur)
    }
    /// CMPXCHG16B: compare-and-swap of the 16 bytes at `addr`, atomic with respect to other
    /// threads' 16-byte swaps; returns what the memory held.
    fn cmpxchg16(&mut self, addr: u32, old: u128, new: u128) -> Result<u128, Fault> {
        let cur = u128::from(self.read64(addr)?) | u128::from(self.read64(addr.wrapping_add(8))?) << 64;
        if cur == old {
            self.write64(addr, lo64(new))?;
            self.write64(addr.wrapping_add(8), lo64(new >> 64))?;
        }
        Ok(cur)
    }
    /// Copy plain guest memory with memmove semantics. False leaves it untouched
    /// so the interpreter can fault at the exact failing iteration.
    fn bulk_copy(&mut self, _dst: u32, _src: u32, _len: u32) -> bool { false }

    /// Fill plain guest memory with repetitions of a little-endian element.
    /// False leaves the destination untouched.
    fn bulk_fill(&mut self, _dst: u32, _len: u32, _element: &[u8]) -> bool { false }

    /// Copy up to `buf.len()` instruction bytes at `addr`; returns how many
    /// were readable (decoding fails cleanly on a short fetch).
    fn fetch(&mut self, addr: u32, buf: &mut [u8]) -> usize {
        for (i, b) in buf.iter_mut().enumerate() {
            match self.read8(addr.wrapping_add(lo32(i))) {
                Ok(v) => *b = v,
                Err(_) => return i,
            }
        }
        buf.len()
    }
}

/// [`Flat`] with its first 64 KB unmapped (as Windows: a null pointer's page and the ones after
/// it fault): what the differential tests against the host CPU run in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Guarded(pub alloc::vec::Vec<u8>);

impl Guarded {
    /// where the mapped memory starts
    pub const LOW: u32 = 0x1_0000;
}

impl Memory for Guarded {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        if a < Guarded::LOW {
            return Err(Fault { addr: a, write: false });
        }
        Flat::read8_in(&self.0, a)
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        if a < Guarded::LOW {
            return Err(Fault { addr: a, write: true });
        }
        Flat::write8_in(&mut self.0, a, v)
    }
}

/// Read-only memory: `.0` at address `AT` (a code image), anything else and every write faults:
/// what the JIT's tests translate from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rom<const AT: u32>(pub alloc::vec::Vec<u8>);

impl<const AT: u32> Memory for Rom<AT> {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        a.checked_sub(AT).and_then(|o| usize::try_from(o).ok()).and_then(|o| self.0.get(o)).copied().ok_or(Fault { addr: a, write: false })
    }
    fn write8(&mut self, a: u32, _: u8) -> Result<(), Fault> {
        Err(Fault { addr: a, write: true })
    }
}

/// Memory that is one byte vector from address 0 (anything past its end faults): what tests
/// and tools run code in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flat(pub alloc::vec::Vec<u8>);

impl Flat {
    fn read8_in(m: &[u8], a: u32) -> Result<u8, Fault> {
        usize::try_from(a).ok().and_then(|i| m.get(i)).copied().ok_or(Fault { addr: a, write: false })
    }
    fn write8_in(m: &mut [u8], a: u32, v: u8) -> Result<(), Fault> {
        match usize::try_from(a).ok().and_then(|i| m.get_mut(i)) {
            Some(b) => {
                *b = v;
                Ok(())
            }
            None => Err(Fault { addr: a, write: true }),
        }
    }
}

impl Memory for Flat {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        Flat::read8_in(&self.0, a)
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        Flat::write8_in(&mut self.0, a, v)
    }
}

/// A general register, in encoding order. [`Gprs`] is indexed by it and by nothing else, so a
/// register written as a number (`regs[3]`: EBX? the fourth pushed?) doesn't compile.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Gpr { Eax, Ecx, Edx, Ebx, Esp, Ebp, Esi, Edi }

impl Gpr {
    /// Encoding order
    pub const ALL: [Gpr; 8] = [Gpr::Eax, Gpr::Ecx, Gpr::Edx, Gpr::Ebx, Gpr::Esp, Gpr::Ebp, Gpr::Esi, Gpr::Edi];
    /// The register a 3-bit field of an encoding names (the low three bits of `n`)
    #[inline]
    pub const fn low3(n: u32) -> Gpr {
        match n & 7 {
            0 => Gpr::Eax,
            1 => Gpr::Ecx,
            2 => Gpr::Edx,
            3 => Gpr::Ebx,
            4 => Gpr::Esp,
            5 => Gpr::Ebp,
            6 => Gpr::Esi,
            // (n & 7 is 7 here)
            _ => Gpr::Edi,
        }
    }
    /// Register `i` of the encoding (0..8), None above
    #[inline]
    pub fn at(i: usize) -> Option<Gpr> {
        Gpr::ALL.get(i).copied()
    }
    /// The encoding number (0..8)
    #[inline]
    pub const fn num(self) -> usize {
        match self {
            Gpr::Eax => 0,
            Gpr::Ecx => 1,
            Gpr::Edx => 2,
            Gpr::Ebx => 3,
            Gpr::Esp => 4,
            Gpr::Ebp => 5,
            Gpr::Esi => 6,
            Gpr::Edi => 7,
        }
    }
}

/// A 64-bit general register (long mode), in encoding order: RAX..RDI are the [`Gpr`]s with
/// their upper halves ([`Cpu::hi`]), R8..R15 [`Cpu::r8`]. [`Cpu::gpr`] takes it (or a [`Gpr`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Reg64 { Rax, Rcx, Rdx, Rbx, Rsp, Rbp, Rsi, Rdi, R8, R9, R10, R11, R12, R13, R14, R15 }

impl Reg64 {
    /// Encoding order
    pub const ALL: [Reg64; 16] = [
        Reg64::Rax, Reg64::Rcx, Reg64::Rdx, Reg64::Rbx, Reg64::Rsp, Reg64::Rbp, Reg64::Rsi, Reg64::Rdi,
        Reg64::R8, Reg64::R9, Reg64::R10, Reg64::R11, Reg64::R12, Reg64::R13, Reg64::R14, Reg64::R15,
    ];
    /// Register `i` of the encoding (0..16), None above
    #[inline]
    pub fn at(i: usize) -> Option<Reg64> {
        Reg64::ALL.get(i).copied()
    }
    /// The register a 4-bit field (REX bit + 3 bits) names: the low four bits of `n`
    #[inline]
    pub const fn low4(n: usize) -> Reg64 {
        match n & 15 {
            0 => Reg64::Rax,
            1 => Reg64::Rcx,
            2 => Reg64::Rdx,
            3 => Reg64::Rbx,
            4 => Reg64::Rsp,
            5 => Reg64::Rbp,
            6 => Reg64::Rsi,
            7 => Reg64::Rdi,
            8 => Reg64::R8,
            9 => Reg64::R9,
            10 => Reg64::R10,
            11 => Reg64::R11,
            12 => Reg64::R12,
            13 => Reg64::R13,
            14 => Reg64::R14,
            // (n & 15 is 15 here)
            _ => Reg64::R15,
        }
    }
    /// RAX..RDI as their 32-bit [`Gpr`], R8..R15 as their number - 8 in [`Cpu::r8`]
    #[inline]
    pub const fn split(self) -> Result<Gpr, usize> {
        match self {
            Reg64::Rax => Ok(Gpr::Eax),
            Reg64::Rcx => Ok(Gpr::Ecx),
            Reg64::Rdx => Ok(Gpr::Edx),
            Reg64::Rbx => Ok(Gpr::Ebx),
            Reg64::Rsp => Ok(Gpr::Esp),
            Reg64::Rbp => Ok(Gpr::Ebp),
            Reg64::Rsi => Ok(Gpr::Esi),
            Reg64::Rdi => Ok(Gpr::Edi),
            Reg64::R8 => Err(0),
            Reg64::R9 => Err(1),
            Reg64::R10 => Err(2),
            Reg64::R11 => Err(3),
            Reg64::R12 => Err(4),
            Reg64::R13 => Err(5),
            Reg64::R14 => Err(6),
            Reg64::R15 => Err(7),
        }
    }
    /// The encoding number (0..16)
    #[inline]
    pub const fn num(self) -> usize {
        match self.split() {
            Ok(g) => g.num(),
            Err(k) => 8 + k,
        }
    }
}

impl Reg64 {
    /// The 64-bit register a general register operand of any width is part of (AH: RAX)
    pub fn of(r: iced_x86::Register) -> Option<Reg64> {
        if r.is_gpr() { Reg64::at(r.full_register().number()) } else { None }
    }
}

impl From<Gpr> for Reg64 {
    #[inline]
    fn from(g: Gpr) -> Reg64 {
        match g {
            Gpr::Eax => Reg64::Rax,
            Gpr::Ecx => Reg64::Rcx,
            Gpr::Edx => Reg64::Rdx,
            Gpr::Ebx => Reg64::Rbx,
            Gpr::Esp => Reg64::Rsp,
            Gpr::Ebp => Reg64::Rbp,
            Gpr::Esi => Reg64::Rsi,
            Gpr::Edi => Reg64::Rdi,
        }
    }
}

pub const EAX: Gpr = Gpr::Eax;
pub const ECX: Gpr = Gpr::Ecx;
pub const EDX: Gpr = Gpr::Edx;
pub const EBX: Gpr = Gpr::Ebx;
pub const ESP: Gpr = Gpr::Esp;
pub const EBP: Gpr = Gpr::Ebp;
pub const ESI: Gpr = Gpr::Esi;
pub const EDI: Gpr = Gpr::Edi;

/// The eight 32-bit general registers (or the upper halves of RAX..RDI), indexed by [`Gpr`].
/// `repr(transparent)`: the JIT addresses register `r` at the field's offset + 4 * r.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Gprs([u32; 8]);
const _: () = assert!(size_of::<Gprs>() == 32);

impl Gprs {
    /// From the values in encoding order
    pub const fn new(v: [u32; 8]) -> Gprs {
        Gprs(v)
    }
    /// The values in encoding order
    pub const fn values(&self) -> [u32; 8] {
        self.0
    }
    /// (register, value) in encoding order
    pub fn iter(&self) -> impl Iterator<Item = (Gpr, u32)> + '_ {
        Gpr::ALL.into_iter().zip(self.0)
    }
    /// (register, value) in encoding order, the values to change
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Gpr, &mut u32)> {
        Gpr::ALL.into_iter().zip(self.0.iter_mut())
    }
}

impl core::ops::Index<Gpr> for Gprs {
    type Output = u32;
    #[inline]
    fn index(&self, r: Gpr) -> &u32 {
        let [eax, ecx, edx, ebx, esp, ebp, esi, edi] = &self.0;
        match r {
            Gpr::Eax => eax,
            Gpr::Ecx => ecx,
            Gpr::Edx => edx,
            Gpr::Ebx => ebx,
            Gpr::Esp => esp,
            Gpr::Ebp => ebp,
            Gpr::Esi => esi,
            Gpr::Edi => edi,
        }
    }
}

impl core::ops::IndexMut<Gpr> for Gprs {
    #[inline]
    fn index_mut(&mut self, r: Gpr) -> &mut u32 {
        let [eax, ecx, edx, ebx, esp, ebp, esi, edi] = &mut self.0;
        match r {
            Gpr::Eax => eax,
            Gpr::Ecx => ecx,
            Gpr::Edx => edx,
            Gpr::Ebx => ebx,
            Gpr::Esp => esp,
            Gpr::Ebp => ebp,
            Gpr::Esi => esi,
            Gpr::Edi => edi,
        }
    }
}

/// A segment register (16-bit mode), in encoding order. [`Segs`] is indexed by it only.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Seg { Es, Cs, Ss, Ds, Fs, Gs }

impl Seg {
    /// Encoding order
    pub const ALL: [Seg; 6] = [Seg::Es, Seg::Cs, Seg::Ss, Seg::Ds, Seg::Fs, Seg::Gs];
    /// Segment register `i` of the encoding (0..6), None above (6 and 7 are #UD)
    #[inline]
    pub fn at(i: usize) -> Option<Seg> {
        Seg::ALL.get(i).copied()
    }
    /// The encoding number (0..6)
    #[inline]
    pub const fn num(self) -> usize {
        match self {
            Seg::Es => 0,
            Seg::Cs => 1,
            Seg::Ss => 2,
            Seg::Ds => 3,
            Seg::Fs => 4,
            Seg::Gs => 5,
        }
    }
}

/// One value per segment register (selectors, bases), indexed by [`Seg`]; `repr(transparent)`
/// for the JIT's field offsets.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct Segs<T>([T; 6]);

impl<T: Copy> Segs<T> {
    /// From the values in encoding order
    pub const fn new(v: [T; 6]) -> Segs<T> {
        Segs(v)
    }
    /// The values in encoding order
    pub const fn values(&self) -> [T; 6] {
        self.0
    }
}

impl<T> core::ops::Index<Seg> for Segs<T> {
    type Output = T;
    #[inline]
    fn index(&self, s: Seg) -> &T {
        let [es, cs, ss, ds, fs, gs] = &self.0;
        match s {
            Seg::Es => es,
            Seg::Cs => cs,
            Seg::Ss => ss,
            Seg::Ds => ds,
            Seg::Fs => fs,
            Seg::Gs => gs,
        }
    }
}

impl<T> core::ops::IndexMut<Seg> for Segs<T> {
    #[inline]
    fn index_mut(&mut self, s: Seg) -> &mut T {
        let [es, cs, ss, ds, fs, gs] = &mut self.0;
        match s {
            Seg::Es => es,
            Seg::Cs => cs,
            Seg::Ss => ss,
            Seg::Ds => ds,
            Seg::Fs => fs,
            Seg::Gs => gs,
        }
    }
}

/// Architectural state of one guest thread. `repr(C)`: the JIT addresses the fields directly.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct Cpu {
    pub regs: Gprs,
    pub eip: u32,
    /// EFLAGS (bit 1 is kept set). Its status flags are stale while `lazy.op` is set: use
    /// [`Cpu::flags`] / [`Cpu::materialize`].
    pub eflags: u32,
    /// Status flags of the last JIT-executed operation, computed only when needed.
    pub lazy: lazy::Lazy,
    /// For the JIT's dispatcher (crates/kura): block hash table, its mask, and how many more
    /// blocks to run before returning.
    pub jit_hash: u32,
    pub jit_mask: u32,
    pub jit_budget: i32,
    /// JIT: ring of recent jumps between regions (see jit::emit_lookup)
    pub jit_ring: u32,
    pub fs_base: u32,
    pub gs_base: u32,
    /// [start, end): jumping here stops execution with [`Exit::Trap`]
    /// (sake's API trap addresses).
    pub trap_lo: u32,
    pub trap_hi: u32,
    /// Value returned by RDTSC when the embedder set no clock ([`set_tsc_source`]).
    pub tsc: u64,
    pub fpu: Fpu,
    /// MMX registers (kept apart from the x87 stack: code does not mix them).
    pub mm: [u64; 8],
    pub xmm: [u128; 8],
    pub mxcsr: u32,
    /// x86-64 (long mode) state. In 32-bit mode none of it is used; RIP, RSP and the FS/GS
    /// bases always fit in 32 bits (all guest memory is below 4GB), so they stay in `eip`,
    /// `regs` and `fs_base`/`gs_base`. `hi` holds the upper halves of RAX..RDI.
    pub mode64: u32,
    pub hi: Gprs,
    pub r8: [u64; 8],
    pub xmm8: [u128; 8],
    /// JIT: guest calls currently nested as wasm calls (see jit::Gen::call_out)
    pub jit_depth: u32,
    /// Long mode: the upper half of RIP. Memory is addressed modulo 4GB (an image whose base is
    /// above 4GB and can't be relocated lives at the base's low half, see Wine's virtual.c),
    /// but RIP keeps its full value for what code computes from it (RIP-relative addresses,
    /// return addresses, unwinding).
    pub rip_hi: u32,
    /// JIT: the thread's call cache (function mode: a direct-mapped table of (target, hash
    /// table entry) pairs that call sites index by their own address, see jit::Gen::call_fn)
    pub jit_calls: u32,
    /// JIT: the host's bitmap of pages with translated code (a bit per 4 KB page), which
    /// translations of code that may rewrite code test their stores against
    /// (jit::set_watch), and the address such a store hit (0: none; the host drops the
    /// translations, see kura's jit::smc_hit) with its length in bytes (jit_smc_len: the
    /// host meets the whole span with the code's, review e18bf44)
    pub jit_code_pages: u32,
    pub jit_smc: u32,
    pub jit_smc_len: u32,
    /// JIT: the start of a block whose code is no longer what it was translated from (code the
    /// host has its translations check on entry, jit::set_fresh); the host drops them (0: none)
    pub jit_stale: u32,
    /// The interpreter's scratch vector register (XMM16 to it): a VEX instruction's
    /// non-destructive source runs its legacy form on a copy here (interp/vex.rs)
    pub vex_tmp: u128,
    /// 16-bit protected mode (Win16): code, stack and addresses are 16-bit, and memory
    /// operands add the base of their segment. `eip` is IP (an offset in CS).
    pub mode16: u32,
    /// Selectors and bases of ES CS SS DS FS GS (16-bit mode only)
    pub sreg: Segs<u16>,
    pub sbase: Segs<u32>,
    /// 16-bit mode: a table of segment bases (u32 per selector index, sel >> 3) that loading
    /// a segment register reads
    pub ldt: u32,
    /// The upper halves (bits 128..255) of YMM0..15, and of the scratch register (16). Legacy
    /// SSE instructions keep them; VEX.128 instructions clear their destination's.
    pub ymmh: [u128; 17],
}
// (the JIT addresses fields by offset_of!; ymmh stays last with no padding after it, on wasm32
// as on the host)
const _: () = assert!(size_of::<Cpu>() == core::mem::offset_of!(Cpu, ymmh) + size_of::<[u128; 17]>());

/// `Cpu::ldt` value for real-mode segment loading in 16-bit mode (base = selector * 16): what
/// the hardware-recorded 80386 real-mode suite (crates/koji/tests/singlestep.rs) runs on.
pub const LDT_REAL: u32 = u32::MAX;

impl Cpu {
    /// 16-bit mode: the linear address of CS:IP.
    #[inline]
    pub fn pc16(&self) -> u32 {
        self.sbase[Seg::Cs].wrapping_add(self.eip & 0xffff)
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu {
            regs: Gprs::new([0; 8]), eip: 0, eflags: 0x202, lazy: lazy::Lazy::default(), jit_hash: 0, jit_mask: 0,
            jit_budget: 0, jit_ring: 0, fs_base: 0, gs_base: 0, trap_lo: 0, trap_hi: 0, tsc: 0,
            fpu: Fpu::default(), mm: [0; 8], xmm: [0; 8], mxcsr: 0x1f80,
            mode64: 0, hi: Gprs::new([0; 8]), r8: [0; 8], xmm8: [0; 8], jit_depth: 0, rip_hi: 0, jit_calls: 0, jit_code_pages: 0, jit_smc: 0, jit_smc_len: 0, jit_stale: 0,
            vex_tmp: 0, mode16: 0, sreg: Segs::new([0; 6]), sbase: Segs::new([0; 6]), ldt: 0, ymmh: [0; 17],
        }
    }
}

/// Why [`step`] stopped. EIP points at the instruction that caused it
/// (for `Trap`, at the trap address itself).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The instruction completed.
    Continue,
    /// Control reached the trap range.
    Trap(u32),
    /// Page fault.
    Fault(Fault),
    /// #DE (DIV / IDIV by zero or overflow).
    DivideError,
    /// #UD, or an instruction this core does not implement.
    InvalidOpcode,
    /// INT n (INT3 = 3).
    Interrupt(u8),
    /// SYSCALL (long mode); EIP is past it, RCX and R11 are set as the instruction sets them.
    Syscall,
    /// HLT (privileged in user mode; the embedder decides).
    Halt,
    /// The single-step trap (#DB): an instruction that began with TF set completed (32/64-bit
    /// modes); EIP is past it.
    SingleStep,
    /// #XM: an SSE floating-point exception whose MXCSR mask bit is clear. The MXCSR flags of
    /// the instruction are set, its destination is not written and EIP points at it.
    SimdException,
}

/// FXSAVE image (512 bytes) of the x87/SSE state at `a`.
pub fn fxsave<M: Memory + ?Sized>(cpu: &Cpu, mem: &mut M, a: u32) -> Result<(), Fault> {
    let f = &cpu.fpu;
    let tw = !f.empty; // abridged: 1 = valid
    mem.write16(a, f.cw)?;
    mem.write16(a.wrapping_add(2), f.status())?;
    mem.write8(a.wrapping_add(4), tw)?;
    mem.write8(a.wrapping_add(5), 0)?;
    for o in (6..24).step_by(2) {
        mem.write16(a.wrapping_add(o), 0)?;
    }
    mem.write32(a.wrapping_add(24), cpu.mxcsr)?;
    mem.write32(a.wrapping_add(28), 0xffff)?;
    for i in 0..8u32 {
        // (an 80-bit value as it is: the precise mode's, or an exact FILD integer)
        let b = cpu.fpu.st80(i).to_bytes();
        for (k, x) in b.iter().enumerate() {
            mem.write8(a.wrapping_add(32 + 16 * i + lo32(k)), *x)?;
        }
        for k in 10..16 {
            mem.write8(a.wrapping_add(32 + 16 * i + k), 0)?;
        }
    }
    for i in 0..if cpu.mode64 != 0 { 16 } else { 8 } {
        let x = cpu.xmm(us(i));
        mem.write64(a.wrapping_add(160 + 16 * i), lo64(x))?;
        mem.write64(a.wrapping_add(168 + 16 * i), lo64(x >> 64))?;
    }
    Ok(())
}

/// Loads the x87/SSE state from an FXSAVE image at `a`.
pub fn fxrstor<M: Memory + ?Sized>(cpu: &mut Cpu, mem: &mut M, a: u32) -> Result<(), Fault> {
    let cw = mem.read16(a)?;
    let sw = mem.read16(a.wrapping_add(2))?;
    let tw = mem.read8(a.wrapping_add(4))?;
    let mxcsr = mem.read32(a.wrapping_add(24))?;
    let mut regs = [f80::F80::default(); 8];
    for (i, r) in regs.iter_mut().enumerate() {
        let mut b = [0u8; 10];
        for (k, x) in b.iter_mut().enumerate() {
            *x = mem.read8(a.wrapping_add(32 + 16 * lo32(i) + lo32(k)))?;
        }
        *r = f80::F80::from_bytes(&b);
    }
    let mut xmm = [0u128; 16];
    for (i, x) in xmm.iter_mut().enumerate().take(if cpu.mode64 != 0 { 16 } else { 8 }) {
        let lo = mem.read64(a.wrapping_add(160 + 16 * lo32(i)))?;
        let hi = mem.read64(a.wrapping_add(168 + 16 * lo32(i)))?;
        *x = u128::from(lo) | u128::from(hi) << 64;
    }
    let f = &mut cpu.fpu;
    f.cw = cw;
    f.set_status(sw);
    f.empty = !tw;
    f.exact = 0;
    f.extv = 0;
    for (i, r) in regs.iter().enumerate() {
        f.load80(lo32(i), *r);
    }
    cpu.mxcsr = mxcsr;
    crate::jit::mx_modes(mxcsr, cpu.eip);
    for (i, x) in xmm.iter().enumerate().take(if cpu.mode64 != 0 { 16 } else { 8 }) {
        cpu.set_xmm(i, *x);
    }
    Ok(())
}

#[cfg(test)]
mod fx_tests {
    extern crate std;
    use super::*;
    use proptest::prelude::*;

    /// Memory up to `top` (exclusive), faulting above; a write log for what landed where.
    struct Top(u64, alloc::collections::BTreeMap<u32, u8>);
    impl Memory for Top {
        fn read8(&mut self, a: u32) -> Result<u8, Fault> {
            if u64::from(a) < self.0 { Ok(self.1.get(&a).copied().unwrap_or(0)) } else { Err(Fault { addr: a, write: false }) }
        }
        fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
            if u64::from(a) < self.0 {
                self.1.insert(a, v);
                Ok(())
            } else {
                Err(Fault { addr: a, write: true })
            }
        }
    }

    proptest! {
        /// FXSAVE / FXRSTOR with the image anywhere, past the top of the address space too:
        /// a fault (never an overflow panic: tests and debug builds check), and with all of it
        /// mapped a round trip of the state.
        #[test]
        fn fx_image_anywhere(a in prop_oneof![u32::MAX - 600..=u32::MAX, 0xffff_fc00u32..=u32::MAX, 0x1000u32..0x2000], top_gap in prop_oneof![Just(0u32), 0u32..1024], xmm: [u128; 8], cw in 0u16..0x1000) {
            let mut cpu = Cpu::default();
            for (i, x) in xmm.iter().enumerate() {
                cpu.set_xmm(i, *x);
            }
            cpu.fpu.cw = cw | 0x40;
            let top = (1u64 << 32) - u64::from(top_gap);
            let mut m = Top(top, Default::default());
            let r = fxsave(&cpu, &mut m, a);
            let fits = u64::from(a) + 512 <= top;
            if fits {
                prop_assert!(r.is_ok());
                let mut back = Cpu::default();
                prop_assert!(fxrstor(&mut back, &mut m, a).is_ok());
                for i in 0..8 {
                    prop_assert_eq!(back.xmm(i), cpu.xmm(i));
                }
            } else {
                // (a wrapped image is not one the tests ask for: only that nothing panics)
                let _ = fxrstor(&mut Cpu::default(), &mut m, a);
            }
        }
    }
}
