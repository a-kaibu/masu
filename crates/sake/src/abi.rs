//! Windows' pointer-sized integers (WPARAM, LPARAM, LRESULT, LONG_PTR) as values of their own.
//!
//! WPARAM is UINT_PTR, LPARAM and LRESULT are LONG_PTR: 64 bits in a 64-bit process. sake kept
//! them as u32 (review 6852be1f: PostMessage read wParam/lParam with `Ctx::a`, the queue held
//! u32, and an application message's wParam 0x1122334455667788 came back as 0x55667788). Each
//! type keeps the whole value; one is made from a guest argument, a guest result or a named
//! kind of 32-bit value (`dword`: a WORD/DWORD/UINT, zero-extended as MAKEWPARAM/MAKELPARAM do;
//! `int`: an INT/LONG, sign-extended; `addr`: a guest address, its upper half restored as
//! mem::widen does; `handle`: a HANDLE/HWND, sign-extended as Windows sign-extends 32-bit
//! handles), never from a bare u32 (no From<u32>). The low 32 bits are taken by name (`lo32`),
//! for LOWORD/HIWORD-style readers and for x86's 4-byte slots.
//!
//! x86 values are kept canonically: WPARAM zero-extended, LPARAM/LRESULT sign-extended from
//! their 32 bits, so a value read from an x86 guest and written back is the same 4 bytes.

/// Which width the guest uses: x64's 8-byte slots or x86's 4-byte ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
}

impl Arch {
    pub fn current() -> Arch {
        if crate::mem::x64() { Arch::X64 } else { Arch::X86 }
    }
}

/// How a handle of 32 significant bits widens: sign-extended (INVALID_HANDLE_VALUE, HWND_TOPMOST
/// and the pseudo-handles stay negative, as Windows' HandleToLong/LongToHandle make them).
fn handle_bits(h: u32) -> i64 {
    i64::from(h.cast_signed())
}

macro_rules! ptr_int {
    ($(#[$m:meta])* $name:ident, $canon:expr) => {
        $(#[$m])*
        #[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
        pub struct $name(u64);

        /// Its bits, as the u32/u64 it replaced printed (state dumps and goldens read them)
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Debug::fmt(&self.0, f)
            }
        }

        impl $name {
            pub const ZERO: $name = $name(0);
            /// A WORD/DWORD/UINT value (zero-extended)
            pub fn dword(v: u32) -> $name {
                $name(u64::from(v))
            }
            /// An INT/LONG value (sign-extended)
            pub fn int(v: i32) -> $name {
                $name(i64::from(v).cast_unsigned())
            }
            /// The 32 bits of a LONG (sign-extended): a bit field the system makes as a LONG
            /// (a key message's lParam, MAKELONG coordinates of queued mouse input)
            pub fn long(v: u32) -> $name {
                $name::int(v.cast_signed())
            }
            /// A guest address (an address in an image above 4G gets its upper half back)
            pub fn addr(a: u32) -> $name {
                $name::from_bits(crate::mem::widen(a))
            }
            /// A HANDLE / HWND (sign-extended)
            pub fn handle(h: u32) -> $name {
                $name::from_bits(handle_bits(h).cast_unsigned())
            }
            /// The value as `arch` keeps it from the 64 bits of a guest register or slot: x86's
            /// upper half is not the value's
            pub fn from_guest(bits: u64, arch: Arch) -> $name {
                match arch {
                    Arch::X64 => $name::from_bits(bits),
                    Arch::X86 => $canon(kome::num::lo32(bits)),
                }
            }
            pub const fn from_bits(bits: u64) -> $name {
                $name(bits)
            }
            /// All 64 bits (what x64 passes and keeps)
            pub const fn bits(self) -> u64 {
                self.0
            }
            /// The low 32 bits: an x86 slot, or a reader of LOWORD/HIWORD
            pub fn lo32(self) -> u32 {
                kome::num::lo32(self.bits())
            }
            /// What a guest of `arch` reads back: x64 the 64 bits, x86 the low 32 zero-extended
            pub fn guest_bits(self, arch: Arch) -> u64 {
                match arch {
                    Arch::X64 => self.bits(),
                    Arch::X86 => u64::from(self.lo32()),
                }
            }
        }
    };
}

ptr_int!(
    /// WPARAM (UINT_PTR)
    Wparam,
    Wparam::dword
);
ptr_int!(
    /// LPARAM (LONG_PTR)
    Lparam,
    |v: u32| Lparam::int(v.cast_signed())
);
ptr_int!(
    /// LRESULT (LONG_PTR): a window procedure's or a hook's answer
    Lresult,
    |v: u32| Lresult::int(v.cast_signed())
);
ptr_int!(
    /// LONG_PTR / ULONG_PTR kept by Get/SetWindowLongPtr and Get/SetClassLongPtr
    LongPtr,
    |v: u32| LongPtr::int(v.cast_signed())
);

