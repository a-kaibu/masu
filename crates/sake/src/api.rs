//! The import boundary: every Win32 function the game imports is a trap address. A call
//! there leaves the translated code; `dispatch` runs the Rust implementation with the
//! arguments still on the guest stack and returns to the caller as stdcall/cdecl would.

use kome::locks::{Lock as _, Rw as _};
use crate::{HashMap, HashSet};
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use koji::{Cpu, EAX, EDX, ESP};

use crate::mem::{r32, w32};

/// Trap range of the x86 core: [TRAP_LO, TRAP_HI).
pub const TRAP_LO: u32 = 0xfe00_0000;
/// The address of trap `i` (4 bytes each from TRAP_LO)
pub fn trap_at(i: u32) -> u32 {
    crate::mem::ga(TRAP_LO).at(i, 4).get()
}
/// The trap number of trap address `a` (trap_at's inverse; below TRAP_LO wraps far past any)
pub fn trap_no(a: u32) -> u32 {
    crate::mem::ga(a).since(crate::mem::ga(TRAP_LO)) / 4
}
pub const TRAP_HI: u32 = 0xffff_1000;
/// Return address of calls from Rust into guest code.
pub const RET_TRAP: u32 = 0xffff_0000;
/// Where the guest's exception dispatch loop reports a handler's result (see seh.rs).
pub const SEH_TRAP: u32 = 0xffff_0010;
/// Threads end by returning here.
pub const EXIT_TRAP: u32 = 0xffff_0020;
/// x64: where a language-specific exception handler returns to the dispatcher (seh64.rs).
pub const SEH64_TRAP: u32 = 0xffff_0030;

pub type Handler = fn(&mut Ctx) -> u32;
/// `f` as a Handler (a fn item coerced: the first of a list of them gives the list its type)
pub const fn handler(f: Handler) -> Handler {
    f
}

pub struct Ctx<'a> {
    pub cpu: &'a mut Cpu,
    /// ESP at the call (the return address)
    pub esp: u32,
    pub edx: Option<u32>,
    /// the handler has set EIP/ESP itself (no return to the caller)
    pub jumped: bool,
    /// a double for ST0 (functions returning floating point)
    pub st0: Option<f64>,
    /// the trap address called (handlers shared by many entries tell them apart by it)
    pub trap: u32,
    /// x86: a thiscall function whose `this` (ECX) is argument 0, the stack arguments 1...
    /// (as on x64, where `this` is the first argument): msvcp140 / msvcp120 and their vtables
    pub this: Option<u32>,
    /// the called function's pointer-sized out-parameters (outs::mask), as this program's
    /// architecture numbers its arguments: `a(i)` of one is a handler's mistake (Args makes it
    /// a compile error where the handler is the function's)
    pub outs: u64,
}

/// A function's pointer-sized out-parameters by argument index (bit i): x64's arguments, and
/// x86's stack dwords, which come after a GUID's four or a double's two (Ctx::a's `i` on each).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OutMask {
    pub x64: u64,
    pub x86: u64,
}

impl OutMask {
    pub const NONE: OutMask = OutMask { x64: 0, x86: 0 };
    /// The one this program's arguments are numbered by
    #[inline]
    pub fn of_arch(self, x64: bool) -> u64 {
        if x64 { self.x64 } else { self.x86 }
    }
}

/// A pointer-sized out-parameter (Win32 metadata: a pointer, a handle, an interface, a SIZE_T, an
/// SRWLOCK...: 8 bytes on x64), as `c.out(i)` gives it. It is not an address the 32-bit writers
/// take (it is no u32, no Ga): what it can write is whole, so a handler cannot leave the upper
/// half on x64 as the caller had it (review 2026-11-15: outptr_audit searched for `w32(c.a(i)`,
/// which a local variable got past; now it checks that such an argument is only ever c.out(i)).
/// ```compile_fail
/// fn f(c: &mut sake::api::Ctx) { sake::mem::w32(c.out(1), 0) }
/// ```
/// ```compile_fail
/// let _ = sake::api::Out::at(0x1000); // only sake itself names a slot of its own
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Out(u32);

impl Out {
    /// The pointer-sized slot at `a` that the caller found itself (inside a structure of its own:
    /// a std::mutex's SRWLOCK, a local the result goes to). Handlers take theirs from `c.out(i)`.
    #[inline]
    pub(crate) fn at(a: u32) -> Out {
        Out(a)
    }
    /// `v` as a pointer-sized value (mem::putp: NULL skipped; small negatives sign-extended)
    #[inline]
    pub fn put(self, v: u32) {
        crate::mem::putp(self.0, v)
    }
    /// Guest address `v` as a pointer (mem::put_addr: x64 widens it)
    #[inline]
    pub fn put_addr(self, v: u32) {
        crate::mem::put_addr(self.0, v)
    }
    /// A pointer-sized integer (ULONG_PTR, SIZE_T, a widened address): all 8 bytes on x64, the
    /// low 4 on x86, where the slot is 4 bytes (review 2026-11-16: put64 wrote 8 bytes into a
    /// 4-byte slot of a 32-bit program); NULL skipped.
    #[inline]
    pub fn put_size(self, v: u64) {
        if crate::mem::x64() {
            crate::mem::put64(self.0, v)
        } else {
            // (the whole x86 slot: putp, not put32, which kura::outs asks)
            crate::mem::putp(self.0, kome::num::lo32(v))
        }
    }
    /// What it holds now (mem::rp: its low half on x64)
    #[inline]
    pub fn get(self) -> u32 {
        crate::mem::rp(self.0)
    }
    #[inline]
    pub fn is_null(self) -> bool {
        self.0 == 0
    }
    /// Whether the slot cannot be written whole (reserved, decommitted, read-only memory)
    pub fn is_unwritable(self) -> bool {
        let n = crate::mem::ps();
        crate::mem::accessible(self.0, n, true, false) < n
    }
    // (no address of it as a plain u32, which a 32-bit writer would take: review 2026-11-16,
    // Out::addr was one. What it is used for instead, each by its name:)
    /// An SRWLOCK, INIT_ONCE or CONDITION_VARIABLE: the word sake's waits work on atomically
    /// (its low half on x64, where the rest stays 0 as InitializeSRWLock wrote it).
    #[inline]
    pub fn lock_word(self) -> LockWord {
        LockWord(self.0)
    }
    /// What the caller passed, where the metadata's out-parameter is an object sake made itself
    /// (a GpMatrix*, a UBreakIterator*, a GLUquadric*, an XAPO): the object, read, not written.
    #[inline]
    pub fn object(self) -> Object {
        Object(self.0)
    }
    /// Slot `i` of an array of pointer-sized slots starting here.
    #[inline]
    pub fn nth(self, i: u32) -> Out {
        Out(crate::mem::ga(self.0).at(i, crate::mem::ps()).get())
    }
}

/// The lock word an [`Out`] names (SRWLOCK, INIT_ONCE, CONDITION_VARIABLE): no guest address
/// (it is no `Into<Ga>`): only sake's waits take it, as an atomic word or the key their waiters
/// park on (review 2026-11-17: `lock_word()` was a u32, and `mem::w32(out.lock_word(), v)`
/// compiled).
/// ```compile_fail
/// fn f(c: &mut sake::api::Ctx) { sake::mem::w32(c.out(0).lock_word(), 0) }
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(transparent)]
pub struct LockWord(u32);

impl LockWord {
    /// The word, atomic (k32's rule: an unaligned one has a shadow of its own)
    #[inline]
    pub(crate) fn atomic(self) -> &'static std::sync::atomic::AtomicU32 {
        crate::k32::lock_atomic(self.0)
    }
    /// The key its waiters park on in sake's waits (an address only as a name)
    #[inline]
    pub(crate) fn key(self) -> u32 {
        self.0
    }
    /// Its slot again, for a call that takes the lock itself (the callback InitOnceExecuteOnce
    /// runs, a reacquire after a save state)
    #[inline]
    pub(crate) fn slot(self) -> Out {
        Out(self.0)
    }
    /// The word a save state's reacquire record keeps, and back
    #[inline]
    pub(crate) fn saved(self) -> u64 {
        u64::from(self.0)
    }
    #[inline]
    pub(crate) fn from_saved(v: u64) -> LockWord {
        LockWord(kome::num::lo32(v))
    }
}

/// An object sake made itself that the caller passed back through an out-parameter slot of the
/// metadata's (a GpMatrix*, a UBreakIterator*, a GLUquadric*): no guest address to write (it is
/// no `Into<Ga>`), only the key sake's own tables know it by (review 2026-11-17: `object()` was a
/// u32 any writer took).
/// ```compile_fail
/// fn f(c: &mut sake::api::Ctx) { sake::mem::w32(c.out(0).object(), 0) }
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(transparent)]
pub struct Object(u32);

impl Object {
    /// The key sake's table of such objects has it under (the address sake gave the program)
    #[inline]
    pub fn key(self) -> u32 {
        self.0
    }
}

/// The words of a call into code that writes an out-parameter itself (guest code: Ctx::call; the
/// host: d3d11's device calls; a COM method), `words![a, out, ..]`: an Out goes in as its address
/// whole, and that is the only way it becomes a u32 (review 2026-11-16: `to_callee()` was a u32
/// a 32-bit writer could take; `w32(out.to_callee(), v)` compiled).
/// ```compile_fail
/// fn f(c: &mut sake::api::Ctx) { sake::mem::w32(c.out(1).to_callee(), 0) }
/// ```
/// ```compile_fail
/// // (the word of it is sake's own, and inside sake only words! names it: outptr_audit)
/// fn f(c: &mut sake::api::Ctx) -> u32 { use sake::api::callee::Word; c.out(1).__word() }
/// ```
pub(crate) mod callee {
    /// (named by `words!` alone: outptr_audit fails on `__word` / `callee::` anywhere else)
    pub trait Word {
        #[doc(hidden)]
        fn __word(self) -> u32;
    }
    impl Word for u32 {
        #[inline]
        fn __word(self) -> u32 {
            self
        }
    }
    impl Word for super::Out {
        #[inline]
        fn __word(self) -> u32 {
            self.0
        }
    }
}

/// (see [`callee`])
macro_rules! words {
    ($($e:expr),* $(,)?) => { [$($crate::api::callee::Word::__word($e)),*] };
}
pub(crate) use words;

/// A function's pointer-sized out-parameters as Win32 metadata gives them (bit `i`: argument
/// `i`): the types of [`outs`], one per function of src/outptrs.txt, generated by build.rs.
pub trait Outs {
    /// x64's arguments
    const MASK: u64;
    /// x86's stack dwords
    const MASK32: u64;
}

/// One type per Win32 function with pointer-sized out-parameters (build.rs, from src/outptrs.txt:
/// Microsoft's Win32 metadata through tools/win32md), and `mask(name)` for the dispatcher.
pub mod outs {
    include!(concat!(env!("OUT_DIR"), "/outs.rs"));
}

/// The arguments of a handler of the function `F` (`c.args::<outs::F>()`): `a::<I>()` is no
/// value for one of its pointer-sized out-parameters and `out::<I>()` is one only for them, so
/// either mistake does not compile (review 2026-11-16: `c.a(i)` took an out-parameter as a u32,
/// which a 32-bit writer then wrote half of on x64). Ctx's index-taking a() is not reached
/// through it (no Deref): the rest of Ctx is `ctx()`.
/// ```compile_fail
/// // GetProcessAffinityMask's arguments 1 and 2 are pointer-sized out-parameters (a handler is
/// // compiled as the function it is registered as: the pointer below)
/// fn f(c: &mut sake::api::Ctx) -> u32 { c.args::<sake::api::outs::GetProcessAffinityMask>().a::<1>() }
/// let _: sake::api::Handler = f;
/// ```
/// ```compile_fail
/// fn f(c: &mut sake::api::Ctx) -> u32 { c.args::<sake::api::outs::GetProcessAffinityMask>().out::<0>().get() }
/// let _: sake::api::Handler = f;
/// ```
/// ```compile_fail
/// // (the index-taking a() of Ctx is not reached through it)
/// fn f(c: &mut sake::api::Ctx) -> u32 { c.args::<sake::api::outs::GetProcessAffinityMask>().a(1) }
/// ```
/// ```compile_fail
/// // (nor through a dereference: Args is no Ctx)
/// fn f(c: &mut sake::api::Ctx) -> u32 { let a = c.args::<sake::api::outs::GetProcessAffinityMask>(); (*a).a(1) }
/// ```
/// ```compile_fail
/// // (nor by an index known only at run time: a_range's bounds are constants)
/// fn f(c: &mut sake::api::Ctx, i: u32) -> u32 { c.args::<sake::api::outs::AllocateAndInitializeSid>().a_at(i) }
/// ```
/// ```compile_fail
/// // MFTEnumEx's IMFActivate*** is x64's argument 4 and x86's dword 7 (a GUID by value first):
/// // one index for both does not compile
/// fn f(c: &mut sake::api::Ctx) -> u32 { c.args::<sake::api::outs::MFTEnumEx>().out::<4>().get() }
/// let _: sake::api::Handler = f;
/// ```
/// ```compile_fail
/// // (nor a value past the GUID by one index: x86's dword 5 is x64's argument 2)
/// fn f(c: &mut sake::api::Ctx) -> u32 { c.args::<sake::api::outs::MFTEnumEx>().a::<5>() }
/// let _: sake::api::Handler = f;
/// ```
/// ```compile_fail
/// // (a range over an out-parameter: AllocateAndInitializeSid's PSID* is argument 10)
/// fn f(c: &mut sake::api::Ctx) -> u32 { c.args::<sake::api::outs::AllocateAndInitializeSid>().a_range::<2, 11>().sum() }
/// let _: sake::api::Handler = f;
/// ```
/// ```
/// fn f(c: &mut sake::api::Ctx) -> u32 {
///     let a = c.args::<sake::api::outs::GetProcessAffinityMask>();
///     a.a::<0>() | a.out::<1>().get()
/// }
/// let _: sake::api::Handler = f;
/// fn g(c: &mut sake::api::Ctx) -> u32 {
///     let a = c.args::<sake::api::outs::MFTEnumEx>();
///     a.a_arch::<2, 5>() | a.out_arch::<4, 7>().get() | a.a_range::<0, 1>().sum::<u32>()
/// }
/// let _: sake::api::Handler = g;
/// ```
/// QueryInterface's arguments: the helpers several interfaces' QueryInterface share take these
pub type QiArgs<'c, 'a> = Args<'c, 'a, outs::iface::IUnknown::QueryInterface>;

