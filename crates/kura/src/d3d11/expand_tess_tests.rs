//! Tessellation in the expanded draws: the tess factors of a patch's constants (the winref
//! d3d11tess probe's hull shaders, as hlslc compiles them and dxwgsl reads their
//! declarations), the domain points and primitives a draw's patches make (property tests
//! against d3d11::tess), and tessellated draws on the WebGPU model: which draw (Direct3D 11's
//! rules), the vertex, hull and domain shader passes, stream output of the domain shader, the
//! store drawn, HSInvocations and DSInvocations.

use super::super::super::query_tests::{cc, code, dc, device, made, mem, no_model_errors, put, query, rd1, release_all, u64s, wait};
use super::*;
use crate::d3d9::trace_tests::SERIAL;
use proptest::prelude::*;

fn compiled(entry: &str, target: &str) -> Vec<u8> {
    let src = hlslc::probe_source(include_str!("../../../../tests/winref/probes/d3d11tess.c"));
    hlslc::compile(&src, &[], &mut |_, _| None, entry, target, 0).unwrap_or_else(|e| panic!("{entry}: {e}"))
}

fn stages_of(hs: &str, ds: &str) -> Stages {
    let hs_code = compiled(hs, "hs_5_0");
    let ds_code = compiled(ds, "ds_5_0");
    let decl = dxwgsl::expand::tess_decl(&hs_code).unwrap_or_else(|e| panic!("{hs}: {e}"));
    let (Some(partitioning), Some(output), Some(domain)) = (Partitioning::of(decl.partitioning), Output::of(decl.output), Domain::of(decl.domain)) else { panic!("{hs}: {decl:?}") };
    Stages { hs: 1, ds: 2, hs_code, ds_code, decl, partitioning, output, domain }
}

/// Patch constant registers holding `v` for each factor of `s` (in TessFactor's order).
fn regs_with(s: &Stages, v: &[(TessFactor, f32)]) -> Vec<[u32; 4]> {
    let mut regs = vec![[0u32; 4]; us(s.decl.pc_regs())];
    for &(f, x) in v {
        let &(_, r, c) = s.decl.factors.iter().find(|e| e.0 == f).unwrap_or_else(|| panic!("no {f:?}"));
        regs[us(r)][usize::from(c)] = x.to_bits();
    }
    regs
}

#[test]
fn factors_of_the_patch_constants() {
    // (SV_TessFactor[0] of an isoline: its density, [1] its detail)
    let s = stages_of("hs_line_int", "ds_line");
    let f = factors(&s, &regs_with(&s, &[(TessFactor::LineDensity, 3.0), (TessFactor::LineDetail, 5.0)]));
    assert_eq!(f, Factors::Isoline { density: 3.0, detail: 5.0 });
    let s = stages_of("hs_quad_int", "ds_quad");
    let v = [(TessFactor::QuadU0, 1.0), (TessFactor::QuadV0, 2.0), (TessFactor::QuadU1, 3.0), (TessFactor::QuadV1, 4.0), (TessFactor::QuadInsideU, 5.0), (TessFactor::QuadInsideV, 6.0)];
    assert_eq!(factors(&s, &regs_with(&s, &v)), Factors::Quad { edges: [1.0, 2.0, 3.0, 4.0], inside: [5.0, 6.0] });
    // ([maxtessfactor(5.5)]: above it 5.5, a NaN stays NaN and culls)
    let s = stages_of("hs_tri_max", "ds_tri");
    assert_eq!(s.decl.max_tess, Some(5.5));
    let f = factors(&s, &regs_with(&s, &[(TessFactor::TriU0, 2.0), (TessFactor::TriV0, 7.0), (TessFactor::TriW0, f32::NAN), (TessFactor::TriInside, 64.0)]));
    let Factors::Tri { edges, inside } = f else { panic!("{f:?}") };
    assert_eq!((edges[0], edges[1], inside), (2.0, 5.5, 5.5));
    assert!(edges[2].is_nan());
    // (a factor in no register: 0, the patch culled)
    let s = stages_of("hs_tri_int", "ds_tri");
    assert_eq!(factors(&s, &[]), Factors::Tri { edges: [0.0; 3], inside: 0.0 });
}