/// MSG's layout: (size, wParam's offset, lParam's offset, time's offset). x64 aligns wParam
/// to 8 after HWND (8) and UINT (4): hwnd 0, message 8, wParam 16, lParam 24, time 32, pt 36,
/// lPrivate 44, 48 bytes; x86 7 dwords, 28 bytes (winuser.h).
pub const fn msg_layout(arch: Arch) -> MsgLayout {
    match arch {
        Arch::X64 => MsgLayout { size: 48, message: 8, wparam: 16, lparam: 24, time: 32, pt: 36, private: Some(44) },
        Arch::X86 => MsgLayout { size: 28, message: 4, wparam: 8, lparam: 12, time: 16, pt: 20, private: None },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsgLayout {
    pub size: u32,
    pub message: u32,
    pub wparam: u32,
    pub lparam: u32,
    pub time: u32,
    pub pt: u32,
    /// lPrivate (x64 only: x86's MSG ends after pt)
    pub private: Option<u32>,
}

// (the layouts against winuser.h's sizes, at compile time)
const _: () = assert!(msg_layout(Arch::X64).size == 48 && msg_layout(Arch::X86).size == 28);
const _: () = assert!(msg_layout(Arch::X64).wparam % 8 == 0 && msg_layout(Arch::X64).lparam % 8 == 0);

/// An offset (or a size) in a Windows structure on each architecture: from the headers
/// (layouts, generated by build.rs from crates/apitrace/src/layouts.txt), never written by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Off {
    x86: u32,
    x64: u32,
}

impl Off {
    pub const fn new(x86: u32, x64: u32) -> Off {
        Off { x86, x64 }
    }
    pub const fn at(self, arch: Arch) -> u32 {
        match arch {
            Arch::X86 => self.x86,
            Arch::X64 => self.x64,
        }
    }
}

/// The headers' structures: `layouts::DXGI_SWAP_CHAIN_DESC::OutputWindow` and the like.
#[allow(clippy::all, clippy::pedantic, clippy::restriction)]
pub mod layouts {
    include!(concat!(env!("OUT_DIR"), "/layouts.rs"));
}

/// Writes a guest pointer-sized slot: x64 8 bytes, x86 the low 4.
pub fn write_slot(at: crate::mem::Ga, v: u64, arch: Arch) {
    match arch {
        Arch::X64 => crate::mem::w64(at, v),
        Arch::X86 => crate::mem::w32(at, kome::num::lo32(v)),
    }
}

/// Reads a guest pointer-sized slot (x86: zero-extended; the types' from_guest canonicalize).
pub fn read_slot(at: crate::mem::Ga, arch: Arch) -> u64 {
    match arch {
        Arch::X64 => crate::mem::r64(at),
        Arch::X86 => u64::from(crate::mem::r32(at)),
    }
}

impl crate::api::Ctx<'_> {
    /// Argument `i` as a WPARAM (x64: all 64 bits)
    pub fn wparam(&self, i: u32) -> Wparam {
        Wparam::from_guest(self.z(i), Arch::current())
    }
    /// Argument `i` as an LPARAM (x64: all 64 bits; x86: sign-extended)
    pub fn lparam(&self, i: u32) -> Lparam {
        Lparam::from_guest(self.z(i), Arch::current())
    }
    /// Argument `i` as a LONG_PTR (x64: all 64 bits; x86: sign-extended)
    pub fn long_ptr(&self, i: u32) -> LongPtr {
        LongPtr::from_guest(self.z(i), Arch::current())
    }
    /// Returns an LRESULT: x64 in all of RAX, x86 in EAX
    pub fn ret_lresult(&mut self, r: Lresult) -> u32 {
        self.ret_bits(r.bits())
    }
    /// Returns a LONG_PTR: x64 in all of RAX, x86 in EAX
    pub fn ret_long_ptr(&mut self, r: LongPtr) -> u32 {
        self.ret_bits(r.bits())
    }
    fn ret_bits(&mut self, bits: u64) -> u32 {
        match Arch::current() {
            Arch::X64 => self.ret64(bits),
            Arch::X86 => kome::num::lo32(bits),
        }
    }
}

ptr_int!(
    /// One argument of a guest call, at the width the callee takes it (x64: a whole register or
    /// stack slot; x86: its 4 bytes). Made by kind, as the other types are, or from one of them.
    Arg,
    Arg::dword
);

impl From<Wparam> for Arg {
    fn from(v: Wparam) -> Arg {
        Arg::from_bits(v.bits())
    }
}
impl From<Lparam> for Arg {
    fn from(v: Lparam) -> Arg {
        Arg::from_bits(v.bits())
    }
}
impl From<LongPtr> for Arg {
    fn from(v: LongPtr) -> Arg {
        Arg::from_bits(v.bits())
    }
}

