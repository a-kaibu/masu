//! The output merger's integer stores in the pixel shader: Direct3D 11.1's logic operations
//! (D3D11_LOGIC_OP, CreateBlendState1's LogicOpEnable / LogicOp per render target) and the
//! saturation of an integer target's plain writes. WebGPU has no logic ops and leaves an
//! integer attachment's out-of-range values to the implementation, so the shader reads the
//! target's texel from a copy made before the primitive (binding [`LOGIC_DST`] + target) and
//! writes the stored channels itself.
//!
//! As WARP measured them (winref d3d11logic):
//! - a logic op works on the channel's low bits: the source is the shader's output & the
//!   channel's mask (not saturated), the destination the bits the target holds, the result
//!   kept in the channel's width; UINT and SINT targets alike (FORMAT_SUPPORT2 names only the
//!   UINT formats, WARP applies the op to SINT ones too); UNORM, SNORM and FLOAT targets ignore it;
//! - without a logic op an integer target saturates: UINT to 0..max, SINT to its range;
//! - a multisampled target: the op on each covered sample's bits, the shader still run once a
//!   pixel (its position, interpolation and SV_Coverage a pixel's: crates/kura d3d11 logic.rs
//!   draws it in two passes, the shader's ([`Store::Stash`]) then [`resolve_wgsl`]'s), a shader
//!   of sample rate a sample's own.
//!
//! The page draws a primitive whose op reads the destination by itself, after the copy, so a
//! later primitive of the same draw reads what an earlier one wrote (crates/kura d3d11 logic.rs).

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

/// Whether an integer target's channels are signed (SINT) or not (UINT).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Sign {
    Unsigned,
    Signed,
}

/// How the pixel shader writes an integer target:
/// - `Plain`: its output saturated (no logic op);
/// - `Direct`: the logic op's result over the target's copy (a target that is not
///   multisampled, or a shader of sample rate);
/// - `Stash`: the source's bits into a multisampled UINT stash of the target's channels
///   ([`Logic::stash_bits`]), the samples it covers marked (`mark`) (a pixel-rate shader over a
///   multisampled target: the resolve pass, [`resolve_wgsl`], applies the op to each sample it
///   marked);
/// - `Snorm`: an 8-bit SNORM target drawn as its SINT twin (crate::snorm): the output's bytes,
///   blended over the twin's copy when the target blends.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Store {
    Plain,
    Direct(Op),
    Stash { op: Op, mark: Mark },
    Snorm(Option<crate::snorm::Blend>),
}

/// Where a draw's stash pass marks the samples it covers:
/// - `Slot`: an R8_UINT attachment at that output location, 1 at a covered sample: a location
///   of its own (fewer than 8 targets), or a stash target's, that target's source then in a
///   storage buffer ([`LOGIC_SRC`]: 8 targets and no stash that can carry the mark; the shader
///   still once a pixel, its source a pixel's);
/// - `Carried`: bit `bit` of channel `channel` of target `by`'s stash, a stash wider than its
///   target ([`carry`]: 8 targets, no location free).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub enum Mark {
    Slot(u32),
    Carried { by: u32, channel: u32, bit: u32 },
}

impl Mark {
    /// The channel and bit of the mark's texel that says covered.
    pub fn flag(self) -> (u32, u32) {
        match self {
            Mark::Slot(_) => (0, 0),
            Mark::Carried { channel, bit, .. } => (channel, bit),
        }
    }
}

/// A stash of channels `bits` that carries the mark: its channels (a format WebGPU
/// multisamples: 8 or 16 bits a channel, one or two or four channels), the mark's channel and
/// bit; None when no such format is wider (four 16-bit or any 32-bit channels).
/// - one or two channels of 8 or 16 bits: a channel more (R to RG, RG to RGBA), the mark its
///   bit 0;
/// - four channels of 10 bits or less (RGBA8, RGB10A2): RGBA16, the mark the bit above
///   channel 0's.
pub fn carry(bits: [u32; 4]) -> Option<([u32; 4], u32, u32)> {
    let n = bits.iter().take_while(|&&b| b != 0).count();
    let w = bits[0];
    match (n, w) {
        (1, 8 | 16) => Some(([w, w, 0, 0], 1, 0)),
        (2, 8 | 16) => Some(([w, w, w, w], 2, 0)),
        (4, _) if bits.iter().all(|&b| b <= 10) => Some(([16, 16, 16, 16], 0, w)),
        _ => None,
    }
}

/// An integer target for the pixel shader: its slot, how it is written, the bits of each of
/// its channels (0: a channel the format has not), their sign, whether it is multisampled.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, PartialOrd, Ord)]
pub struct Logic {
    pub target: u32,
    pub store: Store,
    pub bits: [u32; 4],
    pub sign: Sign,
    pub multisampled: bool,
}

impl Logic {
    /// The logic op, if any.
    pub fn op(&self) -> Option<Op> {
        match self.store {
            Store::Plain | Store::Snorm(_) => None,
            Store::Direct(op) | Store::Stash { op, .. } => Some(op),
        }
    }

