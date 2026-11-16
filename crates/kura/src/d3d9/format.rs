//! D3DFORMATs: their memory layout, and what the page keeps them as.
//!
//! Color textures are all BGRA8 on the page (converted when uploaded), except float formats.
//!
//! The formats kura knows are `Format`, and every table is an exhaustive match on it: a format
//! added to `formats!` does not compile until each table says what it is. The guest's numbers
//! stay u32 constants (the D3DFORMAT values); the `pub fn`s take those and answer for an unknown
//! one as they always did (format_golden_tests pins every answer).

use kome::num::*;
use super::gpu::{FMT_BGRA8, FMT_DEPTH24S8, FMT_RGBA16F, FMT_RGBA32F};

macro_rules! formats {
    ($($name:ident = $v:expr,)*) => {
        $(pub const $name: u32 = $v;)*
        /// A D3DFORMAT kura knows.
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Format {
            $($name,)*
        }
        impl Format {
            /// The guest's D3DFORMAT number as a known format.
            pub const fn from_raw(v: u32) -> Option<Format> {
                match v {
                    $($name => Some(Format::$name),)*
                    _ => None,
                }
            }
            /// Its D3DFORMAT number.
            pub const fn raw(self) -> u32 {
                match self {
                    $(Format::$name => $name,)*
                }
            }
        }
    };
}

formats! {
    R8G8B8 = 20,
    A8R8G8B8 = 21,
    X8R8G8B8 = 22,
    R5G6B5 = 23,
    X1R5G5B5 = 24,
    A1R5G5B5 = 25,
    A4R4G4B4 = 26,
    A8 = 28,
    X4R4G4B4 = 30,
    A8B8G8R8 = 32,
    X8B8G8R8 = 33,
    L8 = 50,
    A8L8 = 51,
    A4L4 = 52,
    P8 = 41,
    D16_LOCKABLE = 70,
    D32 = 71,
    D15S1 = 73,
    D24S8 = 75,
    D24X8 = 77,
    D24X4S4 = 79,
    D16 = 80,
    D32F_LOCKABLE = 82,
    D24FS8 = 83,
    INDEX16 = 101,
    INDEX32 = 102,
    A2B10G10R10 = 31,
    G16R16 = 34,
    A2R10G10B10 = 35,
    A16B16G16R16 = 36,
    V8U8 = 60,
    L6V5U5 = 61,
    X8L8V8U8 = 62,
    Q8W8V8U8 = 63,
    V16U16 = 64,
    A2W10V10U10 = 67,
    L16 = 81,
    Q16W16V16U16 = 110,
    R16F = 111,
    G16R16F = 112,
    R32F = 114,
    G32R32F = 115,
    A16B16G16R16F = 113,
    A32B32G32R32F = 116,
    DXT1 = u32::from_le_bytes(*b"DXT1"),
    DXT2 = u32::from_le_bytes(*b"DXT2"),
    DXT3 = u32::from_le_bytes(*b"DXT3"),
    DXT4 = u32::from_le_bytes(*b"DXT4"),
    DXT5 = u32::from_le_bytes(*b"DXT5"),
}

// (every u32 is a usize where kura runs: wasm32 and 64-bit hosts)
const _: () = assert!(usize::BITS >= u32::BITS);

/// A float rounded to a whole number as that number: NaN 0, out of range the nearest end (what
/// `as` does; the only float-to-integer step here).
fn whole(v: f32) -> i64 {
    v as i64
}

impl Format {
    /// (bytes per block, block size in pixels); None for the index buffer formats.
    pub const fn layout(self) -> Option<(u32, u32)> {
        use Format::*;
        Some(match self {
            A8R8G8B8 | X8R8G8B8 | A8B8G8R8 | X8B8G8R8 | D32 | D24S8 | D24X8 | D24X4S4 | D32F_LOCKABLE | D24FS8 => (4, 1),
            A2B10G10R10 | A2R10G10B10 | G16R16 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | G16R16F | R32F => (4, 1),
            V8U8 | L6V5U5 | L16 | R16F => (2, 1),
            A16B16G16R16 | Q16W16V16U16 | G32R32F => (8, 1),
            R8G8B8 => (3, 1),
            R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | X4R4G4B4 | A8L8 | D16 | D16_LOCKABLE | D15S1 => (2, 1),
            A8 | L8 | A4L4 | P8 => (1, 1),
            A16B16G16R16F => (8, 1),
            A32B32G32R32F => (16, 1),
            DXT1 => (8, 4),
            DXT2 | DXT3 | DXT4 | DXT5 => (16, 4),
            INDEX16 | INDEX32 => return None,
        })
    }

    /// Whether Windows has textures of it (and kura a layout).
    pub const fn supported(self) -> bool {
        use Format::*;
        // (Windows has no X8L8V8U8, L6V5U5 and A2W10V10U10 textures: tests/winref/golden/d3d9-32)
        match self {
            X8L8V8U8 | L6V5U5 | A2W10V10U10 => false,
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | Q8W8V8U8 | V16U16 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => self.layout().is_some(),
        }
    }

    /// Depth formats: none, without stencil bits, with them.
    pub const fn depth(self) -> Depth {
        use Format::*;
        match self {
            D16_LOCKABLE | D32 | D24X8 | D16 | D32F_LOCKABLE => Depth::Plain,
            D15S1 | D24S8 | D24X4S4 | D24FS8 => Depth::Stencil,
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => Depth::None,
        }
    }

    /// Formats a render target can have.
    pub const fn renderable(self) -> bool {
        use Format::*;
        // (and those Windows answers for: tests/winref/golden/d3d9-32, d3d9rtfmt-32: X8B8G8R8 is
        // not one, the 10- and 16-bit ones, L16, Q8W8V8U8 and the 16-bit floats and G32R32F are)
        match self {
            A8R8G8B8 | X8R8G8B8 | A8B8G8R8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A16B16G16R16F | A32B32G32R32F | A4R4G4B4 | A8 | L8 | A8L8 | A2R10G10B10 | A2B10G10R10 | G16R16 | A16B16G16R16 | L16 | R16F | G16R16F | R32F | G32R32F | V8U8 | Q8W8V8U8 => true,
            R8G8B8 | X4R4G4B4 | X8B8G8R8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | L6V5U5 | X8L8V8U8 | V16U16 | A2W10V10U10 | Q16W16V16U16 | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => false,
        }
    }

    /// What the page keeps it as (gpu::FMT_*).
    pub const fn page_format(self) -> u32 {
        use Format::*;
        match self {
            D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 => FMT_DEPTH24S8,
            A16B16G16R16F => FMT_RGBA16F,
            A32B32G32R32F => FMT_RGBA32F,
            A2B10G10R10 | A2R10G10B10 | G16R16 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F => match self.wide() {
                Some(p) => p,
                None => FMT_BGRA8,
            },
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | INDEX16 | INDEX32 | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => FMT_BGRA8,
        }
    }

    /// Formats of more than 8 bits a channel, signed ones, and those with missing channels: the
    /// page format they become (their texels as floats: 16-bit floats when those hold every value,
    /// 32-bit ones otherwise; SNORM's k / 127 too: in 16 bits 64 / 127 is 0.50390625, which
    /// shows as 0x80 where Windows' 0.50394 shows as 0x81, winref d3d9rtfmt V8U8).
    const fn wide(self) -> Option<u32> {
        use Format::*;
        Some(match self {
            A2B10G10R10 | A2R10G10B10 | R16F | G16R16F => FMT_RGBA16F,
            V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | A2W10V10U10 => FMT_RGBA32F,
            G16R16 | A16B16G16R16 | V16U16 | L16 | Q16W16V16U16 | R32F | G32R32F => FMT_RGBA32F,
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => return None,
        })
    }

    /// target_channels
    const fn target_channels(self) -> (u32, bool) {
        use Format::*;
        match self {
            A8 => (8, false),
            L8 | L16 => (1, true),
            A8L8 => (9, true),
            R16F | R32F => (1, false),
            G16R16 | G16R16F | G32R32F | V8U8 | V16U16 => (3, false),
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | A2R10G10B10 | A16B16G16R16 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | A2W10V10U10 | Q16W16V16U16 | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => (0xf, false),
        }
    }

    /// target_steps
    const fn target_steps(self) -> [u32; 4] {
        use Format::*;
        match self {
            R5G6B5 => [31, 63, 31, 0],
            X1R5G5B5 => [31, 31, 31, 0],
            A1R5G5B5 => [31, 31, 31, 1],
            A4R4G4B4 => [15, 15, 15, 15],
            X4R4G4B4 => [15, 15, 15, 0],
            A2R10G10B10 | A2B10G10R10 => [1023, 1023, 1023, 3],
            G16R16 => [65535, 65535, 0, 0],
            A16B16G16R16 => [65535; 4],
            // (SNORM, -1 to 1: bit 31; Windows samples 0x80 drawn to them as 0x81, 64 / 127)
            V8U8 => [SIGNED | 127, SIGNED | 127, 0, 0],
            Q8W8V8U8 => [SIGNED | 127; 4],
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | A8 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | L6V5U5 | X8L8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => [0; 4],
        }
    }

