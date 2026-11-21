//! An HLSL compiler (what d3dcompiler_*.dll's D3DCompile does): preprocessor (pp.rs), parser
//! (parse.rs), lowering with every call inlined (lower.rs, intrinsics.rs, prelude.rs) and shader
//! model 4/5 DXBC output (sm4.rs), or shader model 2/3 bytecode (sm3.rs, through the Direct3D 9
//! assembler in d3d9asm.rs) for Direct3D 9. Effects (.fx) compile to fx_2_0 (fx.rs).
//!
//! Source text in, bytecode out; std only. The dxwgsl crate turns the output into WGSL.

// (Kani 0.68's compiler, nightly 2026-08-20, has these Rust 1.100 library features still
// unstable: formal/kani.sh builds this crate with it; masu-lint kani-features)
#![cfg_attr(kani, feature(vec_from_fn, bool_toggle, trim_prefix_suffix))]

// (safe Rust, no shared state: tools/ci/sanitize.sh leaves the crate out of TSan, where it can
// have no data race to find, #242)
#![forbid(unsafe_code)]

// (a u32 count or register as an index: lossless on every target, wasm32 and 64-bit hosts)
const _: () = assert!(usize::BITS >= u32::BITS);

/// A count the formats keep in 16 bits (u16::MAX past that)
pub(crate) fn sat16<T: TryInto<u16>>(x: T) -> u16 {
    x.try_into().unwrap_or(u16::MAX)
}

mod ast;
pub mod d3d9asm;
mod fx;
mod intrinsics;
mod lower;
mod parse;
pub mod pp;
mod prelude;
pub mod sm3;
pub mod sm4;

/// D3DCOMPILE_* flags used here
pub const PACK_MATRIX_ROW_MAJOR: u32 = 1 << 3;
pub const IEEE_STRICTNESS: u32 = 1 << 13;
/// (not a D3DCOMPILE flag) the blob as D3D10CompileShader writes it: `sm4::Shader::d3d10`
pub const D3D10_BLOB: u32 = 1 << 31;

/// The pipeline stage a target compiles for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Vertex,
    Pixel,
    Geometry,
    /// hs_5_0: the control point phase and the patch constant function's
    Hull,
    /// ds_5_0
    Domain,
    Compute,
}

pub struct Target {
    pub stage: Stage,
    pub major: u32,
    pub minor: u32,
}

impl Target {
    pub fn pixel(&self) -> bool {
        self.stage == Stage::Pixel
    }
}

/// "ps_4_0", "vs_5_0", "vs_4_0_level_9_3"...
pub fn parse_target(t: &str) -> Result<Target, String> {
    let v: Vec<&str> = t.split('_').collect();
    if v.len() < 3 {
        return Err(format!("unknown target {t}"));
    }
    let stage = match v[0] {
        "ps" => Stage::Pixel,
        "vs" => Stage::Vertex,
        "gs" => Stage::Geometry,
        "hs" => Stage::Hull,
        "ds" => Stage::Domain,
        "cs" => Stage::Compute,
        s => return Err(format!("{s} shaders are not supported")),
    };
    let major: u32 = v[1].parse().map_err(|_| format!("unknown target {t}"))?;
    let minor: u32 = if v[2] == "sw" {
        0
    } else {
        v[2].parse().unwrap_or(0)
    };
    let min_major = match stage {
        Stage::Vertex | Stage::Pixel => 0,
        Stage::Geometry | Stage::Compute => 4,
        // (hs_5_0, ds_5_0 only: tessellation is Direct3D 11's)
        Stage::Hull | Stage::Domain => 5,
    };
    if stage != Stage::Vertex && stage != Stage::Pixel && (major < min_major || v.len() > 3) {
        return Err(format!("unknown target {t}"));
    }
    Ok(Target { stage, major, minor })
}

