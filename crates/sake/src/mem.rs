//! Guest memory: the low 2 GiB of the wasm linear memory, guest address = host address.
//!
//! The whole range is already backed (emscripten's INITIAL_MEMORY covers it and the host's
//! data starts at 0x80000000), so reserving and committing is bookkeeping only. Pages are
//! zero when first handed out; released ones are zeroed again before reuse.

use kome::locks::Lock as _;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// (not Windows' 0x10000: koji's JIT range-checks accesses with a 32 KB margin above its
/// GUEST_LO of 0x10000, and a block whose address is below that bails to the interpreter on
/// every access: Borland's memory manager put its table at 0x10000)
pub const LO: u32 = 0x3_0000;
pub const HI: u32 = 0x7ff0_0000;
/// Where sake's own data starts (crates/sake/build.sh GLOBAL_BASE, koji::jit::GUEST_HI)
pub const GUEST_TOP: u32 = 0x8000_0000;
const GRAN: u32 = 0x1_0000;
use kome::num::us;
pub const PAGE: u32 = 0x1000;

// (usize holds every u32: wasm32 and 64-bit hosts; the two conversions below are the only casts
// between guest addresses and the host's)
const _: () = assert!(std::mem::size_of::<usize>() >= std::mem::size_of::<u32>());
/// The host pointer of guest address `a` (guest address = host address)
#[inline(always)]
fn host<T>(a: u32) -> *mut T {
    std::ptr::with_exposed_provenance_mut(kome::num::us(a))
}

/// A guest address: a value, x86 address arithmetic only. Its arithmetic wraps at 4 GiB as the
/// x86's does (an offset past the top comes round to the bottom: what the guest's own code
/// computes); a length or a count is not one (its arithmetic is checked where it is done). Made
/// from a u32 by `ga(a)` (or `.into()`), given back by `.get()`. It does not touch memory: the
/// accessors here do (`mem::r32(a)`, `mem::w32(a, v)`), each through a checked [`Range`]
/// (review 2026-11-15).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Ga(u32);

/// Guest address `a`
#[inline(always)]
pub const fn ga(a: u32) -> Ga {
    Ga(a)
}

impl Ga {
    #[inline(always)]
    pub const fn get(self) -> u32 {
        self.0
    }
    /// Element `i` of an array of `size`-byte elements here (wrapping, as the x86 computes it)
    #[inline(always)]
    pub const fn at(self, i: u32, size: u32) -> Ga {
        Ga(self.0.wrapping_add(i.wrapping_mul(size)))
    }
    /// How far past `base` this is (wrapping)
    #[inline(always)]
    pub const fn since(self, base: Ga) -> u32 {
        self.0.wrapping_sub(base.0)
    }
    /// The end of `n` bytes from here, unwrapped (a range check: past GUEST_TOP or 4 GiB shows)
    #[inline(always)]
    pub fn end(self, n: u32) -> u64 {
        // (two u32s: the sum is below 2^33, never wraps)
        u64::from(self.0).wrapping_add(u64::from(n))
    }
}
impl std::ops::Add<u32> for Ga {
    type Output = Ga;
    #[inline(always)]
    fn add(self, o: u32) -> Ga {
        Ga(self.0.wrapping_add(o))
    }
}
impl std::ops::Sub<u32> for Ga {
    type Output = Ga;
    #[inline(always)]
    fn sub(self, o: u32) -> Ga {
        Ga(self.0.wrapping_sub(o))
    }
}
impl std::ops::AddAssign<u32> for Ga {
    #[inline(always)]
    fn add_assign(&mut self, o: u32) {
        self.0 = self.0.wrapping_add(o);
    }
}
impl From<u32> for Ga {
    #[inline(always)]
    fn from(a: u32) -> Ga {
        Ga(a)
    }
}
impl From<Ga> for u32 {
    #[inline(always)]
    fn from(a: Ga) -> u32 {
        a.0
    }
}

// ---------------------------------------------------------------------------------------------
// 64-bit programs (PE32+): pointers, handles and SIZE_Ts are 8 bytes in their structures. Guest
// addresses are still below 4G (and below HI): the upper halves are written as zero and ignored
// when read.

static X64: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the program is a 64-bit one (set by the loader before anything runs).
#[inline(always)]
pub fn x64() -> bool {
    X64.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_x64() {
    X64.store(true, std::sync::atomic::Ordering::Relaxed);
    kura::d3d9::set_ptr_size(8);
}

/// Size of a pointer (4 or 8).
#[inline(always)]
pub fn ps() -> u32 {
    if x64() { 8 } else { 4 }
}

/// Reads a pointer-sized field (its low half on x64).
#[inline(always)]
pub fn rp(a: impl Into<Ga>) -> u32 {
    r32(a)
}

/// Writes a pointer-sized field (zero-extended on x64; INVALID_HANDLE_VALUE and other small
/// negative values sign-extended).
#[inline(always)]
pub fn wp(a: impl Into<Ga>, v: u32) {
    let a = a.into();
    if x64() {
        w64(a, u64::from(v) | if v >= 0xffff_f000 { 0xffff_ffff_0000_0000 } else { 0 });
    } else {
        // (the whole of an x86 pointer: not w32, which kura::outs asks)
        wr(a, v);
    }
}

/// wp if `a` is not NULL.
#[inline]
pub fn putp(a: impl Into<Ga>, v: u32) {
    let a = a.into();
    if a.get() != 0 {
        wp(a, v)
    }
}

/// The 64-bit value of guest address `a` as code sees it: an allocation that stands for one
/// above 4G (a fixed image based there, a VirtualAlloc asked there: `set_high`) lives at its
/// address's low half, and addresses in it get the upper half back.
#[inline]
pub fn widen(a: u32) -> u64 {
    if HIGH.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return u64::from(a);
    }
    u64::from(a) | u64::from(find(a).map_or(0, |(_, r)| r.hi)) << 32
}

/// How many allocations stand for addresses above 4G (widen's fast path: none).
static HIGH: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Makes the allocation at `base` the one at `hi:base` (x64): addresses in it widen to that.
/// Memory is addressed modulo 4G (koji), so allocations above 4G can't share a low half; two
/// of them (or one and a plain low allocation) that would are refused by `reserve` already.
pub fn set_high(base: u32, hi: u32) -> bool {
    let mut sp = SPACE.locked();
    let Some(r) = sp.get_mut(&base) else { return false };
    if r.hi == 0 && hi != 0 {
        HIGH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    } else if r.hi != 0 && hi == 0 {
        HIGH.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
    r.hi = hi;
    true
}

/// The upper half of the address the allocation holding `a` stands for (0: a plain one, or
/// none there).
pub fn high_of(a: u32) -> u32 {
    if HIGH.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return 0;
    }
    find(a).map_or(0, |(_, r)| r.hi)
}

/// Writes guest address `v` as a pointer (x64: its 64-bit value, `widen`) if `a` is not NULL.
pub fn put_addr(a: u32, v: u32) {
    if a != 0 {
        if x64() { w64(a, widen(v)) } else { wr(ga(a), v) }
    }
}

// ---------------------------------------------------------------------------------------------
// reading and writing guest memory: one rule, one checked type

/// The guest's first 64 KiB: never handed out (NULL pointers fault on Windows too).
pub const NULL_TOP: u32 = 0x1_0000;

/// Guest memory `a..a + n` that sake may read and write for the program: past the NULL pages
/// and below GUEST_TOP (sake's own heap and stacks are above it: a bad pointer an API is given,
/// which Wine's tests pass on purpose, would corrupt sake itself). Made only by [`range`]; the
/// accesses that touch memory take one, so none goes unchecked (review 2026-11-15: w32 allowed
/// address 0, copy checked only its destination, write checked a length cut to 32 bits).
/// ```compile_fail
/// let r = sake::mem::Range { a: 0, n: 4 };
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    a: u32,
    n: u32,
}

/// `n` bytes at `a` if they are all guest memory sake may use (see [`Range`]); n == 0 always is.
#[inline(always)]
pub fn range(a: u32, n: u32) -> Option<Range> {
    (n == 0 || (a >= NULL_TOP && ga(a).end(n) <= u64::from(GUEST_TOP))).then_some(Range { a, n })
}
/// [`range`] for a host length (a Rust slice's): None past 4 GiB too
#[inline]
pub fn range_of(a: u32, n: usize) -> Option<Range> {
    range(a, u32::try_from(n).ok()?)
}
/// range(), and a note the first time a write is refused
#[inline(always)]
fn for_write(a: u32, n: u32) -> Option<Range> {
    let r = range(a, n);
    if r.is_none() {
        refused(a, n);
    }
    r
}

impl Range {
    #[inline(always)]
    pub fn addr(self) -> u32 {
        self.a
    }
    #[inline(always)]
    pub fn len(self) -> u32 {
        self.n
    }
    #[inline(always)]
    pub fn is_empty(self) -> bool {
        self.n == 0
    }
    /// The host pointer of its start (guest address = host address)
    #[inline(always)]
    fn ptr<T>(self) -> *mut T {
        host(self.a)
    }
    /// An `N`-byte value at its start (`N` <= its length)
    #[inline(always)]
    fn read<T: Copy>(self) -> T {
        debug_assert!(std::mem::size_of::<T>() <= us(self.n));
        // SAFETY: a Range is guest memory (backed: the module doc) at least size_of::<T>() long
        // (the callers make it so); read_unaligned allows any address.
        unsafe { self.ptr::<T>().cast_const().read_unaligned() }
    }
    #[inline(always)]
    fn write<T: Copy>(self, v: T) {
        debug_assert!(std::mem::size_of::<T>() <= us(self.n));
        // SAFETY: as in read: guest memory, never sake's own.
        unsafe { self.ptr::<T>().write_unaligned(v) }
    }
}

#[cold]
#[inline(never)]
fn refused(a: u32, n: u32) {
    if a < NULL_TOP {
        // (a NULL pointer: Wine's tests pass many, an API refuses them as Windows faults)
        return;
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("sake: a write of {n} bytes to {a:#x} (not guest memory) was dropped"));
}

/// The value at `a`; 0 where that is not guest memory (Windows would fault)
#[inline(always)]
fn rd<T: Copy + Default>(a: Ga) -> T {
    range(a.get(), kome::num::u32_of(std::mem::size_of::<T>())).map_or_else(T::default, Range::read)
}
/// `v` at `a`; dropped where that is not guest memory
#[inline(always)]
fn wr<T: Copy>(a: Ga, v: T) {
    if let Some(r) = for_write(a.get(), kome::num::u32_of(std::mem::size_of::<T>())) {
        r.write(v);
    }
}