    /// target_x
    const fn target_x(self) -> bool {
        use Format::*;
        match self {
            X8R8G8B8 | X1R5G5B5 | X4R4G4B4 | R5G6B5 => true,
            R8G8B8 | A8R8G8B8 | A1R5G5B5 | A4R4G4B4 | A8 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => false,
        }
    }

    /// no_alpha
    const fn no_alpha(self) -> bool {
        use Format::*;
        match self {
            X8R8G8B8 | X8B8G8R8 | R5G6B5 | X1R5G5B5 | X4R4G4B4 | R8G8B8 | L8 => true,
            A8R8G8B8 | A1R5G5B5 | A4R4G4B4 | A8 | A8B8G8R8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => false,
        }
    }

    /// same_on_page
    const fn same_on_page(self) -> bool {
        use Format::*;
        match self {
            A8R8G8B8 | X8R8G8B8 | A16B16G16R16F => true,
            R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => false,
        }
    }

    /// The block formats (DXTn): bytes a block.
    const fn dxt(self) -> Option<usize> {
        use Format::*;
        match self {
            DXT1 => Some(8),
            DXT2 | DXT3 | DXT4 | DXT5 => Some(16),
            R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F => None,
        }
    }
}

/// Whether a format is a depth format, and with stencil bits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Depth {
    None,
    Plain,
    Stencil,
}

pub fn supported(fmt: u32) -> bool {
    Format::from_raw(fmt).is_some_and(Format::supported)
}

pub fn is_depth(fmt: u32) -> bool {
    match Format::from_raw(fmt).map(Format::depth) {
        Some(Depth::Plain | Depth::Stencil) => true,
        Some(Depth::None) | None => false,
    }
}

/// Depth formats with stencil bits.
pub fn has_stencil(fmt: u32) -> bool {
    match Format::from_raw(fmt).map(Format::depth) {
        Some(Depth::Stencil) => true,
        Some(Depth::None | Depth::Plain) | None => false,
    }
}

/// Formats a render target can have.
pub fn renderable(fmt: u32) -> bool {
    Format::from_raw(fmt).is_some_and(Format::renderable)
}

/// (bytes per block, block size in pixels)
pub fn layout(fmt: u32) -> Option<(u32, u32)> {
    Format::from_raw(fmt)?.layout()
}

/// Bytes per row (of blocks) and rows (of blocks) of a w x h image.
pub fn pitch_rows(fmt: u32, w: u32, h: u32) -> (u32, u32) {
    let (bpb, bs) = layout(fmt).unwrap_or((4, 1));
    let bw = w.div_ceil(bs).max(1);
    let bh = h.div_ceil(bs).max(1);
    // rows 4-byte aligned, as drivers do
    ((bw * bpb).next_multiple_of(4), bh)
}

pub fn page_format(fmt: u32) -> u32 {
    Format::from_raw(fmt).map_or(FMT_BGRA8, Format::page_format)
}

/// Bytes per pixel on the page.
pub fn page_bpp(fmt: u32) -> u32 {
    match page_format(fmt) {
        FMT_RGBA16F => 8,
        FMT_RGBA32F => 16,
        _ => 4,
    }
}

/// An n-bit UNORM channel (n <= 16, v < 2^n) as c / (2^n - 1).
fn unorm(v: u32, bits: u32) -> f32 {
    f32::from(lo16(v)) / f32::from(lo16((1 << bits) - 1))
}

/// An n-bit SNORM channel (n <= 16; the low n bits of v) as c / (2^(n-1) - 1), the most negative
/// value -1.
fn snorm(v: u32, bits: u32) -> f32 {
    let m = 1u32 << (bits - 1);
    let x = f32::from(lo16((v & ((m << 1) - 1)) ^ m)) - f32::from(lo16(m));
    (x / f32::from(lo16(m - 1))).max(-1.0)
}

/// A texel of a `wide` format as R, G, B, A: UNORM n bits c / (2^n - 1), SNORM c / (2^(n-1) - 1)
/// with the most negative value -1, and what a format does not have reads 1 (Direct3D 9: R32F
/// is (r, 1, 1, 1), V8U8 (u, v, 1, 1)).
pub fn texel(fmt: u32, p: &[u8]) -> [f32; 4] {
    use Format::*;
    let (u, s) = (unorm, snorm);
    let w16 = |i: usize| u32::from(u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]));
    let w32 = |i: usize| u32::from_le_bytes([p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]]);
    let h = |i: usize| crate::d3d11::format::f16_to_f32(u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]));
    let f = |i: usize| f32::from_bits(w32(i));
    let b = |i: usize| u32::from(p[i]);
    let Some(fmt) = Format::from_raw(fmt) else { return [0.0, 0.0, 0.0, 1.0] };
    match fmt {
        A2B10G10R10 => {
            let v = w32(0);
            [u(v & 1023, 10), u((v >> 10) & 1023, 10), u((v >> 20) & 1023, 10), u(v >> 30, 2)]
        }
        A2R10G10B10 => {
            let v = w32(0);
            [u((v >> 20) & 1023, 10), u((v >> 10) & 1023, 10), u(v & 1023, 10), u(v >> 30, 2)]
        }
        A2W10V10U10 => {
            let v = w32(0);
            [s(v, 10), s(v >> 10, 10), s(v >> 20, 10), u(v >> 30, 2)]
        }
        G16R16 => [u(w16(0), 16), u(w16(1), 16), 1.0, 1.0],
        A16B16G16R16 => [u(w16(0), 16), u(w16(1), 16), u(w16(2), 16), u(w16(3), 16)],
        L16 => {
            let l = u(w16(0), 16);
            [l, l, l, 1.0]
        }
        V8U8 => [s(b(0), 8), s(b(1), 8), 1.0, 1.0],
        L6V5U5 => {
            let v = w16(0);
            [s(v, 5), s(v >> 5, 5), u(v >> 10, 6), 1.0]
        }
        X8L8V8U8 => [s(b(0), 8), s(b(1), 8), u(b(2), 8), 1.0],
        Q8W8V8U8 => [s(b(0), 8), s(b(1), 8), s(b(2), 8), s(b(3), 8)],
        V16U16 => [s(w16(0), 16), s(w16(1), 16), 1.0, 1.0],
        Q16W16V16U16 => [s(w16(0), 16), s(w16(1), 16), s(w16(2), 16), s(w16(3), 16)],
        R16F => [h(0), 1.0, 1.0, 1.0],
        G16R16F => [h(0), h(1), 1.0, 1.0],
        R32F => [f(0), 1.0, 1.0, 1.0],
        G32R32F => [f(0), f(1), 1.0, 1.0],
        R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A16B16G16R16F | A32B32G32R32F | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => [0.0, 0.0, 0.0, 1.0],
    }
}

/// Render targets of the format on the page: the channels drawing writes (a WebGPU write mask:
/// red 1, green 2, blue 4, alpha 8; COLORWRITEENABLE's bits) and whether samplers read red as
/// the luminance (rgb = r: the page's green and blue are not the texel's). What a format does
/// not keep reads as `texel` reads it: A8's color 0, the rest 1 (clears write that).
pub fn target_channels(fmt: u32) -> (u32, bool) {
    Format::from_raw(fmt).map_or((0xf, false), Format::target_channels)
}

/// The steps a render target of the format keeps a channel in where the page holds more (R, G,
/// B, A: 2^bits - 1; 0, as the page keeps it): what a draw writes is rounded to them, as Windows
/// writes R5G6B5 and the rest (winref d3d9rtfmt, #53). (The X bits: target_x)
pub fn target_steps(fmt: u32) -> [u32; 4] {
    Format::from_raw(fmt).map_or([0; 4], Format::target_steps)
}

/// target_steps: a channel's steps go from -1 (SNORM)
pub const SIGNED: u32 = 0x8000_0000;

/// Render targets with X bits where the page keeps alpha (or with no alpha at all, R5G6B5):
/// blending reads their alpha as 1, and a draw leaves the X bits 0 while a clear keeps the
/// color's alpha there, as Windows (winref d3d9rtfmt: X8R8G8B8, X1R5G5B5, #53).
pub fn target_x(fmt: u32) -> bool {
    Format::from_raw(fmt).is_some_and(Format::target_x)
}

/// A D3DCOLOR as a clear of a render target of the format leaves it on the page
/// (target_channels).
pub fn target_color(fmt: u32, c: u32) -> u32 {
    let (mask, lum) = target_channels(fmt);
    let [b, g, r, a] = c.to_le_bytes();
    let fill = if fmt == A8 { 0 } else { 0xff };
    let pick = |bit: u32, v: u8| if mask & bit != 0 { v } else { fill };
    let r = pick(1, r);
    let (g, b) = if lum { (r, r) } else { (pick(2, g), pick(4, b)) };
    let a = if mask & 8 != 0 { a } else { 0xff };
    // (to the format's steps, as 8 bits: target_steps)
    let st = target_steps(fmt);
    let q = |v: u8, m: u32| if m == 0 || m > 255 { v } else { ext((u32::from(v) * m + 127) / 255, m.trailing_ones()) };
    u32::from_le_bytes([q(b, st[2]), q(g, st[1]), q(r, st[0]), q(a, st[3])])
}