    /// Whether the target's location is the mark's: its source in the [`LOGIC_SRC`] buffer.
    pub fn buffered(&self) -> bool {
        matches!(self.store, Store::Stash { mark: Mark::Slot(at), .. } if at == self.target)
    }

    /// The channels of the target's stash: its own, or [`carry`]'s when it carries the mark.
    pub fn stash_bits(&self) -> [u32; 4] {
        match self.store {
            Store::Stash { mark: Mark::Carried { by, .. }, .. } if by == self.target => carry(self.bits).map_or(self.bits, |c| c.0),
            Store::Plain | Store::Direct(_) | Store::Stash { .. } | Store::Snorm(_) => self.bits,
        }
    }

    /// Whether the target's copy is read (by the shader, `Direct` and a blending `Snorm`, or
    /// the resolve pass).
    pub fn reads_copy(&self) -> bool {
        match self.store {
            Store::Snorm(b) => b.is_some_and(|b| b.reads_dst()),
            Store::Plain | Store::Direct(_) | Store::Stash { .. } => self.op().is_some_and(Op::reads_dst),
        }
    }

    /// The channels' masks as a WGSL vec4<u32>.
    pub fn max_wgsl(&self) -> String {
        let max: Vec<String> = self.bits.iter().map(|&b| format!("{}u", channel_max(b))).collect();
        format!("vec4<u32>({})", max.join(", "))
    }

    /// WGSL of `bits` (a vec4<u32> of the channels' bits, within their masks) as the
    /// attachment's value: as they are (UINT), sign-extended from each channel's width (SINT).
    pub fn bits_out_wgsl(&self, bits: &str) -> String {
        match self.sign {
            Sign::Unsigned => bits.to_owned(),
            Sign::Signed => {
                let sh: Vec<String> = self.bits.iter().map(|&b| format!("{}u", 32u32.saturating_sub(b).min(31))).collect();
                let on: Vec<&str> = self.bits.iter().map(|&b| if b == 0 { "false" } else { "true" }).collect();
                format!("select(vec4<i32>(0), bitcast<vec4<i32>>(({bits}) << vec4<u32>({0})) >> vec4<u32>({0}), vec4<bool>({1}))", sh.join(", "), on.join(", "))
            }
        }
    }

    /// WGSL of the attachment's value `v` (vec4<u32>, or vec4<i32> for SINT) as the channels'
    /// bits within their masks.
    pub fn bits_in_wgsl(&self, v: &str) -> String {
        match self.sign {
            Sign::Unsigned => format!("({v} & {})", self.max_wgsl()),
            Sign::Signed => format!("(bitcast<vec4<u32>>({v}) & {})", self.max_wgsl()),
        }
    }

    /// WGSL of the shader's output bits `o` (vec4<u32>) saturated to the channels, as the
    /// attachment's value.
    pub fn saturate_wgsl(&self, o: &str) -> String {
        match self.sign {
            Sign::Unsigned => format!("min({o}, {})", self.max_wgsl()),
            Sign::Signed => {
                let lo: Vec<String> = self.bits.iter().map(|&b| if b == 0 { "0".to_owned() } else { format!("{}", i64::from(channel_max(b) >> 1).saturating_neg().saturating_sub(1)) }).collect();
                let hi: Vec<String> = self.bits.iter().map(|&b| format!("{}", channel_max(b) >> 1)).collect();
                format!("clamp(bitcast<vec4<i32>>({o}), vec4<i32>({}), vec4<i32>({}))", lo.join(", "), hi.join(", "))
            }
        }
    }

    /// The attachment's WGSL scalar type.
    pub fn scalar(&self) -> &'static str {
        match self.sign {
            Sign::Unsigned => "u32",
            Sign::Signed => "i32",
        }
    }
}

/// The largest value of a channel of `bits` (0..=32).
pub fn channel_max(bits: u32) -> u32 {
    if bits >= 32 { u32::MAX } else { (1u32 << bits).wrapping_sub(1) }
}

/// The channel's bits saturated from `v` (a shader's output bits): UINT to 0..max, SINT to
/// -2^(bits-1)..2^(bits-1)-1 (as two's complement bits of the channel).
pub fn saturate(bits: u32, sign: Sign, v: u32) -> u32 {
    let max = channel_max(bits);
    match sign {
        Sign::Unsigned => v.min(max),
        Sign::Signed if bits == 0 => 0,
        Sign::Signed => {
            let hi = i64::from(max >> 1);
            let x = i64::from(v.cast_signed()).clamp(hi.saturating_neg().saturating_sub(1), hi);
            u32::try_from(x & i64::from(max)).unwrap_or(0)
        }
    }
}

/// What the target holds (its channels' bits) after a pixel of source `s` (the shader's
/// output bits) over `d` (the target's channels' bits), by the module's rule; an SNORM target
/// with no second source and a constant of 0 ([`result_with`]).
pub fn result(l: &Logic, s: [u32; 4], d: [u32; 4]) -> [u32; 4] {
    result_with(l, s, [0; 4], [0.0; 4], d)
}