#[inline(always)]
pub fn r8(a: impl Into<Ga>) -> u8 {
    rd(a.into())
}
#[inline(always)]
pub fn r16(a: impl Into<Ga>) -> u16 {
    rd(a.into())
}
#[inline(always)]
pub fn r32(a: impl Into<Ga>) -> u32 {
    rd(a.into())
}
#[inline(always)]
pub fn r64(a: impl Into<Ga>) -> u64 {
    rd(a.into())
}
#[inline(always)]
pub fn w8(a: impl Into<Ga>, v: u8) {
    wr(a.into(), v)
}
#[inline(always)]
pub fn w16(a: impl Into<Ga>, v: u16) {
    wr(a.into(), v)
}
#[inline(always)]
pub fn w32(a: impl Into<Ga>, v: u32) {
    let a = a.into();
    // (a 32-bit write to one of the running call's pointer-sized out-parameters: short of it on
    // x64, and the same mistake on x86, where the tests run, kura::outs; wp / putp / api::Out
    // write those whole)
    kura::outs::wrote32(a.get());
    wr(a, v)
}
#[inline(always)]
pub fn w64(a: impl Into<Ga>, v: u64) {
    wr(a.into(), v)
}
/// Writes `v` if `a` is not NULL.
#[inline]
pub fn put32(a: impl Into<Ga>, v: u32) {
    let a = a.into();
    if a.get() != 0 {
        w32(a, v)
    }
}
/// Writes `v` if `a` is not NULL.
#[inline]
pub fn put64(a: impl Into<Ga>, v: u64) {
    let a = a.into();
    if a.get() != 0 {
        w64(a, v)
    }
}
/// A copy of guest memory `a..a + n`; empty where that is not all guest memory
pub fn bytes(a: u32, n: u32) -> Vec<u8> {
    match range(a, n) {
        // (the copy is made inside raw, whose closure is this one: nothing else runs meanwhile)
        Some(r) => raw(r, <[u8]>::to_vec),
        None => Vec::new(),
    }
}
/// The `N` bytes at `a`, copied (a GUID, a key, a header); zeros where that is not guest memory
pub fn array<const N: usize>(a: u32) -> [u8; N] {
    match range(a, kome::num::u32_of(N)) {
        Some(r) => raw(r, |s| s.try_into().unwrap_or([0; N])),
        None => [0; N],
    }
}
/// `f` with a copy of guest memory `a..a + n` to change; the bytes `f` changed are written
/// back when it returns (the others are left as they are then). None (and `f` not run) where
/// that is not all guest memory. `f` holds sake's own buffer, not the guest's memory: it may
/// read and write guest memory itself meanwhile (`mem::w8`, another `update` of the same bytes:
/// what the outer one changed is written last), and the guest's own threads may too, without
/// two Rust references to one byte (review 2026-11-16: `view_mut` inside `view_mut` gave two
/// `&mut` to the same guest bytes from safe code).
pub fn update<R>(a: u32, n: u32, f: impl FnOnce(&mut [u8]) -> R) -> Option<R> {
    let r = for_write(a, n)?;
    let was = raw(r, <[u8]>::to_vec);
    let mut buf = was.clone();
    let out = f(&mut buf);
    raw_mut(r, |s| {
        for ((d, new), old) in s.iter_mut().zip(&buf).zip(&was) {
            if new != old {
                *d = *new;
            }
        }
    });
    Some(out)
}
/// `f` with guest memory `a..a + n` itself to read (no copy: decoders, hashes of large
/// buffers); empty where that is not all guest memory.
///
/// # Safety
/// Nothing writes those bytes while `f` runs: `f` does not write guest memory (no `mem::w*`,
/// `write`, `update`, guest code), and no other thread does (the program handed the buffer to
/// the call it is in, as it does a WriteFile's).
/// ```compile_fail
/// sake::mem::view_unchecked(0x1000, 4, |s| s[0]); // unsafe to call
/// ```
pub unsafe fn view_unchecked<R>(a: u32, n: u32, f: impl FnOnce(&[u8]) -> R) -> R {
    match range(a, n) {
        Some(r) => raw(r, f),
        None => f(&[]),
    }
}
/// `f` with guest memory `a..a + n` itself to write (no copy: a file read or a decoded frame
/// into the program's buffer); None (and `f` not run) where that is not all guest memory.
///
/// # Safety
/// Nothing else reads or writes those bytes while `f` runs: `f` does not touch guest memory
/// (no `mem::r*`/`w*`, `bytes`, `update`, guest code), and no other thread does (the program
/// handed the buffer to the call it is in, as it does a ReadFile's).
/// ```compile_fail
/// sake::mem::view_mut_unchecked(0x1000, 4, |s| s[0] = 1); // unsafe to call
/// ```
pub unsafe fn view_mut_unchecked<R>(a: u32, n: u32, f: impl FnOnce(&mut [u8]) -> R) -> Option<R> {
    for_write(a, n).map(|r| raw_mut(r, f))
}
/// raw over `a..a + n`, empty where that is not all guest memory (this module's own scans)
fn raw_or_empty<R>(a: u32, n: u32, f: impl FnOnce(&[u8]) -> R) -> R {
    match range(a, n) {
        Some(r) => raw(r, f),
        None => f(&[]),
    }
}
/// The one place guest memory is read as a Rust slice: only with closures of this module
/// (which touch no other guest memory) or under view_unchecked's contract.
fn raw<R>(r: Range, f: impl FnOnce(&[u8]) -> R) -> R {
    if r.is_empty() {
        return f(&[]);
    }
    // SAFETY: a Range is guest memory (the low 2 GiB of the wasm memory, all of it backed: the
    // module doc), not empty: its pointer is not NULL. No Rust code writes it while `f` runs:
    // this module's closures write nothing, view_unchecked's caller promises it. (The guest's
    // own translated code is no Rust code: a program that writes a buffer it handed to a call
    // races with itself, as on Windows.) The borrow ends with `f`.
    f(unsafe { std::slice::from_raw_parts(r.ptr::<u8>().cast_const(), us(r.n)) })
}
/// The one place guest memory is written as a Rust slice (as raw: this module's closures or
/// view_mut_unchecked's contract).
fn raw_mut<R>(r: Range, f: impl FnOnce(&mut [u8]) -> R) -> R {
    if r.is_empty() {
        return f(&mut []);
    }
    // SAFETY: as in raw, and nothing else reads it meanwhile; the borrow ends with `f`.
    f(unsafe { std::slice::from_raw_parts_mut(r.ptr::<u8>(), us(r.n)) })
}
/// The host pointer of guest memory `a..a + n`, None where that is not all guest memory: for
/// code that walks it by pointer (the software rasterizers). Reading or writing through it is
/// the caller's unsafe, inside that range.
pub fn host_ptr(a: u32, n: u32) -> Option<*const u8> {
    range(a, n).map(|r| r.ptr::<u8>().cast_const())
}
/// host_ptr for writing
pub fn host_ptr_mut(a: u32, n: u32) -> Option<*mut u8> {
    for_write(a, n).map(Range::ptr::<u8>)
}
pub fn fill(a: u32, n: u32, v: u8) {
    if let Some(r) = for_write(a, n) {
        raw_mut(r, |s| s.fill(v));
    }
}
/// Zeroes [a, a+n) without writing to the pages in it that are zero already. The browser backs
/// a page of the wasm memory with RAM at its first write and never takes it back (wasm cannot
/// give memory back), while a read of a page never written costs nothing (the kernel's shared
/// zero page). Zeroing what the program reserved but never used (a 256 MB block of which it
/// committed a few pages, a HEAP_ZERO_MEMORY block it never fills) would make all of it RAM.
/// Short ranges are filled plainly.
pub fn zero(a: u32, n: u32) {
    if n < ZERO_SKIP_MIN {
        return fill(a, n, 0);
    }
    let Some(r) = for_write(a, n) else { return };
    raw_mut(r, |s| {
        // (pages of the guest's: a starts where it starts, the rest on page boundaries)
        let first = us(PAGE.saturating_sub(a % PAGE)).min(s.len());
        let (head, rest) = s.split_at_mut(first);
        for p in std::iter::once(head).chain(rest.chunks_mut(us(PAGE))) {
            if !is_zero(p) {
                p.fill(0);
            }
        }
    });
}
/// (below this zero() fills: one or two pages)
const ZERO_SKIP_MIN: u32 = 2 * PAGE;

/// Whether every byte of `s` is 0 (stops at the first 256-byte block that is not; within a
/// block, no early exit, so that it is vectorized).
pub fn is_zero(s: &[u8]) -> bool {
    let mut c = s.chunks_exact(256);
    for b in &mut c {
        if b.iter().fold(0u8, |x, &y| x | y) != 0 {
            return false;
        }
    }
    c.remainder().iter().all(|&b| b == 0)
}

/// `n` bytes from `src` to `dst` (they may overlap); nothing where either is not all guest memory
pub fn copy(dst: u32, src: u32, n: u32) {
    let (Some(s), Some(d)) = (range(src, n), for_write(dst, n)) else { return };
    if n == 0 {
        return;
    }
    // SAFETY: both are guest memory n bytes long (Ranges); ptr::copy allows the overlap.
    unsafe { std::ptr::copy(s.ptr::<u8>().cast_const(), d.ptr::<u8>(), us(n)) }
}
/// `data` (host memory sake holds) at `dst`; nothing where that is not all guest memory
pub fn write(dst: u32, data: &[u8]) {
    let Some(d) = range_of(dst, data.len()) else {
        refused(dst, kome::num::u32_of(data.len()));
        return;
    };
    raw_mut(d, |s| s.copy_from_slice(data));
}