/// A clear's color of a render target of the format as R, G, B, A: target_color, and SNORM
/// channels at their k / 127 (an 8-bit value cannot hold them: Windows' Q8W8V8U8 cleared with
/// alpha 0x80 shows 0x81, 64 / 127).
pub fn target_clear(fmt: u32, c: u32) -> [f32; 4] {
    let [b, g, r, a] = target_color(fmt, c).to_le_bytes();
    let v = [r, g, b, a].map(|v| f32::from(v) / 255.0);
    let st = target_steps(fmt);
    core::array::from_fn(|i| if st[i] & SIGNED != 0 { let m = f32::from(lo16(st[i] & !SIGNED)); (v[i] * m).round() / m } else { v[i] })
}

/// A page texel (`page_bpp` bytes: BGRA8, or 16- or 32-bit float RGBA) as R, G, B, A.
pub fn page_texel(fmt: u32, p: &[u8]) -> [f32; 4] {
    match page_format(fmt) {
        FMT_RGBA16F => core::array::from_fn(|i| crate::d3d11::format::f16_to_f32(u16::from_le_bytes([p[i * 2], p[i * 2 + 1]]))),
        FMT_RGBA32F => core::array::from_fn(|i| f32::from_le_bytes([p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]])),
        _ => [p[2], p[1], p[0], p[3]].map(|v| f32::from(v) / 255.0),
    }
}