pub struct Args<'c, 'a, F: Outs> {
    c: &'c mut Ctx<'a>,
    f: std::marker::PhantomData<fn() -> F>,
}

impl<'a, F: Outs> Args<'_, 'a, F> {
    /// Argument `I`, not one of F's pointer-sized out-parameters (those are `out::<I>()`), the
    /// same index on x64 and x86 (for a function with a GUID or a double by value before it:
    /// `a_arch`)
    #[inline]
    pub fn a<const I: u32>(&self) -> u32 {
        const { assert!(I >= 64 || (F::MASK | F::MASK32) >> I & 1 == 0, "a pointer-sized out-parameter: out::<I>(), not a::<I>()") };
        const { assert!(F::MASK == F::MASK32 || I < F::MASK.trailing_zeros() && I < F::MASK32.trailing_zeros(), "past an argument that x64 and x86 number apart: a_arch::<X64, X86>()") };
        self.c.a(I)
    }
    /// Argument `I`, one of F's pointer-sized out-parameters (the same index on x64 and x86)
    #[inline]
    pub fn out<const I: u32>(&self) -> Out {
        const { assert!(I < 64 && F::MASK >> I & 1 != 0 && F::MASK32 >> I & 1 != 0, "not a pointer-sized out-parameter of this function on both x64 and x86: a::<I>(), or out_arch::<X64, X86>()") };
        self.c.out(I)
    }
    /// Argument X64 on x64, X86 on x86 (after a GUID or a double by value), neither one of F's
    /// pointer-sized out-parameters
    #[inline]
    pub fn a_arch<const X64: u32, const X86: u32>(&self) -> u32 {
        const { assert!((X64 >= 64 || F::MASK >> X64 & 1 == 0) && (X86 >= 64 || F::MASK32 >> X86 & 1 == 0), "a pointer-sized out-parameter: out_arch") };
        self.c.a(if crate::mem::x64() { X64 } else { X86 })
    }
    /// Out-parameter X64 on x64, X86 on x86
    #[inline]
    pub fn out_arch<const X64: u32, const X86: u32>(&self) -> Out {
        const { assert!(X64 < 64 && X86 < 64 && F::MASK >> X64 & 1 != 0 && F::MASK32 >> X86 & 1 != 0, "not a pointer-sized out-parameter of this function: a_arch") };
        self.c.out(if crate::mem::x64() { X64 } else { X86 })
    }
    /// Arguments LO..HI as values (a variable count of them: AllocateAndInitializeSid's
    /// sub-authorities), none of them one of F's pointer-sized out-parameters on either
    #[inline]
    pub fn a_range<const LO: u32, const HI: u32>(&self) -> impl Iterator<Item = u32> + '_ {
        const { assert!(LO <= HI && HI <= 64 && (F::MASK | F::MASK32) & ((1u64.wrapping_shl(HI)).wrapping_sub(1) & !((1u64 << LO) - 1)) == 0 && (HI < 64 || (F::MASK | F::MASK32) >> LO == 0), "a pointer-sized out-parameter in the range") };
        (LO..HI).map(|i| self.c.a(i))
    }
    /// The rest of Ctx, for what is not an argument (the CPU, calls, the result): helpers given
    /// it that read an argument still meet the run-time check (Ctx::a of an out-parameter fails
    /// a test, x86 and x64)
    #[inline]
    pub fn ctx(&mut self) -> &mut Ctx<'a> {
        self.c
    }
    /// (a reading of `ctx`'s, no arguments)
    #[inline]
    pub fn ctx_ref(&self) -> &Ctx<'a> {
        self.c
    }
}

/// Whether taking argument `i` as a value takes one of the out-parameters `outs` (Ctx::not_out:
/// the mask of the program's architecture)
#[inline]
fn out_taken(outs: u64, i: u32) -> bool {
    i < 64 && outs >> i & 1 != 0
}

#[cfg(test)]
mod outs_tests {
    use super::*;
    use proptest::prelude::*;

    /// outptrs.txt as the audit reads it: name -> its out-parameters (x64 field 3, x86 field 4)
    fn table() -> HashMap<&'static str, OutMask> {
        let mut t: HashMap<&str, OutMask> = HashMap::default();
        for l in include_str!("outptrs.txt").lines() {
            let v: Vec<&str> = l.split(' ').collect();
            assert_eq!(v.len(), 4, "{l}");
            let e = t.entry(v[1]).or_default();
            for i in v[2].split(',') {
                e.x64 |= 1u64 << i.parse::<u32>().unwrap();
            }
            for i in v[3].split(',') {
                e.x86 |= 1u64 << i.parse::<u32>().unwrap();
            }
        }
        t
    }

    /// The generated table is the metadata's, every function of it on both architectures
    /// (differential: build.rs's parse against this one), and a name it lacks has none.
    #[test]
    fn generated_masks_are_the_metadatas() {
        let t = table();
        assert!(t.len() > 3000, "{}", t.len());
        for (name, m) in &t {
            assert_eq!(outs::mask(name), *m, "{name}");
            assert_eq!(m.x64.count_ones(), m.x86.count_ones(), "{name}: one out-parameter each");
        }
        for name in ["GetTickCount", "Sleep", "", "getprocessaffinitymask"] {
            assert_eq!(outs::mask(name), OutMask::NONE, "{name}");
        }
        assert_eq!(<outs::GetProcessAffinityMask as Outs>::MASK, 0b110);
        assert_eq!(<outs::GetProcessAffinityMask as Outs>::MASK32, 0b110);
        assert_eq!(<outs::InitializeSRWLock as Outs>::MASK, 0b1);
        // (a GUID by value: x64 one argument, x86 four dwords)
        assert_eq!(<outs::MFTEnumEx as Outs>::MASK, 1 << 4);
        assert_eq!(<outs::MFTEnumEx as Outs>::MASK32, 1 << 7);
        let differ = t.values().filter(|m| m.x64 != m.x86).count();
        assert!((50..400).contains(&differ), "{differ} functions numbered apart");
    }

    proptest! {
        /// Any function of the table, either architecture and any argument: taken as a value,
        /// it is reported exactly when it is one of the function's out-parameters there.
        #[test]
        fn a_value_is_refused_only_for_an_out_parameter(k in 0usize..4000, i in 0u32..80, x64: bool) {
            let t = table();
            let mut names: Vec<_> = t.keys().copied().collect();
            names.sort_unstable();
            let name = names[k % names.len()];
            let m = outs::mask(name).of_arch(x64);
            let want = if x64 { t[name].x64 } else { t[name].x86 };
            prop_assert_eq!(out_taken(m, i), i < 64 && want >> i & 1 != 0);
            prop_assert!(!out_taken(0, i));
        }
    }

    /// The run-time check on x86 (the program this test is): an x86 stack dword that is an
    /// out-parameter, taken as a value, fails; the others read their slot. MFTEnumEx's GUID by
    /// value moves its IMFActivate*** from x64's argument 4 to x86's dword 7, which the x64
    /// numbering would have missed (and refused dword 4, the flags).
    #[test]
    fn x86_reports_its_own_out_parameters() {
        let _g = crate::test_guest::enter();
        assert!(!crate::mem::x64());
        let sp = crate::mem::heap_alloc(4 + 4 * 12, true);
        for i in 0..12 {
            crate::mem::w32(crate::mem::ga(sp + 4).at(i, 4), 0x100 + i);
        }
        let mut cpu = koji::Cpu::default();
        let c = Ctx { cpu: &mut cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: outs::mask("MFTEnumEx").of_arch(false) };
        for i in 0..12u32 {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.a(i)));
            if i == 7 {
                assert!(r.is_err(), "dword 7 taken as a value");
            } else {
                assert_eq!(r.ok(), Some(0x100 + i), "dword {i}");
            }
        }
        assert_eq!(c.out(7), Out(0x107));
    }

    proptest! {
        /// Any function of the table on x86: every out dword of it, taken as a value, fails;
        /// every other dword reads its slot (the same check dispatch gives every handler, typed
        /// or not, and every helper given its Ctx).
        #[test]
        fn x86_check_is_the_tables(k in 0usize..4000) {
            let t = table();
            let mut names: Vec<_> = t.keys().copied().collect();
            names.sort_unstable();
            let name = names[k % names.len()];
            let _g = crate::test_guest::enter();
            let sp = crate::mem::heap_alloc(4 + 4 * 24, true);
            let mut cpu = koji::Cpu::default();
            let c = Ctx { cpu: &mut cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: outs::mask(name).of_arch(false) };
            for i in 0..24u32 {
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.a(i)));
                prop_assert_eq!(r.is_err(), t[name].x86 >> i & 1 != 0, "{} dword {}", name, i);
            }
        }
    }

    /// A vtable's name says its interface where the metadata has that vtable and this one is its
    /// start, by every method's stack bytes; otherwise none (its slots are not the metadata's).
    #[test]
    fn vtable_names_their_interface() {
        let unk = [12, 4, 4];
        assert_eq!(vtable_iface("IDirectSoundNotify", [12, 4, 4, 12].into_iter()), Some("IDirectSoundNotify"));
        assert_eq!(vtable_iface("VMR9::IPin", unk.into_iter()), Some("IPin"));
        assert_eq!(vtable_iface("MPEG-1 IPin", unk.into_iter()), Some("IPin"));
        assert_eq!(vtable_iface("IVideoWindow(windowless)", unk.into_iter()), Some("IVideoWindow"));
        // (a method taking other bytes than the metadata's fails a test; a version the metadata
        // lacks, or no interface, is none)
        assert!(std::panic::catch_unwind(|| vtable_iface("IDirectSoundNotify", [12, 4, 4, 16].into_iter())).is_err());
        assert_eq!(vtable_iface("IXAudio2SourceVoice_27", unk.into_iter()), None);
        assert_eq!(vtable_iface("CDialog", unk.into_iter()), None);
        // (IGraphBuilder with IFilterGraph2's methods after)
        assert_eq!(vtable_iface("IDirectSoundNotify", [12, 4, 4, 12, 4].into_iter()), Some("IDirectSoundNotify"));
    }

    /// The traps of a vtable get its methods' out-parameters, watched for 32-bit writes and not
    /// (yet) for values; a function's are both.
    #[test]
    fn a_vtable_carries_its_methods_out_parameters() {
        let _g = crate::test_guest::enter();
        fn h(_: &mut Ctx) -> u32 {
            0
        }
        let v = vtable("IDirectSoundNotify", &[(h, 12), (h, 4), (h, 4), (h, 12)]);
        let qi = crate::mem::rp(v);
        let (_, m, o) = hot(kome::num::us(trap_no(qi))).expect("published");
        assert_eq!(o, outs::com("IDirectSoundNotify", 0));
        assert_eq!(o, OutMask { x64: 0b100, x86: 0b100 });
        assert!(m.value_check);
        let (_, m, _) = hot(kome::num::us(trap_no(export("kernel32", "GetProcessAffinityMask")))).expect("published");
        assert!(m.value_check);
    }

    /// A handler taking its method's arguments goes; one taking another's fails a test, where
    /// the metadata has the method; where it has not (a version of sake's own name, a method
    /// past the end of the metadata's vtable) nothing is known to hold it to (host-tests
    /// xaudio2, xfile: IXAudio2_27, IDirectXFileData).
    #[test]
    fn a_handler_is_held_to_its_methods_out_parameters_where_they_are_known() {
        let _g = crate::test_guest::enter();
        fn h(_: &mut Ctx) -> u32 {
            0
        }
        let typed_as = |v: u32, slot: u32, f: fn(&mut Ctx)| {
            let a = crate::mem::rp(crate::mem::ga(v).at(slot, crate::mem::ps()));
            let (_, m, o) = hot(kome::num::us(trap_no(a))).expect("published");
            let sp = crate::mem::heap_alloc(64, true);
            let mut cpu = koji::Cpu::default();
            let mut c = Ctx { cpu: &mut cpu, esp: sp, edx: None, jumped: false, st0: None, trap: a, this: None, outs: if m.value_check { o.of_arch(crate::mem::x64()) } else { 0 } };
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut c))).is_ok()
        };
        let qi = |c: &mut Ctx| {
            c.args::<outs::iface::IUnknown::QueryInterface>();
        };
        struct NoOuts;
        impl Outs for NoOuts {
            const MASK: u64 = 0;
            const MASK32: u64 = 0;
        }
        let none = |c: &mut Ctx| {
            c.args::<NoOuts>();
        };
        let known = vtable("IDirectSoundNotify", &[(h, 12), (h, 4), (h, 4), (h, 12), (h, 8)]);
        assert!(typed_as(known, 0, qi));
        assert!(!typed_as(known, 0, none));
        assert!(typed_as(known, 1, none));
        assert!(!typed_as(known, 1, qi));
        // (past the metadata's 4 methods)
        assert!(typed_as(known, 4, qi) && typed_as(known, 4, none));
        let own = vtable("IXAudio2_27", &[(h, 12), (h, 4), (h, 4)]);
        assert!(typed_as(own, 0, qi) && typed_as(own, 0, none) && typed_as(own, 1, qi));
    }

    /// The slots watched are where the caller's out-parameters are: a 32-bit write to one fails,
    /// to anything else not (kura::outs; x86 here, the same arguments as x64's numbering).
    #[test]
    fn a_32_bit_write_to_a_watched_slot_fails() {
        let _g = crate::test_guest::enter();
        let sp = crate::mem::heap_alloc(4 + 4 * 4, true);
        for i in 0..4u32 {
            crate::mem::w32(crate::mem::ga(sp + 4).at(i, 4), 0x5000 + 16 * i);
        }
        let mut cpu = koji::Cpu::default();
        let c = Ctx { cpu: &mut cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: 0 };
        let guard = out_slots(&c, 0b1010);
        let caught = |a: u32| std::panic::catch_unwind(|| kura::outs::wrote32(a)).is_err();
        assert!(caught(0x5010) && caught(0x5030));
        assert!(!caught(0x5000) && !caught(0x5020) && !caught(0x5014));
        drop(guard);
        assert!(!caught(0x5010));
    }

    /// An import gets its function's out-parameters where dispatch reads them, with its handler.
    #[test]
    fn an_entry_carries_its_out_parameters() {
        for (dll, name, want) in [("kernel32", "GetProcessAffinityMask", 0b110), ("kernel32", "InitializeSRWLock", 0b1), ("kernel32", "GetTickCount", 0)] {
            let a = export(dll, name);
            let (_, _, outs) = hot(kome::num::us(trap_no(a))).expect("published");
            assert_eq!(outs, OutMask { x64: want, x86: want }, "{dll}!{name}");
        }
    }
}

