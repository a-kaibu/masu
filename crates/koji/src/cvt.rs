//! Integer conversions that say what they do, instead of `as`: `lo32(v)` keeps the low 32 bits
//! of any integer (two's complement for signed ones: a narrower signed value sign-extends, as
//! `as` does), `us(v)` is an index from a u32/u64 guest value. `T::cast_signed` /
//! `cast_unsigned` (std) reinterpret at the same width, `From` widens. A lo16 where a lo32 was
//! meant gives a u16, which does not type-check where a u32 is needed. Each is one byte pick or
//! extension per type (no u128 in between: they compile to what the `as` did).

/// A primitive integer: its low 8/16/32/64 bits.
pub trait Int: Copy {
    fn lo8(self) -> u8;
    fn lo16(self) -> u16;
    fn lo32(self) -> u32;
    fn lo64(self) -> u64;
}

impl Int for u8 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        u16::from(self)
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        u32::from(self)
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        u64::from(self)
    }
}

impl Int for u16 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        u32::from(self)
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        u64::from(self)
    }
}

impl Int for u32 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        let b = self.to_le_bytes();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        u64::from(self)
    }
}

impl Int for u64 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        let b = self.to_le_bytes();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        let b = self.to_le_bytes();
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

impl Int for u128 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        let b = self.to_le_bytes();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        let b = self.to_le_bytes();
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

impl Int for i8 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        i16::from(self).cast_unsigned()
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        i32::from(self).cast_unsigned()
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        i64::from(self).cast_unsigned()
    }
}

impl Int for i16 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        i32::from(self).cast_unsigned()
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        i64::from(self).cast_unsigned()
    }
}

impl Int for i32 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        let b = self.to_le_bytes();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        i64::from(self).cast_unsigned()
    }
}

impl Int for i64 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        let b = self.to_le_bytes();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        let b = self.to_le_bytes();
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

impl Int for i128 {
    #[inline(always)]
    fn lo8(self) -> u8 {
        let b = self.to_le_bytes();
        u8::from_le_bytes([b[0]])
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        let b = self.to_le_bytes();
        u16::from_le_bytes([b[0], b[1]])
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        let b = self.to_le_bytes();
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        let b = self.to_le_bytes();
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

impl Int for usize {
    #[inline(always)]
    fn lo8(self) -> u8 {
        Ptr::from_le_bytes(self.to_le_bytes()).lo8()
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        Ptr::from_le_bytes(self.to_le_bytes()).lo16()
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        Ptr::from_le_bytes(self.to_le_bytes()).lo32()
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        Ptr::from_le_bytes(self.to_le_bytes()).lo64()
    }
}

impl Int for isize {
    #[inline(always)]
    fn lo8(self) -> u8 {
        SPtr::from_le_bytes(self.to_le_bytes()).lo8()
    }
    #[inline(always)]
    fn lo16(self) -> u16 {
        SPtr::from_le_bytes(self.to_le_bytes()).lo16()
    }
    #[inline(always)]
    fn lo32(self) -> u32 {
        SPtr::from_le_bytes(self.to_le_bytes()).lo32()
    }
    #[inline(always)]
    fn lo64(self) -> u64 {
        SPtr::from_le_bytes(self.to_le_bytes()).lo64()
    }
}

// (usize/isize as the fixed-width type of their size)
#[cfg(target_pointer_width = "64")]
type Ptr = u64;
#[cfg(target_pointer_width = "64")]
type SPtr = i64;
#[cfg(target_pointer_width = "32")]
type Ptr = u32;
#[cfg(target_pointer_width = "32")]
type SPtr = i32;

/// The low 8 bits.
#[inline(always)]
pub fn lo8(v: impl Int) -> u8 {
    v.lo8()
}

/// The low 16 bits.
#[inline(always)]
pub fn lo16(v: impl Int) -> u16 {
    v.lo16()
}

/// The low 32 bits.
#[inline(always)]
pub fn lo32(v: impl Int) -> u32 {
    v.lo32()
}

/// The low 64 bits.
#[inline(always)]
pub fn lo64(v: impl Int) -> u64 {
    v.lo64()
}

/// The low bits that fit a usize (all of a u32's: usize is at least 32 bits, see below).
#[inline(always)]
pub fn us(v: impl Int) -> usize {
    #[cfg(target_pointer_width = "64")]
    let w = usize::from_le_bytes(v.lo64().to_le_bytes());
    #[cfg(target_pointer_width = "32")]
    let w = usize::from_le_bytes(v.lo32().to_le_bytes());
    w
}

// (`us` of a u32 never drops bits: koji runs on 32- and 64-bit targets)
const _: () = assert!(usize::BITS >= 32);
