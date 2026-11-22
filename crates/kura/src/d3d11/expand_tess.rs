//! Tessellation in an expanded draw (expand.rs `run`): which draws tessellate (the bound hull
//! and domain shaders, the topology: Direct3D 11's rules at feature level 11_0), the tess
//! factors of the hull shader pass's patch constants, the domain points the tessellator
//! (d3d11::tess) makes of them for the domain shader pass.

use super::super::tess::{self, Domain, Factors, Output, Partitioning};
use super::super::*;
use dxwgsl::{TessDecl, TessFactor};

#[cfg(test)]
thread_local! {
    /// The patch constants the next hull shader pass gives (its hpc words), for the WebGPU
    /// model, which runs no shader: in place of what it reads back (zeros).
    pub static MODEL_PATCH_CONSTANTS: std::cell::RefCell<Option<Vec<u32>>> = const { std::cell::RefCell::new(None) };
}

/// A draw's hull and domain shaders: objects, bytecode, the hull shader's declarations.
pub struct Stages {
    pub hs: u32,
    pub ds: u32,
    pub hs_code: Vec<u8>,
    pub ds_code: Vec<u8>,
    pub decl: TessDecl,
    pub partitioning: Partitioning,
    pub output: Output,
    pub domain: Domain,
}

/// What the bound stages make of a draw.
pub enum Tess {
    /// no hull or domain shader, a topology that is not a patch list: no tessellation
    Off,
    /// stages and topology that do not go together (a patch list without both shaders, a hull
    /// or domain shader with another topology, a patch list of another control point count
    /// than the hull shader's, a domain shader of another domain or of another control point
    /// count than the hull shader makes): an error the Direct3D 11 spec leaves undefined, the
    /// debug layer's to report (measured on Windows: WARP crashes in such a draw, an NVIDIA
    /// driver draws something of its own), drawn as nothing here
    Nothing,
    On(Box<Stages>),
}

/// Control points of each patch of a D3D11_PRIMITIVE_TOPOLOGY (33..64: 1..32), None for the
/// others.
pub fn patch_points(topology: u32) -> Option<u32> {
    (33..=64).contains(&topology).then(|| topology.saturating_sub(32))
}

fn code(d: &D11, id: u32) -> Option<Vec<u8>> {
    match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::Shader(s)) => Some(s.code.clone()),
        _ => None,
    }
}

/// The draw's tessellation by the bound hull (st.shaders[1]) and domain (st.shaders[2])
/// shaders and the topology.
pub fn stages(d: &D11, st: &State) -> Tess {
    let (hs, ds) = (st.shaders[1], st.shaders[2]);
    let Some(cps) = patch_points(st.topology) else {
        return if hs == 0 && ds == 0 { Tess::Off } else { Tess::Nothing };
    };
    let (Some(hs_code), Some(ds_code)) = (code(d, hs), code(d, ds)) else { return Tess::Nothing };
    let (Ok(decl), Ok(dd)) = (dxwgsl::expand::tess_decl(&hs_code), dxwgsl::expand::tess_decl(&ds_code)) else { return Tess::Nothing };
    let (Some(partitioning), Some(output), Some(domain)) = (Partitioning::of(decl.partitioning), Output::of(decl.output), Domain::of(decl.domain)) else { return Tess::Nothing };
    if !decl.hull || dd.hull || decl.in_cps != cps || dd.domain != decl.domain || dd.in_cps != decl.out_points() {
        return Tess::Nothing;
    }
    Tess::On(Box::new(Stages { hs, ds, hs_code, ds_code, decl, partitioning, output, domain }))
}

/// A patch's tess factors from its patch constant registers (the hull shader pass's hpc: the
/// fork and join phases' outputs), clamped to [maxtessfactor] as min() does it: a NaN becomes the
/// maximum (measured on Windows, winref d3d11tess "tri maxtessfactor(5.5)": a NaN edge draws
/// as 5.5 there, while without [maxtessfactor] it culls the patch).
pub fn factors(s: &Stages, regs: &[[u32; 4]]) -> Factors {
    let get = |f: TessFactor| -> f32 {
        let v = s.decl.factors.iter().find(|e| e.0 == f).and_then(|&(_, r, c)| regs.get(us(r)).and_then(|v| v.get(usize::from(c)))).map_or(0.0, |&b| f32::from_bits(b));
        match s.decl.max_tess {
            Some(m) if v > m || v.is_nan() => m,
            Some(_) | None => v,
        }
    };
    match s.domain {
        Domain::Isoline => Factors::Isoline { density: get(TessFactor::LineDensity), detail: get(TessFactor::LineDetail) },
        Domain::Tri => Factors::Tri { edges: [get(TessFactor::TriU0), get(TessFactor::TriV0), get(TessFactor::TriW0)], inside: get(TessFactor::TriInside) },
        Domain::Quad => Factors::Quad {
            edges: [get(TessFactor::QuadU0), get(TessFactor::QuadV0), get(TessFactor::QuadU1), get(TessFactor::QuadV1)],
            inside: [get(TessFactor::QuadInsideU), get(TessFactor::QuadInsideV)],
        },
    }
}

/// The domain points of a draw's patches, for the domain shader pass.
#[derive(Debug, Default, PartialEq)]
pub struct Points {
    /// dxwgsl::expand::DPT words a point: patch, u, v, w, SV_PrimitiveID
    pub dpts: Vec<u32>,
    /// the tessellator's primitives (Output::corners points each), as point numbers
    pub prims: Vec<Vec<u32>>,
    /// each primitive's patch's SV_PrimitiveID
    pub ids: Vec<u32>,
}

impl Points {
    pub fn count(&self) -> u32 {
        lo32(self.dpts.len().checked_div(us(dxwgsl::expand::DPT)).unwrap_or(0))
    }

    /// The domain shader runs Direct3D counts as DSInvocations: one a corner of each primitive
    /// (WARP's count, winref d3d11tess).
    pub fn corner_runs(&self) -> u64 {
        self.prims.iter().fold(0u64, |n, p| n.saturating_add(kome::num::u64_of(p.len())))
    }
}

/// Tessellates `patches` patches (`per_instance` an instance: SV_PrimitiveID counts from 0 in
/// each), patch k's constants `regs(k)`.
pub fn points(s: &Stages, patches: u32, per_instance: u32, regs: impl Fn(u32) -> Vec<[u32; 4]>) -> Points {
    let mut p = Points::default();
    let corners = us(s.output.corners());
    for k in 0..patches {
        let id = k.checked_rem(per_instance).unwrap_or(0);
        let t = tess::tessellate(s.partitioning, s.output, factors(s, &regs(k)));
        let base = p.count();
        for &[u, v] in &t.points {
            // (a tri's third coordinate as the hardware gives it: 1 - u - v)
            let w = match s.domain {
                Domain::Tri => 1.0 - u - v,
                Domain::Isoline | Domain::Quad => 0.0,
            };
            p.dpts.extend_from_slice(&[k, u.to_bits(), v.to_bits(), w.to_bits(), id]);
        }
        for c in t.indices.chunks_exact(corners) {
            p.prims.push(c.iter().map(|&i| base.saturating_add(i)).collect());
            p.ids.push(id);
        }
    }
    p
}

/// The topology the tessellator's primitives are, for the stages after it.
pub fn topo(o: Output) -> super::Topo {
    match o {
        Output::Point => super::Topo::Points,
        Output::Line => super::Topo::Lines,
        Output::TriangleCw | Output::TriangleCcw => super::Topo::Triangles,
    }
}

#[cfg(test)]
#[path = "expand_tess_tests.rs"]
mod tests;