/// The running call's pointer-sized out-parameters (bits `m` of its arguments) watched by
/// kura::outs: the slots' addresses as the caller passed them.
fn out_slots(c: &Ctx, m: u64) -> kura::outs::Guard {
    kura::outs::set_namer(|t| name_of(t).unwrap_or_else(|| format!("{t:#x}")));
    let at = (0..64u32).filter(|&i| m >> i & 1 != 0).map(|i| c.slot(i));
    kura::outs::enter(kura::outs::Slots::new(c.trap, at))
}

/// How a COM method's handler takes its arguments (set_com_outs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComArgs {
    /// as its method's (c.args::<outs::iface::I::M>(), first thing): Ctx::a of an
    /// out-parameter reported, as a function's is, and c.args of another's
    Typed,
    /// every one as a word handed on to kura (d3d9, d3d11), whose writers are watched for a
    /// 32-bit write to an out slot (kura::outs) instead
    Words,
}

/// The trap `addr` (a COM method's: com.rs's and api::vtable's vtables, d3d9's, d3d11's) gets
/// method `slot` of `iface`'s pointer-sized out-parameters (outs::com, Microsoft's metadata):
/// watched for 32-bit writes (kura::outs), and with ComArgs::Typed its handler's Ctx::a of one
/// reported. A method the metadata does not have (an interface of its own, IXAudio2_27 or
/// IDirectXFileData, or one past the end of the metadata's vtable) has none known: its handler
/// is not held to any (its c.args not checked against the call's, Ctx::args).
pub(crate) fn set_com_outs(addr: u32, iface: &str, slot: u32, how: ComArgs) {
    let known = outs::com_dwords(iface).is_some_and(|d| kome::num::us(slot) < d.len());
    let m = outs::com(iface, slot);
    if known && m == OutMask::NONE && how == ComArgs::Typed {
        return;
    }
    let i = kome::num::us(trap_no(addr));
    let mut e = ENTRIES.w();
    if let Some(x) = e.get_mut(i) {
        x.outs = m;
        x.value_check = known && how == ComArgs::Typed;
        publish(i, x);
    }
}

/// The handler at `trap` takes its arguments as a function's or method's whose pointer-sized
/// out-parameters (`typed`) are not the called one's (`called`): a test fails; a game goes on,
/// and the log says so once.
#[cold]
fn typed_as_another(trap: u32, typed: u64, called: u64) {
    // (a method whose out-parameters are not known: none to hold its handler to)
    if !hot(kome::num::us(trap_no(trap))).is_some_and(|h| h.1.value_check) {
        return;
    }
    let what = format!("{}: its handler takes the arguments of one with out-parameters {typed:#b}, it has {called:#b}", name_of(trap).unwrap_or_else(|| format!("{trap:#x}")));
    assert!(!cfg!(test) && !cfg!(debug_assertions) && !kura::outs::strict(), "{what}");
    static SAID: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
    if SAID.locked().get_or_insert_with(HashSet::default).insert(trap) {
        eprintln!("sake: {what}");
    }
}

/// A pointer-sized out-parameter of the function at `trap` taken as a value (Ctx::a, z, a64):
/// a test fails; a game goes on, and the log says so once per function and argument.
#[cold]
fn out_as_value(trap: u32, i: u32) {
    let what = format!("{} argument {i} is a pointer-sized out-parameter: c.out({i}), not a value", name_of(trap).unwrap_or_else(|| format!("{trap:#x}")));
    // (under test: fails there)
    assert!(!cfg!(test) && !cfg!(debug_assertions) && !kura::outs::strict(), "{what}");
    static SAID: Mutex<Option<HashSet<(u32, u32)>>> = Mutex::new(None);
    if SAID.locked().get_or_insert_with(HashSet::default).insert((trap, i)) {
        eprintln!("sake: {what}");
    }
}

impl<'a> Ctx<'a> {
    /// The arguments as the function `F`'s: its pointer-sized out-parameters only as Out.
    #[inline]
    pub fn args<F: Outs>(&mut self) -> Args<'_, 'a, F> {
        // (a handler taking another function's or method's arguments: its out-parameters are
        // not these; called through a trap only, a test's own Ctx has none)
        let want = if crate::mem::x64() { F::MASK } else { F::MASK32 };
        if self.trap != 0 && want != self.outs {
            typed_as_another(self.trap, want, self.outs);
        }
        Args { c: self, f: std::marker::PhantomData }
    }
}

impl Ctx<'_> {
    /// Argument `i`, a pointer-sized out-parameter
    #[inline]
    pub fn out(&self, i: u32) -> Out {
        Out(self.slot(i))
    }
    /// Argument `i` as a value (not one of the function's pointer-sized out-parameters: those
    /// are `out(i)`; taken so, it is reported, and a test fails)
    #[inline]
    pub fn a(&self, i: u32) -> u32 {
        self.not_out(i);
        self.slot(i)
    }
    /// Argument `i` as a BOOLEAN (UCHAR): its low byte only, the rest of the slot is the
    /// caller's garbage (masu-lint boolean-arg)
    #[inline]
    pub fn boolean(&self, i: u32) -> bool {
        kome::num::lo8(self.a(i)) != 0
    }
    /// (an out-parameter taken as a value: a handler's mistake; under test it fails, a game
    /// goes on and the log names it once; x86 and x64, each by its own numbering, OutMask)
    #[inline]
    fn not_out(&self, i: u32) {
        if out_taken(self.outs, i) {
            out_as_value(self.trap, i);
        }
    }
    /// Argument `i`'s slot (x64: its low half)
    #[inline]
    fn slot(&self, i: u32) -> u32 {
        if crate::mem::x64() {
            return kome::num::lo32(self.raw64(i));
        }
        match self.this {
            Some(t) if i == 0 => t,
            Some(_) => crate::mem::r32(crate::mem::ga(self.esp).at(i, 4)),
            None => crate::mem::r32((crate::mem::ga(self.esp) + 4).at(i, 4)),
        }
    }
    /// A pointer-sized argument (size_t, ptrdiff_t): x64 all 64 bits, x86 the slot.
    pub fn z(&self, i: u32) -> u64 {
        self.not_out(i);
        if crate::mem::x64() { self.raw64(i) } else { u64::from(self.slot(i)) }
    }
    /// The 64-bit integer argument `i` (x64: one argument; x86: the slots `i`, `i + 1`).
    pub fn q(&self, i: u32) -> u64 {
        if crate::mem::x64() {
            return self.a64(i);
        }
        u64::from(self.a(i)) | u64::from(self.a(i.saturating_add(1))) << 32
    }
    /// The double argument `i` (x64: XMM `i` or its stack slot; x86: the slots `i`, `i + 1`).
    pub fn d(&self, i: u32) -> f64 {
        if crate::mem::x64() {
            return f64::from_bits(self.xmm(i));
        }
        f64::from_bits(self.q(i))
    }
    /// Where an object passed by value as argument `i` is: x64 passes one bigger than 8 bytes
    /// by address (the argument), x86 copies it onto the stack (from slot `i` on).
    pub fn byval(&self, i: u32) -> u32 {
        if crate::mem::x64() {
            return self.a(i);
        }
        match self.this {
            Some(_) => crate::mem::ga(self.esp).at(i, 4).get(),
            None => (crate::mem::ga(self.esp) + 4).at(i, 4).get(),
        }
    }
    /// x64: the whole argument `i` (RCX, RDX, R8, R9, then the stack past the home space).
    pub fn a64(&self, i: u32) -> u64 {
        self.not_out(i);
        self.raw64(i)
    }
    fn raw64(&self, i: u32) -> u64 {
        match i {
            0 => self.cpu.gpr(koji::Reg64::Rcx),
            1 => self.cpu.gpr(koji::Reg64::Rdx),
            2 => self.cpu.gpr(koji::Reg64::R8),
            3 => self.cpu.gpr(koji::Reg64::R9),
            _ => crate::mem::r64((crate::mem::ga(self.esp) + 8).at(i, 8)),
        }
    }
    /// x64: floating-point argument `i` (XMM0-3, then the stack), as a double's bits.
    pub fn xmm(&self, i: u32) -> u64 {
        if i < 4 { kome::num::lo64(self.cpu.xmm(kome::num::us(i))) } else { crate::mem::r64((crate::mem::ga(self.esp) + 8).at(i, 8)) }
    }
    /// A POINT passed by value at slots `i`, `i + 1` (x64: the one 8-byte argument `i`; the
    /// index is the same when only 4-byte arguments come before it).
    pub fn pt(&self, i: u32) -> (i32, i32) {
        if crate::mem::x64() {
            let v = self.a64(i);
            return (kome::num::lo32(v).cast_signed(), kome::num::lo32(v >> 32).cast_signed());
        }
        (self.i(i), self.i(i.saturating_add(1)))
    }
    /// The float argument `i` (x64: XMM0-3, then the stack slot's low half).
    pub fn f(&self, i: u32) -> f32 {
        if crate::mem::x64() {
            return f32::from_bits(kome::num::lo32(self.xmm(i)));
        }
        f32::from_bits(self.a(i))
    }
    /// A D2D1_POINT_2F passed by value at slots `i`, `i + 1` (x64: the one 8-byte argument
    /// `i`); and the slot after it.
    pub fn pt_f(&self, i: u32) -> ((f32, f32), u32) {
        if crate::mem::x64() {
            let v = self.a64(i);
            return ((f32::from_bits(kome::num::lo32(v)), f32::from_bits(kome::num::lo32(v >> 32))), i.saturating_add(1));
        }
        ((f32::from_bits(self.a(i)), f32::from_bits(self.a(i.saturating_add(1)))), i.saturating_add(2))
    }
    /// Calls guest code at `f` with `args`; the float arguments `floats` (their indexes in
    /// `args`, which hold their bits) go in XMM registers as well on x64.
    pub fn call_f(&mut self, f: u32, args: &[u32], floats: &[usize]) -> u32 {
        if crate::mem::x64() {
            for &i in floats {
                if i < 4 && let Some(&v) = args.get(i) {
                    self.cpu.set_xmm(i, u128::from(v));
                }
            }
        }
        crate::call(self.cpu, f, args)
    }
    pub fn i(&self, i: u32) -> i32 {
        self.a(i).cast_signed()
    }
    /// The double at 32-bit stack slots i, i+1 (x64: argument i/2, the double's register)
    pub fn f64(&self, i: u32) -> f64 {
        if crate::mem::x64() {
            return f64::from_bits(self.xmm(i / 2));
        }
        f64::from_bits(u64::from(self.a(i)) | u64::from(self.a(i.saturating_add(1))) << 32)
    }
    /// Calls guest code at `f` with `args` (stdcall or cdecl alike); returns EAX.
    pub fn call(&mut self, f: u32, args: &[u32]) -> u32 {
        crate::call(self.cpu, f, args)
    }
    pub fn ret64(&mut self, v: u64) -> u32 {
        self.edx = Some(kome::num::lo32(v >> 32));
        kome::num::lo32(v)
    }
}