/// A texel of the format (`layout` bytes into `out`) from its R, G, B, A, as a GPU writes a
/// render target: UNORM round(clamp(v, 0, 1) * (2^n - 1)), SNORM round(clamp(v, -1, 1) *
/// (2^(n-1) - 1)), luminance from red, what the format has no bits for dropped. False for a
/// format it does not know.
pub fn pack(fmt: u32, c: [f32; 4], out: &mut [u8]) -> bool {
    use Format::*;
    // (n <= 16 bits: the results fit in u32 as they are)
    let u = |v: f32, bits: u32| {
        let m = f32::from(lo16((1u32 << bits) - 1));
        if v.is_nan() { 0 } else { u32::try_from(whole((v.clamp(0.0, 1.0) * m).round())).unwrap_or(0) }
    };
    let s = |v: f32, bits: u32| {
        let m = f32::from(lo16((1u32 << (bits - 1)) - 1));
        let x = if v.is_nan() { 0 } else { whole((v.clamp(-1.0, 1.0) * m).round()) };
        let [b0, b1, b2, b3, ..] = x.to_le_bytes();
        u32::from_le_bytes([b0, b1, b2, b3]) & ((1u32 << bits) - 1)
    };
    let h = |v: f32| crate::d3d11::format::f16_bits(v);
    let [r, g, b, a] = c;
    let mut put = |bytes: &[u8]| out[..bytes.len()].copy_from_slice(bytes);
    // (four 16-bit channels, on the stack: a readback packs millions of texels)
    let four16 = |f: &dyn Fn(f32) -> u16| {
        let mut x = [0u8; 8];
        for (k, v) in [r, g, b, a].into_iter().enumerate() {
            x[2 * k..2 * k + 2].copy_from_slice(&f(v).to_le_bytes());
        }
        x
    };
    let floats = |v: [f32; 4]| {
        let mut x = [0u8; 16];
        for (k, v) in v.into_iter().enumerate() {
            x[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
        }
        x
    };
    let u8_ = |v: f32| lo8(u(v, 8));
    let Some(fmt) = Format::from_raw(fmt) else { return false };
    match fmt {
        A8R8G8B8 | X8R8G8B8 => put(&[u8_(b), u8_(g), u8_(r), u8_(a)]),
        A8B8G8R8 | X8B8G8R8 => put(&[u8_(r), u8_(g), u8_(b), u8_(a)]),
        R8G8B8 => put(&[u8_(b), u8_(g), u8_(r)]),
        R5G6B5 => put(&lo16(u(r, 5) << 11 | u(g, 6) << 5 | u(b, 5)).to_le_bytes()),
        X1R5G5B5 | A1R5G5B5 => put(&lo16(u(a, 1) << 15 | u(r, 5) << 10 | u(g, 5) << 5 | u(b, 5)).to_le_bytes()),
        A4R4G4B4 | X4R4G4B4 => put(&lo16(u(a, 4) << 12 | u(r, 4) << 8 | u(g, 4) << 4 | u(b, 4)).to_le_bytes()),
        A8 => put(&[u8_(a)]),
        L8 => put(&[u8_(r)]),
        A8L8 => put(&[u8_(r), u8_(a)]),
        A4L4 => put(&[lo8(u(a, 4) << 4 | u(r, 4))]),
        A2R10G10B10 => put(&(u(a, 2) << 30 | u(r, 10) << 20 | u(g, 10) << 10 | u(b, 10)).to_le_bytes()),
        A2B10G10R10 => put(&(u(a, 2) << 30 | u(b, 10) << 20 | u(g, 10) << 10 | u(r, 10)).to_le_bytes()),
        G16R16 => put(&(u(g, 16) << 16 | u(r, 16)).to_le_bytes()),
        A16B16G16R16 => put(&four16(&|v| lo16(u(v, 16)))),
        L16 => put(&lo16(u(r, 16)).to_le_bytes()),
        V8U8 => put(&[lo8(s(r, 8)), lo8(s(g, 8))]),
        Q8W8V8U8 => put(&[lo8(s(r, 8)), lo8(s(g, 8)), lo8(s(b, 8)), lo8(s(a, 8))]),
        V16U16 => put(&(s(g, 16) << 16 | s(r, 16)).to_le_bytes()),
        Q16W16V16U16 => put(&four16(&|v| lo16(s(v, 16)))),
        R16F => put(&h(r).to_le_bytes()),
        G16R16F => put(&(u32::from(h(g)) << 16 | u32::from(h(r))).to_le_bytes()),
        A16B16G16R16F => put(&four16(&h)),
        R32F => put(&r.to_le_bytes()),
        G32R32F => put(&floats(c)[..8]),
        A32B32G32R32F => put(&floats(c)),
        P8 | X8L8V8U8 | L6V5U5 | A2W10V10U10 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => return false,
    }
    true
}

/// Whether the format has no alpha (samplers then read alpha as 1). (DXT1 has: its 1-bit
/// transparent black, as Windows samples it.)
pub fn no_alpha(fmt: u32) -> bool {
    Format::from_raw(fmt).is_some_and(Format::no_alpha)
}

/// An n-bit UNORM value as 8 bits: round(v * 255 / (2^n - 1)), what a GPU samples it as (D3D's
/// UNORM rules: v / (2^n - 1) as a float, then rounded to the nearest 8-bit value). (Bit
/// replication, (v << 3) | (v >> 2), is off by one for 14 of the 5- and 6-bit values.)
fn ext(v: u32, bits: u32) -> u8 {
    let max = (1 << bits) - 1;
    lo8((v * 255 + max / 2) / max)
}

fn rgb565(c: u16) -> [u8; 3] {
    let c = u32::from(c);
    [ext(c & 31, 5), ext((c >> 5) & 63, 6), ext(c >> 11, 5)]
}

/// One w x h region of `fmt` (rows `pitch` bytes apart; for block formats, block rows and a
/// region aligned to blocks) as BGRA8 (or the float formats as they are), `w*bpp` bytes a row.
/// Formats whose pixels the page takes as they are (convert() only copies them).
pub fn same_on_page(fmt: u32) -> bool {
    Format::from_raw(fmt).is_some_and(Format::same_on_page)
}

/// A P8 texel through a palette of 256 PALETTEENTRYs (red, green, blue, flags: Direct3D 9
/// takes the flags as the alpha), as BGRA8.
pub fn p8_texel(pal: &[u8; 1024], i: u8) -> [u8; 4] {
    let e = &pal[usize::from(i) * 4..][..4];
    [e[2], e[1], e[0], e[3]]
}

/// convert() of a P8 region through a palette (p8_texel), `w*4` bytes a row.
pub unsafe fn convert_p8(src: *const u8, pitch: usize, w: u32, h: u32, pal: &[u8; 1024], out: &mut Vec<u8>) {
    let (w, h) = (us(w), us(h));
    let at = out.len();
    out.resize(at + w * h * 4, 0);
    let dst = &mut out[at..];
    for y in 0..h {
        let s = std::slice::from_raw_parts(src.add(y * pitch), w);
        for (x, &i) in s.iter().enumerate() {
            dst[(y * w + x) * 4..][..4].copy_from_slice(&p8_texel(pal, i));
        }
    }
}

pub unsafe fn convert(fmt: u32, src: *const u8, pitch: usize, w: u32, h: u32, out: &mut Vec<u8>) {
    use Format::*;
    let (w, h) = (us(w), us(h));
    let page = us(page_bpp(fmt));
    let at = out.len();
    out.resize(at + w * h * page, 0);
    let dst = &mut out[at..];
    let row = |y: usize| src.add(y * pitch);
    let f = Format::from_raw(fmt);
    if let Some((f, size)) = f.and_then(|f| Some((f, f.dxt()?))) {
        for by in 0..h.div_ceil(4) {
            for bx in 0..w.div_ceil(4) {
                let b = std::slice::from_raw_parts(row(by).add(bx * size), size);
                #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
                if bx * 4 + 4 <= w && by * 4 + 4 <= h {
                    dxt_block_simd(f, b, dst.as_mut_ptr().add((by * 4 * w + bx * 4) * 4), w * 4);
                    continue;
                }
                let px = decode_dxt_block(f, b);
                for (i, p) in px.iter().enumerate() {
                    let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                    if x < w && y < h {
                        dst[(y * w + x) * 4..][..4].copy_from_slice(p);
                    }
                }
            }
        }
        return;
    }
    let w16 = |s: *const u8, x: usize| u32::from(u16::from_le_bytes([*s.add(x * 2), *s.add(x * 2 + 1)]));
    for y in 0..h {
        let s = row(y);
        let d = &mut dst[y * w * page..][..w * page];
        let Some(f) = f else {
            d.fill(0);
            continue;
        };
        match f {
            A8R8G8B8 | X8R8G8B8 | A16B16G16R16F | A32B32G32R32F => std::ptr::copy_nonoverlapping(s, d.as_mut_ptr(), w * page),
            A8B8G8R8 | X8B8G8R8 => {
                for x in 0..w {
                    let p = std::slice::from_raw_parts(s.add(x * 4), 4);
                    d[x * 4..][..4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
                }
            }
            R8G8B8 => {
                for x in 0..w {
                    let p = std::slice::from_raw_parts(s.add(x * 3), 3);
                    d[x * 4..][..4].copy_from_slice(&[p[0], p[1], p[2], 255]);
                }
            }
            R5G6B5 => {
                for x in 0..w {
                    let [b, g, r] = rgb565(lo16(w16(s, x)));
                    d[x * 4..][..4].copy_from_slice(&[b, g, r, 255]);
                }
            }
            X1R5G5B5 | A1R5G5B5 => {
                for x in 0..w {
                    let c = w16(s, x);
                    let a = if f == A1R5G5B5 { ext(c >> 15, 1) } else { 255 };
                    d[x * 4..][..4].copy_from_slice(&[ext(c & 31, 5), ext((c >> 5) & 31, 5), ext((c >> 10) & 31, 5), a]);
                }
            }
            A4R4G4B4 | X4R4G4B4 => {
                for x in 0..w {
                    let c = w16(s, x);
                    let a = if f == A4R4G4B4 { ext(c >> 12, 4) } else { 255 };
                    d[x * 4..][..4].copy_from_slice(&[ext(c & 15, 4), ext((c >> 4) & 15, 4), ext((c >> 8) & 15, 4), a]);
                }
            }
            A8 => {
                for x in 0..w {
                    d[x * 4..][..4].copy_from_slice(&[0, 0, 0, *s.add(x)]);
                }
            }
            L8 | P8 => {
                for x in 0..w {
                    let l = *s.add(x);
                    d[x * 4..][..4].copy_from_slice(&[l, l, l, 255]);
                }
            }
            A8L8 => {
                for x in 0..w {
                    let (l, a) = (*s.add(x * 2), *s.add(x * 2 + 1));
                    d[x * 4..][..4].copy_from_slice(&[l, l, l, a]);
                }
            }
            A4L4 => {
                for x in 0..w {
                    let v = u32::from(*s.add(x));
                    let l = ext(v & 15, 4);
                    d[x * 4..][..4].copy_from_slice(&[l, l, l, ext(v >> 4, 4)]);
                }
            }
            A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F => {
                let bpp = us(f.layout().map_or(4, |l| l.0));
                for x in 0..w {
                    let t = texel(fmt, std::slice::from_raw_parts(s.add(x * bpp), bpp));
                    for (c, v) in t.into_iter().enumerate() {
                        if page == 16 {
                            d[x * 16 + c * 4..][..4].copy_from_slice(&v.to_le_bytes());
                        } else {
                            d[x * 8 + c * 2..][..2].copy_from_slice(&crate::d3d11::format::f16_bits(v).to_le_bytes());
                        }
                    }
                }
            }
            D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | DXT1 | DXT2 | DXT3 | DXT4 | DXT5 => d.fill(0),
        }
    }
}

/// 16 BGRA pixels of one DXT block.
/// A DXTn block's color palette (B G R A) and its 2-bit index of each pixel (row by row, 2
/// bits a pixel from the low ones).
fn dxt_palette(fmt: Format, b: &[u8]) -> ([[u8; 4]; 4], u32) {
    let dxt1 = fmt == Format::DXT1;
    let col = if dxt1 { b } else { &b[8..] };
    let c0 = u16::from_le_bytes([col[0], col[1]]);
    let c1 = u16::from_le_bytes([col[2], col[3]]);
    // (565 to 8 bits by repeating the high bits, then the thirds truncated: WARP's BC1-BC3,
    // winref d3d11tex 71-78)
    let rep = |v: u32, n: u32| lo8(v << (8 - n) | v >> (2 * n - 8));
    let rgb = |c: u16| {
        let c = u32::from(c);
        [rep(c & 31, 5), rep((c >> 5) & 63, 6), rep(c >> 11, 5)]
    };
    let (p0, p1) = (rgb(c0), rgb(c1));
    let mix = |a: u8, b: u8, wa: u32, wb: u32, d: u32| lo8((u32::from(a) * wa + u32::from(b) * wb) / d);
    let mut pal = [[0u8; 4]; 4];
    pal[0] = [p0[0], p0[1], p0[2], 255];
    pal[1] = [p1[0], p1[1], p1[2], 255];
    if c0 > c1 || !dxt1 {
        for i in 0..3 {
            pal[2][i] = mix(p0[i], p1[i], 2, 1, 3);
            pal[3][i] = mix(p0[i], p1[i], 1, 2, 3);
        }
        pal[2][3] = 255;
        pal[3][3] = 255;
    } else {
        for i in 0..3 {
            pal[2][i] = mix(p0[i], p1[i], 1, 1, 2);
        }
        pal[2][3] = 255;
        pal[3] = [0, 0, 0, 0];
    }
    (pal, u32::from_le_bytes([col[4], col[5], col[6], col[7]]))
}

/// A DXT2-5 block's alpha of each pixel (row by row); None for DXT1 (the palette's).
fn dxt_alpha(fmt: Format, b: &[u8]) -> Option<[u8; 16]> {
    use Format::*;
    let mut out = [0u8; 16];
    match fmt {
        DXT2 | DXT3 => {
            for (i, o) in out.iter_mut().enumerate() {
                let v = (b[i / 2] >> ((i & 1) * 4)) & 15;
                *o = v * 17;
            }
        }
        DXT4 | DXT5 => {
            let (a0, a1) = (u32::from(b[0]), u32::from(b[1]));
            let mut ap = [0u32; 8];
            ap[0] = a0;
            ap[1] = a1;
            // (rounded to nearest, as WARP: winref d3d11tex BC3)
            if a0 > a1 {
                for (i, k) in (1..7u32).zip(2..) {
                    ap[k] = ((7 - i) * a0 + i * a1 + 3) / 7;
                }
            } else {
                for (i, k) in (1..5u32).zip(2..) {
                    ap[k] = ((5 - i) * a0 + i * a1 + 2) / 5;
                }
                ap[6] = 0;
                ap[7] = 255;
            }
            let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
            for (i, o) in out.iter_mut().enumerate() {
                *o = lo8(ap[usize::from((bits >> (i * 3)).to_le_bytes()[0] & 7)]);
            }
        }
        R8G8B8 | A8R8G8B8 | X8R8G8B8 | R5G6B5 | X1R5G5B5 | A1R5G5B5 | A4R4G4B4 | A8 | X4R4G4B4 | A8B8G8R8 | X8B8G8R8 | L8 | A8L8 | A4L4 | P8 | D16_LOCKABLE | D32 | D15S1 | D24S8 | D24X8 | D24X4S4 | D16 | D32F_LOCKABLE | D24FS8 | INDEX16 | INDEX32 | A2B10G10R10 | G16R16 | A2R10G10B10 | A16B16G16R16 | V8U8 | L6V5U5 | X8L8V8U8 | Q8W8V8U8 | V16U16 | A2W10V10U10 | L16 | Q16W16V16U16 | R16F | G16R16F | R32F | G32R32F | A16B16G16R16F | A32B32G32R32F | DXT1 => return None,
    }
    Some(out)
}

fn decode_dxt_block(fmt: Format, b: &[u8]) -> [[u8; 4]; 16] {
    let (pal, bits) = dxt_palette(fmt, b);
    let mut out = [[0u8; 4]; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = pal[usize::from(lo8(bits >> (i * 2)) & 3)];
    }
    if let Some(a) = dxt_alpha(fmt, b) {
        for (o, a) in out.iter_mut().zip(a) {
            o[3] = a;
        }
    }
    out
}

/// (public for crates/decbench's tests; a format that is not DXT2-5 decodes as DXT1)
#[doc(hidden)]
pub fn decode_dxt(fmt: u32, b: &[u8]) -> [[u8; 4]; 16] {
    decode_dxt_block(Format::from_raw(fmt).unwrap_or(Format::DXT1), b)
}

/// decode_dxt of a whole block into 4 rows `stride` bytes apart, its palette looked up with
/// one swizzle a row (wasm SIMD128).
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
unsafe fn dxt_block_simd(fmt: Format, b: &[u8], dst: *mut u8, stride: usize) {
    use core::arch::wasm32::*;
    // (a row's index byte -> the swizzle picking each of its 4 pixels' palette bytes)
    static MASKS: std::sync::OnceLock<[[u8; 16]; 256]> = std::sync::OnceLock::new();
    let masks = MASKS.get_or_init(|| std::array::from_fn(|r| std::array::from_fn(|i| (((r >> ((i / 4) * 2)) & 3) * 4 + i % 4) as u8)));
    let (pal, bits) = dxt_palette(fmt, b);
    let pal = v128_load(pal.as_ptr() as *const v128);
    let alpha = dxt_alpha(fmt, b).map(|a| v128_load(a.as_ptr() as *const v128));
    let keep = u32x4_splat(0x00ff_ffff);
    for r in 0..4 {
        let mut px = i8x16_swizzle(pal, v128_load(masks[((bits >> (8 * r)) & 0xff) as usize].as_ptr() as *const v128));
        if let Some(a) = alpha {
            // (row r's 4 alpha bytes into byte 3 of each pixel)
            let s = match r {
                0 => u8x16_shuffle::<16, 16, 16, 0, 16, 16, 16, 1, 16, 16, 16, 2, 16, 16, 16, 3>(a, a),
                1 => u8x16_shuffle::<16, 16, 16, 4, 16, 16, 16, 5, 16, 16, 16, 6, 16, 16, 16, 7>(a, a),
                2 => u8x16_shuffle::<16, 16, 16, 8, 16, 16, 16, 9, 16, 16, 16, 10, 16, 16, 16, 11>(a, a),
                _ => u8x16_shuffle::<16, 16, 16, 12, 16, 16, 16, 13, 16, 16, 16, 14, 16, 16, 16, 15>(a, a),
            };
            px = v128_bitselect(px, s, keep);
        }
        v128_store(dst.add(r * stride) as *mut v128, px);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // References written from the Direct3D docs ("D3DFORMAT", "Block Compression"), in floats,
    // independent of the code above: a channel of n bits is c / (2^n - 1), 8-bit outputs are
    // rounded to nearest; the page wants B, G, R, A bytes.

    fn unorm(c: u32, bits: u32) -> f64 {
        c as f64 / ((1u32 << bits) - 1) as f64
    }
    fn byte(f: f64) -> u8 {
        (f.clamp(0.0, 1.0) * 255.0).round() as u8
    }
    fn conv(fmt: u32, src: &[u8], w: u32, h: u32) -> Vec<u8> {
        let (pitch, _) = pitch_rows(fmt, w, h);
        let mut padded = Vec::new();
        let (bpb, bs) = layout(fmt).unwrap();
        let row = (w.div_ceil(bs) * bpb) as usize;
        for r in src.chunks(row) {
            padded.extend_from_slice(r);
            padded.resize(padded.len().next_multiple_of(pitch as usize), 0xcd);
        }
        let mut out = Vec::new();
        unsafe { convert(fmt, padded.as_ptr(), pitch as usize, w, h, &mut out) };
        out
    }

    /// Reading a render target back: a texel through convert() to the page, page_texel and pack
    /// is the texel again (what a format does not keep as pack writes it: X bits set, the most
    /// negative SNORM value as -1's), for random texels of every format pack knows (random
    /// floats: numbers only).
    #[test]
    fn wide_formats_pack_as_the_byte_lists_did() {
        // (the per-texel Vec form these replaced, as the reference; random channels, NaN and
        // out-of-range ones too)
        let mut x = 0x5eed_f00d_u64;
        for _ in 0..20000 {
            let mut f = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                match x % 8 {
                    0 => f32::NAN,
                    1 => (x >> 8) as f32 - 3.0e9,
                    _ => ((x >> 11) as f64 / (1u64 << 53) as f64 * 3.0 - 1.5) as f32,
                }
            };
            let c = [f(), f(), f(), f()];
            let u = |v: f32| if v.is_nan() { 0 } else { (v.clamp(0.0, 1.0) * 65535.0).round() as u16 };
            let s = |v: f32| if v.is_nan() { 0 } else { (v.clamp(-1.0, 1.0) * 32767.0).round() as i32 as u16 };
            let h = crate::d3d11::format::f16_bits;
            for (fmt, want) in [
                (A16B16G16R16, c.iter().flat_map(|&v| u(v).to_le_bytes()).collect::<Vec<u8>>()),
                (Q16W16V16U16, c.iter().flat_map(|&v| s(v).to_le_bytes()).collect()),
                (A16B16G16R16F, c.iter().flat_map(|&v| h(v).to_le_bytes()).collect()),
                (G32R32F, c[..2].iter().flat_map(|v| v.to_le_bytes()).collect()),
                (A32B32G32R32F, c.iter().flat_map(|v| v.to_le_bytes()).collect()),
            ] {
                let mut out = [0xaau8; 20];
                assert!(pack(fmt, c, &mut out));
                assert_eq!(&out[..want.len()], &want[..], "{fmt} {c:?}");
                assert!(out[want.len()..].iter().all(|&b| b == 0xaa), "{fmt}: wrote past the texel");
            }
        }
    }

    #[test]
    fn pack_undoes_the_page() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let fmts = [
            A8R8G8B8, X8R8G8B8, A8B8G8R8, X8B8G8R8, R8G8B8, R5G6B5, X1R5G5B5, A1R5G5B5, A4R4G4B4, X4R4G4B4, A8, L8, A8L8, A2R10G10B10,
            A2B10G10R10, G16R16, A16B16G16R16, L16, V8U8, Q8W8V8U8, V16U16, Q16W16V16U16, R16F, G16R16F, A16B16G16R16F, R32F, G32R32F,
            A32B32G32R32F,
        ];
        for fmt in fmts {
            let (bpp, _) = layout(fmt).unwrap();
            for _ in 0..4000 {
                let mut t: Vec<u8> = (0..bpp).map(|_| next() as u8).collect();
                // (numbers only: no NaN halves or floats)
                match fmt {
                    R16F | G16R16F | A16B16G16R16F => t.chunks_exact_mut(2).for_each(|h| h[1] &= 0xbb),
                    R32F | G32R32F | A32B32G32R32F => t.chunks_exact_mut(4).for_each(|f| f[3] &= 0xbf),
                    _ => {}
                }
                let page = conv(fmt, &t, 1, 1);
                let c = page_texel(fmt, &page);
                let mut back = vec![0u8; bpp as usize];
                assert!(pack(fmt, c, &mut back), "format {fmt}");
                let mut want = t.clone();
                match fmt {
                    X1R5G5B5 => want[1] |= 0x80,
                    X4R4G4B4 => want[1] |= 0xf0,
                    V8U8 | Q8W8V8U8 => want.iter_mut().filter(|b| **b == 0x80).for_each(|b| *b = 0x81),
                    V16U16 | Q16W16V16U16 => want.chunks_exact_mut(2).filter(|w| w == &[0, 0x80]).for_each(|w| w[0] = 1),
                    _ => {}
                }
                assert_eq!(back, want, "format {fmt}: texel {t:02x?} page {page:02x?} as {c:?}");
            }
        }
    }

    /// Colors in render targets of fewer bits, as Windows samples them after drawing them
    /// (winref d3d9rtfmt-32's "color" lines): rounded to the format's steps, then widened.
    #[test]
    fn target_steps_as_windows() {
        let cases: [(u32, [(u32, u32); 3]); 4] = [
            (R5G6B5, [(0xff8000, 0xff8200), (0x406020, 0x426121), (0xc060c0, 0xbd61bd)]),
            (X1R5G5B5, [(0xff8000, 0xff8400), (0x406020, 0x426321), (0xc060c0, 0xbd63bd)]),
            (A1R5G5B5, [(0xff8000, 0xff8400), (0x406020, 0x426321), (0xc060c0, 0xbd63bd)]),
            (A4R4G4B4, [(0xff8000, 0xff8800), (0x406020, 0x446622), (0xc060c0, 0xbb66bb)]),
        ];
        for (f, cs) in cases {
            for (c, want) in cs {
                assert_eq!(target_color(f, 0xff00_0000 | c) & 0xff_ffff, want, "format {f} color {c:06x}");
            }
        }
        // (and the shader's steps are what the format's bits hold)
        for f in [R5G6B5, X1R5G5B5, A1R5G5B5, A4R4G4B4, X4R4G4B4, A2R10G10B10, A2B10G10R10, G16R16, A16B16G16R16] {
            let mut out = [0u8; 8];
            let st = target_steps(f);
            for (i, &m) in st.iter().enumerate() {
                if m == 0 {
                    continue;
                }
                // every step k / m packs to k
                for k in [0, 1, m / 2, m - 1, m] {
                    let mut c = [0.0f32; 4];
                    c[i] = k as f32 / m as f32;
                    assert!(pack(f, c, &mut out));
                    let back = page_texel_of(f, &out)[i];
                    assert_eq!((back * m as f32).round() as u32, k, "format {f} channel {i} step {k}/{m}: {back}");
                }
            }
        }
        // SNORM: every step k / 127 from -127 packs to k
        for f in [V8U8, Q8W8V8U8] {
            let mut out = [0u8; 8];
            for (i, &m) in target_steps(f).iter().enumerate() {
                if m == 0 {
                    continue;
                }
                assert_eq!(m, SIGNED | 127);
                for k in [-127i32, -64, -1, 0, 1, 64, 126, 127] {
                    let mut c = [0.0f32; 4];
                    c[i] = k as f32 / 127.0;
                    assert!(pack(f, c, &mut out));
                    assert_eq!(out[i] as i8 as i32, k, "format {f} channel {i} step {k}");
                    let back = page_texel_of(f, &out)[i];
                    assert_eq!((back * 127.0).round() as i32, k);
                }
            }
        }
        // (cleared with 0x80406020: Q8W8V8U8 is R, G, B, A 32 / 127, 48 / 127, 16 / 127, 64 / 127;
        // V8U8's blue and alpha read 1; others as target_color's bytes)
        assert_eq!(target_clear(Q8W8V8U8, 0x80406020), [32.0 / 127.0, 48.0 / 127.0, 16.0 / 127.0, 64.0 / 127.0]);
        assert_eq!(target_clear(V8U8, 0x80406020), [32.0 / 127.0, 48.0 / 127.0, 1.0, 1.0]);
        assert_eq!(target_clear(A8R8G8B8, 0x80406020), [64.0 / 255.0, 96.0 / 255.0, 32.0 / 255.0, 128.0 / 255.0]);
        // (Windows' V8U8 / Q8W8V8U8 "color" line: 0x80 drawn is 64 / 127, shown as 0x81)
        let g = (0x80 as f32 / 255.0 * 127.0).round() / 127.0;
        assert_eq!((g * 255.0).round() as u32, 0x81);
        // formats the page keeps whole: no steps
        for f in [A8R8G8B8, X8R8G8B8, A8, L8, A8L8, A16B16G16R16F, R32F] {
            assert_eq!(target_steps(f), [0; 4]);
        }
    }

    /// A texel of a format as R, G, B, A, through the page as a sampler reads it
    fn page_texel_of(f: u32, t: &[u8]) -> [f32; 4] {
        let page = conv(f, t, 1, 1);
        page_texel(f, &page)
    }

    /// Every value of every 16-bit format.
    #[test]
    fn every_16_bit_texel() {
        // (format, [(shift, bits) of B, G, R, A] (bits 0: none, alpha then 1.0))
        let fmts: [(u32, [(u32, u32); 4]); 6] = [
            (R5G6B5, [(0, 5), (5, 6), (11, 5), (0, 0)]),
            (X1R5G5B5, [(0, 5), (5, 5), (10, 5), (0, 0)]),
            (A1R5G5B5, [(0, 5), (5, 5), (10, 5), (15, 1)]),
            (A4R4G4B4, [(0, 4), (4, 4), (8, 4), (12, 4)]),
            (X4R4G4B4, [(0, 4), (4, 4), (8, 4), (0, 0)]),
            (A8L8, [(0, 8), (0, 8), (0, 8), (8, 8)]),
        ];
        for (fmt, ch) in fmts {
            let src: Vec<u8> = (0..=u16::MAX).flat_map(|v| v.to_le_bytes()).collect();
            let out = conv(fmt, &src, 256, 256);
            for v in 0..=u16::MAX as u32 {
                let want: Vec<u8> = ch.iter().map(|&(sh, bits)| if bits == 0 { 255 } else { byte(unorm((v >> sh) & ((1 << bits) - 1), bits)) }).collect();
                assert_eq!(&out[v as usize * 4..][..4], &want[..], "format {fmt} texel {v:#06x}");
            }
        }
    }

    /// The wide formats against their definitions ("D3DFORMAT": channels from the lowest bit,
    /// UNORM c / (2^n - 1), SNORM two's complement c / (2^(n-1) - 1) clamped to -1, missing
    /// channels 1), every value of the 16-bit ones and of each channel of the others, through
    /// convert() to the page's floats.
    #[test]
    fn wide_formats_against_their_definitions() {
        // (format, bytes, [(shift, bits, kind) of R, G, B, A]: kind 0 none (1.0), 1 unorm, 2 snorm, 3 f16, 4 f32)
        type Ch = (u32, u32, u8);
        let fmts: [(u32, u32, [Ch; 4]); 16] = [
            (A2B10G10R10, 4, [(0, 10, 1), (10, 10, 1), (20, 10, 1), (30, 2, 1)]),
            (A2R10G10B10, 4, [(20, 10, 1), (10, 10, 1), (0, 10, 1), (30, 2, 1)]),
            (A2W10V10U10, 4, [(0, 10, 2), (10, 10, 2), (20, 10, 2), (30, 2, 1)]),
            (G16R16, 4, [(0, 16, 1), (16, 16, 1), (0, 0, 0), (0, 0, 0)]),
            (A16B16G16R16, 8, [(0, 16, 1), (16, 16, 1), (32, 16, 1), (48, 16, 1)]),
            (L16, 2, [(0, 16, 1), (0, 16, 1), (0, 16, 1), (0, 0, 0)]),
            (V8U8, 2, [(0, 8, 2), (8, 8, 2), (0, 0, 0), (0, 0, 0)]),
            (L6V5U5, 2, [(0, 5, 2), (5, 5, 2), (10, 6, 1), (0, 0, 0)]),
            (X8L8V8U8, 4, [(0, 8, 2), (8, 8, 2), (16, 8, 1), (0, 0, 0)]),
            (Q8W8V8U8, 4, [(0, 8, 2), (8, 8, 2), (16, 8, 2), (24, 8, 2)]),
            (V16U16, 4, [(0, 16, 2), (16, 16, 2), (0, 0, 0), (0, 0, 0)]),
            (Q16W16V16U16, 8, [(0, 16, 2), (16, 16, 2), (32, 16, 2), (48, 16, 2)]),
            (R16F, 2, [(0, 16, 3), (0, 0, 0), (0, 0, 0), (0, 0, 0)]),
            (G16R16F, 4, [(0, 16, 3), (16, 16, 3), (0, 0, 0), (0, 0, 0)]),
            (R32F, 4, [(0, 32, 4), (0, 0, 0), (0, 0, 0), (0, 0, 0)]),
            (G32R32F, 8, [(0, 32, 4), (32, 32, 4), (0, 0, 0), (0, 0, 0)]),
        ];
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for (fmt, bytes, ch) in fmts {
            assert_eq!(layout(fmt), Some((bytes, 1)));
            // texels: every 16-bit value of 2-byte formats, else each channel through all its
            // values (up to 65536) with the others random, and the edges
            let mut texels: Vec<u64> = Vec::new();
            if bytes == 2 {
                texels.extend(0..=0xffffu64);
            } else {
                for &(sh, bits, kind) in &ch {
                    if kind == 0 {
                        continue;
                    }
                    let n = if bits >= 16 { 1u64 << 16 } else { 1u64 << bits };
                    for v in 0..n {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        let v = if bits == 32 { (v << 16) | (seed & 0xffff) } else { v };
                        let mask = if bits == 64 { !0 } else { ((1u64 << bits) - 1) << sh };
                        texels.push((seed & !mask) | (v << sh) & mask);
                    }
                }
            }
            let src: Vec<u8> = texels.iter().flat_map(|t| t.to_le_bytes()[..bytes as usize].to_vec()).collect();
            let w = texels.len() as u32;
            let out = conv(fmt, &src, w, 1);
            let page = page_bpp(fmt) as usize;
            for (i, &t) in texels.iter().enumerate() {
                for (c, &(sh, bits, kind)) in ch.iter().enumerate() {
                    let v = if bits == 0 { 0 } else { (t >> sh) & ((1u64 << bits) - 1) };
                    let want: f64 = match kind {
                        0 => 1.0,
                        1 => v as f64 / ((1u64 << bits) - 1) as f64,
                        2 => {
                            let x = if v >> (bits - 1) != 0 { v as i64 - (1i64 << bits) } else { v as i64 };
                            (x as f64 / ((1i64 << (bits - 1)) - 1) as f64).max(-1.0)
                        }
                        3 => crate::d3d11::format::f16_to_f32(v as u16) as f64,
                        _ => f32::from_bits(v as u32) as f64,
                    };
                    let got = if page == 16 {
                        f32::from_le_bytes(out[i * 16 + c * 4..][..4].try_into().unwrap())
                    } else {
                        crate::d3d11::format::f16_to_f32(u16::from_le_bytes(out[i * 8 + c * 2..][..2].try_into().unwrap()))
                    } as f64;
                    // (the page's float of the value: f16 for the formats of 10 bits and less, the
                    // f32 nearest for the others)
                    let near = if page == 16 { want as f32 as f64 } else { crate::d3d11::format::f16_to_f32(crate::d3d11::format::f16_bits(want as f32)) as f64 };
                    assert!(got == near || got.is_nan() && near.is_nan(), "format {fmt} texel {t:#x} channel {c}: {got} want {want}");
                }
            }
        }
    }

    /// Every value of every 8-bit format, and the byte orders of the 24- and 32-bit ones.
    #[test]
    fn every_8_bit_texel_and_byte_orders() {
        let src: Vec<u8> = (0..=255).collect();
        for v in 0..=255u32 {
            let i = v as usize * 4;
            assert_eq!(conv(A8, &src, 256, 1)[i..i + 4], [0, 0, 0, v as u8]);
            assert_eq!(conv(L8, &src, 256, 1)[i..i + 4], [v as u8, v as u8, v as u8, 255]);
            let (l, a) = (byte(unorm(v & 15, 4)), byte(unorm(v >> 4, 4)));
            assert_eq!(conv(A4L4, &src, 256, 1)[i..i + 4], [l, l, l, a]);
        }
        // D3DFMT_R8G8B8: B, G, R in memory; A8B8G8R8: R, G, B, A
        assert_eq!(conv(R8G8B8, &[1, 2, 3, 4, 5, 6], 2, 1), [1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!(conv(A8B8G8R8, &[1, 2, 3, 4], 1, 1), [3, 2, 1, 4]);
        assert_eq!(conv(X8B8G8R8, &[1, 2, 3, 4], 1, 1), [3, 2, 1, 4]);
        assert_eq!(conv(A8R8G8B8, &[1, 2, 3, 4], 1, 1), [1, 2, 3, 4]);
    }

    /// The reference BC1-3 decoder ("Block Compression (Direct3D 10)"): 16 B, G, R, A pixels in floats.
    fn ref_dxt(fmt: u32, b: &[u8]) -> [[f64; 4]; 16] {
        let col = if fmt == DXT1 { b } else { &b[8..] };
        let c0 = u16::from_le_bytes([col[0], col[1]]) as u32;
        let c1 = u16::from_le_bytes([col[2], col[3]]) as u32;
        let e = |c: u32| [unorm(c & 31, 5), unorm((c >> 5) & 63, 6), unorm(c >> 11, 5), 1.0];
        let (p0, p1) = (e(c0), e(c1));
        let lerp = |a: [f64; 4], b: [f64; 4], t: f64| [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * t);
        let pal = if c0 > c1 || fmt != DXT1 {
            [p0, p1, lerp(p0, p1, 1.0 / 3.0), lerp(p0, p1, 2.0 / 3.0)]
        } else {
            [p0, p1, lerp(p0, p1, 0.5), [0.0; 4]]
        };
        let bits = u32::from_le_bytes([col[4], col[5], col[6], col[7]]);
        let mut out = [[0.0; 4]; 16];
        for (i, o) in out.iter_mut().enumerate() {
            *o = pal[((bits >> (2 * i)) & 3) as usize];
            match fmt {
                DXT2 | DXT3 => o[3] = unorm(((b[i / 2] >> (4 * (i & 1))) & 15) as u32, 4),
                DXT4 | DXT5 => {
                    let (a0, a1) = (b[0] as f64 / 255.0, b[1] as f64 / 255.0);
                    let code = ((u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]) >> (3 * i)) & 7) as usize;
                    o[3] = match code {
                        0 => a0,
                        1 => a1,
                        k if b[0] > b[1] => ((8 - k) as f64 * a0 + (k - 1) as f64 * a1) / 7.0,
                        6 => 0.0,
                        7 => 1.0,
                        k => ((6 - k) as f64 * a0 + (k - 1) as f64 * a1) / 5.0,
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Our 8-bit pixels against the reference: interpolated values may be off by one (the
    /// block formats' tolerance), the endpoints and alpha codes 0/1 must be exact.
    fn check_block(fmt: u32, b: &[u8]) {
        let got = decode_dxt(fmt, b);
        for (i, (g, r)) in got.iter().zip(ref_dxt(fmt, b)).enumerate() {
            for c in 0..4 {
                let want = byte(r[c]) as i32;
                assert!((g[c] as i32 - want).abs() <= 1, "format {fmt:#x} block {b:02x?} pixel {i} channel {c}: {} want {want}", g[c]);
            }
        }
    }

    /// Every pair of endpoint values of each channel, in both modes, every palette index; every
    /// pair of DXT5 alpha endpoints with every code; every DXT3 alpha.
    #[test]
    fn dxt_blocks_against_the_reference() {
        // indices 0, 1, 2, 3 in pixels 0-3 (and again)
        let idx = 0b11_10_01_00_11_10_01_00_11_10_01_00_11_10_01_00u32.to_le_bytes();
        for (sh, bits) in [(0, 5), (5, 6), (11, 5)] {
            for a in 0..1u32 << bits {
                for b in 0..1u32 << bits {
                    // the other channels pick the mode: c0 > c1, then c0 <= c1 (and c0 == c1
                    // when a == b: the 3-color mode too)
                    for (oa, ob) in [(0xffffu32, 0u32), (0, 0xffff), (0, 0)] {
                        let m = !(((1 << bits) - 1) << sh) & 0xffff;
                        let c0 = (a << sh) | (oa & m);
                        let c1 = (b << sh) | (ob & m);
                        let mut blk = [0u8; 8];
                        blk[..2].copy_from_slice(&(c0 as u16).to_le_bytes());
                        blk[2..4].copy_from_slice(&(c1 as u16).to_le_bytes());
                        blk[4..].copy_from_slice(&idx);
                        check_block(DXT1, &blk);
                        let mut b16 = [0u8; 16];
                        b16[8..].copy_from_slice(&blk);
                        check_block(DXT3, &b16);
                        check_block(DXT5, &b16);
                    }
                }
            }
        }
        for a0 in 0..=255u8 {
            for a1 in 0..=255u8 {
                // codes 0..7 in pixels 0..7 and 8..15
                let codes: u64 = (0..16).map(|i| ((i % 8) as u64) << (3 * i)).sum();
                let mut b = [0u8; 16];
                b[0] = a0;
                b[1] = a1;
                b[2..8].copy_from_slice(&codes.to_le_bytes()[..6]);
                check_block(DXT5, &b);
            }
        }
        let mut b = [0u8; 16];
        for (i, v) in b[..8].iter_mut().enumerate() {
            *v = (i as u8 * 2) | ((i as u8 * 2 + 1) << 4);
        }
        check_block(DXT3, &b);
    }


    proptest::proptest! {
        /// convert_p8 against a texel-by-texel lookup, for any palette, region and pitch
        /// (and a prefix in `out` stays as it was).
        #[test]
        fn convert_p8_matches_lookup(pal in proptest::collection::vec(0u8.., 1024), w in 1u32..17, h in 1u32..9,
                                     pad in 0usize..5, seed in 0u64.., prefix in proptest::collection::vec(0u8.., 0..9)) {
            let pal: [u8; 1024] = pal.try_into().unwrap();
            let pitch = w as usize + pad;
            let mut r = seed | 1;
            let src: Vec<u8> = (0..pitch * (h as usize - 1) + w as usize).map(|_| { r ^= r << 13; r ^= r >> 7; r ^= r << 17; r as u8 }).collect();
            let mut out = prefix.clone();
            unsafe { convert_p8(src.as_ptr(), pitch, w, h, &pal, &mut out) };
            proptest::prop_assert_eq!(&out[..prefix.len()], &prefix[..]);
            let mut want = prefix.clone();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let i = src[y * pitch + x] as usize;
                    want.extend([pal[i * 4 + 2], pal[i * 4 + 1], pal[i * 4], pal[i * 4 + 3]]);
                }
            }
            proptest::prop_assert_eq!(out, want);
        }
    }

    /// What each format is, as the D3D9 docs list them.
    #[test]
    fn format_table() {
        use super::super::gpu::{FMT_BGRA8, FMT_DEPTH24S8, FMT_RGBA16F, FMT_RGBA32F};
        // (format, depth, render target, no alpha, kept as it is on the page)
        let all = [
            (R8G8B8, false, false, true, false),
            (A8R8G8B8, false, true, false, true),
            (X8R8G8B8, false, true, true, true),
            (R5G6B5, false, true, true, false),
            (X1R5G5B5, false, true, true, false),
            (A1R5G5B5, false, true, false, false),
            (A4R4G4B4, false, true, false, false),
            (A8, false, true, false, false),
            (X4R4G4B4, false, false, true, false),
            (A8B8G8R8, false, true, false, false),
            (X8B8G8R8, false, false, true, false),
            (L8, false, true, true, false),
            (A8L8, false, true, false, false),
            (A4L4, false, false, false, false),
            (P8, false, false, false, false),
            (D16_LOCKABLE, true, false, false, false),
            (D32, true, false, false, false),
            (D15S1, true, false, false, false),
            (D24S8, true, false, false, false),
            (D24X8, true, false, false, false),
            (D24X4S4, true, false, false, false),
            (D16, true, false, false, false),
            (D32F_LOCKABLE, true, false, false, false),
            (D24FS8, true, false, false, false),
            (A16B16G16R16F, false, true, false, true),
            (A32B32G32R32F, false, true, false, false),
            (DXT1, false, false, false, false),
            (DXT2, false, false, false, false),
            (DXT3, false, false, false, false),
            (DXT4, false, false, false, false),
            (DXT5, false, false, false, false),
        ];
        let wide16 = [A2B10G10R10, A2R10G10B10, R16F, G16R16F];
        // (the signed ones too: k / 127 in 16 bits is off where 8 bits show it)
        let wide32 = [G16R16, A16B16G16R16, V16U16, L16, Q16W16V16U16, R32F, G32R32F, V8U8, L6V5U5, X8L8V8U8, Q8W8V8U8, A2W10V10U10];
        for f in wide16.into_iter().chain(wide32) {
            assert!(supported(f) != matches!(f, X8L8V8U8 | L6V5U5 | A2W10V10U10) && !is_depth(f) && renderable(f) == matches!(f, A2R10G10B10 | A2B10G10R10 | G16R16 | A16B16G16R16 | L16 | R16F | G16R16F | R32F | G32R32F | V8U8 | Q8W8V8U8) && !no_alpha(f) && !same_on_page(f), "{f}");
            let (page, bpp) = if wide16.contains(&f) { (FMT_RGBA16F, 8) } else { (FMT_RGBA32F, 16) };
            assert_eq!((page_format(f), page_bpp(f)), (page, bpp), "{f}");
        }
        for (f, depth, rt, no_a, same) in all {
            assert!(supported(f), "{f}");
            assert_eq!((is_depth(f), renderable(f), no_alpha(f), same_on_page(f)), (depth, rt, no_a, same), "{f}");
            let (page, bpp) = match f {
                _ if depth => (FMT_DEPTH24S8, 4),
                A16B16G16R16F => (FMT_RGBA16F, 8),
                A32B32G32R32F => (FMT_RGBA32F, 16),
                _ => (FMT_BGRA8, 4),
            };
            assert_eq!((page_format(f), page_bpp(f)), (page, bpp), "{f}");
        }
        for f in [0, 1, 27, 40, 100, INDEX16, INDEX32, 65, 117, u32::MAX] {
            assert!(!supported(f) && !is_depth(f) && !renderable(f) && !no_alpha(f) && !same_on_page(f), "{f}");
            assert_eq!(page_format(f), FMT_BGRA8);
        }
    }

    /// A region that ends inside a block: only its pixels are written, row by row.
    /// BC1 and BC3 as WARP decodes them (winref d3d11tex 71 / 77, row 0 of the probe's random
    /// blocks): ends widened by repeating their high bits, colour thirds truncated, alpha
    /// sevenths and fifths rounded
    #[test]
    fn dxt_as_warp() {
        // (the probe's xorshift fill of format f)
        let fill = |f: u64| {
            let mut xs = 0x9e37_79b9_7f4a_7c15u64 ^ f.wrapping_mul(0x0100_0000_01b3);
            (0..128 * 4)
                .map(|_| {
                    xs ^= xs << 13;
                    xs ^= xs >> 7;
                    xs ^= xs << 17;
                    (xs >> 16) as u8
                })
                .collect::<Vec<u8>>()
        };
        let want71 = [[247, 154, 107, 255], [231, 154, 140, 255], [239, 154, 123, 255], [239, 154, 123, 255], [109, 79, 150, 255], [41, 101, 214, 255], [41, 101, 214, 255], [178, 57, 87, 255]];
        let want77 = [[74, 113, 49, 67], [74, 113, 49, 158], [106, 37, 148, 128], [106, 37, 148, 128], [148, 59, 93, 118], [123, 94, 145, 102], [173, 24, 41, 134], [173, 24, 41, 53]];
        for (f, fmt, size, want) in [(71u64, DXT1, 8, want71), (77, DXT5, 16, want77)] {
            let d = fill(f);
            for x in 0..8 {
                let px = decode_dxt(fmt, &d[x / 4 * size..x / 4 * size + size])[x % 4];
                // (BGRA here, RGBA there)
                assert_eq!([px[2], px[1], px[0], px[3]], want[x], "format {f} texel {x}");
            }
        }
    }

    #[test]
    fn dxt_partial_blocks() {
        let mut blk = [0u8; 8];
        blk[..2].copy_from_slice(&0xf800u16.to_le_bytes()); // red
        let src: Vec<u8> = blk.iter().chain(blk.iter()).copied().collect();
        let out = conv(DXT1, &src, 5, 3);
        assert_eq!(out.len(), 5 * 3 * 4);
        assert!(out.chunks(4).all(|p| p == [0, 0, 255, 255]));
    }

    /// Every format on every region up to 9 x 9 from a source exactly as long as the region
    /// needs (the last row only as long as its texels): run under Miri (formal/miri.sh), a read
    /// past it or a write outside the pixels added is an error.
    #[test]
    fn every_format_on_exact_buffers() {
        let fmts = [
            R8G8B8, A8R8G8B8, X8R8G8B8, R5G6B5, X1R5G5B5, A1R5G5B5, A4R4G4B4, A8, X4R4G4B4, A8B8G8R8,
            X8B8G8R8, L8, A8L8, A4L4, P8, A16B16G16R16F, A32B32G32R32F, DXT1, DXT2, DXT3, DXT4, DXT5,
        ];
        for fmt in fmts {
            let (bpb, bs) = layout(fmt).unwrap();
            for w in 1..=9 {
                for h in 1..=9 {
                    let (pitch, rows) = pitch_rows(fmt, w, h);
                    let len = (pitch * (rows - 1) + w.div_ceil(bs) * bpb) as usize;
                    let src: Vec<u8> = (0..len).map(|i| (i * 37) as u8).collect();
                    let mut out = vec![7u8; 3];
                    unsafe { convert(fmt, src.as_ptr(), pitch as usize, w, h, &mut out) };
                    assert_eq!(out.len(), 3 + (w * h * page_bpp(fmt)) as usize);
                    assert_eq!(out[..3], [7, 7, 7]);
                    if fmt == P8 {
                        // (and through a palette)
                        let pal: [u8; 1024] = std::array::from_fn(|i| i as u8 ^ 0x55);
                        let mut out = vec![7u8; 3];
                        unsafe { convert_p8(src.as_ptr(), pitch as usize, w, h, &pal, &mut out) };
                        assert_eq!(out.len(), 3 + (w * h * 4) as usize);
                        assert_eq!(out[..3], [7, 7, 7]);
                    }
                }
            }
        }
    }

    #[test]
    fn pitches_and_rows() {
        assert_eq!(pitch_rows(A8R8G8B8, 3, 2), (12, 2));
        assert_eq!(pitch_rows(R8G8B8, 3, 1), (12, 1)); // (9 -> 12: rows 4-byte aligned)
        assert_eq!(pitch_rows(L8, 1, 1), (4, 1));
        assert_eq!(pitch_rows(DXT1, 1, 1), (8, 1)); // a whole block for a 1x1 level
        assert_eq!(pitch_rows(DXT5, 9, 5), (48, 2));
        for f in [A8R8G8B8, R5G6B5, A8, DXT1, DXT3, A16B16G16R16F, A32B32G32R32F] {
            let (bpb, bs) = layout(f).unwrap();
            for w in 1..20 {
                let (p, _) = pitch_rows(f, w, 1);
                assert!(p >= w.div_ceil(bs) * bpb && p % 4 == 0);
            }
        }
    }
}

/// Kani proofs (formal/kani.sh): the UNORM expansion and the pitch arithmetic for every input.
#[cfg(kani)]
mod proofs {
    use super::*;

    const FORMATS: [u32; 22] = [
        R8G8B8, A8R8G8B8, X8R8G8B8, R5G6B5, X1R5G5B5, A1R5G5B5, A4R4G4B4, A8, X4R4G4B4, A8B8G8R8,
        X8B8G8R8, L8, A8L8, A4L4, P8, A16B16G16R16F, A32B32G32R32F, DXT1, DXT2, DXT3, DXT4, DXT5,
    ];

    #[kani::proof]
    fn ext_is_rounded_unorm() {
        let bits: u32 = *[1u32, 4, 5, 6].get(kani::any_where(|&i: &usize| i < 4)).unwrap();
        let v: u32 = kani::any_where(|&v| v < 1 << bits);
        let max = (1 << bits) - 1;
        let e = u32::from(ext(v, bits));
        // |e - v * 255 / max| <= 1/2, in integers
        assert!(e <= 255);
        assert!((2 * e * max).abs_diff(2 * v * 255) <= max);
    }

    /// A P8 texel is its palette entry (red, green, blue, flags as the alpha) as BGRA8, for
    /// every palette and index.
    #[kani::proof]
    fn p8_texel_is_the_entry() {
        let pal: [u8; 1024] = kani::any();
        let i: u8 = kani::any();
        let t = p8_texel(&pal, i);
        let k = i as usize * 4;
        assert!(t == [pal[k + 2], pal[k + 1], pal[k], pal[k + 3]]);
    }

    /// No overflow for any level Direct3D 9 allows (8192 x 8192 at most), and the pitch covers a row.
    #[kani::proof]
    fn pitch_rows_no_overflow() {
        let fmt = FORMATS[kani::any_where(|&i: &usize| i < FORMATS.len())];
        let w: u32 = kani::any_where(|&w| (1..=8192).contains(&w));
        let h: u32 = kani::any_where(|&h| (1..=8192).contains(&h));
        let (bpb, bs) = layout(fmt).unwrap();
        let (pitch, rows) = pitch_rows(fmt, w, h);
        assert!(pitch >= w.div_ceil(bs) * bpb && pitch % 4 == 0 && pitch < w * bpb + 4);
        assert!(rows == h.div_ceil(bs));
    }
}