/// Preprocessed tokens of `src`.
pub fn preprocess(
    src: &str,
    defines: &[(String, String)],
    include: &mut dyn FnMut(&str, bool) -> Option<String>,
) -> Result<Vec<pp::Token>, String> {
    pp::preprocess(src, defines, include)
}

/// HLSL source to shader model 2/3 assembly (for tests and dumps).
pub fn assembly(
    src: &str,
    defines: &[(String, String)],
    include: &mut dyn FnMut(&str, bool) -> Option<String>,
    entry: &str,
    target: &str,
    flags: u32,
) -> Result<String, String> {
    let t = parse_target(target)?;
    let (prog, pre) = parse_program(src, defines, include)?;
    let shader = lower_entry(&prog, &pre, entry, &[], &t, flags)?;
    Ok(sm3::assembly(shader)?.0)
}

/// The program and the prelude (intrinsics written in HLSL).
fn parse_program(
    src: &str,
    defines: &[(String, String)],
    include: &mut dyn FnMut(&str, bool) -> Option<String>,
) -> Result<(ast::Program, &'static ast::Program), String> {
    let toks = pp::preprocess(src, defines, include)?;
    let mut p = parse::Parser::new(toks);
    p.program()?;
    Ok((p.prog, prelude_program()?))
}

/// The prelude parsed once (it does not depend on the source, its defines or includes): each
/// D3DCompile / D3DXCompileShader / effect paid for preprocessing and parsing it again
fn prelude_program() -> Result<&'static ast::Program, String> {
    static PRE: std::sync::OnceLock<Result<ast::Program, String>> = std::sync::OnceLock::new();
    PRE.get_or_init(|| {
        let mut none = |_: &str, _: bool| None;
        let pre_toks = pp::preprocess(&prelude::source(), &[], &mut none)?;
        let mut pp2 = parse::Parser::new(pre_toks);
        pp2.program().map_err(|e| format!("prelude: {e}"))?;
        Ok(pp2.prog)
    })
    .as_ref()
    .map_err(|e| e.clone())
}

/// Entry point `entry` (with `args` for its uniform parameters) lowered for target `t`.
fn lower_entry(prog: &ast::Program, pre: &ast::Program, entry: &str, args: &[ast::Expr], t: &Target, flags: u32) -> Result<sm4::Shader, String> {
    // (the prelude shares the program's struct table: it has no structs)
    let minor = if t.major >= 5 { 0 } else if t.major == 4 { t.minor.min(1) } else { t.minor };
    let o = lower::Opts { stage: t.stage, major: t.major.min(5), minor, row_major: flags & PACK_MATRIX_ROW_MAJOR != 0, ieee: flags & IEEE_STRICTNESS != 0 };
    lower::L::new(o, prog, pre).entry(entry, args)
}

fn finish(shader: sm4::Shader, t: &Target) -> Result<Vec<u8>, String> {
    if t.major < 4 {
        return sm3::finish(shader);
    }
    sm4::finish(shader)
}

/// HLSL source to a DXBC blob (or shader model 2/3 bytecode).
pub fn compile(
    src: &str,
    defines: &[(String, String)],
    include: &mut dyn FnMut(&str, bool) -> Option<String>,
    entry: &str,
    target: &str,
    flags: u32,
) -> Result<Vec<u8>, String> {
    let t = parse_target(target)?;
    let (prog, pre) = parse_program(src, defines, include)?;
    let mut sh = lower_entry(&prog, &pre, entry, &[], &t, flags)?;
    sh.d3d10 = flags & D3D10_BLOB != 0;
    finish(sh, &t)
}

/// Effect source (.fx) to a compiled fx_2_0 effect.
pub fn compile_effect(
    src: &str,
    defines: &[(String, String)],
    include: &mut dyn FnMut(&str, bool) -> Option<String>,
    flags: u32,
) -> Result<Vec<u8>, String> {
    let (prog, pre) = parse_program(src, defines, include)?;
    fx::effect(&prog, &pre, flags)
}

#[cfg(test)]
mod tests;