struct Entry {
    dll: String,
    name: String,
    f: Option<Handler>,
    bytes: u32,
    cdecl: bool,
    known: bool,
    /// x86 thiscall with `this` as argument 0 (Ctx::this)
    this_arg: bool,
    /// a bare yield, not work (waits::handoff::Handoff::worked): 1 always (SwitchToThread),
    /// 2 when its argument 0 is 0 (Sleep(0), SleepEx(0, ..))
    yields: u8,
    /// its pointer-sized out-parameters (outs::mask, outs::com: set_com_outs)
    outs: OutMask,
    /// whether its handler is held to taking them only as Out (Ctx::a of one reported): a
    /// function's; a COM method's not yet (their handlers take `putp(c.a(i), ..)` still, which
    /// the 32-bit write check, kura::outs, covers meanwhile)
    value_check: bool,
}

fn yields(name: &str) -> u8 {
    match name {
        "SwitchToThread" | "NtYieldExecution" => 1,
        "Sleep" | "SleepEx" => 2,
        _ => 0,
    }
}

static ENTRIES: RwLock<Vec<Entry>> = RwLock::new(Vec::new());

/// Entries at most: their traps stay below RET_TRAP (the traps past it are the run loop's own).
const MAX_ENTRIES: usize = const_us((RET_TRAP - TRAP_LO) / 4);
/// A u32 as usize in a const (kome::num::us is not a const fn): its bytes, little-endian.
const fn const_us(x: u32) -> usize {
    const _: () = assert!(std::mem::size_of::<usize>() >= 4);
    let [a, b, c, d] = x.to_le_bytes();
    let mut w = [0u8; std::mem::size_of::<usize>()];
    w[0] = a;
    w[1] = b;
    w[2] = c;
    w[3] = d;
    usize::from_le_bytes(w)
}
#[cfg(test)]
#[test]
fn const_us_is_us() {
    for x in [0, 1, 0xff, 0x100, 0xdead_beef, u32::MAX, (RET_TRAP - TRAP_LO) / 4] {
        assert_eq!(const_us(x), kome::num::us(x));
    }
}

/// What `dispatch` reads of an entry, without a lock (review C-1: the RwLock's reader count,
/// one cache line written by every call of every guest thread). Slots in chunks of
/// HOT_CHUNK made on first use and never freed (a trap's index never changes), written
/// only under ENTRIES' write lock (`publish`), read with Acquire. `tag` and `data` are the
/// handler's own (`set_tag`: a COM method's id, a GL function), 0 when none.
struct Hot {
    /// The entry's handler and flags as one pointer (review 2026-11-16: two atomics let a
    /// reader take a new handler with the old flags): one of PUBLISHED's, null when none yet.
    cur: AtomicPtr<Published>,
    tag: AtomicU32,
    data: AtomicUsize,
}

/// A handler and its flags as a reader takes them, together.
#[derive(Clone, Copy, Debug)]
struct Published {
    f: Option<Handler>,
    meta: u32,
    /// the function's pointer-sized out-parameters (Ctx::outs: of_arch)
    outs: OutMask,
}

/// Every (handler, flags) pair ever published, one each: a slot points at one of these. They
/// live as long as the process (a reader may still hold the pointer a slot had), and there are
/// as many as there are pairs, not publishes: an entry published again with what it had takes
/// the same one (review 2026-11-16: publish leaked a Box per host_fn call, 6000 in a test).
/// (keyed by the pair: a handler's address, which two identical functions may share, is
/// still the code a reader calls)
static PUBLISHED: Mutex<Option<HashMap<(Option<Handler>, u32, OutMask), &'static Published>>> = Mutex::new(None);

/// The one `p` of PUBLISHED (made the first time)
fn interned(p: Published) -> &'static Published {
    let mut g = PUBLISHED.locked();
    let m = g.get_or_insert_with(HashMap::default);
    *m.entry((p.f, p.meta, p.outs)).or_insert_with(|| Box::leak(Box::new(p)))
}

const HOT_CHUNK: usize = 4096;
static HOT: [AtomicPtr<Hot>; MAX_ENTRIES.div_ceil(HOT_CHUNK)] = [const { AtomicPtr::new(std::ptr::null_mut()) }; MAX_ENTRIES.div_ceil(HOT_CHUNK)];

/// An entry's flags as one word (Hot::meta)
#[derive(Clone, Copy, Debug, PartialEq)]
struct Meta {
    bytes: u32,
    cdecl: bool,
    known: bool,
    this_arg: bool,
    yields: u8,
    value_check: bool,
}

const META_SET: u32 = 1 << 31;
const META_BYTES: u32 = (1 << 20) - 1;

impl Meta {
    fn of(e: &Entry) -> Meta {
        Meta { bytes: e.bytes, cdecl: e.cdecl, known: e.known, this_arg: e.this_arg, yields: e.yields, value_check: e.value_check }
    }
    /// (bytes: 20 bits, a stdcall function's arguments are far fewer; more is cut to META_BYTES)
    fn pack(self) -> u32 {
        META_SET | self.bytes.min(META_BYTES) | u32::from(self.cdecl) << 20 | u32::from(self.known) << 21 | u32::from(self.this_arg) << 22 | (u32::from(self.yields) & 3) << 23 | u32::from(self.value_check) << 25
    }
    fn unpack(w: u32) -> Option<Meta> {
        (w & META_SET != 0).then_some(Meta { bytes: w & META_BYTES, cdecl: w >> 20 & 1 != 0, known: w >> 21 & 1 != 0, this_arg: w >> 22 & 1 != 0, yields: kome::num::lo8(w >> 23 & 3), value_check: w >> 25 & 1 != 0 })
    }
}

/// Slot `i` of the hot table (made when `make`: callers hold ENTRIES' write lock).
fn hot_slot(i: usize, make: bool) -> Option<&'static Hot> {
    let c = HOT.get(i / HOT_CHUNK)?;
    let mut p = c.load(Ordering::Acquire);
    if p.is_null() {
        if !make {
            return None;
        }
        let chunk: Box<[Hot]> = (0..HOT_CHUNK).map(|_| Hot { cur: AtomicPtr::new(std::ptr::null_mut()), tag: AtomicU32::new(0), data: AtomicUsize::new(0) }).collect();
        p = Box::leak(chunk).as_mut_ptr();
        c.store(p, Ordering::Release);
    }
    // SAFETY: p is a leaked chunk of HOT_CHUNK Hots (made above or by an earlier call), never freed;
    // i % HOT_CHUNK is inside it.
    let q = unsafe { p.add(i % HOT_CHUNK) };
    // SAFETY: q points at one of the chunk's Hots, alive for ever (leaked).
    Some(unsafe { &*q })
}

/// Entry `i` (of `e`, ENTRIES write-locked) into the hot table.
fn publish(i: usize, e: &Entry) {
    // (always Some: export asserts i < MAX_ENTRIES, which HOT covers)
    let Some(s) = hot_slot(i, true) else { return };
    let p = interned(Published { f: e.f, meta: Meta::of(e).pack(), outs: e.outs });
    // (Release: the pair is written before a reader that sees its pointer reads it; it is
    // PUBLISHED's, never written again)
    s.cur.store(std::ptr::from_ref(p).cast_mut(), Ordering::Release);
}

/// Entry `i`'s handler and flags, None when there is no entry `i`.
fn hot(i: usize) -> Option<(Option<Handler>, Meta, OutMask)> {
    let s = hot_slot(i, false)?;
    let p = s.cur.load(Ordering::Acquire);
    // SAFETY: non-null only from publish, a &'static of PUBLISHED's (leaked, never freed nor
    // written); the Acquire pairs with publish's Release store.
    let p = unsafe { p.as_ref() }?;
    Some((p.f, Meta::unpack(p.meta)?, p.outs))
}

/// Gives the trap at `addr` the handler's own `tag` (not 0) and `data`, read back without a
/// lock by [`tag_of`]: what a handler shared by many traps tells them apart by (it was a
/// locked HashMap by trap address: review C-2).
pub fn set_tag(addr: u32, tag: u32, data: usize) {
    debug_assert!(tag != 0);
    let Some(s) = addr.checked_sub(TRAP_LO).and_then(|o| hot_slot(kome::num::us(o) / 4, false)) else { return };
    s.data.store(data, Ordering::Relaxed);
    s.tag.store(tag, Ordering::Release);
}

/// The trap's [`set_tag`] tag and data; tag 0 when it has none.
pub fn tag_of(addr: u32) -> (u32, usize) {
    let Some(s) = addr.checked_sub(TRAP_LO).and_then(|o| hot_slot(kome::num::us(o) / 4, false)) else { return (0, 0) };
    let t = s.tag.load(Ordering::Acquire);
    (t, if t != 0 { s.data.load(Ordering::Relaxed) } else { 0 })
}
static INDEX: Mutex<Option<HashMap<(String, String), u32>>> = Mutex::new(None);
static IMPLS: RwLock<Option<HashMap<String, Handler>>> = RwLock::new(None);

struct Spec {
    /// (dll, name) -> (cdecl, bytes, thiscall); the names are specs.txt's own (54000 lines: a
    /// String for each of them cost every process start 20+ ms)
    by_name: HashMap<(&'static str, &'static str), (bool, u32, bool)>,
    by_ord: HashMap<(&'static str, u32), &'static str>,
    dlls: HashSet<&'static str>,
}

fn specs() -> &'static Spec {
    static S: OnceLock<Spec> = OnceLock::new();
    S.get_or_init(|| {
        let mut s = Spec { by_name: HashMap::default(), by_ord: HashMap::default(), dlls: HashSet::default() };
        for l in include_str!("specs.txt").lines() {
            let mut it = l.split(' ');
            let v: [&'static str; 5] = std::array::from_fn(|_| it.next().unwrap_or(""));
            if v[4].is_empty() {
                continue;
            }
            // (thiscall: `this` in ECX, the callee pops the rest)
            let cdecl = !matches!(v[2], "stdcall" | "thiscall");
            let bytes: u32 = v[3].parse::<u32>().unwrap_or(0).saturating_sub(if v[2] == "thiscall" { 4 } else { 0 });
            s.dlls.insert(v[0]);
            s.by_name.entry((v[0], v[1])).or_insert((cdecl, bytes, v[2] == "thiscall"));
            if let Ok(o) = v[4].parse::<u32>() {
                s.by_ord.insert((v[0], o), v[1]);
            }
        }
        s.dlls.insert("sake");
        s
    })
}

/// The functions the spec lists for `dll` (a key: "kernel32"), sorted.
pub fn spec_names(dll: &str) -> Vec<String> {
    let mut v: Vec<String> = specs().by_name.keys().filter(|(d, _)| *d == dll).map(|(_, n)| n.to_string()).collect();
    v.sort();
    v
}

/// Whether a system DLL has this export on Windows: its spec lists it, or we implement it under
/// that DLL's own name (DLLs without a spec: assumed). find_impl's forwarder fallback (kernel32
/// <-> ntdll, msvcrt) is for imports; GetProcAddress on ntdll or msvcrt for a kernel32 function
/// gives NULL on Windows, and mono takes the first module that answers
pub fn exported_by(dll: &str, name: &str) -> bool {
    let d = dll_key(dll);
    let sp = specs();
    !sp.dlls.contains(d.as_str())
        || sp.by_name.contains_key(&(d.as_str(), name))
        || IMPLS.r().as_ref().is_some_and(|m| m.contains_key(&format!("{d}!{name}")))
}

/// "KERNEL32.dll" -> "kernel32"
pub fn dll_key(dll: &str) -> String {
    let d = crate::pe::base_name(dll);
    let k = [".dll", ".drv", ".ax", ".ocx"].iter().find_map(|x| d.strip_suffix(x)).unwrap_or(&d);
    // API sets (Wine's PE DLLs and VC++ 2015+ programs import these)
    if k.starts_with("api-ms-win-crt-") {
        return "ucrtbase".into();
    }
    // (Windows' security.dll forwards everything to secur32)
    if k == "security" {
        return "secur32".into();
    }
    for (pre, dll) in [("api-ms-win-core-winrt-", "combase"), ("api-ms-win-core-com-", "ole32"), ("api-ms-win-core-registry-", "advapi32"), ("api-ms-win-core-shlwapi-", "shlwapi")] {
        if k.starts_with(pre) {
            return dll.into();
        }
    }
    if k.starts_with("api-ms-win-core-") || k.starts_with("api-ms-win-appmodel-") || k.starts_with("ext-ms-win-kernel32-") {
        return "kernelbase".into();
    }
    // (the others by the DLL that implements them)
    for (pre, dll) in [("ntuser-", "user32"), ("rtcore-ntuser-", "user32"), ("gdi-", "gdi32"), ("security-", "advapi32"), ("shell-", "shell32"), ("shcore-", "shcore")] {
        if k.strip_prefix("api-ms-win-").or_else(|| k.strip_prefix("ext-ms-win-")).is_some_and(|r| r.starts_with(pre)) {
            return dll.into();
        }
    }
    k.to_string()
}

/// Whether `dll` is one we implement (or at least know the exports of).
pub fn is_system(dll: &str) -> bool {
    let k = dll_key(dll);
    specs().dlls.contains(k.as_str()) || (crate::mem::x64() && matches!(k.as_str(), "msvcp140" | "vcruntime140_1"))
}

/// The C/C++ runtimes.
pub fn is_crt(key: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit() || b == b'_');
    matches!(key, "msvcrt" | "ucrtbase" | "crtdll")
        || key.strip_prefix("msvcr").is_some_and(digits)
        || key.strip_prefix("msvcp").is_some_and(digits)
        || key.starts_with("vcruntime")
}