/// NUL-terminated byte string at `a` (without the NUL).
pub fn cstr(a: u32) -> Vec<u8> {
    if a == 0 {
        return Vec::new();
    }
    // (one scan of the guest memory up to GUEST_TOP: an unterminated string stops there
    // instead of wrapping past 4 GB, review b07)
    if a < GUEST_TOP {
        return raw_or_empty(a, GUEST_TOP.saturating_sub(a), |s| s.iter().take_while(|&&c| c != 0).copied().collect());
    }
    let mut n = 0;
    while r8(ga(a) + n) != 0 {
        n = n.wrapping_add(1);
    }
    bytes(a, n)
}
/// The bytes of `n` UTF-16 units in guest memory (saturating at 4 GiB)
pub fn wbytes(n: usize) -> u32 {
    kome::num::u32_of(n).saturating_mul(2)
}
/// NUL-terminated UTF-16 string at `a`.
pub fn wstr(a: u32) -> Vec<u16> {
    if a == 0 {
        return Vec::new();
    }
    if a < GUEST_TOP {
        return raw_or_empty(a, GUEST_TOP.saturating_sub(a) & !1, |s| {
            let units = s.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c));
            let n = units.clone().position(|c| c == 0).unwrap_or(s.len() / 2);
            let mut v = Vec::with_capacity(n);
            v.extend(units.take(n));
            v
        });
    }
    let mut v = Vec::new();
    let mut p = ga(a);
    loop {
        let c = r16(p);
        if c == 0 {
            break;
        }
        v.push(c);
        p += 2;
    }
    v
}
/// Up to `n` bytes, stopping at a NUL.
pub fn cstrn(a: u32, n: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for i in 0..n {
        let c = r8(ga(a) + i);
        if c == 0 {
            break;
        }
        v.push(c);
    }
    v
}
/// Up to `n` UTF-16 units, stopping at a NUL.
pub fn wstrn(a: u32, n: u32) -> Vec<u16> {
    let mut v = Vec::new();
    for i in 0..n {
        let c = r16(ga(a).at(i, 2));
        if c == 0 {
            break;
        }
        v.push(c);
    }
    v
}
/// Copies `s` + NUL to `dst` (at most `cap` bytes including the NUL); returns bytes without NUL.
pub fn put_cstr(dst: u32, cap: u32, s: &[u8]) -> u32 {
    if cap == 0 || dst == 0 {
        return 0;
    }
    let n = (kome::num::lo32(s.len())).min(cap.saturating_sub(1));
    write(dst, s.get(..us(n)).unwrap_or(s));
    w8(ga(dst) + n, 0);
    n
}
pub fn put_wstr(dst: u32, cap: u32, s: &[u16]) -> u32 {
    if cap == 0 || dst == 0 {
        return 0;
    }
    let n = (kome::num::lo32(s.len())).min(cap.saturating_sub(1));
    for (i, &u) in (0..n).zip(s) {
        w16(ga(dst).at(i, 2), u);
    }
    w16(ga(dst).at(n, 2), 0);
    n
}

// ---------------------------------------------------------------------------------------------
// the address space

#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub size: u32,
    pub prot: u32,
    /// MEM_COMMIT (0x1000) or MEM_RESERVE (0x2000)
    pub state: u32,
    pub alloc_base: u32,
    pub alloc_prot: u32,
    /// MEM_IMAGE 0x1000000, MEM_MAPPED 0x40000, MEM_PRIVATE 0x20000
    pub kind: u32,
    /// x64: the upper half of the address it stands for (`set_high`), 0 for a plain one
    pub hi: u32,
}

/// Allocations by base. Each allocation is one region here (commit state is kept per
/// allocation, which is all games look at).
static SPACE: Mutex<BTreeMap<u32, Region>> = Mutex::new(BTreeMap::new());

/// Save states (snap.rs): the guest memory a state holds, its allocations.
pub fn snap_ranges(v: &mut Vec<(u32, u32)>) {
    let sp = SPACE.lock().unwrap_or_else(|e| e.into_inner());
    v.extend(sp.iter().map(|(&a, r)| (a, r.size)));
}

/// `v` rounded up to a multiple of `a` (a power of two), wrapping at 4 GiB as an address does
fn align_up(v: u32, a: u32) -> u32 {
    v.wrapping_add(a.wrapping_sub(1)) & !a.wrapping_sub(1)
}
/// `size` (at least 1) rounded up to whole pages, None past the guest address space (a size
/// near 4 GB wrapped to 0 and reserved an empty region the program took for ~4 GB, review b07)
fn page_size(size: u32) -> Option<u32> {
    let n = u64::from(size.max(1)).div_ceil(u64::from(PAGE)).saturating_mul(u64::from(PAGE));
    (n <= u64::from(HI)).then_some(kome::num::lo32(n))
}

/// Reserves `size` bytes at `want` (0 = anywhere, lowest first; `top` = highest first).
pub fn reserve(want: u32, size: u32, prot: u32, kind: u32, top: bool) -> Option<u32> {
    let size = page_size(size)?;
    let mut sp = SPACE.locked();
    let base = if want != 0 {
        let b = want & !(GRAN - 1);
        let end = b.checked_add(size)?;
        if b < LO || end > HI {
            return None;
        }
        let clash = sp.range(..end).next_back().is_some_and(|(&a, r)| ga(a).end(r.size) > u64::from(b));
        if clash {
            return None;
        }
        b
    } else {
        let holes = holes(&sp, size);
        if top {
            // (each hole holds `size`: hi - size does not go below lo)
            let &(_, hi) = holes.iter().rev().find(|(lo, hi)| hi.saturating_sub(size) & !(GRAN - 1) >= *lo)?;
            hi.saturating_sub(size) & !(GRAN - 1)
        } else {
            holes.first()?.0
        }
    };
    insert(&mut sp, base, size, prot, kind);
    Some(base)
}

/// The free ranges [lo, hi) of the address space that hold `size` bytes.
fn holes(sp: &BTreeMap<u32, Region>, size: u32) -> Vec<(u32, u32)> {
    let mut holes = Vec::new();
    let mut at = LO;
    for (&a, r) in sp.iter() {
        if a >= at.saturating_add(size) {
            holes.push((at, a));
        }
        at = at.max(align_up((ga(a) + r.size).get(), GRAN));
    }
    if HI >= at.saturating_add(size) {
        holes.push((at, HI));
    }
    holes
}

/// (tests: test_guest::enter mapped the guest's address space)
#[cfg(test)]
pub static TEST_MAPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn insert(sp: &mut BTreeMap<u32, Region>, base: u32, size: u32, prot: u32, kind: u32) {
    // (tests: free space is zero, as release leaves it; bytes there were written by someone
    // who did not own them, and a new reservation would show them as its contents, #242)
    #[cfg(test)]
    {
        let n = size.min(16 * PAGE);
        if TEST_MAPPED.load(std::sync::atomic::Ordering::Relaxed) && range(base, n).is_some() {
            assert!(raw_or_empty(base, n, is_zero), "free guest memory at {base:#x} is not zero: something wrote there without reserving it");
        }
    }
    sp.insert(base, Region { size, prot, state: 0x2000, alloc_base: base, alloc_prot: prot, kind, hi: 0 });
    PAGES.locked().insert(base, vec![0; kome::num::us(size / PAGE)]);
}

/// Reserves `size` bytes at an `align`-aligned address (a power of two, at least GRAN), the
/// lowest one free; None when no free range holds all of them. (Never fewer: the program
/// takes the whole block as its own. A smaller block it once got at the top of a full address
/// space ran on over the TEBs, stacks and DLL images above, お仕置きLesson 体験版, Unity
/// 6000.4.)
pub fn reserve_aligned(size: u32, align: u32, prot: u32, kind: u32) -> Option<u32> {
    let size = page_size(size)?;
    let mut sp = SPACE.locked();
    let base = aligned_fit(&holes(&sp, size), size, align)?;
    insert(&mut sp, base, size, prot, kind);
    Some(base)
}

/// The lowest `align`-aligned address with `size` bytes free in one of `holes` ([lo, hi))
fn aligned_fit(holes: &[(u32, u32)], size: u32, align: u32) -> Option<u32> {
    holes.iter().find_map(|&(lo, hi)| {
        let b = ga(lo).end(align).saturating_sub(1) & !u64::from(align).saturating_sub(1);
        (b.saturating_add(u64::from(size)) <= u64::from(hi)).then_some(kome::num::lo32(b))
    })
}

/// Whether [a, a+n) lies inside one allocation (VirtualAlloc's MEM_COMMIT of a range that runs
/// past its reservation's end fails on Windows, ERROR_INVALID_ADDRESS)
pub fn in_one_allocation(a: u32, n: u32) -> bool {
    find(a).is_some_and(|(b, r)| ga(a).end(n.max(1)) <= ga(b).end(r.size))
}

/// The size a program reserves to find an aligned block inside (twice the block less a page,
/// Unity 6's allocators: 512 MB - 4 KB for a 256 MB block on a 256 MB boundary): the block
/// size, when `size` is that (blocks of 16 MB and up).
pub fn over_reserved_block(size: u32) -> Option<u32> {
    let s = size.checked_add(PAGE)?;
    (s.is_power_of_two() && s >= 32 << 20).then_some(s / 2)
}

/// Per-page commit state of each allocation: 0 reserved, COMMITTED | protection.
static PAGES: Mutex<BTreeMap<u32, Vec<u32>>> = Mutex::new(BTreeMap::new());
const COMMITTED: u32 = 0x8000_0000;

/// Tells the JIT which pages are writable and executable (Windows protection `prot`, 0: none):
/// it interprets W|X pages outside the main exe and checks stores on the main exe's.
fn jit_prot(a: u32, n: u32, prot: u32) {
    // PAGE_NOACCESS, PAGE_GUARD: any access faults; PAGE_READONLY, PAGE_EXECUTE(_READ): a
    // write (prot 0: decommitted or released, see decommit_range; reserved pages are not
    // checked)
    use kura::jit::{READ_FAULTS, WRITE_FAULTS};
    let none = prot & 0xff == 0x01 || prot & 0x100 != 0;
    let kind = if none {
        READ_FAULTS | WRITE_FAULTS
    } else if prot & 0xff != 0 && prot & 0xcc == 0 {
        WRITE_FAULTS
    } else {
        0
    };
    kura::jit::fault_pages(a, n, kind, none);
    let wx = prot & (0x40 | 0x80) != 0;
    let p = if prot == 0 { 0 } else if wx { libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC } else { libc::PROT_READ };
    kura::jit::pages_protected(us(a), us(n), p);
}

/// The pages [first, last) of an allocation of `size` bytes that [off, off + n) touches (`off`
/// inside it), cut at its end: in 64 bits, since `n` is the guest's (VirtualProtect,
/// NtProtectVirtualMemory, VirtualFree's size): `off + n + PAGE - 1` in u32 wrapped for n near
/// 0xFFFFFFFF, `last` came out before `first` and `p[first..last]` panicked, the VM gone, or
/// `(last - first) * PAGE` underflowed into the JIT's page tables (review A-13).
/// The bytes of pages first..last (last >= first, both within an allocation below 4 GiB)
fn span(first: u32, last: u32) -> u32 {
    last.saturating_sub(first).saturating_mul(PAGE)
}
pub(crate) fn page_span(off: u32, n: u32, size: u32) -> (u32, u32) {
    let pages = size / PAGE;
    let first = (off / PAGE).min(pages);
    let last = kome::num::lo32((ga(off).end(n)).div_ceil(u64::from(PAGE)).min(u64::from(pages)));
    (first, last.max(first))
}

