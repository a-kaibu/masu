//! Tessellation's passes (expand.rs's order: after the vertex pass, before the geometry
//! shader's):
//!
//! 1. [`hs_pass`]: the hull shader once per patch, its phases functions of their own: the
//!    control point phase once per output control point (vOutputControlPointID), each fork and
//!    join phase once per instance (vForkInstanceID, vJoinInstanceID). The input control points
//!    come from `vout` (the patch's vertex items, matched by semantic); the output control
//!    points go to `hcp[(patch * out_cps + c) * cp_regs + r]`, the patch constants to
//!    `hpc[patch * pc_regs + r]`.
//! 2. kura reads `hpc` back and tessellates each patch (kura d3d11::tess): its domain points
//!    into `dpts` (DPT words each).
//! 3. [`ds_pass`]: the domain shader once per domain point, the patch's control points and
//!    constants from hcp and hpc by semantic; its outputs into `vout` as the vertex pass's
//!    (what the geometry shader, stream output, the clipper and the store's draw read).
//!
//! Bindings besides expand.rs's: 432 hcp, 433 hpc, 434 dpts.

use super::super::{bits, declarations, module_decls, parse, Phase, TessDecl, W, PATCH_CONSTANTS};
use super::{store_regs, xd_decl, xyzw, ITEM};
use crate::dxbc::SigElement;
use crate::tpf::{self, Opcode};
use crate::{Binding, Wgsl};

pub const B_HCP: u32 = 432;
pub const B_HPC: u32 = 433;
pub const B_DPTS: u32 = 434;

/// Words of a domain point in `dpts`: the patch (of all the draw's), u, v, w (f32 bits: w is
/// 1 - u - v of a tri, 0 otherwise), the patch's SV_PrimitiveID (its number in its instance).
pub const DPT: u32 = 5;

/// A hull or domain shader's declarations (Err: not one, or not valid).
pub fn tess_decl(code: &[u8]) -> Result<TessDecl, String> {
    let (c, p) = parse(code)?;
    if p.kind != tpf::HULL && p.kind != tpf::DOMAIN {
        return Err(format!("program type {} for a hull or domain shader", p.kind));
    }
    let mut w = W::new(false, c.signature(false), c.signature(true));
    declarations(&mut w, &p)?;
    w.tess.ok_or("no tessellation declarations".into())
}

/// Registers an output control point keeps in hcp: the hull shader's outputs (or, without a
/// control point phase, its inputs passed through) + 1.
pub fn hs_cp_regs(hs: &[u8]) -> u32 {
    let osgn = crate::signature(hs, true);
    let isgn = crate::signature(hs, false);
    store_regs(&osgn, 0).max(store_regs(&isgn, 0))
}

/// The registers of `w`'s control point arrays: the signature's and the declared ones'.
fn cp_regs(sig: &[SigElement], decl: &std::collections::BTreeMap<u32, u32>) -> u32 {
    let declared = decl.keys().next_back().map_or(0, |r| r.saturating_add(1));
    store_regs(sig, 0).max(declared).min(PATCH_CONSTANTS)
}

/// The instructions of `p` translated phase by phase into `w`: each phase's body (none for
/// hs_decls).
fn phases(w: &mut W, p: &tpf::Program) -> Result<Vec<(Phase, String)>, String> {
    let mut out = Vec::new();
    let close = |w: &mut W, out: &mut Vec<(Phase, String)>| -> Result<(), String> {
        if !w.blocks.is_empty() {
            return Err(format!("unclosed block in the {:?} phase", w.phase));
        }
        let body = std::mem::take(&mut w.body);
        if w.phase != Phase::Decls {
            out.push((w.phase, body));
        }
        w.last_op = None;
        Ok(())
    };
    w.phase = Phase::Decls;
    for i in &p.insts {
        if let Some(ph) = Phase::of(i.op) {
            close(w, &mut out)?;
            w.phase = ph;
            continue;
        }
        if matches!(i.op, Opcode::DerivRtx | Opcode::DerivRty | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine | Opcode::Discard) {
            return Err(format!("instruction {:#x} outside a pixel shader", i.code));
        }
        w.inst(i)?;
    }
    close(w, &mut out)?;
    Ok(out)
}