/// DLLs we implement even when the game ships its own copy.
pub fn is_core(dll: &str) -> bool {
    // (x64: the C runtime and vcruntime too, which the C++ exception support is part of)
    if crate::mem::x64() && matches!(dll_key(dll).as_str(), "ucrtbase" | "vcruntime140" | "vcruntime140_1") {
        return true;
    }
    matches!(
        dll_key(dll).as_str(),
        "kernel32" | "kernelbase" | "ntdll" | "user32" | "gdi32" | "advapi32" | "ole32" | "oleaut32" | "comctl32"
            | "shell32" | "version" | "winmm" | "imm32" | "dsound" | "ddraw" | "comdlg32" | "shlwapi" | "msvcrt" | "msvfw32" | "msimg32"
    )
}

pub fn implement(dll: &str, name: &str, f: Handler) {
    IMPLS.w().get_or_insert_with(HashMap::default).insert(format!("{dll}!{name}"), f);
}

pub fn find_impl(dll: &str, name: &str) -> Option<Handler> {
    let g = IMPLS.r();
    let m = g.as_ref()?;
    if let Some(f) = m.get(&format!("{dll}!{name}")) {
        return Some(*f);
    }
    // kernel32 <-> kernelbase <-> ntdll forwarders, and the DLLs that export another's functions
    for d in forwards(dll).iter().chain(&["kernel32", "kernelbase", "ntdll", "msvcrt", "user32"]) {
        if let Some(f) = m.get(&format!("{d}!{name}")) {
            return Some(*f);
        }
    }
    // the UCRT's _o_ exports (api-ms-win-crt-private, for Windows' own DLLs) are its plain ones
    if dll == "ucrtbase" && let Some(plain) = name.strip_prefix("_o_") {
        drop(g);
        return find_impl(dll, plain);
    }
    None
}

/// The DLLs whose implementations `dll` (a key) also exports: shcore has shlwapi's string,
/// registry and stream helpers (shlwapi forwards them there since Windows 8), olepro32 is
/// oleaut32's OLE Automation picture/font functions, xaudio2_9 is xaudio2_8 (and both carry
/// X3DAudio, x3daudio1_7's).
fn forwards(dll: &str) -> &'static [&'static str] {
    match dll {
        "shcore" => &["shlwapi"],
        "olepro32" => &["oleaut32"],
        "xaudio2_9" => &["xaudio2_8", "x3daudio1_7"],
        "xaudio2_8" => &["x3daudio1_7"],
        _ => &[],
    }
}

/// Whether a system DLL exports `name` (specs.txt).
pub fn exports(dll: &str, name: &str) -> bool {
    specs().by_name.contains_key(&(dll_key(dll).as_str(), name))
}

/// Name of ordinal `ord` of a system DLL.
pub fn ordinal_name(dll: &str, ord: u32) -> Option<String> {
    specs().by_ord.get(&(dll_key(dll).as_str(), ord)).map(|n| n.to_string())
}

/// The trap address of a system DLL's export (made on first use).
/// `export` of a literal name, looked up once per call site: handlers that call other APIs at
/// run time (afd's CancelIoEx, ...) took the INDEX lock and built String keys on every call.
/// The address of an export never changes once made.
#[macro_export]
macro_rules! export_once {
    ($dll:literal, $name:literal) => {{
        static A: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        *A.get_or_init(|| $crate::api::export($dll, $name))
    }};
}

pub fn export(dll: &str, name: &str) -> u32 {
    let d = dll_key(dll);
    let key = (d.clone(), name.to_string());
    let mut idx = INDEX.locked();
    let idx = idx.get_or_insert_with(HashMap::default);
    if let Some(&i) = idx.get(&key) {
        return trap_at(i);
    }
    let sp = specs();
    let spec = sp.by_name.get(&(d.as_str(), name)).copied().or_else(|| {
        forwards(&d).iter().chain(&["kernel32", "kernelbase", "ntdll"]).find_map(|k| sp.by_name.get(&(*k, name)).copied())
    });
    let f = find_impl(&d, name);
    let mut e = ENTRIES.w();
    let i = kome::num::lo32(e.len());
    assert!((kome::num::us(i)) < MAX_ENTRIES, "sake: more than {MAX_ENTRIES} distinct imports ({dll}!{name})");
    let this_arg = spec.is_some_and(|s| s.2) && THIS_ARG0.contains(&d.as_str());
    e.push(Entry {
        dll: d,
        name: name.to_string(),
        f,
        bytes: spec.map_or(0, |s| s.1),
        cdecl: spec.is_some_and(|s| s.0),
        known: spec.is_some() || f.is_some(),
        this_arg,
        yields: yields(name),
        outs: outs::mask(name),
        value_check: true,
    });
    if let Some(x) = e.last() {
        publish(kome::num::us(i), x);
    }
    idx.insert(key, i);
    trap_at(i)
}

/// The DLLs whose thiscall functions take `this` as argument 0 on x86 (Ctx::this): their
/// handlers are x64's too. (msvcp90's and the others' read ECX themselves.)
const THIS_ARG0: [&str; 2] = ["msvcp140", "msvcp120"];

/// "dll!name" of a trap address (for logs).
pub fn name_of(addr: u32) -> Option<String> {
    let i = addr.checked_sub(TRAP_LO)? / 4;
    ENTRIES.r().get(kome::num::us(i)).map(|e| format!("{}!{}", e.dll, e.name))
}

/// Whether the function at a trap address is cdecl, and its stdcall argument bytes.
pub fn args_of(addr: u32) -> (bool, u32) {
    let i = kome::num::us(addr.wrapping_sub(TRAP_LO) / 4);
    hot(i).map_or((true, 0), |(_, m, _)| (m.cdecl, m.bytes))
}

/// The DLL key ("ucrtbase", "msvcrt"...) of a trap address.
pub fn dll_of(addr: u32) -> Option<String> {
    let i = addr.checked_sub(TRAP_LO)? / 4;
    ENTRIES.r().get(kome::num::us(i)).map(|e| e.dll.clone())
}

/// Whether the API at trap address `addr` has an implementation.
/// The trap is a function of the DLL's spec (implemented or not).
pub fn is_known(addr: u32) -> bool {
    let i = kome::num::us(addr.wrapping_sub(TRAP_LO) / 4);
    hot(i).is_some_and(|(_, m, _)| m.known)
}
/// The handler of a trap address.
pub fn handler_of(addr: u32) -> Option<Handler> {
    let i = kome::num::us(addr.wrapping_sub(TRAP_LO) / 4);
    hot(i).and_then(|(f, _, _)| f)
}

pub fn implemented(addr: u32) -> bool {
    let i = kome::num::us(addr.wrapping_sub(TRAP_LO) / 4);
    hot(i).is_some_and(|(f, _, _)| f.is_some())
}

pub fn trace() -> u32 {
    static T: OnceLock<u32> = OnceLock::new();
    *T.get_or_init(|| std::env::var("SAKE_TRACE").ok().and_then(|s| s.parse().ok()).unwrap_or(0))
}

/// Counting API calls by name (SAKE_API_STATS=<seconds>), and the time spent in them (µs: a
/// call that waits, WaitMessage or WaitForSingleObject, counts its wait too)
static COUNTS: Mutex<Option<HashMap<u32, u64>>> = Mutex::new(None);
static TIMES: Mutex<Option<HashMap<u32, u64>>> = Mutex::new(None);

fn stats_on() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| {
        let Some(s) = std::env::var("SAKE_API_STATS").ok().and_then(|s| s.parse::<f64>().ok()) else { return false };
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs_f64(s));
            let m = COUNTS.locked().take().unwrap_or_default();
            let mut v: Vec<(u32, u64)> = m.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            let total: u64 = v.iter().map(|x| x.1).sum();
            let top: Vec<String> = v.iter().take(40).map(|(i, n)| format!("{}={n}", name_of(trap_at(*i)).unwrap_or_default())).collect();
            eprintln!("sake api {total} calls/{s}s: {}", top.join(" "));
            let m = TIMES.locked().take().unwrap_or_default();
            let mut v: Vec<(u32, u64)> = m.into_iter().collect();
            v.sort_by(|a, b| b.1.cmp(&a.1));
            let total: u64 = v.iter().map(|x| x.1).sum();
            let top: Vec<String> = v.iter().take(40).map(|(i, us)| format!("{}={}ms", name_of(trap_at(*i)).unwrap_or_default(), us / 1000)).collect();
            eprintln!("sake api time {}ms/{s}s: {}", total / 1000, top.join(" "));
        });
        true
    })
}

/// SAKE_THREADS=<seconds>: every so often, each guest thread's last API calls (name, first
/// arguments, caller, ms), to see what a stalled game's threads wait for without the cost of
/// formatting a trace line per call. SAKE_THREADS_STACKS=1 adds each thread's registers and stack.
// (lock-free: relaxed stores into a slot per thread, so watching barely changes the timing)
const RECENT_N: usize = 8;
const RECENT_THREADS: usize = 128;
static RECENT: [[[std::sync::atomic::AtomicU32; 6]; RECENT_N]; RECENT_THREADS] = [const { [const { [const { std::sync::atomic::AtomicU32::new(0) }; 6] }; RECENT_N] }; RECENT_THREADS];
static RECENT_POS: [std::sync::atomic::AtomicU32; RECENT_THREADS] = [const { std::sync::atomic::AtomicU32::new(0) }; RECENT_THREADS];

fn recent_slot(tid: u32) -> Option<usize> {
    let k = kome::num::us(tid.wrapping_sub(0x20) / 4);
    (k < RECENT_THREADS).then_some(k)
}

fn threads_on() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| {
        let Some(s) = std::env::var("SAKE_THREADS").ok().and_then(|s| s.parse::<f64>().ok()) else { return false };
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs_f64(s));
            use std::sync::atomic::Ordering::Relaxed;
            let mut out = format!("sake threads at {} ms:", crate::k32::ticks_ms().wrapping_sub(60_000));
            for (k, (slots, at)) in RECENT.iter().zip(&RECENT_POS).enumerate() {
                let pos = kome::num::us(at.load(Relaxed));
                if pos == 0 {
                    continue;
                }
                out += &format!("\n  {:04x}:", k.saturating_mul(4).saturating_add(0x20));
                for n in pos.saturating_sub(RECENT_N)..pos {
                    let Some(slot) = slots.get(n % RECENT_N) else { continue };
                    let [f, a1, a2, a3, ret, ms] = slot.each_ref().map(|x| x.load(Relaxed));
                    out += &format!(" {}({a1:x},{a2:x},{a3:x})@{ret:x}/{ms}", name_of(trap_at(f)).unwrap_or_default());
                }
                // (waiting for a critical section: its LockCount, RecursionCount, OwningThread, LockSemaphore)
                if let Some(slot) = slots.get(pos.saturating_sub(1) % RECENT_N) {
                    let [f, cs, ..] = slot.each_ref().map(|x| x.load(Relaxed));
                    if name_of(trap_at(f)).is_some_and(|n| n.contains("EnterCriticalSection")) && cs >= 0x10000 {
                        out += &format!(" [cs {cs:x}: {:x} {:x} owner {:x} {:x}]", crate::mem::r32(crate::mem::ga(cs) + 4), crate::mem::r32(crate::mem::ga(cs) + 8), crate::mem::r32(crate::mem::ga(cs) + 12), crate::mem::r32(crate::mem::ga(cs) + 16));
                    }
                }
            }
            eprintln!("{out}");
            // SAKE_THREADS_STACKS=1: also each thread's registers and the code on its stack
            if std::env::var_os("SAKE_THREADS_STACKS").is_some() {
                eprintln!("{}", kura::crashlog::snapshot_text());
            }
        });
        true
    })
}