/// Commits a whole allocation.
pub fn commit(base: u32, prot: u32) {
    let size = SPACE.locked().get(&base).map(|r| r.size);
    if let Some(n) = size {
        commit_range(base, n, prot);
    }
}

/// Commits the pages of [a, a+n) (inside one allocation); false if they are not reserved.
pub fn commit_range(a: u32, n: u32, prot: u32) -> bool {
    let Some((b, r)) = find(a) else { return false };
    let (first, last) = page_span(ga(a).since(ga(b)), n.max(1), r.size);
    if let Some(p) = PAGES.locked().get_mut(&b) {
        for x in p.iter_mut().take(us(last)).skip(us(first)) {
            *x = COMMITTED | prot;
        }
    }
    if let Some(r) = SPACE.locked().get_mut(&b) {
        r.state = 0x1000;
        r.prot = prot;
    }
    jit_prot(ga(b).at(first, PAGE).get(), span(first, last), prot);
    true
}

/// Decommits (and zeroes) the pages of [a, a+n); n = 0: to the end of the allocation.
pub fn decommit_range(a: u32, n: u32) -> bool {
    let Some((b, r)) = find(a) else { return false };
    let (first, last) = if n == 0 { (ga(a).since(ga(b)) / PAGE, r.size / PAGE) } else { page_span(ga(a).since(ga(b)), n, r.size) };
    let mut g = PAGES.locked();
    if let Some(p) = g.get_mut(&b) {
        for (i, x) in (first..last).zip(p.iter_mut().skip(us(first))) {
            if *x != 0 {
                let at = ga(b).at(i, PAGE).get();
                if !raw_or_empty(at, PAGE, is_zero) {
                    fill(at, PAGE, 0);
                }
                *x = 0;
            }
        }
    }
    drop(g);
    jit_prot(ga(b).at(first, PAGE).get(), span(first, last), 0);
    // an access to a decommitted page faults
    kura::jit::fault_pages(ga(b).at(first, PAGE).get(), span(first, last), kura::jit::READ_FAULTS | kura::jit::WRITE_FAULTS, false);
    true
}

/// VirtualQuery: (allocation base, region, run start, run size, MEM_COMMIT/MEM_RESERVE, protection).
pub fn query(a: u32) -> Option<(u32, Region, u32, u32, u32, u32)> {
    let (b, r) = find(a)?;
    let g = PAGES.locked();
    let p = g.get(&b)?;
    let i = kome::num::us(ga(a).since(ga(b)) / PAGE);
    let &v = p.get(i)?;
    let run = kome::num::u32_of(p.iter().skip(i).take_while(|&&x| x == v).count());
    let state = if v & COMMITTED != 0 { 0x1000 } else { 0x2000 };
    Some((b, r, ga(b).at(kome::num::u32_of(i), PAGE).get(), run.saturating_mul(PAGE), state, if v & COMMITTED != 0 { v & !COMMITTED } else { 0 }))
}

/// How many of the `n` bytes from `a` on can be read (`write` false) or written: committed pages
/// whose protection allows it (no PAGE_GUARD). `code_too`: executable pages count as writable,
/// as for WriteProcessMemory (it makes them writable for the copy; read-only data it refuses).
pub fn accessible(a: u32, n: u32, write: bool, code_too: bool) -> u32 {
    let mut at = u64::from(a);
    let end = ga(a).end(n);
    while at < end {
        if at < u64::from(LO) {
            break;
        }
        let Some((_, _, run, size, state, prot)) = query(kome::num::lo32(at)) else { break };
        let ok = state == 0x1000
            && prot & 0x100 == 0
            && if write { prot & 0xcc != 0 || code_too && prot & 0x30 != 0 } else { prot & 0xfe != 0 };
        if !ok {
            break;
        }
        at = ga(run).end(size);
    }
    kome::num::lo32(at.min(end).saturating_sub(u64::from(a)))
}

/// Protection of the page at `a` (VirtualProtect), changing it to `prot` for [a, a+n).
pub fn protect(a: u32, n: u32, prot: u32) -> Option<u32> {
    let (b, r) = find(a)?;
    let mut g = PAGES.locked();
    let p = g.get_mut(&b)?;
    let (first, last) = page_span(ga(a).since(ga(b)), n.max(1), r.size);
    let old = p.get(us(first))? & !COMMITTED;
    for x in p.iter_mut().take(us(last)).skip(us(first)) {
        if *x & COMMITTED != 0 {
            *x = COMMITTED | prot;
        }
    }
    drop(g);
    jit_prot(ga(b).at(first, PAGE).get(), span(first, last), prot);
    // writable code may be rewritten: the JIT drops what it translated there (QLIE patches its
    // code with VirtualProtect)
    if prot & 0xcc != 0 {
        kura::jit::code_changed(us(a), kome::num::us(ga(b).at(last, PAGE).since(ga(a))));
    }
    Some(if old == 0 { r.prot } else { old })
}

/// An access to `a` faulted: whether its page was a guard page (PAGE_GUARD), which it is no
/// longer (STATUS_GUARD_PAGE_VIOLATION, once; the next access goes through).
pub fn guard_hit(a: u32) -> bool {
    let Some((b, _)) = find(a) else { return false };
    let i = kome::num::us(ga(a).since(ga(b)) / PAGE);
    let mut g = PAGES.locked();
    let Some(x) = g.get_mut(&b).and_then(|p| p.get_mut(i)) else { return false };
    if *x & COMMITTED == 0 || *x & 0x100 == 0 {
        return false;
    }
    *x &= !0x100;
    let prot = *x & !COMMITTED;
    drop(g);
    jit_prot(ga(b).at(kome::num::u32_of(i), PAGE).get(), PAGE, prot);
    true
}

/// A guard page at `a` was hit: if it is inside the 32-bit thread stack of `teb`, the page
/// below becomes the guard page (unless it is the stack's last one) and StackLimit the hit
/// page's base.
pub fn stack_grow(teb: u32, a: u32) -> bool {
    let (dealloc, base) = (r32(ga(teb) + 0xe0c), r32(ga(teb) + 4));
    if dealloc == 0 || !(dealloc..base).contains(&a) {
        return false;
    }
    let page = a & !(PAGE - 1);
    if u64::from(page) >= ga(dealloc).end(PAGE) {
        commit_range((ga(page) - PAGE).get(), PAGE, 0x04 | 0x100);
    }
    if page < r32(ga(teb) + 8) {
        w32(ga(teb) + 8, page);
    }
    true
}

/// Whether `a` is in the lowest page of the thread stack of `teb` (between DeallocationStack
/// and a page above it), which new_teb leaves inaccessible.
pub fn stack_floor(teb: u32, a: u32) -> bool {
    let dealloc = if x64() { r64(ga(teb) + 0x1478) } else { u64::from(r32(ga(teb) + 0xe0c)) };
    dealloc != 0 && (dealloc..dealloc.saturating_add(u64::from(PAGE))).contains(&(u64::from(a)))
}

/// The allocation holding `a`.
pub fn find(a: u32) -> Option<(u32, Region)> {
    let sp = SPACE.locked();
    let (&b, r) = sp.range(..=a).next_back()?;
    (u64::from(a) < ga(b).end(r.size)).then_some((b, *r))
}

pub fn release(base: u32) -> bool {
    // zeroed before it leaves the map: once out, another thread may reserve it (a new thread's
    // stack was zeroed under it)
    let mut sp = SPACE.locked();
    let Some(r) = sp.get(&base).copied() else { return false };
    kura::jit::code_changed(us(base), us(r.size));
    jit_prot(base, r.size, 0);
    zero(base, r.size);
    if r.hi != 0 {
        HIGH.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
    sp.remove(&base);
    PAGES.locked().remove(&base);
    true
}

/// The allocations, for logs (SAKE_TRACE_MEM): base, size, kind; adjacent small ones merged.
pub fn dump() -> String {
    let sp = SPACE.locked();
    let mut out = String::new();
    let pages = PAGES.locked();
    for (&b, r) in sp.iter() {
        // (pages committed, and the end of the last one, in KB from the start)
        let (n, top) = pages.get(&b).map_or((0, 0), |p| {
            (p.iter().filter(|&&x| x & COMMITTED != 0).count(), p.iter().rposition(|&x| x & COMMITTED != 0).map_or(0, |i| i.saturating_add(1)))
        });
        out += &format!("  {b:08x}-{:08x} {:6} KB kind {:x} state {:x}, committed {} KB up to +{} KB\n", ga(b).end(r.size), r.size >> 10, r.kind, r.state, n.saturating_mul(4), top.saturating_mul(4));
    }
    out
}

/// The free range at `a` (for VirtualQuery): (start, size).
pub fn free_at(a: u32) -> (u32, u32) {
    let sp = SPACE.locked();
    let lo = sp.range(..=a).next_back().map_or(0, |(&b, r)| (ga(b) + r.size).get());
    let hi = sp.range(a..).next().map_or(GUEST_TOP, |(&b, _)| b);
    let lo = lo & !(PAGE - 1);
    (lo, hi.saturating_sub(lo))
}

/// sake's own guest code (api::stub): the 1 MiB from HI to GUEST_TOP, where reserve never
/// looks, so it is always there to take (a stub page allocated on first use could fail)
pub const STUB_LO: u32 = HI;
pub const STUB_SIZE: u32 = GUEST_TOP - HI;
const _: () = assert!(STUB_SIZE == 0x10_0000 && STUB_LO % GRAN == 0);
/// The stub area: one committed read-write-execute allocation of its own once first asked for
pub fn stub_area() -> (u32, u32) {
    let mut sp = SPACE.locked();
    if !sp.contains_key(&STUB_LO) {
        insert(&mut sp, STUB_LO, STUB_SIZE, 0x40, 0x20000);
        drop(sp);
        commit(STUB_LO, 0x40);
    }
    (STUB_LO, STUB_SIZE)
}

/// Committed read-write memory anywhere.
pub fn alloc(size: u32) -> Option<u32> {
    let b = reserve(0, size, 4, 0x20000, false)?;
    commit(b, 4);
    Some(b)
}

// ---------------------------------------------------------------------------------------------
// the guest heap (HeapAlloc, GlobalAlloc, LocalAlloc, malloc): dlmalloc over guest regions,
// with an 8-byte header holding the size the caller asked for

struct Sys;

// SAFETY: Sys hands dlmalloc only guest memory it reserved for the heap and never hands the same
// range out twice; dlmalloc is the only user of what it returns.
unsafe impl dlmalloc::Allocator for Sys {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let Some(size) = u32::try_from(size).ok().filter(|&n| n <= HI - GRAN).map(|n| align_up(n, GRAN)) else {
            return (std::ptr::null_mut(), 0, 0);
        };
        match alloc(size) {
            Some(b) => (host::<u8>(b), us(size), 0),
            None => (std::ptr::null_mut(), 0, 0),
        }
    }
    fn remap(&self, _ptr: *mut u8, _old: usize, _new: usize, _can_move: bool) -> *mut u8 {
        std::ptr::null_mut()
    }
    fn free_part(&self, _ptr: *mut u8, _old: usize, _new: usize) -> bool {
        false
    }
    fn free(&self, ptr: *mut u8, _size: usize) -> bool {
        release(kome::num::lo32(ptr.expose_provenance()))
    }
    fn can_release_part(&self, _flags: u32) -> bool {
        false
    }
    fn allocates_zeros(&self) -> bool {
        true
    }
    fn page_size(&self) -> usize {
        kome::num::us(PAGE)
    }
}