const PAIRS: [(&str, &str); 8] = [
    ("hs_tri_int", "ds_tri"),
    ("hs_tri_even", "ds_tri"),
    ("hs_tri_point", "ds_tri"),
    ("hs_quad_odd", "ds_quad"),
    ("hs_quad_pow2", "ds_quad"),
    ("hs_quad_point", "ds_quad"),
    ("hs_line_even", "ds_line"),
    ("hs_line_point", "ds_line"),
];

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]
    /// A draw's points are each patch's tessellation in patch order: DPT words a point (the
    /// patch, u, v, a tri's w = 1 - u - v, the patch's SV_PrimitiveID: its number in its
    /// instance), the primitives the tessellator's indices moved past the earlier patches'
    /// points, each its patch's id.
    #[test]
    fn points_are_the_tessellators(pair in 0usize..PAIRS.len(), raw in prop::collection::vec(prop::sample::select(vec![0.0f32, -1.0, f32::NAN, 1.0, 1.5, 2.0, 3.3, 4.0, 6.75, 9.0, 70.0]), 6 * 4), patches in 1u32..5, per in 1u32..4) {
        let s = stages_of(PAIRS[pair].0, PAIRS[pair].1);
        let order: Vec<TessFactor> = s.decl.factors.iter().map(|e| e.0).collect();
        let regs = |k: u32| regs_with(&s, &order.iter().zip(&raw[us(k) * 6..]).map(|(&f, &v)| (f, v)).collect::<Vec<_>>());
        let p = points(&s, patches, per, regs);
        let mut at = 0u32;
        let mut prims = 0usize;
        for k in 0..patches {
            let t = tess::tessellate(s.partitioning, s.output, factors(&s, &regs(k)));
            let n = lo32(t.points.len());
            for (i, &[u, v]) in (0u32..).zip(&t.points) {
                let w = us((at + i) * dxwgsl::expand::DPT);
                let tri = if s.domain == Domain::Tri { 1.0 - u - v } else { 0.0 };
                prop_assert_eq!(&p.dpts[w..w + 5], &[k, u.to_bits(), v.to_bits(), tri.to_bits(), k % per][..]);
            }
            for c in t.indices.chunks_exact(us(s.output.corners())) {
                prop_assert_eq!(&p.prims[prims], &c.iter().map(|&i| i + at).collect::<Vec<_>>());
                prop_assert_eq!(p.ids[prims], k % per);
                prop_assert!(p.prims[prims].iter().all(|&i| i >= at && i < at + n));
                prims += 1;
            }
            at += n;
        }
        prop_assert_eq!(p.count(), at);
        prop_assert_eq!(p.prims.len(), prims);
    }
}

#[test]
fn patch_points_of_the_topologies() {
    assert_eq!((patch_points(32), patch_points(33), patch_points(64), patch_points(65)), (None, Some(1), Some(32), None));
    for t in [1, 2, 3, 4, 5, 10, 11, 12, 13] {
        assert_eq!(patch_points(t), None);
    }
}