/// SAKE_TRACE_FILTER=a,b: trace only functions whose name contains one of these.
fn trace_match(i: u32) -> bool {
    // SAKE_TRACE_AFTER=<ms>: only from then on
    static AFTER: OnceLock<u32> = OnceLock::new();
    let after = *AFTER.get_or_init(|| std::env::var("SAKE_TRACE_AFTER").ok().and_then(|s| s.parse().ok()).unwrap_or(0));
    if after > 0 && crate::k32::ticks_ms().wrapping_sub(60_000) < after {
        return false;
    }
    static F: OnceLock<Vec<String>> = OnceLock::new();
    let f = F.get_or_init(|| std::env::var("SAKE_TRACE_FILTER").map(|s| s.split(',').map(String::from).collect()).unwrap_or_default());
    if f.is_empty() {
        return true;
    }
    static HIT: Mutex<Option<HashMap<u32, bool>>> = Mutex::new(None);
    let mut g = HIT.locked();
    *g.get_or_insert_with(HashMap::default).entry(i).or_insert_with(|| {
        let n = name_of(trap_at(i)).unwrap_or_default();
        f.iter().any(|x| n.contains(x.as_str()))
    })
}

thread_local! {
    static RING: std::cell::RefCell<std::collections::VecDeque<String>> = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Prints (and clears) this thread's last API calls (SAKE_TRACE=3).
pub fn dump_ring(why: &str) {
    RING.with(|r| {
        let Ok(mut r) = r.try_borrow_mut() else { return };
        if r.is_empty() {
            return;
        }
        eprintln!("sake: last API calls before {why}:");
        for l in r.drain(..) {
            eprintln!("  {l}");
        }
    });
}

/// (MASU_API_COUNTS: calls by trap index)
static USAGE: Mutex<Vec<u64>> = Mutex::new(Vec::new());

fn usage(i: u32) {
    {
        let mut u = USAGE.locked();
        if u.is_empty() {
            kura::usage::more(|| {
                let u = USAGE.locked().clone();
                let e = ENTRIES.r();
                u.iter()
                    .enumerate()
                    .filter(|x| *x.1 > 0)
                    .filter_map(|(i, &n)| {
                        let x = e.get(i)?;
                        let stub = x.f.is_none() || crate::com::is_stub(trap_at(kome::num::u32_of(i)));
                        Some((format!("{}:{}!{}", if stub { "stub" } else { "api" }, x.dll, x.name), n))
                    })
                    .collect()
            });
        }
        if u.len() <= kome::num::us(i) {
            u.resize(kome::num::us(i).saturating_add(1), 0);
        }
        if let Some(n) = u.get_mut(kome::num::us(i)) {
            *n = (*n).saturating_add(1);
        }
    }
    kura::usage::tick();
}

pub fn dispatch(cpu: &mut Cpu, addr: u32) {
    // (a process on one processor: the call lets it go, the program's code after takes it back)
    let _uni = crate::uni::Api::new();
    crate::comrpc::pump(cpu, false);
    let i = trap_no(addr);
    let Some((f, Meta { bytes, cdecl, known, this_arg, yields, value_check }, outs)) = hot(kome::num::us(i)) else {
        crate::fatal(cpu, &format!("jump to unknown trap address {addr:#x}"));
    };
    let esp = cpu.regs[ESP];
    let sw = switches();
    if sw & SW_DEBUG != 0 {
        debug_enter(cpu, addr, i, esp, bytes, sw);
    }
    crate::k32::check_terminate();
    let top = crate::snap::api_enter(addr, esp);
    let me = crate::cur();
    crate::obj::HANDOFF.entered(me.tid);
    let tid = me.tid;
    // (a call that may wait, or any while a thread is suspended: GetThreadContext from another
    // thread reads this copy of where it stands, as returned from the call)
    let api = (esp, if crate::mem::x64() { 8 } else { (if cdecl { 0 } else { bytes }).saturating_add(4) });
    let outer = crate::set_stopped((yields != 0 || crate::k32::SUSPENDS.any()).then(|| crate::Stopped { cpu: Box::new(cpu.clone()), api }));
    let this = (this_arg && !crate::mem::x64()).then(|| cpu.regs[koji::ECX]);
    let x64 = crate::mem::x64();
    let mut c = Ctx { cpu, esp, edx: None, jumped: false, st0: None, trap: addr, this, outs: if value_check { outs.of_arch(x64) } else { 0 } };
    // (the caller's pointer-sized slots watched while it runs, kura::outs: a 32-bit write
    // leaves half of one on x64, and is the same mistake on x86, where the tests run)
    let m = outs.of_arch(x64);
    let _slots = (m != 0).then(|| out_slots(&c, m));
    if yields == 0 || (yields == 2 && c.a(0) != 0) {
        crate::obj::HANDOFF.worked(tid);
    }
    let t0 = (sw & SW_STATS != 0).then(std::time::Instant::now);
    // (SAKE_SHIRUSU=1: the call for a shirusu Trace, crates/sake/src/shirusu.rs)
    let shirusu = if sw & SW_SHIRUSU != 0 {
        let x64 = crate::mem::x64();
        let n = if x64 { 12 } else { (bytes / 4).min(12) };
        let args: Vec<u64> = (0..n).map(|k| if x64 { c.a64(k) } else { u64::from(c.a(k)) }).collect();
        crate::shirusu::enter(i, &args)
    } else {
        None
    };
    let r = match f {
        Some(f) => f(&mut c),
        None => unimplemented_call(&mut c, i, known),
    };
    if let Some(t0) = t0 {
        let mut times = TIMES.locked();
        let t = times.get_or_insert_with(HashMap::default).entry(i).or_default();
        *t = t.saturating_add(kome::num::lo64(t0.elapsed().as_micros()));
    }
    if top {
        crate::snap::api_leave();
    }
    let _ = crate::set_stopped(outer);
    if let Some(o) = shirusu.filter(|_| !c.jumped) {
        let ret = match c.edx {
            Some(d) => u64::from(d) << 32 | u64::from(r),
            None if crate::mem::x64() && r >= 0xffff_f000 => kome::num::lo64(i64::from(r.cast_signed())),
            None if crate::mem::x64() => crate::mem::widen(r),
            None => u64::from(r),
        };
        crate::shirusu::leave(o, ret);
    }
    if c.jumped {
        crate::tctx::apply_pending(c.cpu);
        return;
    }
    let (edx, st0) = (c.edx, c.st0);
    if sw & SW_TRACE2 != 0 {
        eprintln!("sake {:04x}:   -> {r:x}", crate::cur().tid);
    }
    // (SAKE_TRACE=3: the results too, in the thread's ring)
    if sw & SW_TRACE != 0 && trace() == 3 && trace_match(i) {
        RING.with(|q| {
            if let Ok(mut q) = q.try_borrow_mut() {
                if q.len() >= 300 {
                    q.pop_front();
                }
                q.push_back(format!("  -> {r:x} ({})", name_of(addr).unwrap_or_default()));
            }
        });
    }
    finish(cpu, esp, r, edx, st0, cdecl, bytes);
}

/// The debug switches, read once (they are environment variables): one word for `dispatch`
/// instead of a OnceLock each (review C-1).
const SW_SET: u32 = 1 << 31;
const SW_USAGE: u32 = 1;
const SW_STATS: u32 = 2;
const SW_THREADS: u32 = 4;
const SW_WATCH: u32 = 8;
const SW_TRACE: u32 = 16;
const SW_TRACE2: u32 = 32;
const SW_SHIRUSU: u32 = 64;
/// (what `debug_enter` looks at)
const SW_DEBUG: u32 = SW_USAGE | SW_STATS | SW_THREADS | SW_WATCH | SW_TRACE;

fn switches() -> u32 {
    static SW: AtomicU32 = AtomicU32::new(0);
    let s = SW.load(Ordering::Relaxed);
    if s != 0 {
        return s;
    }
    let tr = trace();
    let s = SW_SET
        | if kura::usage::on() { SW_USAGE } else { 0 }
        | if stats_on() { SW_STATS } else { 0 }
        | if threads_on() { SW_THREADS } else { 0 }
        | if watch_addr() != 0 { SW_WATCH } else { 0 }
        | if tr > 0 { SW_TRACE } else { 0 }
        | if tr == 2 { SW_TRACE2 } else { 0 }
        | if crate::shirusu::on() { SW_SHIRUSU } else { 0 };
    SW.store(s, Ordering::Relaxed);
    s
}

/// SAKE_WATCH=<hex address>: its value printed whenever it changed before an API call
fn watch_addr() -> u32 {
    static W: OnceLock<u32> = OnceLock::new();
    *W.get_or_init(|| std::env::var("SAKE_WATCH").ok().and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok()).unwrap_or(0))
}

/// What the debug switches `sw` do before a call (out of `dispatch`'s way when none is on)
#[cold]
#[inline(never)]
fn debug_enter(cpu: &mut Cpu, addr: u32, i: u32, esp: u32, bytes: u32, sw: u32) {
    if sw & SW_USAGE != 0 {
        usage(i);
    }
    if sw & SW_STATS != 0 {
        let mut counts = COUNTS.locked();
        let n = counts.get_or_insert_with(HashMap::default).entry(i).or_default();
        *n = n.saturating_add(1);
    }
    if sw & SW_THREADS != 0 && let Some(k) = recent_slot(crate::cur().tid) {
        use std::sync::atomic::Ordering::Relaxed;
        if let (Some(slots), Some(at)) = (RECENT.get(k), RECENT_POS.get(k)) {
            let n = kome::num::us(at.load(Relaxed));
            let r = [i, crate::mem::r32(crate::mem::ga(esp) + 4), crate::mem::r32(crate::mem::ga(esp) + 8), crate::mem::r32(crate::mem::ga(esp) + 12), r32(esp), crate::k32::ticks_ms().wrapping_sub(60_000)];
            for (x, v) in slots.get(n % RECENT_N).into_iter().flatten().zip(r) {
                x.store(v, Relaxed);
            }
            at.store(kome::num::u32_of(n).saturating_add(1), Relaxed);
        }
    }
    if sw & SW_WATCH != 0 {
        static LAST: AtomicU32 = AtomicU32::new(0);
        let w = watch_addr();
        let v = r32(w);
        if LAST.swap(v, Ordering::Relaxed) != v {
            eprintln!("sake watch {w:#x} = {v:#x} before {}, from {:x}", name_of(addr).unwrap_or_default(), r32(esp));
        }
    }
    let tr = trace();
    if sw & SW_TRACE != 0 && trace_match(i) {
        let n = name_of(addr).unwrap_or_default();
        let x64 = crate::mem::x64();
        let nargs = if x64 { 4.max(bytes / 4).min(8) } else { (bytes / 4).min(8) };
        let args: Vec<String> = (0..nargs)
            .map(|k| if x64 { format!("{:x}", Ctx { cpu: &mut *cpu, esp, edx: None, jumped: false, st0: None, trap: addr, this: None, outs: 0 }.a64(k)) } else { format!("{:x}", crate::mem::r32((crate::mem::ga(esp) + 4).at(k, 4))) })
            .collect();
        let from = if x64 { crate::mem::r64(esp) } else { u64::from(r32(esp)) };
        let line = format!("sake {:04x} {}: {n}({}) from {:x}", crate::cur().tid, crate::k32::ticks_ms().wrapping_sub(60_000), args.join(","), from);
        if tr == 3 {
            RING.with(|r| {
                let mut r = r.borrow_mut();
                if r.len() >= 300 {
                    r.pop_front();
                }
                r.push_back(line);
            });
        } else {
            eprintln!("{line}");
        }
    }
}

/// A call to a function sake has no implementation of: its spec's result, said once.
#[cold]
#[inline(never)]
fn unimplemented_call(c: &mut Ctx, i: u32, known: bool) -> u32 {
    let n = name_of(c.trap).unwrap_or_default();
    if !known {
        crate::fatal(c.cpu, &format!("call to {n}, which we neither implement nor know the arguments of"));
    }
    static SEEN: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
    let r = unimplemented_result(&n);
    if SEEN.locked().get_or_insert_with(HashSet::default).insert(i) {
        eprintln!("sake: unimplemented {n} (returns {r:#x}), from {:#x}", r32(c.esp));
    }
    r
}

/// Returns from an API call to its caller with result `r` (+ EDX, ST0), popping what stdcall pops.
fn finish(cpu: &mut Cpu, esp: u32, r: u32, edx: Option<u32>, st0: Option<f64>, cdecl: bool, bytes: u32) {
    if crate::mem::x64() {
        // RAX: a 64-bit result (ret64), a small negative one (INVALID_HANDLE_VALUE,
        // pseudo-handles) sign-extended, an address in an image above 4G widened
        let rax = match edx {
            Some(d) => u64::from(d) << 32 | u64::from(r),
            None if r >= 0xffff_f000 => kome::num::lo64(i64::from(r.cast_signed())),
            None => crate::mem::widen(r),
        };
        cpu.set_gpr(EAX, rax);
        if let Some(v) = st0 {
            cpu.set_xmm(0, u128::from(v.to_bits()));
        }
        let ret = crate::mem::r64(esp);
        cpu.eip = kome::num::lo32(ret);
        cpu.rip_hi = kome::num::lo32(ret >> 32);
        cpu.set_gpr(ESP, u64::from(esp) + 8);
        crate::tctx::apply_pending_after_call(cpu);
        return;
    }
    cpu.regs[EAX] = r;
    if let Some(d) = edx {
        cpu.regs[EDX] = d;
    }
    if let Some(v) = st0 {
        cpu.fpu.push(v);
    }
    cpu.eip = r32(esp);
    cpu.regs[ESP] = (crate::mem::ga(esp) + 4 + if cdecl { 0 } else { bytes }).get();
    crate::tctx::apply_pending_after_call(cpu);
}