static HEAP: Mutex<dlmalloc::Dlmalloc<Sys>> = Mutex::new(dlmalloc::Dlmalloc::new_with_allocator(Sys));

/// (16: x64 code keeps SSE values in heap blocks and expects them 16-byte aligned; the size the
/// caller asked for is at p-8, the magic at p-4)
const HDR: u32 = 16;
const MAGIC: u32 = 0x6e69_7776;
/// a small block whose size class (16-byte steps up to SLAB_MAX) bounds its capacity: freed
/// ones go to fast.rs's per-class lists in guest memory (marked SLAB_FREE while there)
pub const SLAB: u32 = 0x6e69_7777;
pub const SLAB_FREE: u32 = 0x6e69_7778;
pub const SLAB_MAX: u32 = 512;

/// The free lists of small blocks: a spin lock word, then the head of class k (k*16 bytes)
/// at +4k for k in 1..=32. The x86 stubs in fast.rs pop and push them.
pub fn slab_table() -> u32 {
    static T: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *T.get_or_init(|| heap_alloc(4 * (1 + SLAB_MAX / 16), true))
}

fn slab_class(size: u32) -> u32 {
    size.div_ceil(16).max(1)
}

/// A small block's class for good (at p-12, tagged): its size word changes with in-place
/// reallocations, and a 512-byte block shrunk to 8 went back to the 16-byte list when freed,
/// for ever (the big classes kept being refilled from the heap). crt.c and fast.rs read it too.
pub const SLAB_CAP: u32 = 0x5ca9_0000;

/// The class of small block `p`: its tagged word, or (a block from before the word, a state
/// saved earlier) its size's.
fn slab_cap(p: u32) -> u32 {
    let k = r32(ga(p) - 12).wrapping_sub(SLAB_CAP);
    if (1..=SLAB_MAX / 16).contains(&k) { k } else { slab_class(r32(ga(p) - 8)) }
}

/// (tests: blocks made for the class lists)
#[cfg(test)]
pub(crate) static SLAB_MADE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn slab_lock(t: u32) -> &'static std::sync::atomic::AtomicU32 {
    // SAFETY: t is a slab's lock word: 4-aligned guest memory the slab owns for as long as it exists
    // (slabs are never freed), only ever accessed atomically.
    unsafe { std::sync::atomic::AtomicU32::from_ptr(host::<u32>(t)) }
}

/// heap_alloc for HeapAlloc/malloc: small sizes come from the class lists (refilled here when
/// empty, a few blocks at a time), so the x86 fast path can serve the next ones.
pub fn heap_alloc_small(size: u32, zero: bool) -> u32 {
    heap_alloc_in(0, size, zero)
}

/// Every small block ever made (they never go back to dlmalloc): what heap_destroy walks
static SLABS: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// The big blocks of each private heap (HeapCreate), by heap
static BIG: Mutex<Option<std::collections::HashMap<u32, std::collections::HashSet<u32>>>> = Mutex::new(None);

/// heap_alloc_small for HeapAlloc of private heap `heap` (0: no heap of its own, as malloc's
/// and the process heap's): the block carries its heap at p-16 (the x86 stubs and crt.c write
/// it too), so heap_destroy can free what a heap still holds.
pub fn heap_alloc_in(heap: u32, size: u32, zero: bool) -> u32 {
    if size > SLAB_MAX {
        let p = heap_alloc(size, zero);
        if p != 0 && heap != 0 {
            w32(ga(p) - 16, heap);
            BIG.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Default::default).entry(heap).or_default().insert(p);
        }
        return p;
    }
    use std::sync::atomic::Ordering;
    let k = slab_class(size);
    let t = slab_table();
    let lock = slab_lock(t);
    while lock.swap(1, Ordering::Acquire) != 0 {
        std::hint::spin_loop();
    }
    let head = r32(ga(t).at(k, 4));
    let mut made = Vec::new();
    let p = if head != 0 {
        w32(ga(t).at(k, 4), r32(head));
        // (its heap goes with it while the lists are held: heap_destroy walks under the lock)
        w32(ga(head) - 16, heap);
        head
    } else {
        // a new block for the caller and a few for the list
        let n = 8192u32.checked_div(k.saturating_mul(16)).unwrap_or(0).clamp(2, 256);
        let mut first = 0;
        for i in 0..n {
            let b = heap_alloc(k.saturating_mul(16), false);
            if b == 0 {
                break;
            }
            w32(ga(b) - 12, SLAB_CAP | k);
            w32(ga(b) - 16, heap);
            made.push(b);
            #[cfg(test)]
            SLAB_MADE.fetch_add(1, Ordering::Relaxed);
            if i == 0 {
                first = b;
            } else {
                w32(ga(b) - 4, SLAB_FREE);
                w32(b, r32(ga(t).at(k, 4)));
                w32(ga(t).at(k, 4), b);
            }
        }
        first
    };
    lock.store(0, Ordering::Release);
    // (after the lists' lock: heap_destroy takes this one first)
    if !made.is_empty() {
        SLABS.lock().unwrap_or_else(|e| e.into_inner()).extend(made);
    }
    if p == 0 {
        return 0;
    }
    w32(ga(p) - 8, size);
    w32(ga(p) - 4, SLAB);
    if zero {
        fill(p, size, 0);
    }
    p
}

/// heap_free for HeapFree/free: small blocks back to their class list.
pub fn heap_free_small(p: u32) -> bool {
    if p < LO + HDR || r32(ga(p) - 4) != SLAB {
        return heap_free(p);
    }
    use std::sync::atomic::Ordering;
    let k = slab_cap(p);
    let t = slab_table();
    let lock = slab_lock(t);
    while lock.swap(1, Ordering::Acquire) != 0 {
        std::hint::spin_loop();
    }
    w32(ga(p) - 4, SLAB_FREE);
    w32(p, r32(ga(t).at(k, 4)));
    w32(ga(t).at(k, 4), p);
    lock.store(0, Ordering::Release);
    true
}

/// Block `p` (heap_alloc_in's) as one of private heap `heap` (0: none), as a block that moved
/// in HeapReAlloc
pub fn heap_retag(p: u32, heap: u32) {
    use std::sync::atomic::Ordering;
    if !heap_owns(p) {
        return;
    }
    if r32(ga(p) - 4) == SLAB {
        let lock = slab_lock(slab_table());
        while lock.swap(1, Ordering::Acquire) != 0 {
            std::hint::spin_loop();
        }
        w32(ga(p) - 16, heap);
        lock.store(0, Ordering::Release);
        return;
    }
    let mut m = BIG.lock().unwrap_or_else(|e| e.into_inner());
    let m = m.get_or_insert_with(Default::default);
    let old = r32(ga(p) - 16);
    if old != 0 {
        m.get_mut(&old).map(|s| s.remove(&p));
    }
    w32(ga(p) - 16, heap);
    if heap != 0 {
        m.entry(heap).or_default().insert(p);
    }
}

/// HeapDestroy: frees every block heap `heap` still holds (its small ones back to their class
/// lists, its big ones to the heap); the number freed. Programs that make a heap, fill it and
/// destroy it (script VMs, per-scene arenas, CRTs of unloaded DLLs) kept every block.
pub fn heap_destroy(heap: u32) -> usize {
    use std::sync::atomic::Ordering;
    if heap == 0 {
        return 0;
    }
    let mut n: usize = 0;
    let t = slab_table();
    let lock = slab_lock(t);
    let slabs = SLABS.lock().unwrap_or_else(|e| e.into_inner());
    while lock.swap(1, Ordering::Acquire) != 0 {
        std::hint::spin_loop();
    }
    for &b in slabs.iter() {
        if r32(ga(b) - 4) == SLAB && r32(ga(b) - 16) == heap {
            let k = slab_cap(b);
            w32(ga(b) - 4, SLAB_FREE);
            w32(b, r32(ga(t).at(k, 4)));
            w32(ga(t).at(k, 4), b);
            n = n.saturating_add(1);
        }
    }
    lock.store(0, Ordering::Release);
    drop(slabs);
    let big = BIG.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|m| m.remove(&heap)).unwrap_or_default();
    // (freed in address order: the heap's layout after it is the same run to run)
    let mut big: Vec<u32> = big.into_iter().collect();
    big.sort_unstable();
    for p in big {
        if r32(ga(p) - 4) == MAGIC && r32(ga(p) - 16) == heap && heap_free(p) {
            n = n.saturating_add(1);
        }
    }
    n
}

pub fn heap_alloc(size: u32, zero: bool) -> u32 {
    let Some(total) = size.checked_add(HDR) else { return 0 };
    // SAFETY: dlmalloc's malloc has no precondition beyond the size; its memory is Sys's (guest memory).
    let p = unsafe { HEAP.locked().malloc(us(total), 16) }.expose_provenance();
    let p = kome::num::lo32(p);
    if p == 0 {
        return 0;
    }
    let p = ga(p) + HDR;
    w32(p - 16, 0);
    w32(p - 8, size);
    w32(p - 4, MAGIC);
    if zero {
        crate::mem::zero(p.get(), size);
    }
    p.get()
}