/// Calls guest code at `f` (stdcall/cdecl on x86, the x64 convention) with `args` at their
/// widths; the result as the callee left it (x64: RAX; x86: EAX, zero-extended), for the
/// caller to take as the type the callback returns.
pub fn call(cpu: &mut koji::Cpu, f: u32, args: &[Arg]) -> u64 {
    match Arch::current() {
        Arch::X64 => {
            let a: Vec<u64> = args.iter().map(|a| a.bits()).collect();
            crate::call64(cpu, crate::mem::widen(f), &a)
        }
        Arch::X86 => {
            let a: Vec<u32> = args.iter().map(|a| a.lo32()).collect();
            u64::from(crate::call(cpu, f, &a))
        }
    }
}

/// Calls a window procedure (or a hook, a dialog procedure: any WNDPROC-shaped callback) at
/// `proc_` with its four arguments at full width; its LRESULT at full width.
pub fn call_wndproc(cpu: &mut koji::Cpu, proc_: u32, hwnd: u32, msg: u32, w: Wparam, l: Lparam) -> Lresult {
    let r = call(cpu, proc_, &[Arg::handle(hwnd), Arg::dword(msg), w.into(), l.into()]);
    Lresult::from_guest(r, Arch::current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// x64 keeps every bit; x86 keeps the low 32, LPARAM/LRESULT sign-extended, WPARAM not
        #[test]
        fn values_keep_their_width(v: u64) {
            prop_assert_eq!(Wparam::from_guest(v, Arch::X64).bits(), v);
            prop_assert_eq!(Lparam::from_guest(v, Arch::X64).bits(), v);
            prop_assert_eq!(Lresult::from_guest(v, Arch::X64).bits(), v);
            let lo = kome::num::lo32(v);
            prop_assert_eq!(Wparam::from_guest(v, Arch::X86).bits(), u64::from(lo));
            prop_assert_eq!(Lparam::from_guest(v, Arch::X86).bits(), i64::from(lo.cast_signed()).cast_unsigned());
            prop_assert_eq!(Lparam::from_guest(v, Arch::X86).guest_bits(Arch::X86), u64::from(lo));
            prop_assert_eq!(Lresult::from_guest(v, Arch::X86).lo32(), lo);
        }

        /// A 32-bit value's kinds widen as Windows' macros do
        #[test]
        fn dword_int_handle(v: u32) {
            prop_assert_eq!(Wparam::dword(v).bits(), u64::from(v));
            prop_assert_eq!(Lparam::dword(v).bits(), u64::from(v));
            prop_assert_eq!(Lparam::int(v.cast_signed()).bits(), i64::from(v.cast_signed()).cast_unsigned());
            prop_assert_eq!(Lparam::handle(v), Lparam::int(v.cast_signed()));
            prop_assert_eq!(Wparam::dword(v).lo32(), v);
        }
    }

    #[test]
    fn the_reviews_value_survives_x64() {
        let w = Wparam::from_guest(0x1122_3344_5566_7788, Arch::X64);
        assert_eq!(w.bits(), 0x1122_3344_5566_7788);
        assert_ne!(w.bits(), u64::from(w.lo32()));
        assert_eq!(Lparam::from_guest(0xffff_ffff_ffff_fffe, Arch::X64), Lparam::int(-2));
    }

    #[test]
    fn msg_layout_is_winusers() {
        let x = msg_layout(Arch::X64);
        assert_eq!((x.message, x.wparam, x.lparam, x.time, x.pt, x.private), (8, 16, 24, 32, 36, Some(44)));
        let y = msg_layout(Arch::X86);
        assert_eq!((y.message, y.wparam, y.lparam, y.time, y.pt, y.private), (4, 8, 12, 16, 20, None));
    }

    /// The header layouts the D2D HWND render target uses, at both widths (clang's record
    /// layouts of the mingw headers): the HWND and OutputWindow are pointers, so x64 moves
    /// everything after them.
    #[test]
    fn d2d_swap_chain_layouts() {
        use layouts::{D2D1_HWND_RENDER_TARGET_PROPERTIES as P, DXGI_SWAP_CHAIN_DESC as D};
        let both = |o: Off| (o.at(Arch::X86), o.at(Arch::X64));
        assert_eq!(both(P::SIZE), (16, 24));
        assert_eq!((both(P::hwnd), both(P::pixelSize_width), both(P::pixelSize_height), both(P::presentOptions)), ((0, 0), (4, 8), (8, 12), (12, 16)));
        assert_eq!(both(D::SIZE), (60, 72));
        assert_eq!((both(D::BufferDesc_Width), both(D::BufferDesc_Height), both(D::BufferDesc_Format)), ((0, 0), (4, 4), (16, 16)));
        assert_eq!((both(D::SampleDesc_Count), both(D::BufferUsage), both(D::BufferCount)), ((28, 28), (36, 36), (40, 40)));
        assert_eq!((both(D::OutputWindow), both(D::Windowed), both(D::SwapEffect), both(D::Flags)), ((44, 48), (48, 56), (52, 60), (56, 64)));
    }
}