/// Writes a small x86 stub `code` into the stub area (mem::stub_area, 16-byte aligned);
/// its address. None once the area's 1 MiB is used up (sake's stubs and hooks take a few KB).
pub fn stub(code: &[u8]) -> Option<u32> {
    stub_in(code)
}
/// stub, for code with no way on without it: 0 when the area is full (the guest's call there
/// faults, an access violation its handlers see, where a panic ended the process)
pub fn stub_or_0(code: &[u8]) -> u32 {
    stub_in(code).unwrap_or(0)
}
fn stub_in(code: &[u8]) -> Option<u32> {
    static USED: Mutex<u32> = Mutex::new(0);
    let (base, size) = crate::mem::stub_area();
    let mut used = USED.locked();
    let n = u32::try_from(code.len()).ok()?.checked_add(15)? & !15;
    if n > size.saturating_sub(*used) {
        return None;
    }
    let a = (crate::mem::ga(base) + *used).get();
    crate::mem::write(a, code);
    *used = used.saturating_add(n);
    Some(a)
}

/// A guest "function" that is a trap: an address code can call (callbacks we hand out).
pub fn host_fn(name: &str, f: Handler, bytes: u32) -> u32 {
    implement("sake", name, f);
    let a = export("sake", name);
    let i = trap_no(a);
    let mut e = ENTRIES.w();
    // (export made entry i)
    if let Some(x) = e.get_mut(kome::num::us(i)) {
        // (made again with another handler: that one, as IMPLS has it now)
        x.f = Some(f);
        x.bytes = bytes;
        x.known = true;
        publish(kome::num::us(i), x);
    }
    let _ = { let x: fn(u32, u32) = w32; x };
    a
}

/// A host function called as a thiscall virtual function (x86: `this` in ECX is argument 0,
/// the callee pops `bytes` of stack arguments; x64: the plain convention).
pub fn host_fn_this(name: &str, f: Handler, bytes: u32) -> u32 {
    let a = host_fn(name, f, bytes);
    let i = kome::num::us(trap_no(a));
    let mut e = ENTRIES.w();
    if let Some(x) = e.get_mut(i) {
        x.this_arg = true;
        publish(i, x);
    }
    a
}

/// A COM vtable of traps (x64: 8-byte entries). `name` says the interface ("IDirectDraw7",
/// "VMR9::IPin", "MPEG-1 IPin", "IVideoWindow(windowless)"): where the metadata has its vtable
/// and this one is the start of it, each method gets its out-parameters (set_com_outs).
pub fn vtable(name: &str, methods: &[(Handler, u32)]) -> u32 {
    let k = crate::mem::ps();
    let v = crate::mem::heap_alloc(k.saturating_mul(kome::num::lo32(methods.len())), true);
    let iface = vtable_iface(name, methods.iter().map(|m| m.1));
    for (i, (f, bytes)) in methods.iter().enumerate() {
        let a = host_fn(&format!("{name}::{i}"), *f, *bytes);
        // (none: a vtable the metadata does not have, its methods' out-parameters not known)
        set_com_outs(a, iface.unwrap_or(""), kome::num::u32_of(i), ComArgs::Typed);
        crate::mem::wp(crate::mem::ga(v).at(kome::num::u32_of(i), k), a);
    }
    v
}

/// The interface of the metadata a vtable called `name` with methods taking `bytes` is: its
/// name's last word without a "(...)", when its methods take the stack bytes the metadata's
/// vtable of that interface has (past its end: a derived interface's methods) (a vtable of an older version, IXAudio2SourceVoice_28 say, has
/// none: its slots are not the metadata's).
fn vtable_iface(name: &str, bytes: impl ExactSizeIterator<Item = u32>) -> Option<&str> {
    let last = name.rsplit([':', ' ']).next()?;
    let n = last.split('(').next()?;
    let d = outs::com_dwords(n)?;
    let b: Vec<u32> = bytes.collect();
    let first_apart = b.iter().zip(d).position(|(&b, &w)| b != 4 * u32::from(w));
    if let Some(k) = first_apart {
        let what = format!("sake: vtable {name} is not the metadata's {n} from slot {k}: {:?} bytes against {:?} (another version: a name of its own, IXAudio2_27)", b.get(k), d.get(k).map(|&w| 4 * u32::from(w)));
        // (under test it fails: a vtable of sake's with the wrong stack bytes pops the wrong
        // amount on x86, IWMHeaderInfo3's and IWMProfile3's did)
        assert!(!cfg!(test), "{what}");
        if cfg!(debug_assertions) {
            eprintln!("{what}");
        }
        return None;
    }
    // (more methods than the metadata's: a derived interface's past its end, which have none)
    Some(n)
}

#[cfg(test)]
mod dll_key_tests {
    use super::dll_key;

    /// Module names as programs import or load them, and the DLL whose exports answer (on
    /// Windows the API sets resolve to these hosts; security.dll forwards to secur32)
    #[test]
    fn names_to_keys() {
        for (name, key) in [
            ("KERNEL32.dll", "kernel32"),
            ("C:\\Windows\\System32\\USER32.DLL", "user32"),
            ("winmm", "winmm"),
            ("wineoss.drv", "wineoss"),
            ("quartz.ax", "quartz"),
            ("comctl32.ocx", "comctl32"),
            ("api-ms-win-crt-runtime-l1-1-0.dll", "ucrtbase"),
            ("Security.dll", "secur32"),
            ("security", "secur32"),
            ("secur32.dll", "secur32"),
            ("api-ms-win-core-winrt-l1-1-0.dll", "combase"),
            ("api-ms-win-core-com-l1-1-0.dll", "ole32"),
            ("api-ms-win-core-registry-l1-1-0.dll", "advapi32"),
            ("api-ms-win-core-shlwapi-legacy-l1-1-0.dll", "shlwapi"),
            ("api-ms-win-core-synch-l1-2-0.dll", "kernelbase"),
            ("api-ms-win-appmodel-runtime-l1-1-2.dll", "kernelbase"),
            ("ext-ms-win-kernel32-package-current-l1-1-0.dll", "kernelbase"),
            ("api-ms-win-ntuser-sysparams-l1-1-0.dll", "user32"),
            ("ext-ms-win-rtcore-ntuser-window-l1-1-0.dll", "user32"),
            ("ext-ms-win-gdi-dc-l1-2-0.dll", "gdi32"),
            ("api-ms-win-security-base-l1-1-0.dll", "advapi32"),
            ("api-ms-win-shell-shellcom-l1-1-0.dll", "shell32"),
            ("api-ms-win-shcore-scaling-l1-1-1.dll", "shcore"),
        ] {
            assert_eq!(dll_key(name), key, "{name}");
        }
    }
}

#[cfg(test)]
mod spec_tests {
    use super::{forwards, specs};

    /// Every function sake implements under a DLL whose exports are all stdcall has its spec
    /// line (the bytes the callee pops): without one it binds as cdecl popping nothing, and a
    /// 32-bit caller's stack is off by the arguments after the call (Inno Setup's ISCC crashed
    /// so in BCryptVerifySignature). The LongPtr functions are x64's only (macros on x86); masu_*
    /// are sake's own, called without arguments.
    #[test]
    fn stdcall_implementations_have_specs() {
        let sp = specs();
        let mut all_stdcall: std::collections::HashMap<&str, bool> = std::collections::HashMap::new();
        for l in include_str!("specs.txt").lines() {
            let v: Vec<&str> = l.split(' ').collect();
            if v.len() >= 5 {
                *all_stdcall.entry(v[0]).or_insert(true) &= v[2] == "stdcall";
            }
        }
        let known = |d: &str, n: &str| {
            sp.by_name.contains_key(&(d, n))
                || forwards(d).iter().chain(&["kernel32", "kernelbase", "ntdll"]).any(|k| sp.by_name.contains_key(&(*k, n)))
        };
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
        let mut missing = Vec::new();
        for f in std::fs::read_dir(dir).unwrap() {
            let p = f.unwrap().path();
            if p.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let s = std::fs::read_to_string(&p).unwrap();
            let mut found: Vec<(String, String)> = Vec::new();
            // implement("dll", "Name", f)
            for part in s.split("implement(\"").skip(1) {
                let mut q = part.split('"');
                let (Some(d), Some(sep), Some(n)) = (q.next(), q.next(), q.next()) else { continue };
                if sep.trim() == "," {
                    found.push((d.to_string(), n.to_string()));
                }
            }
            // reg!("dll"; A, B, ...)
            for part in s.split("reg!(\"").skip(1) {
                let Some((d, rest)) = part.split_once('"') else { continue };
                let Some(body) = rest.trim_start().strip_prefix(';') else { continue };
                let body = body.split(')').next().unwrap_or("");
                for n in body.split(|ch: char| !(ch.is_alphanumeric() || ch == '_')).filter(|n| !n.is_empty()) {
                    found.push((d.to_string(), n.to_string()));
                }
            }
            for (d, n) in found {
                if all_stdcall.get(d.as_str()) == Some(&true) && !n.contains("LongPtr") && !n.starts_with("masu_") && !known(&d, &n) {
                    missing.push(format!("{d}!{n} ({})", p.file_name().unwrap().to_string_lossy()));
                }
            }
        }
        assert!(missing.is_empty(), "implemented without a spec line: {missing:?}");
    }
}

/// What an export sake knows but doesn't implement returns: 0, except D3DX's functions that
/// make an object, for which 0 is S_OK with the object pointer never written (ダイナマリサ3D
/// called through the NULL D3DXCreateCubeTextureFromFileInMemory left, #126): E_NOTIMPL, a
/// failure the game checks for.
pub fn unimplemented_result(name: &str) -> u32 {
    let (dll, f) = name.split_once('!').unwrap_or(("", name));
    let d3dx = dll.to_ascii_lowercase().starts_with("d3dx");
    if d3dx && ["D3DXCreate", "D3DXLoad", "D3DXCompile", "D3DXAssemble", "D3DXPreprocess", "D3DXGetShader", "D3DXSave", "D3DXFilter", "D3DXComputeNormalMap"].iter().any(|p| f.starts_with(p)) {
        0x8000_4001
    } else {
        0
    }
}

#[cfg(test)]
mod unimplemented_tests {
    use super::unimplemented_result;
    #[test]
    fn d3dx_object_makers_fail() {
        assert_eq!(unimplemented_result("d3dx9_33!D3DXCreateCubeTextureFromFileInMemory"), 0x8000_4001);
        assert_eq!(unimplemented_result("D3DX9_43!D3DXLoadMeshFromXInMemory"), 0x8000_4001);
        assert_eq!(unimplemented_result("d3dx8!D3DXCreateSprite"), 0x8000_4001);
        // (matrices, vectors, others: 0 as before)
        assert_eq!(unimplemented_result("d3dx9_33!D3DXMatrixReflect"), 0);
        assert_eq!(unimplemented_result("kernel32!CreateFileTransactedA"), 0);
        assert_eq!(unimplemented_result("comdlg32!GetFileTitleA"), 0);
    }
}

#[cfg(test)]
mod ordinal_tests {
    use super::{dll_key, ordinal_name, specs};
    use proptest::prelude::*;

    /// d3d12.dll's two numbered exports as Windows numbers them: UnityPlayer.dll 6000.3/6000.4
    /// delay-loads both by ordinal only, and a missing one is the helper's c06d007f, the
    /// player's crash at "Discovering subsystems" (dl_RJ01610703, dl_RJ01684055, dl_RJ01690707)
    #[test]
    fn d3d12_ordinals() {
        assert_eq!(ordinal_name("d3d12.dll", 101).as_deref(), Some("D3D12CreateDevice"));
        assert_eq!(ordinal_name("D3D12", 102).as_deref(), Some("D3D12GetDebugInterface"));
        assert_eq!(ordinal_name("d3d12", 103), None);
    }

    /// One ordinal names one export per DLL (a duplicate in specs.txt would make
    /// GetProcAddress by number depend on line order). One name may have several (MFC's do).
    #[test]
    fn ordinals_are_unique() {
        let mut by_ord = std::collections::HashMap::new();
        for l in include_str!("specs.txt").lines() {
            let v: Vec<&str> = l.split(' ').collect();
            let Some(o) = v.get(4).and_then(|o| o.parse::<u32>().ok()) else { continue };
            if let Some(prev) = by_ord.insert((v[0], o), v[1]) {
                assert_eq!(prev, v[1], "{}: ordinal {o} twice", v[0]);
            }
        }
    }

    fn numbered() -> Vec<(String, u32, String)> {
        let mut v: Vec<_> = specs().by_ord.iter().map(|((d, o), n)| (d.to_string(), *o, n.to_string())).collect();
        v.sort();
        v
    }