pub fn heap_size(p: u32) -> u32 {
    if p == 0 || !heap_owns(p) {
        return u32::MAX;
    }
    r32(ga(p) - 8)
}

/// Whether `p` came from heap_alloc (games free pointers of other heaps through ours rarely,
/// but GlobalFree of a handle and the like do happen).
pub fn heap_owns(p: u32) -> bool {
    p >= LO + HDR && matches!(r32(ga(p) - 4), MAGIC | SLAB)
}

pub fn heap_free(p: u32) -> bool {
    if !heap_owns(p) {
        return false;
    }
    let size = r32(ga(p) - 8);
    if r32(ga(p) - 4) == MAGIC && r32(ga(p) - 16) != 0 {
        let h = r32(ga(p) - 16);
        if let Some(m) = BIG.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            m.get_mut(&h).map(|s| s.remove(&p));
        }
    }
    w32(ga(p) - 4, 0);
    // (c_free: the header's size may be below the chunk's after an in-place shrink)
    let _ = size;
    // SAFETY: p - HDR is what malloc returned in heap_alloc (heap_owns(p) above: a live block).
    unsafe { HEAP.locked().c_free(std::ptr::with_exposed_provenance_mut::<u8>(kome::num::us((ga(p) - HDR).get()))) };
    true
}

/// Whether block `p` can take `size` bytes in place: a small block within its class, or any
/// block shrinking
pub fn heap_fits(p: u32, size: u32) -> bool {
    if p == 0 || !heap_owns(p) {
        return false;
    }
    let old = heap_size(p);
    if r32(ga(p) - 4) == SLAB { size <= SLAB_MAX && slab_class(size) <= slab_cap(p) } else { size <= old }
}

pub fn heap_realloc(p: u32, size: u32, zero: bool) -> u32 {
    if p == 0 {
        return heap_alloc_small(size, zero);
    }
    // (a pointer the heap never gave, or gave and took back: its "size" is whatever lies
    // before it, and a copy of that many bytes could reach anywhere)
    if !heap_owns(p) {
        return 0;
    }
    let old = heap_size(p);
    if heap_fits(p, size) {
        if zero && size > old {
            crate::mem::zero((ga(p) + old).get(), size.saturating_sub(old));
        }
        w32(ga(p) - 8, size);
        return p;
    }
    let n = heap_alloc_small(size, false);
    if n == 0 {
        return 0;
    }
    copy(n, p, old.min(size));
    if zero && size > old {
        crate::mem::zero((ga(n) + old).get(), size.saturating_sub(old));
    }
    heap_free_small(p);
    n
}

/// A locked read-modify-write of the dword at `a` (the Interlocked* functions): returns the old
/// value and stores `f(old)`. x86 allows the dword unaligned (mono's JIT keeps counters in
/// packed structures) but wasm atomics fault on it: an unaligned one inside an aligned qword
/// swaps that qword, one that straddles two takes a lock (atomic against the others that do).
/// The guest's word at `a` changed by `f` atomically, its old value; 0 (and nothing changed)
/// where that is not guest memory (Windows would fault)
pub fn atomic_rmw32(a: u32, f: impl Fn(u32) -> u32) -> u32 {
    for_write(a, 4).map_or(0, |r| rmw32_at(us(r.addr()), f))
}
// (fetch_update is try_update on newer toolchains, which the one pinned here lacks)
#[allow(deprecated)]
pub(crate) fn rmw32_at(p: usize, f: impl Fn(u32) -> u32) -> u32 {
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::SeqCst};
    if p % 4 == 0 {
        // SAFETY: p is 4-aligned host memory the caller names (guest memory: backed), used as
        // an atomic word by every writer here
        let a = unsafe { &*std::ptr::with_exposed_provenance::<AtomicU32>(p) };
        return a.fetch_update(SeqCst, SeqCst, |v| Some(f(v))).unwrap_or_else(|v| v);
    }
    let word = p & !7;
    // (word is p with its low 3 bits cleared: p - word is 0..=7)
    let into = p.wrapping_sub(word);
    if into <= 4 {
        // SAFETY: the 8-aligned word holding all 4 bytes, the same memory, atomic as above
        let w = unsafe { &*std::ptr::with_exposed_provenance::<AtomicU64>(word) };
        let sh = kome::num::lo32(into).wrapping_mul(8);
        let m = 0xffff_ffffu64 << sh;
        let mut old = 0;
        // (the closure always answers Some: fetch_update cannot fail)
        let _prev: u64 = w
            .fetch_update(SeqCst, SeqCst, |cur| {
                old = kome::num::lo32((cur & m) >> sh);
                Some((cur & !m) | (u64::from(f(old)) << sh))
            })
            .unwrap_or_else(|v| v);
        return old;
    }
    static SPLIT: Mutex<()> = Mutex::new(());
    let _g = SPLIT.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: 4 bytes of the caller's memory across two words, unaligned access; SPLIT orders
    // the writers that straddle
    let old = unsafe { std::ptr::with_exposed_provenance::<u32>(p).read_unaligned() };
    // SAFETY: as above
    unsafe { std::ptr::with_exposed_provenance_mut::<u32>(p).write_unaligned(f(old)) };
    old
}

#[cfg(test)]
mod tests {
    use super::{over_reserved_block, rmw32_at, GUEST_TOP};

    /// Review A-13: guest sizes near 0xFFFFFFFF in VirtualProtect, VirtualFree(MEM_DECOMMIT) and
    /// commits: the pages run to the allocation's end (the u32 sum wrapped, `p[first..last]`
    /// panicked with last < first, `(last - first) * PAGE` underflowed into the JIT tables).
    #[test]
    fn huge_sizes_stop_at_the_allocation() {
        use super::{commit_range, decommit_range, protect, query, release, reserve, PAGE};
        let _g = crate::test_guest::enter();
        let b = reserve(0, 16 * PAGE, 4, 0x2_0000, false).unwrap();
        assert!(commit_range(b, 16 * PAGE, 4));
        for n in [u32::MAX, u32::MAX - 0xfff, u32::MAX - 0x2fff, 0xffff_f001, 0x8000_0000] {
            assert_eq!(protect(b + 3 * PAGE, n, 2), Some(4), "{n:#x}");
            let q = query(b + 15 * PAGE).unwrap();
            assert_eq!(q.5, 2, "{n:#x}: the last page read-only");
            assert_eq!(query(b + 2 * PAGE).unwrap().5, 4, "{n:#x}: pages before untouched");
            assert!(commit_range(b + 3 * PAGE, n, 4), "{n:#x}");
            assert_eq!(query(b + 15 * PAGE).unwrap().5, 4);
        }
        assert!(decommit_range(b + 5 * PAGE, u32::MAX - 0x3fff));
        assert_eq!(query(b + 15 * PAGE).unwrap().4, 0x2000, "decommitted to the end");
        assert_eq!(query(b + 4 * PAGE).unwrap().4, 0x1000, "the page before stays");
        assert!(release(b));
    }

    proptest::proptest! {
        /// page_span for any offset inside, any size and any allocation: first <= last <= the
        /// allocation's pages; the pages are the ones [off, off + n) touches when it fits.
        #[test]
        fn page_span_any(pages in 1u32..0x10_0000, off_page in 0u32..0x10_0000, within in 0u32..0x1000, n: u32) {
            use super::{page_span, PAGE};
            let size = pages * PAGE;
            let off = (off_page % pages) * PAGE + within;
            let (first, last) = page_span(off, n, size);
            proptest::prop_assert!(first <= last && last <= pages);
            proptest::prop_assert_eq!(first, off / PAGE);
            if kome::num::lo64(off) + kome::num::lo64(n) <= kome::num::lo64(size) {
                proptest::prop_assert_eq!(kome::num::lo64(last), (kome::num::lo64(off) + kome::num::lo64(n)).div_ceil(kome::num::lo64(PAGE)));
            } else {
                proptest::prop_assert_eq!(last, pages);
            }
        }
    }

    /// review 2026-11-14: sake wrote program-given (pointer, length) pairs through an unchecked
    /// slice: into the NULL pages after a failed allocation, or past 2 GB into its own heap
    #[test]
    fn view_mut_refuses_what_is_not_the_guests() {
        use super::{range, update, NULL_TOP};
        assert!(update(0, 0, |s| s.is_empty()) == Some(true));
        assert!(update(0, 16, |_| ()).is_none());
        assert!(update(NULL_TOP - 1, 1, |_| ()).is_none());
        assert!(update(GUEST_TOP, 1, |_| ()).is_none());
        assert!(update(GUEST_TOP - 4, 5, |_| ()).is_none());
        assert!(update(0x40_0000, u32::MAX, |_| ()).is_none());
        assert!(update(0xffff_fff0, 0x20, |_| ()).is_none());
        assert!(range(NULL_TOP, 1).is_some() && range(GUEST_TOP - 4, 4).is_some() && range(0x40_0000, GUEST_TOP - 0x40_0000).is_some());
    }

    proptest::proptest! {
        /// guest_range: empty, or wholly in [64 KiB, 2 GB), no wrapping
        #[test]
        fn guest_range_is_the_guests_half(a in proptest::prelude::any::<u32>(), n in proptest::prelude::any::<u32>()) {
            use super::{range, NULL_TOP};
            let want = n == 0 || (a >= NULL_TOP && (kome::num::lo64(a) + kome::num::lo64(n)) <= kome::num::lo64(GUEST_TOP));
            proptest::prop_assert_eq!(range(a, n).is_some(), want);
            if range(a, n).is_some() && n != 0 {
                proptest::prop_assert!(a.checked_add(n).is_some_and(|e| e <= GUEST_TOP));
            }
        }
    }

    /// Blocks standing for addresses above 4G (#62), as VirtualAlloc and the loader make them:
    /// reserved at the low half, `set_high`; every address inside widens to the high one,
    /// those outside don't; a request whose low half meets a block already there is refused
    /// (koji addresses memory modulo 4G). The bookkeeping only (no guest memory: native test),
    /// in 0x5000_0000..0x7000_0000, where no other test reserves.
    mod high {
        use super::super::{find, high_of, reserve, set_high, widen, HIGH, SPACE};
        use proptest::prelude::*;
        use std::sync::atomic::Ordering::Relaxed;