/// Assignments of `dst[..][e.reg]` from `src_of(f.reg)` for each element `e` of `want` that
/// `have` has by semantic (name, index), component by component.
fn by_semantic(m: &mut String, want: &[SigElement], have: &[SigElement], dst: &dyn Fn(u32) -> String, src_of: &dyn Fn(u32) -> String) {
    for e in want {
        let Some(f) = have.iter().find(|f| f.name.eq_ignore_ascii_case(&e.name) && f.index == e.index && f.stream == 0) else { continue };
        for (dc, sc) in bits(e.mask).into_iter().zip(bits(f.mask)) {
            m.push_str(&format!("    {}.{} = {}.{};\n", dst(e.reg), xyzw(dc), src_of(f.reg), xyzw(sc)));
        }
    }
}

/// The hull shader pass (see the module's comment): one invocation per patch of the draw (xd
/// ITEMS of them), a patch's control points the vertex items `inst * COUNT + p * in_cps + k`
/// (patch `p` of instance `inst`, COUNT / in_cps patches an instance). `vs_osgn`: the vertex
/// shader's outputs, `vs_regs` registers each in vout.
pub fn hs_pass(hs: &[u8], vs_osgn: &[SigElement], vs_regs: u32) -> Result<Wgsl, String> {
    let (c, p) = parse(hs)?;
    if p.kind != tpf::HULL {
        return Err(format!("program type {} for a hull shader", p.kind));
    }
    let mut w = W::new(false, c.signature(false), c.signature(true));
    w.split = true;
    declarations(&mut w, &p)?;
    let t = w.tess.clone().ok_or("no hull shader declarations")?;
    let bodies = phases(&mut w, &p)?;
    let mut m = String::new();
    let mut bindings = Vec::new();
    let mut temps = String::new();
    module_decls(&mut w, &mut m, &mut bindings, 0, Some(&mut temps))?;
    xd_decl(&mut m, &mut bindings);
    let nicp = t.in_cps.max(1);
    let nocp = t.out_points().max(1);
    let nir = cp_regs(&w.isgn, &t.icp).max(1);
    let ocp_regs = hs_cp_regs(hs);
    let pcr = t.pc_regs();
    wln!(m, "@group(0) @binding({}) var<storage, read> vout: array<vec4<u32>>;", super::B_VOUT);
    wln!(m, "@group(0) @binding({B_HCP}) var<storage, read_write> hcp: array<vec4<u32>>;");
    wln!(m, "@group(0) @binding({B_HPC}) var<storage, read_write> hpc: array<vec4<u32>>;");
    for (b, rw) in [(super::B_VOUT, false), (B_HCP, true), (B_HPC, true)] {
        bindings.push((b, Binding::StorageBuffer(rw)));
    }
    wln!(m, "var<private> icp: array<array<vec4<u32>, {nir}>, {nicp}>;");
    wln!(m, "var<private> ocp: array<array<vec4<u32>, {ocp_regs}>, {nocp}>;");
    wln!(m, "var<private> pc: array<vec4<u32>, {PATCH_CONSTANTS}>;");
    m.push_str("var<private> vprim: vec4<u32>;\nvar<private> vocpid: vec4<u32>;\nvar<private> vfork: vec4<u32>;\nvar<private> vjoin: vec4<u32>;\n");
    let mut calls = Vec::new();
    for (k, (ph, body)) in bodies.iter().enumerate() {
        let name = format!("hs_phase{k}");
        w!(m, "\nfn {name}() {{\n{temps}{body}}}\n");
        calls.push((*ph, name));
    }
    wln!(m, "\n@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n    {ITEM}");
    wln!(m, "    let ppi = max(xw(XD_COUNT) / {nicp}u, 1u);\n    let inst = item / ppi;\n    let pid = item % ppi;\n    let base = inst * xw(XD_COUNT) + pid * {nicp}u;\n    vprim = vec4<u32>(pid);");
    // the input control points, from the vertex shader's outputs by semantic
    for k in 0..nicp {
        by_semantic(&mut m, &w.isgn, vs_osgn, &|r| format!("icp[{k}][{r}]"), &|r| format!("vout[(base + {k}u) * {vs_regs}u + {r}u]"));
    }
    // the output control points: the control point phase's, or the input ones passed through
    let outs: Vec<u32> = w.outputs.keys().copied().collect();
    if t.cp_phase {
        for (_, name) in calls.iter().filter(|c| c.0 == Phase::ControlPoint) {
            wln!(m, "    for (var c = 0u; c < {nocp}u; c += 1u) {{\n        vocpid = vec4<u32>(c);");
            for r in &outs {
                wln!(m, "        o{r} = vec4<u32>(0u);");
            }
            wln!(m, "        {name}();");
            for r in &outs {
                wln!(m, "        ocp[c][{r}] = o{r};");
            }
            m.push_str("    }\n");
        }
    } else {
        for k in 0..nocp.min(nicp) {
            by_semantic(&mut m, &w.osgn, &w.isgn, &|r| format!("ocp[{k}][{r}]"), &|r| format!("icp[{k}][{r}]"));
        }
    }
    // the fork phases, then the join phases, each instance in turn
    let (mut f, mut j) = (t.forks.iter(), t.joins.iter());
    for (ph, name) in &calls {
        let (var, n) = match ph {
            Phase::Fork => ("vfork", f.next().copied().unwrap_or(1)),
            Phase::Join => ("vjoin", j.next().copied().unwrap_or(1)),
            Phase::ControlPoint | Phase::Decls => continue,
        };
        wln!(m, "    for (var i = 0u; i < {n}u; i += 1u) {{\n        {var} = vec4<u32>(i);\n        {name}();\n    }}");
    }
    wln!(m, "    for (var c = 0u; c < {nocp}u; c += 1u) {{\n        for (var r = 0u; r < {ocp_regs}u; r += 1u) {{ hcp[(item * {nocp}u + c) * {ocp_regs}u + r] = ocp[c][r]; }}\n    }}");
    wln!(m, "    for (var r = 0u; r < {pcr}u; r += 1u) {{ hpc[item * {pcr}u + r] = pc[r]; }}");
    m.push_str("}\n");
    Ok(Wgsl { code: m, bindings })
}

