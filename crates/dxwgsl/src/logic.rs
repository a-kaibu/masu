//! The output merger's logic operations (Direct3D 11.1 D3D11_LOGIC_OP, CreateBlendState1's
//! LogicOpEnable / LogicOp per render target) in the pixel shader: WebGPU has none, so the
//! shader reads the target's texel from a copy made before the primitive (binding
//! [`LOGIC_DST`] + target) and writes op(source, destination) to the format's channels.
//!
//! The source is the shader's output as the target stores it (an unsigned integer channel
//! saturated to its width, the format's own conversion), the destination the bits the target
//! holds; the op works on the channel's bits and the result keeps the channel's width. A
//! multisampled target is read and written sample by sample (sample_index).
//!
//! The page draws such a primitive by itself, after the copy, so a later primitive of the same
//! draw reads what an earlier one wrote (crates/kura d3d11 draw_logic).

/// The first binding of the targets' copies: LOGIC_DST + the target's slot.
pub const LOGIC_DST: u32 = 960;

/// D3D11_LOGIC_OP.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Op {
    Clear,
    Set,
    Copy,
    CopyInverted,
    Noop,
    Invert,
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Equiv,
    AndReverse,
    AndInverted,
    OrReverse,
    OrInverted,
}

impl Op {
    /// Every op, by its D3D11_LOGIC_OP value.
    pub const ALL: [Op; 16] = [
        Op::Clear,
        Op::Set,
        Op::Copy,
        Op::CopyInverted,
        Op::Noop,
        Op::Invert,
        Op::And,
        Op::Nand,
        Op::Or,
        Op::Nor,
        Op::Xor,
        Op::Equiv,
        Op::AndReverse,
        Op::AndInverted,
        Op::OrReverse,
        Op::OrInverted,
    ];

    /// Of its D3D11_LOGIC_OP value (0..15).
    pub fn of(v: u32) -> Option<Op> {
        Op::ALL.get(usize::try_from(v).ok()?).copied()
    }

    /// Whether the result depends on the destination (the target is copied and the draw split
    /// by primitives for it).
    pub fn reads_dst(self) -> bool {
        match self {
            Op::Clear | Op::Set | Op::Copy | Op::CopyInverted => false,
            Op::Noop | Op::Invert | Op::And | Op::Nand | Op::Or | Op::Nor | Op::Xor | Op::Equiv | Op::AndReverse | Op::AndInverted | Op::OrReverse | Op::OrInverted => true,
        }
    }

    /// Whether the result depends on the source.
    pub fn reads_src(self) -> bool {
        match self {
            Op::Clear | Op::Set | Op::Noop | Op::Invert => false,
            Op::Copy | Op::CopyInverted | Op::And | Op::Nand | Op::Or | Op::Nor | Op::Xor | Op::Equiv | Op::AndReverse | Op::AndInverted | Op::OrReverse | Op::OrInverted => true,
        }
    }

    /// The op on 32 bits (the caller keeps the channel's).
    pub fn apply(self, s: u32, d: u32) -> u32 {
        match self {
            Op::Clear => 0,
            Op::Set => u32::MAX,
            Op::Copy => s,
            Op::CopyInverted => !s,
            Op::Noop => d,
            Op::Invert => !d,
            Op::And => s & d,
            Op::Nand => !(s & d),
            Op::Or => s | d,
            Op::Nor => !(s | d),
            Op::Xor => s ^ d,
            Op::Equiv => !(s ^ d),
            Op::AndReverse => s & !d,
            Op::AndInverted => !s & d,
            Op::OrReverse => s | !d,
            Op::OrInverted => !s | d,
        }
    }

    /// The op in WGSL on vec4<u32> expressions `s` and `d`.
    pub fn wgsl(self, s: &str, d: &str) -> String {
        match self {
            Op::Clear => "vec4<u32>(0u)".to_owned(),
            Op::Set => "vec4<u32>(0xffffffffu)".to_owned(),
            Op::Copy => s.to_owned(),
            Op::CopyInverted => format!("~{s}"),
            Op::Noop => d.to_owned(),
            Op::Invert => format!("~{d}"),
            Op::And => format!("({s} & {d})"),
            Op::Nand => format!("~({s} & {d})"),
            Op::Or => format!("({s} | {d})"),
            Op::Nor => format!("~({s} | {d})"),
            Op::Xor => format!("({s} ^ {d})"),
            Op::Equiv => format!("~({s} ^ {d})"),
            Op::AndReverse => format!("({s} & ~{d})"),
            Op::AndInverted => format!("(~{s} & {d})"),
            Op::OrReverse => format!("({s} | ~{d})"),
            Op::OrInverted => format!("(~{s} | {d})"),
        }
    }
}

/// A target's logic op for the pixel shader: its slot, the op, the bits of each of its
/// channels (0: a channel the format has not), whether it is multisampled.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Logic {
    pub target: u32,
    pub op: Op,
    pub bits: [u32; 4],
    pub multisampled: bool,
}

/// The largest value of a channel of `bits` (0..=32).
pub fn channel_max(bits: u32) -> u32 {
    if bits >= 32 { u32::MAX } else { (1u32 << bits).wrapping_sub(1) }
}

/// What the target holds after a pixel of source `s` (the shader's output) over `d` (the
/// target's channels), by the module's rule.
pub fn result(l: &Logic, s: [u32; 4], d: [u32; 4]) -> [u32; 4] {
    std::array::from_fn(|c| {
        let (bits, sv, dv) = (l.bits.get(c).copied().unwrap_or(0), s.get(c).copied().unwrap_or(0), d.get(c).copied().unwrap_or(0));
        let max = channel_max(bits);
        l.op.apply(sv.min(max), dv & max) & max
    })
}

#[cfg(test)]
#[path = "logic_tests.rs"]
mod tests;
