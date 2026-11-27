//! Direct3D shader bytecode to WGSL.
//!
//! - Direct3D 10/11 (DXBC, shader model 4/5): [`translate`] and [`signature`] here; dxbc.rs reads
//!   the container, tpf.rs the shader model 4/5 program, wgsl.rs writes the WGSL.
//! - Direct3D 8/9 (vs/ps 1.x-3.0 bytecode): [`d3d9`]; the fixed-function pipeline: [`d3d9ff`].
//!
//! Bytes in, WGSL text and what it binds out; no GPU or OS calls, std only.

// (a u32 count or offset as an index: lossless on every target, wasm32 and 64-bit hosts)
const _: () = assert!(usize::BITS >= u32::BITS);

pub mod addr;
pub mod d3d9;
pub mod d3d9ff;
pub mod dxbc;
pub mod logic;
pub mod snorm;
pub mod tpf;
mod wgsl;

/// A finite f32 as WGSL literal text (no suffix): Rust's shortest decimal, unless that decimal
/// read as a double lies past f32's range. Browsers' WGSL compiler (Tint) reads a literal as a
/// double and rejects it when it exceeds f32's largest value ("value cannot be represented as
/// 'f32'"): the shortest text of f32::MAX, 3.4028235e38, does (Unity's shaders use FLT_MAX as
/// 3.402823466e+38); the double's own shortest text (3.4028234663852886e38) is exact.
pub fn f32_text(f: f32) -> String {
    let s = format!("{f:?}");
    match s.parse::<f64>() {
        Ok(d) if d.abs() <= f64::from(f32::MAX) => s,
        _ => format!("{:?}", f64::from(f)),
    }
}

/// A shader's input (`output` false) or output signature.
pub fn signature(code: &[u8], output: bool) -> Vec<crate::dxbc::SigElement> {
    crate::dxbc::Container::parse(code).map_or_default(|c| c.signature(output))
}

/// A hull shader's output or a domain shader's input patch constants (PCSG).
pub fn patch_constants(code: &[u8]) -> Vec<crate::dxbc::SigElement> {
    crate::dxbc::Container::parse(code).map_or_default(|c| c.patch_constants())
}

/// What a shader binds (group = stage)
#[derive(Clone, Debug, PartialEq)]
pub enum Binding {
    /// constant buffer: size the shader reads
    Uniform(u32),
    /// texture: view dimension (0 1d, 1 2d, 2 2d array, 3 cube, 4 cube array, 5 3d, 6
    /// multisampled 2d), sample
    /// type (0 float, 1 uint, 2 sint, 3 depth)
    Texture(u32, u32),
    /// texture t<n> as `ld` reads it (800 + n): a view of every level of the SRV's texture
    /// (its layers as the SRV's), the SRV's first level and level count in BufferRanges. (A view
    /// from a later level is not loaded from: SwiftShader's textureLoad of level l < base of a
    /// view whose base level is `base` reads level 2 * base; measured 2026-11-21, winref
    /// d3d11minlod's "load" row.) View dimension and sample type as for textures
    LoadTexture(u32, u32),
    Sampler(bool),
    /// compute shaders: a write-only storage texture (a typed UAV): view dimension as for
    /// textures, its WebGPU format
    Storage(u32, String),
    /// a storage buffer (the expanded draws' stores, wgsl/expand.rs): read-write or not
    StorageBuffer(bool),
    /// a raw or structured buffer view: t<n> at 500 + n (read-only), u<n> at 600 + n
    /// (read-write); the whole buffer from a 256-byte aligned offset at or before the view's
    /// first byte, the view's range in BufferRanges
    BufferView(bool),
    /// a structured UAV's hidden counter (700 + n): one u32
    Counter,
    /// the ranges of the views bound at these bindings (BUFFER_RANGES, a uniform): a
    /// vec4<u32> each, in order: the view's first dword in what is bound, its bytes, its
    /// elements (raw views: its bytes), 0; for a LoadTexture binding: the SRV's first level,
    /// its level count, 0, 0
    BufferRanges(Vec<u32>),
    /// a pixel shader target's copy (crate::logic, LOGIC_DST + its slot): the texel the target
    /// held before the primitive; multisampled or not, its channels' sign
    LogicDst { multisampled: bool, sign: logic::Sign },
    /// the blend state's constant for SNORM targets' blends (crate::snorm, SNORM_CONSTANT): a
    /// uniform vec4<f32>
    BlendConstant,
}

pub struct Wgsl {
    pub code: String,
    /// (binding, what)
    pub bindings: Vec<(u32, Binding)>,
}

/// DXBC to WGSL; `ps`: the pixel shader a vertex shader feeds (its outputs go to that shader's
/// input registers).
pub fn translate(code: &[u8], pixel: bool, ps: Option<&[u8]>) -> Result<Wgsl, String> {
    crate::wgsl::translate(code, pixel, ps)
}

pub use crate::wgsl::{BUFFER_RANGES, LOAD_TEXTURES, translate_addr, translate_logic, translate_setup, translate_views, sample_rate, texture_load_slots, translate_compute, translate_compute_views, translate_point_sprite, thread_group, uav_slots, GsDecl, Phase, TessDecl, TessFactor, TextureLoadView, PATCH_CONSTANTS};
/// Geometry shaders, stream output and clipper counts as compute passes (wgsl/expand.rs).
pub use crate::wgsl::expand;

/// The "geometry shader" sake's D3DCompile makes of ANGLE's point sprite geometry shader: this
/// tag, then the largest point size (f32). The D3D11 device draws points with it as sprites
/// (translate_point_sprite).
pub const POINT_SPRITE_GS: &[u8; 8] = b"SAKEPSGS";

/// The largest point size of a POINT_SPRITE_GS blob.
pub fn point_sprite_gs(code: &[u8]) -> Option<f32> {
    let rest = code.strip_prefix(POINT_SPRITE_GS.as_slice())?;
    Some(f32::from_le_bytes(rest.get(..4)?.try_into().ok()?))
}

/// WGSL a translator wrote, parsed and validated by naga (as the page's WebGPU will); a panic
/// shows the error and the code.
#[cfg(test)]
pub(crate) fn valid_wgsl(w: &str) -> naga::Module {
    let m = naga::front::wgsl::parse_str(w).unwrap_or_else(|e| panic!("{}\n{w}", e.emit_to_string(w)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&m)
        .unwrap_or_else(|e| panic!("{e:?}\n{w}"));
    m
}

#[cfg(test)]
mod f32_text_tests;