        fn forget(base: u32) {
            let r = SPACE.lock().unwrap().remove(&base).unwrap();
            if r.hi != 0 {
                HIGH.fetch_sub(1, Relaxed);
            }
            super::super::PAGES.lock().unwrap().remove(&base);
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(300))]
            #[test]
            fn widen_restores_the_high_half(reqs in proptest::collection::vec((1u32..0x7ffe, 0x5000u32..0x6f00, 1u32..0x40), 1..12), probes in proptest::collection::vec(0x5000_0000u32..0x7000_0000, 40)) {
                static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
                let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
                let before = HIGH.load(Relaxed);
                let mut got: Vec<(u32, u32, u32)> = Vec::new();
                for &(hi, lo64k, pages) in &reqs {
                    let (lo, size) = (lo64k << 16, pages * 0x1000);
                    let meets = got.iter().any(|&(b, n, _)| lo < b + n && b < lo + size);
                    let r = reserve(lo, size, 4, 0x2_0000, false);
                    prop_assert_eq!(r.is_none(), meets, "{:#x}+{:#x}", lo, size);
                    if let Some(b) = r {
                        prop_assert_eq!(b, lo);
                        prop_assert!(set_high(b, hi));
                        got.push((b, size, hi));
                    }
                }
                prop_assert_eq!(HIGH.load(Relaxed), before + kome::num::lo32(got.len()));
                for a in probes.iter().copied().chain(got.iter().flat_map(|&(b, n, _)| [b, b + n - 1, b + n])) {
                    let want = got.iter().find(|&&(b, n, _)| a.wrapping_sub(b) < n).map_or(0, |g| g.2);
                    prop_assert_eq!(widen(a), kome::num::lo64(a) | (kome::num::lo64(want)) << 32, "{:#x}", a);
                    prop_assert_eq!(high_of(a), want);
                    prop_assert_eq!(find(a).is_some(), got.iter().any(|&(b, n, _)| a.wrapping_sub(b) < n));
                }
                // made plain again, then gone: no high block left
                if let Some(&(b, _, _)) = got.first() {
                    prop_assert!(set_high(b, 0));
                    prop_assert_eq!(widen(b), kome::num::lo64(b));
                }
                for &(b, _, _) in &got {
                    forget(b);
                }
                prop_assert_eq!(HIGH.load(Relaxed), before);
            }
        }
    }

    /// sake's data starts where the JIT's direct guest range ends (build.sh's GLOBAL_BASE too)
    #[test]
    fn guest_top_is_the_jits() {
        assert_eq!(GUEST_TOP, koji::jit::GUEST_HI);
        let build = include_str!("../build.sh");
        assert!(build.contains(&format!("-sGLOBAL_BASE={}", GUEST_TOP)), "build.sh GLOBAL_BASE");
    }

    /// An aligned block is all there or not given: the lowest aligned address whose whole
    /// block lies in one hole (a smaller block at the top of the space once ran Unity 6 over
    /// the TEBs above it)
    mod aligned {
        use super::super::aligned_fit;
        use proptest::prelude::*;

        #[test]
        fn top_of_a_full_space() {
            // (お仕置きLesson: 30 regions of 64 MB taken, 28 MB left under the TEBs)
            let holes = [(0x7c00_0000, 0x7dc1_0000)];
            assert_eq!(aligned_fit(&holes, 0x400_0000, 0x400_0000), None);
            assert_eq!(aligned_fit(&holes, 0x100_0000, 0x100_0000), Some(0x7c00_0000));
            assert_eq!(aligned_fit(&[(0x3_0000, 0x900_0000)], 0x400_0000, 0x400_0000), Some(0x400_0000));
        }

        proptest! {
            #[test]
            fn whole_lowest_aligned(mut cuts in prop::collection::vec(0x3u32..0x7ff0, 0..24), size_pages in 1u32..0x8000, shift in 16u32..28) {
                // holes from sorted cut points (in 64 KB units), every other gap a hole
                cuts.sort();
                cuts.dedup();
                let holes: Vec<(u32, u32)> = cuts.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0] << 16, c[1] << 16)).collect();
                let (size, align) = (size_pages << 12, 1u32 << shift);
                let got = aligned_fit(&holes, size, align);
                let fits = |b: u64| holes.iter().any(|&(lo, hi)| b >= kome::num::lo64(lo) && b + kome::num::lo64(size) <= kome::num::lo64(hi));
                match got {
                    Some(b) => {
                        prop_assert_eq!(b % align, 0);
                        prop_assert!(fits(kome::num::lo64(b)));
                        // nothing lower
                        let mut x = 0u64;
                        while x < kome::num::lo64(b) {
                            prop_assert!(!fits(x));
                            x += kome::num::lo64(align);
                        }
                    }
                    None => {
                        let mut x = 0u64;
                        while x < 0x8000_0000 {
                            prop_assert!(!fits(x));
                            x += kome::num::lo64(align);
                        }
                    }
                }
            }
        }
    }

    /// mem::zero: the range ends up zero and nothing outside it changes, whatever was there
    /// (pages that were zero, pages with one byte set anywhere, ranges cut at any byte); and it
    /// leaves pages that were zero unwritten, so that they never become RAM (the RSS tests).
    mod zeroing {
        use super::super::{is_zero, release, reserve, commit, decommit_range, heap_alloc, heap_free, bytes, update, ga, w8, zero, PAGE};
        use proptest::prelude::*;

        #[test]
        fn is_zero_finds_any_byte() {
            for len in [0usize, 1, 255, 256, 257, 4095, 4096, 8191] {
                let mut v = vec![0u8; len];
                assert!(is_zero(&v));
                for i in 0..len {
                    v[i] = 1;
                    assert!(!is_zero(&v), "len {len} byte {i}");
                    v[i] = 0;
                }
            }
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(200))]
            #[test]
            fn zero_is_fill(start in 0u32..3 * PAGE, len in 0u32..6 * PAGE, dirty in prop::collection::vec((0u32..9 * PAGE, 1u8..=255), 0..40)) {
                let _g = crate::test_guest::enter();
                let b = reserve(0, 10 * PAGE, 4, 0x20000, false).unwrap();
                commit(b, 4);
                for &(at, v) in &dirty {
                    w8(ga(b) + at, v);
                }
                let before = bytes(b, 10 * PAGE);
                zero(b + start, len);
                let after = bytes(b, 10 * PAGE);
                for i in 0..10 * PAGE {
                    let inside = i >= start && i < start + len;
                    prop_assert_eq!(after[kome::num::us(i)], if inside { 0 } else { before[kome::num::us(i)] }, "byte {:#x}", i);
                }
                release(b);
            }
        }

        /// This process's resident memory (smaps Rss of the guest range: the kernel's zero page,
        /// which reads of never-written pages map, is not counted), KiB
        fn guest_rss_kib() -> u64 {
            let (lo, hi) = (kome::num::lo64(super::super::LO), kome::num::lo64(super::super::HI));
            let mut rss = 0;
            let mut inside = false;
            for l in std::fs::read_to_string("/proc/self/smaps").unwrap().lines() {
                if let Some((range, _)) = l.split_once(' ') && let Some((a, b)) = range.split_once('-') && let (Ok(a), Ok(b)) = (u64::from_str_radix(a, 16), u64::from_str_radix(b, 16)) {
                    inside = a < hi && b > lo;
                    continue;
                }
                if inside && let Some(v) = l.strip_prefix("Rss:") {
                    rss += v.trim().trim_end_matches(" kB").trim().parse::<u64>().unwrap();
                }
            }
            rss
        }

        /// Releasing a big reservation the program used one page of, decommitting pages it
        /// committed and never wrote, a zeroed heap block it never writes: none of it becomes
        /// RAM (each used to write zeros over all of it: 64 MB of RAM for a page used).
        #[test]
        fn unused_pages_stay_out_of_ram() {
            let _g = crate::test_guest::enter();
            const BIG: u32 = 64 << 20;
            let slack = |d: u64| assert!(d < 1024, "{d} KiB became resident");
            // reserve, commit all, write one page, release
            let before = guest_rss_kib();
            let b = reserve(0, BIG, 4, 0x20000, false).unwrap();
            commit(b, 4);
            update(b + 0x123_4000, 16, |s| s.fill(7)).unwrap();
            release(b);
            slack(guest_rss_kib().saturating_sub(before));
            // the page written is zero again for whoever gets the block next
            let b2 = reserve(b, BIG, 4, 0x20000, false).unwrap();
            assert!(is_zero(&bytes(b2 + 0x123_4000, PAGE)));
            // commit and decommit without writing
            commit(b2, 4);
            decommit_range(b2, 0);
            release(b2);
            slack(guest_rss_kib().saturating_sub(before));
            // HEAP_ZERO_MEMORY / calloc of fresh memory
            let h = heap_alloc(BIG / 2, true);
            assert_ne!(h, 0);
            assert!(is_zero(&bytes(h, PAGE)) && is_zero(&bytes(h + BIG / 2 - PAGE, PAGE)));
            slack(guest_rss_kib().saturating_sub(before));
            heap_free(h);
            slack(guest_rss_kib().saturating_sub(before));
        }
    }

    /// A commit is inside one allocation or refused: not cut at its end
    #[test]
    fn commit_stays_in_one_allocation() {
        use super::{in_one_allocation, reserve, PAGES, SPACE};
        // (bookkeeping only, then forgotten: other tests own 0x5000_0000..0x7000_0000)
        let b = reserve(0, 0x2_0000, 4, 0x20000, false).unwrap();
        assert!(in_one_allocation(b, 0x2_0000));
        assert!(in_one_allocation(b + 0x1_f000, 0x1000));
        assert!(in_one_allocation(b + 0x1_fffc, 4));
        assert!(!in_one_allocation(b + 0x1_f000, 0x1001));
        assert!(!in_one_allocation(b + 0x1_fffc, 5));
        assert!(!in_one_allocation(b + 0x2_0000, 1));
        assert!(!in_one_allocation(b - 1, 2));
        SPACE.lock().unwrap().remove(&b);
        PAGES.lock().unwrap().remove(&b);
    }

    #[test]
    fn over_reserve_idiom() {
        assert_eq!(over_reserved_block(0x1fff_f000), Some(0x1000_0000));
        assert_eq!(over_reserved_block(0x01ff_f000), Some(0x0100_0000));
        // a plain power of two, small blocks and other sizes: not the idiom
        assert_eq!(over_reserved_block(0x2000_0000), None);
        assert_eq!(over_reserved_block(0x00ff_f000), None);
        assert_eq!(over_reserved_block(0x1fff_e000), None);
        assert_eq!(over_reserved_block(0xffff_f000), None); // (4 GB)
        assert_eq!(over_reserved_block(u32::MAX), None);
    }

    /// every offset in a 16-byte buffer: the dword changes, its neighbours don't
    #[test]
    fn rmw32_any_alignment() {
        #[repr(align(8))]
        struct B([u8; 16]);
        for off in 0..=12usize {
            let mut b = B([0xaa; 16]);
            b.0[off..off + 4].copy_from_slice(&0x1122_3344u32.to_le_bytes());
            let p = b.0.as_mut_ptr().expose_provenance() + off;
            assert_eq!(rmw32_at(p, |v| v.wrapping_add(0x0101_0101)), 0x1122_3344);
            assert_eq!(u32::from_le_bytes(b.0[off..off + 4].try_into().unwrap()), 0x1223_3445, "offset {off}");
            assert!(b.0.iter().take(off).chain(&b.0[off + 4..]).all(|&x| x == 0xaa), "offset {off}");
        }
    }

    /// increments from several threads at an offset inside a qword and one across two: none lost
    #[test]
    fn rmw32_unaligned_threads() {
        #[repr(align(8))]
        struct B([u8; 24]);
        let mut b = Box::new(B([0; 24]));
        let base = b.0.as_mut_ptr().expose_provenance();
        for off in [1usize, 2, 6] {
            let p = base + off;
            let hs: Vec<_> = (0..4).map(|_| std::thread::spawn(move || (0..20000).for_each(|_| { rmw32_at(p, |v| v + 1); }))).collect();
            hs.into_iter().for_each(|h| h.join().unwrap());
            assert_eq!(unsafe { std::ptr::with_exposed_provenance::<u32>(p).read_unaligned() }, 80000, "offset {off}");
            unsafe { std::ptr::with_exposed_provenance_mut::<u32>(p).write_unaligned(0) };
        }
    }
}