/// [`result`] with the second source's output bits `s1` (SV_Target1) and the blend state's
/// constant `k`, which an SNORM target's blend may read (crate::snorm::blend).
pub fn result_with(l: &Logic, s: [u32; 4], s1: [u32; 4], k: [f32; 4], d: [u32; 4]) -> [u32; 4] {
    let byte = |b: i8| u32::from(b.cast_unsigned());
    let snorm = |v: [u32; 4]| v.map(|b| kome::num::lo8(b).cast_signed());
    match l.store {
        Store::Snorm(None) => s.map(|v| byte(crate::snorm::store(f32::from_bits(v)))),
        Store::Snorm(Some(b)) => crate::snorm::blend(&b, s.map(f32::from_bits), s1.map(f32::from_bits), snorm(d), k).map(byte),
        Store::Plain | Store::Direct(_) | Store::Stash { .. } => std::array::from_fn(|c| {
            let (bits, sv, dv) = (l.bits.get(c).copied().unwrap_or(0), s.get(c).copied().unwrap_or(0), d.get(c).copied().unwrap_or(0));
            let max = channel_max(bits);
            match l.op() {
                None => saturate(bits, l.sign, sv),
                Some(op) => op.apply(sv & max, dv & max) & max,
            }
        }),
    }
}

/// The first binding of the multisampled stashes of a pixel-rate shader's sources (the
/// resolve pass's): LOGIC_STASH + the target's slot.
pub const LOGIC_STASH: u32 = 970;
/// The resolve pass's mark of the samples the shader wrote (an r8uint multisampled texture, or
/// the stash that carries it: [`Mark`]).
pub const LOGIC_MARK: u32 = 980;
/// A stash target's source when its location is the mark's ([`Mark::Slot`] at the target's own
/// location): a storage buffer of u32, word 0 the target's width, then 4 words a pixel (row by
/// row from word 4).
pub const LOGIC_SRC: u32 = 990;

/// The WGSL of the resolve pass of a `Stash` target (None for the others): `vs_resolve` the
/// full-target triangle, `fs_resolve` run for each sample: the sample the shader wrote (its
/// mark 1) gets the op of the stash's source over the target's copy, the rest nothing.
pub fn resolve_wgsl(l: &Logic) -> Option<String> {
    let Store::Stash { op, mark } = l.store else { return None };
    let k = l.scalar();
    let (ch, bit) = mark.flag();
    let (dst, stash, mark) = (LOGIC_DST.saturating_add(l.target), LOGIC_STASH.saturating_add(l.target), LOGIC_MARK);
    let load = |t: &str, z: &str| format!("select({z}, textureLoad({t}, q, i32(si)), all(vec2<u32>(q) < textureDimensions({t})))");
    let (stash_decl, s) = if l.buffered() {
        let at = "(4u + (u32(q.y) * stash[0] + u32(q.x)) * 4u)";
        let word = |c: u32| format!("select(0u, stash[{at} + {c}u], {at} + 3u < arrayLength(&stash))");
        (format!("@group(0) @binding({stash}) var<storage, read> stash: array<u32>;"), format!("(vec4<u32>({}, {}, {}, {}) & {})", word(0), word(1), word(2), word(3), l.max_wgsl()))
    } else {
        (format!("@group(0) @binding({stash}) var stash: texture_multisampled_2d<u32>;"), format!("({} & {})", load("stash", "vec4<u32>()"), l.max_wgsl()))
    };
    let d = l.bits_in_wgsl(&load("dst", &format!("vec4<{k}>()")));
    let m = load("mark", "vec4<u32>()");
    let out = l.bits_out_wgsl(&format!("({}) & {}", op.wgsl("s", "d"), l.max_wgsl()));
    Some(format!(
        "@group(0) @binding({dst}) var dst: texture_multisampled_2d<{k}>;\n\
         {stash_decl}\n\
         @group(0) @binding({mark}) var mark: texture_multisampled_2d<u32>;\n\
         @vertex fn vs_resolve(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{\n\
         \x20 return vec4<f32>(f32(i >> 1u) * 4.0 - 1.0, f32(i & 1u) * 4.0 - 1.0, 0.0, 1.0);\n\
         }}\n\
         @fragment fn fs_resolve(@builtin(position) p: vec4<f32>, @builtin(sample_index) si: u32) -> @location(0) vec4<{k}> {{\n\
         \x20 let q = vec2<i32>(p.xy);\n\
         \x20 if (((({m})[{ch}] >> {bit}u) & 1u) == 0u) {{ discard; }}\n\
         \x20 let s = {s};\n\
         \x20 let d = {d};\n\
         \x20 return {out};\n\
         }}\n"
    ))
}

#[cfg(test)]
#[path = "logic_tests.rs"]
mod tests;