/// The hull shader whose outputs a domain shader reads: its output control points' signature
/// and registers (hs_cp_regs), its patch constants' signature (PCSG) and registers (pc_regs),
/// its output control points.
#[derive(Clone, Debug)]
pub struct HsOut {
    pub osgn: Vec<SigElement>,
    pub cp_regs: u32,
    pub pcsg: Vec<SigElement>,
    pub pc_regs: u32,
    pub cps: u32,
}

impl HsOut {
    pub fn of(hs: &[u8]) -> Result<HsOut, String> {
        let t = tess_decl(hs)?;
        if !t.hull {
            return Err("a domain shader where a hull shader is wanted".into());
        }
        // (without a control point phase: its inputs, passed through)
        let osgn = if t.cp_phase { crate::signature(hs, true) } else { crate::signature(hs, false) };
        Ok(HsOut { osgn, cp_regs: hs_cp_regs(hs), pcsg: crate::patch_constants(hs), pc_regs: t.pc_regs(), cps: t.out_points().max(1) })
    }
}

/// The domain shader pass (see the module's comment): one invocation per domain point (xd
/// ITEMS of them, DPT words each in dpts), its outputs (0 .. store_regs) into
/// vout[item * regs + r].
pub fn ds_pass(ds: &[u8], hs: &HsOut) -> Result<Wgsl, String> {
    let (c, p) = parse(ds)?;
    if p.kind != tpf::DOMAIN {
        return Err(format!("program type {} for a domain shader", p.kind));
    }
    let mut w = W::new(false, c.signature(false), c.signature(true));
    w.split = true;
    declarations(&mut w, &p)?;
    let t = w.tess.clone().ok_or("no domain shader declarations")?;
    for i in &p.insts {
        if matches!(i.op, Opcode::DerivRtx | Opcode::DerivRty | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine | Opcode::Discard) {
            return Err(format!("instruction {:#x} outside a pixel shader", i.code));
        }
        w.inst(i)?;
    }
    if !w.blocks.is_empty() {
        return Err("unclosed block".into());
    }
    let mut m = String::new();
    let mut bindings = Vec::new();
    let mut temps = String::new();
    module_decls(&mut w, &mut m, &mut bindings, 0, Some(&mut temps))?;
    xd_decl(&mut m, &mut bindings);
    let nicp = t.in_cps.max(1);
    let nir = cp_regs(&w.isgn, &t.icp).max(1);
    let pcsg = crate::dxbc::Container::parse(ds).map_or_default(|c| c.patch_constants());
    wln!(m, "@group(0) @binding({B_HCP}) var<storage, read> hcp: array<vec4<u32>>;");
    wln!(m, "@group(0) @binding({B_HPC}) var<storage, read> hpc: array<vec4<u32>>;");
    wln!(m, "@group(0) @binding({B_DPTS}) var<storage, read> dpts: array<u32>;");
    wln!(m, "@group(0) @binding({}) var<storage, read_write> vout: array<vec4<u32>>;", super::B_VOUT);
    for (b, rw) in [(B_HCP, false), (B_HPC, false), (B_DPTS, false), (super::B_VOUT, true)] {
        bindings.push((b, Binding::StorageBuffer(rw)));
    }
    wln!(m, "var<private> icp: array<array<vec4<u32>, {nir}>, {nicp}>;");
    wln!(m, "var<private> pc: array<vec4<u32>, {PATCH_CONSTANTS}>;");
    m.push_str("var<private> vprim: vec4<u32>;\nvar<private> vdomain: vec4<u32>;\n");
    w!(m, "\nfn shader_main() {{\n{temps}{}}}\n\n", w.body);
    wln!(m, "@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n    {ITEM}");
    wln!(m, "    let pat = dpts[item * {DPT}u];\n    vdomain = vec4<u32>(dpts[item * {DPT}u + 1u], dpts[item * {DPT}u + 2u], dpts[item * {DPT}u + 3u], 0u);\n    vprim = vec4<u32>(dpts[item * {DPT}u + 4u]);");
    // (the hull shader's output control points: as many as it made, the rest zeros)
    for k in 0..nicp.min(hs.cps) {
        by_semantic(&mut m, &w.isgn, &hs.osgn, &|r| format!("icp[{k}][{r}]"), &|r| format!("hcp[(pat * {}u + {k}u) * {}u + {r}u]", hs.cps, hs.cp_regs));
    }
    by_semantic(&mut m, &pcsg, &hs.pcsg, &|r| format!("pc[{r}]"), &|r| format!("hpc[pat * {}u + {r}u]", hs.pc_regs));
    m.push_str("    shader_main();\n");
    let regs = store_regs(&w.osgn, 0);
    for r in 0..regs {
        let v = if w.outputs.contains_key(&r) { format!("o{r}") } else { "vec4<u32>(0u)".into() };
        wln!(m, "    vout[item * {regs}u + {r}u] = {v};");
    }
    m.push_str("}\n");
    Ok(Wgsl { code: m, bindings })
}