/// Review b07: HeapDestroy frees what a private heap still holds, and only that.
#[cfg(test)]
mod heap_destroy_tests {
    use super::*;
    use proptest::prelude::*;

    fn live(p: u32) -> bool {
        matches!(r32(ga(p) - 4), MAGIC | SLAB)
    }

    #[test]
    fn destroy_frees_small_and_big_blocks_of_its_heap_only() {
        let _g = crate::test_guest::enter();
        let (a, b) = (0x7a_0000, 0x7b_0000);
        let mine: Vec<u32> = (0..300).map(|i| heap_alloc_in(a, [8, 100, 512, 513, 70_000][i % 5], false)).collect();
        let other: Vec<u32> = (0..50).map(|i| heap_alloc_in(b, [16, 4000][i % 2], false)).collect();
        let plain: Vec<u32> = (0..50).map(|i| heap_alloc_small([24, 9000][i % 2], false)).collect();
        for (i, &p) in other.iter().chain(&plain).enumerate() {
            w32(p, kome::num::u32_of(i) ^ 0x5a5a_0000);
        }
        assert_eq!(heap_destroy(a), mine.len());
        assert!(mine.iter().all(|&p| !live(p)), "every block of the heap went");
        for (i, &p) in other.iter().chain(&plain).enumerate() {
            assert!(live(p), "{p:#x} of another heap");
            assert_eq!(r32(p), kome::num::u32_of(i) ^ 0x5a5a_0000, "{p:#x} kept its bytes");
        }
        // (again: nothing left)
        assert_eq!(heap_destroy(a), 0);
        assert_eq!(heap_destroy(b), other.len());
        for p in plain {
            assert!(heap_free_small(p));
        }
    }

    #[test]
    fn a_block_that_moves_stays_its_heaps() {
        let _g = crate::test_guest::enter();
        let h = 0x7c_0000;
        let p = heap_alloc_in(h, 40, false);
        let q = heap_realloc(p, 5000, false);
        heap_retag(q, h);
        let r = heap_realloc(q, 100, false);
        assert_eq!(r, q, "shrinks in place");
        let s = heap_alloc_in(h, 3000, false);
        let t = heap_realloc(s, 200, false);
        heap_retag(t, h);
        assert_eq!(heap_destroy(h), 2);
        assert!(!live(q) && !live(t));
    }

    #[test]
    fn bad_pointers_have_no_size_and_do_not_move() {
        let _g = crate::test_guest::enter();
        let p = heap_alloc(64, true);
        assert!(heap_free(p));
        assert_eq!(heap_size(p), u32::MAX, "a freed block");
        assert_eq!(heap_realloc(p, 10, false), 0);
        assert!(!heap_fits(p, 10));
        let q = heap_alloc(64, true);
        assert_eq!(heap_size(q + 16), u32::MAX, "inside a block");
        assert_eq!(heap_realloc(q + 16, 1000, false), 0);
        assert_eq!(heap_size(4), u32::MAX);
        assert!(heap_free(q));
    }

    #[derive(Debug, Clone)]
    enum Op {
        Alloc(u8, u32),
        Free(usize),
        Realloc(usize, u32),
        Destroy(u8),
    }

    fn op() -> impl Strategy<Value = Op> {
        let size = prop_oneof![4u32..=512, 513u32..20_000];
        prop_oneof![
            4 => (0u8..4, size.clone()).prop_map(|(h, n)| Op::Alloc(h, n)),
            2 => any::<usize>().prop_map(Op::Free),
            2 => (any::<usize>(), size).prop_map(|(i, n)| Op::Realloc(i, n)),
            1 => (1u8..4).prop_map(Op::Destroy),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
        /// Model: each live block has a heap (0: malloc's) and a stamp in its first word; a
        /// destroy frees exactly the live blocks of that heap, every other block keeps its
        /// stamp, and none is handed out twice.
        #[test]
        fn heaps_against_a_model(ops in proptest::collection::vec(op(), 1..120)) {
            let _g = crate::test_guest::enter();
            let heap = |h: u8| if h == 0 { 0 } else { 0x90_0000 + kome::num::lo32(h) * 0x1_0000 };
            // (what a failed case before this one left)
            for h in 1..4 {
                heap_destroy(heap(h));
            }
            let mut blocks: Vec<(u32, u8, u32)> = vec![];
            let mut stamp = 1u32;
            for o in ops {
                match o {
                    Op::Alloc(h, n) => {
                        let p = heap_alloc_in(heap(h), n, false);
                        prop_assert!(p != 0);
                        prop_assert!(blocks.iter().all(|b| b.0 != p), "handed out twice");
                        w32(p, stamp);
                        blocks.push((p, h, stamp));
                        stamp += 1;
                    }
                    Op::Free(i) if !blocks.is_empty() => {
                        let (p, ..) = blocks.swap_remove(i % blocks.len());
                        prop_assert!(heap_free_small(p));
                    }
                    Op::Realloc(i, n) if !blocks.is_empty() => {
                        let i = i % blocks.len();
                        let (p, h, s) = blocks[i];
                        let q = heap_realloc(p, n, false);
                        prop_assert!(q != 0);
                        heap_retag(q, heap(h));
                        prop_assert_eq!(r32(q), s);
                        blocks[i].0 = q;
                    }
                    Op::Destroy(h) => {
                        let n = blocks.iter().filter(|b| b.1 == h).count();
                        prop_assert_eq!(heap_destroy(heap(h)), n);
                        for b in blocks.iter().filter(|b| b.1 == h) {
                            prop_assert!(!live(b.0));
                        }
                        blocks.retain(|b| b.1 != h);
                    }
                    Op::Free(_) | Op::Realloc(..) => {}
                }
                for b in &blocks {
                    prop_assert!(live(b.0));
                    prop_assert_eq!(r32(b.0), b.2);
                }
            }
            for h in 1..4 {
                let n = blocks.iter().filter(|b| b.1 == h).count();
                prop_assert_eq!(heap_destroy(heap(h)), n);
            }
            for b in blocks.iter().filter(|b| b.1 == 0) {
                prop_assert!(heap_free_small(b.0));
            }
        }
    }
}

#[cfg(test)]
mod review_b07_tests {
    use super::*;
    use proptest::prelude::*;

    /// The byte-at-a-time readers the bounded scans replaced
    fn cstr_ref(a: u32) -> Vec<u8> {
        let mut v = Vec::new();
        let mut p = a;
        while r8(p) != 0 {
            v.push(r8(p));
            p += 1;
        }
        v
    }
    fn wstr_ref(a: u32) -> Vec<u16> {
        let mut v = Vec::new();
        let mut p = a;
        while r16(p) != 0 {
            v.push(r16(p));
            p += 2;
        }
        v
    }

    #[test]
    fn sizes_past_the_address_space_are_refused() {
        let _g = crate::test_guest::enter();
        for n in [0xffff_f001u32, 0xffff_ffff, HI + 1, GUEST_TOP] {
            assert_eq!(reserve(0, n, 4, 0x20000, false), None, "{n:#x}");
            assert_eq!(reserve(0, n, 4, 0x20000, true), None, "{n:#x}");
            assert_eq!(reserve_aligned(n, GRAN, 4, 0x20000), None, "{n:#x}");
            assert_eq!(alloc(n), None, "{n:#x}");
        }
        assert_eq!(heap_alloc(0xffff_fff0, false), 0);
    }

    proptest! {
        /// Pages: at least the size, less than a page more, None only past HI
        #[test]
        fn page_sizes_round_up_without_wrapping(size: u32) {
            match page_size(size) {
                Some(n) => {
                    prop_assert!(n % PAGE == 0 && n >= size.max(1) && n - size.max(1) < PAGE && n <= HI);
                }
                None => prop_assert!(kome::num::lo64(size) > kome::num::lo64(HI) - kome::num::lo64(PAGE)),
            }
        }

        /// The bounded scans read what the byte/unit loops read, at any alignment
        #[test]
        fn strings_read_as_the_plain_loops_do(bytes in proptest::collection::vec(any::<u8>(), 0..300), at in 0u32..8, nul in 0usize..320) {
            let _g = crate::test_guest::enter();
            let b = heap_alloc(400, true);
            prop_assert!(b != 0);
            fill(b, 400, 0);
            let mut v = bytes.clone();
            if nul < v.len() {
                v[nul] = 0;
            }
            for (i, &c) in v.iter().enumerate() {
                w8(b + at + kome::num::u32_of(i), c);
            }
            prop_assert_eq!(cstr(b + at), cstr_ref(b + at));
            prop_assert_eq!(wstr(b + at), wstr_ref(b + at));
            heap_free(b);
        }
    }
}

#[cfg(test)]
#[path = "mem_conversion_tests.rs"]
mod conversion_tests;