    proptest! {
        /// Any numbered export, asked for by number under any spelling of its DLL's name (case,
        /// .dll, a path), is the export of that name that the spec has
        #[test]
        fn ordinal_round_trip(i in any::<prop::sample::Index>(), upper in any::<bool>(), suffix in any::<bool>(), path in any::<bool>()) {
            let all = numbered();
            let (d, o, n) = &all[i.index(all.len())];
            let mut name = if upper { d.to_uppercase() } else { d.clone() };
            if suffix { name.push_str(".dll"); }
            if path { name = format!("C:\\Windows\\System32\\{name}"); }
            prop_assume!(dll_key(&name) == *d);
            let got = ordinal_name(&name, *o);
            prop_assert_eq!(got.as_deref(), Some(n.as_str()));
            prop_assert!(specs().by_name.contains_key(&(d.as_str(), n.as_str())));
        }
    }

    /// The imports by ordinal of real game DLLs (the import and the delay-load directories),
    /// from system DLLs sake has a spec for, all have a name: SAKE_IMPORT_DLLS=a.dll:b.dll
    /// (UnityPlayer.dll of 6000.0..6000.5 builds)
    #[test]
    #[ignore]
    fn real_imports_by_ordinal() {
        let Ok(list) = std::env::var("SAKE_IMPORT_DLLS") else { return };
        let mut missing = Vec::new();
        for path in list.split(':') {
            let f = std::fs::read(path).unwrap();
            let u32at = |o: usize| kome::num::us(u32::from_le_bytes(f[o..o + 4].try_into().unwrap()));
            let u64at = |o: usize| u64::from_le_bytes(f[o..o + 8].try_into().unwrap());
            let pe = u32at(0x3c);
            let wide = u16::from_le_bytes([f[pe + 24], f[pe + 25]]) == 0x20b;
            let (n, opt) = (kome::num::us(u16::from_le_bytes([f[pe + 6], f[pe + 7]])), kome::num::us(u16::from_le_bytes([f[pe + 20], f[pe + 21]])));
            let dirs = pe + 24 + if wide { 112 } else { 96 };
            let off = |rva: usize| {
                (0..n).map(|k| pe + 24 + opt + 40 * k).find_map(|s| {
                    let (va, size, ptr) = (u32at(s + 12), u32at(s + 16).max(u32at(s + 8)), u32at(s + 20));
                    (rva >= va && rva < va + size).then(|| rva - va + ptr)
                })
            };
            let cstr = |o: usize| String::from_utf8_lossy(&f[o..o + f[o..].iter().position(|&b| b == 0).unwrap()]).into_owned();
            // (import descriptor: 20 bytes, name at 12, thunks at 0 or 16; delay: 32 bytes,
            // name at 4, names table at 16)
            for (dir, size, name_at, thunks_at) in [(1usize, 20usize, 12usize, 0usize), (13, 32, 4, 16)] {
                let rva = u32at(dirs + 8 * dir);
                let Some(mut d) = (rva != 0).then(|| off(rva)).flatten() else { continue };
                loop {
                    let (name, mut t) = (u32at(d + name_at), u32at(d + thunks_at));
                    if name == 0 {
                        break;
                    }
                    if t == 0 {
                        t = u32at(d + 16);
                    }
                    let dll = cstr(off(name).unwrap());
                    let mut p = off(t).unwrap();
                    loop {
                        let (v, flag) = if wide { (u64at(p), 1u64 << 63) } else { (kome::num::lo64(u32at(p)), 1 << 31) };
                        if v == 0 {
                            break;
                        }
                        if v & flag != 0 && specs().dlls.contains(dll_key(&dll).as_str()) && ordinal_name(&dll, kome::num::lo32(v & 0xffff)).is_none() {
                            missing.push(format!("{path}: {dll} #{}", v & 0xffff));
                        }
                        p += if wide { 8 } else { 4 };
                    }
                    d += size;
                }
            }
        }
        assert!(missing.is_empty(), "imports by ordinal without a name: {missing:?}");
    }
}

/// The lock-free table `dispatch` reads (review C-1) says what ENTRIES says, whatever is
/// exported meanwhile on other threads.
#[cfg(test)]
mod hot_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn meta_round_trips(bytes in 0u32..=META_BYTES, cdecl in any::<bool>(), known in any::<bool>(), this_arg in any::<bool>(), yields in 0u8..3, value_check in any::<bool>()) {
            let m = Meta { bytes, cdecl, known, this_arg, yields, value_check };
            prop_assert_eq!(Meta::unpack(m.pack()), Some(m));
        }
        /// stubs: in the stub area, 16-byte aligned, apart, holding their code; too big: None
        #[test]
        fn stubs_land_in_the_stub_area(a in proptest::collection::vec(any::<u8>(), 1..64), b in proptest::collection::vec(any::<u8>(), 1..64)) {
            let _g = crate::test_guest::enter();
            let (x, y) = (stub(&a).unwrap(), stub(&b).unwrap());
            for (p, code) in [(x, &a), (y, &b)] {
                prop_assert!((crate::mem::STUB_LO..crate::mem::GUEST_TOP).contains(&p) && p % 16 == 0);
                prop_assert_eq!(&crate::mem::bytes(p, kome::num::u32_of(code.len())), &code[..]);
            }
            prop_assert!(y >= x + kome::num::u32_of(a.len()));
            let (base, r) = crate::mem::find(x).unwrap();
            prop_assert_eq!((base, r.size, r.state, r.prot), (crate::mem::STUB_LO, crate::mem::STUB_SIZE, 0x1000, 0x40));
            prop_assert_eq!(stub(&vec![0; 0x10_0001]), None);
            prop_assert_eq!(stub_or_0(&vec![0; 0x10_0001]), 0);
        }
        /// trap_at / trap_no are the old TRAP_LO + 4 * i and (a - TRAP_LO) / 4, wrapping
        #[test]
        fn trap_at_and_no_are_the_old_sums(i in any::<u32>(), a in any::<u32>()) {
            prop_assert_eq!(trap_at(i), TRAP_LO.wrapping_add(i.wrapping_mul(4)));
            prop_assert_eq!(trap_no(a), a.wrapping_sub(TRAP_LO) / 4);
            prop_assert_eq!(trap_no(trap_at(i)), i & 0x3fff_ffff);
        }
    }

    #[test]
    fn unset_slot_is_none() {
        assert_eq!(Meta::unpack(0), None);
        assert!(hot(MAX_ENTRIES).is_none());
        assert!(hot(MAX_ENTRIES - 1).is_none());
        assert_eq!(tag_of(TRAP_LO - 4), (0, 0));
        assert_eq!(tag_of(trap_at(kome::num::lo32(MAX_ENTRIES))), (0, 0));
    }

    fn h1(_: &mut Ctx) -> u32 {
        1
    }
    fn h2(_: &mut Ctx) -> u32 {
        2
    }

    /// Every entry's hot slot against the entry itself
    fn check_all() {
        let e = ENTRIES.r();
        for (i, x) in e.iter().enumerate() {
            let (f, m, _) = hot(i).expect("published");
            assert_eq!(m, Meta::of(x), "{}!{}", x.dll, x.name);
            assert!(match (f, x.f) { (Some(a), Some(b)) => std::ptr::fn_addr_eq(a, b), (None, None) => true, (Some(_), None) | (None, Some(_)) => false }, "{}!{}", x.dll, x.name);
        }
        // (nothing past them: an entry is published under the write lock)
        assert!(hot(e.len()).is_none());
    }

    #[test]
    fn host_fns_and_tags() {
        let a = host_fn("hot_tests_a", h1, 12);
        let b = host_fn_this("hot_tests_b", h2, 8);
        assert_eq!(args_of(a), (false, 12));
        assert!(is_known(a) && implemented(a));
        assert!(handler_of(b).is_some_and(|f| std::ptr::fn_addr_eq(f, handler(h2))));
        let (_, m, _) = hot(kome::num::us((b - TRAP_LO) / 4)).unwrap();
        assert!(m.this_arg);
        // (made again: the same trap, its bytes updated)
        assert_eq!(host_fn("hot_tests_a", h1, 16), a);
        assert_eq!(args_of(a), (false, 16));
        assert_eq!(tag_of(a), (0, 0));
        set_tag(a, 0x1234, 77);
        assert_eq!(tag_of(a), (0x1234, 77));
        assert_eq!(tag_of(b), (0, 0));
        // a function of a DLL's spec sake does not implement: known, no handler
        let s = export("kernel32", "hot_tests_no_such_function");
        assert!(!implemented(s) && !is_known(s));
        check_all();
    }

    /// Threads export new names while others read every published slot: a slot is either not
    /// there yet or whole (its handler and flags as the entry has them).
    #[test]
    fn concurrent_exports() {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut seen = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        // (a slot is published whole: its flags say it is set, its handler one
                        // of the program's or none)
                        for i in 0..MAX_ENTRIES {
                            let Some((_, m, _)) = hot(i) else { break };
                            assert!(m.bytes <= META_BYTES && m.yields < 3);
                            seen = seen.max(i);
                        }
                    }
                    seen
                })
            })
            .collect();
        let writers: Vec<_> = (0..2)
            .map(|w| {
                std::thread::spawn(move || {
                    for k in 0..3000 {
                        let a = host_fn(&format!("hot_tests_c{w}_{k}"), if k % 2 == 0 { h1 } else { h2 }, 4 * (k % 9));
                        let (f, m, _) = hot(kome::num::us(trap_no(a))).unwrap();
                        assert_eq!(m.bytes, 4 * (k % 9));
                        assert!(f.is_some_and(|f| std::ptr::fn_addr_eq(f, handler(if k % 2 == 0 { h1 } else { h2 }))));
                        set_tag(a, k + 1, kome::num::us(k));
                        assert_eq!(tag_of(a), (k + 1, kome::num::us(k)));
                    }
                })
            })
            .collect();
        for w in writers {
            w.join().unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        for r in readers {
            r.join().unwrap();
        }
        check_all();
    }
}

#[cfg(test)]
mod republish_tests {
    use super::*;

    fn r1(_: &mut Ctx) -> u32 {
        1
    }
    fn r2(_: &mut Ctx) -> u32 {
        2
    }

    /// Review 2026-11-16: one entry published again and again (host_fn updating its bytes)
    /// while readers take it: a reader sees one of the pairs that was published, never the
    /// handler of one with the flags of another; and publishing a pair again makes nothing new
    /// (the leak LeakSanitizer found: a Box per publish).
    #[test]
    fn a_republished_entry_is_read_whole() {
        let a = host_fn("republish_tests_a", r1, 4);
        let i = kome::num::us(trap_no(a));
        let pairs: [(Handler, u32); 2] = [(r1, 4), (r2, 8)];
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut n = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        let (f, m, _) = hot(i).expect("published");
                        let f = f.expect("a handler");
                        let k = usize::from(m.bytes == 8);
                        assert!(std::ptr::fn_addr_eq(f, pairs[k].0), "handler of one publish with the flags of another");
                        assert_eq!(m.bytes, pairs[k].1);
                        n += 1;
                    }
                    n
                })
            })
            .collect();
        let pairs_now = || PUBLISHED.locked().as_ref().map_or(0, |m| m.len());
        let mut after_two = 0;
        for k in 0..20_000usize {
            let (f, b) = pairs[k % 2];
            assert_eq!(host_fn("republish_tests_a", f, b), a);
            if k == 1 {
                after_two = pairs_now();
            }
        }
        stop.store(true, Ordering::Relaxed);
        for r in readers {
            assert!(r.join().unwrap() > 0);
        }
        // (two pairs published 20000 times: nothing new after the first of each; other tests'
        // exports may add theirs meanwhile, far fewer than 20000)
        let grown = pairs_now().saturating_sub(after_two);
        assert!(grown < 1000, "{grown} pairs made by publishing two again and again");
    }
}

#[cfg(test)]
mod export_once_tests {
    use proptest::prelude::*;

    fn cancel() -> u32 {
        crate::export_once!("kernel32", "CancelIoEx")
    }

    /// The address a call site keeps is the one export gives, from any thread, every time,
    /// whatever was exported in between (spellings of the DLL included).
    #[test]
    fn a_kept_address_is_exports() {
        let a = cancel();
        assert_eq!(a, super::export("kernel32", "CancelIoEx"));
        assert_eq!(a, super::export("KERNEL32.DLL", "CancelIoEx"));
        let got: Vec<u32> = (0..8).map(|_| std::thread::spawn(cancel)).map(|t| t.join().unwrap()).collect();
        assert!(got.iter().all(|&g| g == a));
    }

    proptest! {
        #[test]
        fn exports_are_stable(names in prop::collection::vec("[A-Z][a-z]{1,8}", 1..20)) {
            let a = cancel();
            let first: Vec<u32> = names.iter().map(|n| super::export("sakeprop", n)).collect();
            prop_assert_eq!(cancel(), a);
            prop_assert_eq!(super::export("kernel32", "CancelIoEx"), a);
            let again: Vec<u32> = names.iter().map(|n| super::export("SAKEPROP.dll", n)).collect();
            prop_assert_eq!(first, again);
        }
    }
}