/// Tessellated draws on the WebGPU model: what Direct3D does not draw dispatches and draws
/// nothing; a patch list with both shaders runs the vertex and hull shader passes, a culled
/// patch nothing more; with the patch constants factors (MODEL_PATCH_CONSTANTS) the domain
/// shader pass, stream output of the domain shader, the store's draw; every pass valid,
/// HSInvocations a patch, DSInvocations a point.
#[test]
fn tessellated_draws_on_the_page() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    let o = mem(2);
    let made_of = |m: u32, c: &[u8]| {
        let (p, n) = code(c);
        assert_eq!(dc(dev, m, &[p, n, 0, o]), 0, "create {m}");
        made(rd1(o))
    };
    let vs = made_of(12, &compiled("vs", "vs_5_0"));
    let s = stages_of("hs_tri_int", "ds_tri");
    let hs = made_of(16, &s.hs_code);
    let ds = made_of(17, &s.ds_code);
    let quad_ds = made_of(17, &compiled("ds_quad", "ds_5_0"));
    // stream output of the domain shader: L and B, 32 bytes a vertex, nothing rasterized
    let (p, n) = code(&s.ds_code);
    let names = mem(2);
    put(names, &[u32::from(b'L'), u32::from(b'B')]);
    let decl = mem(8);
    put(decl, &[0, names, 0, 4 << 8, 0, names + 4, 0, 4 << 8]);
    let stride = mem(1);
    put(stride, &[32]);
    assert_eq!(dc(dev, 14, &[p, n, decl, 2, stride, 1, u32::MAX, 0, o]), 0);
    let so_gs = made(rd1(o));
    let desc = mem(6);
    put(desc, &[4096, 0, 0x10 | 1, 0, 0, 0]);
    assert_eq!(dc(dev, 3, &[desc, 0, o]), 0);
    let so = made(rd1(o));
    let (arr, offs) = (mem(1), mem(1));
    put(arr, &[so]);
    put(offs, &[0]);
    cc(ctx, 11, &[vs, 0, 0]);
    let stats = query(dev, 4);
    let calls = |k: &str| crate::render::model::with(|m| m.calls.get(k).copied().unwrap_or(0));
    // (dispatches, draws, the statistics' HSInvocations and DSInvocations) of 6 control points
    let drawn = |topology: u32, h: u32, d: u32, gs: u32, pc: Option<Vec<u32>>| {
        cc(ctx, 60, &[h, 0, 0]);
        cc(ctx, 64, &[d, 0, 0]);
        cc(ctx, 23, &[gs, 0, 0]);
        cc(ctx, 37, &[u32::from(gs != 0), if gs != 0 { arr } else { 0 }, offs]);
        cc(ctx, 24, &[topology]);
        MODEL_PATCH_CONSTANTS.with(|m| *m.borrow_mut() = pc);
        let (dispatches, draws) = (calls("dispatch"), calls("draw"));
        cc(ctx, 27, &[stats]);
        cc(ctx, 13, &[6, 0]);
        cc(ctx, 28, &[stats]);
        crate::d3d9::gpu::flush();
        crate::d3d9::gpu::pump();
        no_model_errors();
        let st = u64s(&wait(ctx, stats, 88));
        (calls("dispatch") - dispatches, calls("draw") - draws, st[8], st[9])
    };
    // (not drawn: a hull shader with a triangle list, a patch list without a hull shader, a
    // patch list of 4 control points for a hull shader of 3, a domain shader of another domain)
    assert_eq!(drawn(4, hs, ds, 0, None).0, 0);
    assert_eq!(drawn(35, 0, ds, 0, None).0, 0);
    assert_eq!(drawn(35, hs, 0, 0, None).0, 0);
    assert_eq!(drawn(36, hs, ds, 0, None).0, 0);
    assert_eq!(drawn(35, hs, quad_ds, 0, None).0, 0);
    // (factors 0: both patches culled after the hull shader pass)
    assert_eq!(drawn(35, hs, ds, 0, None), (2, 0, 2, 0));
    // (factors 2: the domain shader per point, the clipper's pass, the store drawn)
    let pcr = us(s.decl.pc_regs());
    let mut words = vec![0u32; 2 * pcr * 4];
    for k in 0..2 {
        for &(_, r, c) in &s.decl.factors {
            words[(k * pcr + us(r)) * 4 + usize::from(c)] = 2.0f32.to_bits();
        }
    }
    let per_patch = u64::from(lo32(tess::tessellate(Partitioning::Integer, Output::TriangleCw, Factors::Tri { edges: [2.0; 3], inside: 2.0 }).points.len()));
    assert_eq!(drawn(35, hs, ds, 0, Some(words.clone())), (4, 1, 2, 2 * per_patch));
    // (stream output of the domain shader, nothing rasterized: its pass instead of the
    // clipper's and the draw)
    assert_eq!(drawn(35, hs, ds, so_gs, Some(words)), (4, 0, 2, 2 * per_patch));
    // (the context is the tests' shared one: no shaders of these stages for the next test)
    cc(ctx, 60, &[0, 0, 0]);
    cc(ctx, 64, &[0, 0, 0]);
    cc(ctx, 23, &[0, 0, 0]);
    cc(ctx, 37, &[0, 0, 0]);
    cc(ctx, 24, &[4]);
    release_all();
}
