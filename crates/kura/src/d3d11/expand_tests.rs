//! The expanded draws (expand.rs): the primitives `assemble` makes of every topology (property
//! tests against the topologies' definitions), CreateGeometryShaderWithStreamOutput's checks
//! (winref d3d11gs's error cases), and draws with a geometry shader, stream output, DrawAuto,
//! the queries that count on the page and the buffers stream output wrote read back, on the
//! WebGPU model (its validation rules: every pass's bind groups, usages, copies and maps).

use super::super::query_tests::{cc, code, dc, device, made, mem, no_model_errors, put, query, rd1, release_all, u64s, wait};
use super::*;
use crate::d3d9::trace_tests::SERIAL;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    /// Lists: whole primitives of consecutive vertices, the rest dropped.
    #[test]
    fn lists_are_runs(n in 0u32..64, t in prop::sample::select(vec![Topo::Points, Topo::Lines, Topo::Triangles, Topo::LinesAdj, Topo::TrianglesAdj])) {
        let p = assemble(t, n, |_| false);
        let per = t.input();
        prop_assert_eq!(lo32(p.len()), n / per);
        for (i, prim) in (0u32..).zip(&p) {
            prop_assert_eq!(prim, &(i * per..(i + 1) * per).collect::<Vec<_>>());
        }
    }

    /// Strips, with cuts: no primitive holds a cut or reaches over one; each run of m vertices
    /// makes m - 1 lines, m - 2 triangles, m - 3 adjacency lines, m / 2 - 2 adjacency
    /// triangles; a triangle strip's triangle j holds vertices j..j+3 of its run, the even
    /// ones in order and the odd ones as ODD; an adjacency triangle strip's triangles are
    /// vertices 2i, 2i + 2, 2i + 4 of the run with adjacent ones inside it.
    #[test]
    fn strips_by_runs(cuts in prop::collection::vec(any::<bool>(), 0..48), t in prop::sample::select(vec![Topo::LineStrip, Topo::TriangleStrip, Topo::LineStripAdj, Topo::TriangleStripAdj])) {
        let n = lo32(cuts.len());
        let cut = |k: u32| cuts[us(k)];
        let p = assemble(t, n, cut);
        // the runs
        let mut runs: Vec<Vec<u32>> = vec![Vec::new()];
        for k in 0..n {
            if cut(k) {
                runs.push(Vec::new());
            } else if let Some(r) = runs.last_mut() {
                r.push(k);
            }
        }
        let made = |m: usize| match t {
            Topo::LineStrip => m.saturating_sub(1),
            Topo::TriangleStrip => m.saturating_sub(2),
            Topo::LineStripAdj => m.saturating_sub(3),
            _ => (m / 2).saturating_sub(2),
        };
        prop_assert_eq!(p.len(), runs.iter().map(|r| made(r.len())).sum::<usize>());
        let mut at = 0;
        for r in &runs {
            for j in 0..made(r.len()) {
                let prim = &p[at];
                at += 1;
                prop_assert_eq!(prim.len(), us(t.input()));
                prop_assert!(prim.iter().all(|v| r.contains(v)), "{:?} outside its run {:?}", prim, r);
                match t {
                    Topo::LineStrip => prop_assert_eq!(prim, &vec![r[j], r[j + 1]]),
                    Topo::LineStripAdj => prop_assert_eq!(prim, &r[j..j + 4].to_vec()),
                    Topo::TriangleStrip if j % 2 == 0 => prop_assert_eq!(prim, &r[j..j + 3].to_vec()),
                    Topo::TriangleStrip => prop_assert_eq!(prim, &dxwgsl::expand::ODD.iter().map(|&o| r[j + us(o)]).collect::<Vec<_>>()),
                    _ => {
                        let mut own: Vec<u32> = t.corners().iter().map(|&c| prim[c]).collect();
                        own.sort_unstable();
                        prop_assert_eq!(own, vec![r[2 * j], r[2 * j + 2], r[2 * j + 4]]);
                    }
                }
            }
        }
    }
}

/// A triangle strip with adjacency of 4 triangles: OpenGL's table (and so the one Direct3D
/// draws by), vertex for vertex.
#[test]
fn triangle_strip_adjacency_table() {
    let p = assemble(Topo::TriangleStripAdj, 12, |_| false);
    assert_eq!(p, vec![vec![0, 1, 2, 6, 4, 3], vec![4, 0, 2, 5, 6, 8], vec![4, 2, 6, 10, 8, 7], vec![8, 4, 6, 9, 10, 11]]);
    assert_eq!(assemble(Topo::TriangleStripAdj, 6, |_| false), vec![vec![0, 1, 2, 5, 4, 3]]);
}

fn col(stream: u32) -> dxwgsl::dxbc::SigElement {
    dxwgsl::dxbc::SigElement { name: "COL".into(), index: 0, sysval: 0, component_type: 3, reg: 1, mask: 0xf, stream }
}

fn entry(stream: u32, name: Option<&str>, start: u32, count: u32, slot: u32) -> RawEntry {
    RawEntry { stream, name: name.map(String::from), index: 0, start, count, slot }
}

/// The declarations d3d11gs's "errors" give a vertex shader writing COL (as the probe's).
#[test]
fn stream_output_checks() {
    let osgn = [col(0)];
    let bad = E_INVALIDARG;
    let cases: [(&str, RawEntry, u32, u32, Result<(), u32>); 10] = [
        ("good", entry(0, Some("COL"), 0, 4, 0), 16, 0, Ok(())),
        ("unknown semantic", entry(0, Some("NOPE"), 0, 4, 0), 16, 0, Err(bad)),
        ("stride too small", entry(0, Some("COL"), 0, 4, 0), 12, 0, Err(bad)),
        ("stride not a multiple of 4", entry(0, Some("COL"), 0, 1, 0), 6, 0, Err(bad)),
        ("start + count over 4", entry(0, Some("COL"), 2, 3, 0), 16, 0, Err(bad)),
        ("slot 4", entry(0, Some("COL"), 0, 4, 4), 16, 0, Err(bad)),
        ("stream 1 of a vertex shader", entry(1, Some("COL"), 0, 4, 0), 16, 0, Err(bad)),
        ("rasterized stream 1", entry(0, Some("COL"), 0, 4, 0), 16, 1, Err(bad)),
        ("gap of 0 components", entry(0, None, 0, 0, 0), 16, 0, Err(bad)),
        ("a wider stride", entry(0, Some("COL"), 0, 4, 0), 2048, 0, Ok(())),
    ];
    for (name, e, stride, raster, want) in cases {
        assert_eq!(so_desc(&osgn, false, &[e], &[stride], raster).map(|_| ()), want, "{name}");
    }
    // a geometry shader's streams, rasterized ones, NO_RASTERIZED_STREAM
    let gs = [col(0), col(1)];
    let two = [entry(0, Some("COL"), 0, 1, 0), entry(1, Some("COL"), 0, 1, 1)];
    assert_eq!(so_desc(&gs, true, &two, &[4, 4], NO_RASTERIZED_STREAM).map(|d| d.rasterized), Ok(None));
    assert_eq!(so_desc(&gs, true, &two, &[4, 4], 1).map(|d| d.rasterized), Ok(Some(1)));
    // one slot, two streams
    let mixed = [entry(0, Some("COL"), 0, 1, 0), entry(1, Some("COL"), 0, 1, 0)];
    assert_eq!(so_desc(&gs, true, &mixed, &[8], 0).map(|_| ()), Err(bad));
    assert_eq!(so_desc(&gs, true, &[], &[8], 0).map(|_| ()), Err(bad));
}

const SO_VS: &str = "struct V { float4 p : SV_Position; float4 c : COL; };
V main(uint id : SV_VertexID) { V v; v.p = float4((id & 1) * 2.0 - 1.0, (id >> 1) * 2.0 - 1.0, 0.5, 1); v.c = float4(id, id * 2, 0, 1); return v; }";
const SO_GS: &str = "struct V { float4 p : SV_Position; float4 c : COL; };
[maxvertexcount(4)] void main(triangle V i[3], uint id : SV_PrimitiveID, inout TriangleStream<V> s) {
    for (int k = 0; k < 3; k++) { V v = i[k]; v.c.w = id; s.Append(v); }
    s.RestartStrip();
}";

/// A geometry shader with stream output into a buffer, drawn and counted: every pass the
/// model checks, SO_STATISTICS and PIPELINE_STATISTICS answered, the buffer read back for
/// CopyResource and Map, DrawAuto drawing from it.
#[test]
fn geometry_shader_stream_output_on_the_page() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    let o = mem(2);
    let vs = hlslc::compile(SO_VS, &[], &mut |_, _| None, "main", "vs_4_0", 0).expect("vs");
    let (p, n) = code(&vs);
    assert_eq!(dc(dev, 12, &[p, n, 0, o]), 0);
    cc(ctx, 11, &[made(rd1(o)), 0, 0]);
    let gs = hlslc::compile(SO_GS, &[], &mut |_, _| None, "main", "gs_4_0", 0).expect("gs");
    let (p, n) = code(&gs);
    // D3D11_SO_DECLARATION_ENTRY { 0, "COL", 0, 0, 4, 0 }
    let name = mem(1);
    put(name, &[u32::from_le_bytes(*b"COL\0")]);
    let decl = mem(4);
    put(decl, &[0, name, 0, 4 << 8]);
    let stride = mem(1);
    put(stride, &[16]);
    assert_eq!(dc(dev, 14, &[p, n, decl, 1, stride, 1, 0, 0, o]), 0);
    let g = made(rd1(o));
    cc(ctx, 23, &[g, 0, 0]);
    // the target: 4 triangles' vertices, a vertex buffer too
    let desc = mem(6);
    put(desc, &[12 * 16, 0, 0x10 | 1, 0, 0, 0]);
    assert_eq!(dc(dev, 3, &[desc, 0, o]), 0);
    let so = made(rd1(o));
    let arr = mem(1);
    put(arr, &[so]);
    let offs = mem(1);
    put(offs, &[0]);
    cc(ctx, 37, &[1, arr, offs]);
    let (stats, sos) = (query(dev, 4), query(dev, 6));
    cc(ctx, 27, &[stats]);
    cc(ctx, 27, &[sos]);
    cc(ctx, 24, &[5]);
    let calls = |k: &str| crate::render::model::with(|m| m.calls.get(k).copied().unwrap_or(0));
    let (dispatches, draws) = (calls("dispatch"), calls("draw"));
    cc(ctx, 13, &[6, 0]);
    crate::d3d9::gpu::flush();
    crate::d3d9::gpu::pump();
    // the vertex, geometry shader, stream output and clipper passes, the store drawn
    assert_eq!((calls("dispatch") - dispatches, calls("draw") - draws), (4, 1));
    cc(ctx, 28, &[stats]);
    cc(ctx, 28, &[sos]);
    assert_eq!(wait(ctx, sos, 16).len(), 16);
    let s = u64s(&wait(ctx, stats, 88));
    // (the input assembler's: 6 strip vertices, 4 triangles, the geometry shader run on each)
    assert_eq!((s[0], s[1], s[2], s[3]), (6, 4, 6, 4));
    // read back: a staging copy mapped
    put(desc, &[12 * 16, 3, 0, 0x20000, 0, 0]);
    assert_eq!(dc(dev, 3, &[desc, 0, o]), 0);
    let staging = made(rd1(o));
    cc(ctx, 47, &[staging, so]);
    let mapped = mem(4);
    assert_eq!(cc(ctx, 14, &[staging, 0, 1, 0, mapped]), 0);
    cc(ctx, 15, &[staging, 0]);
    // DrawAuto from it
    cc(ctx, 37, &[0, 0, 0]);
    cc(ctx, 23, &[0, 0, 0]);
    let strides = mem(1);
    put(strides, &[16]);
    cc(ctx, 18, &[0, 1, arr, strides, offs]);
    cc(ctx, 38, &[]);
    crate::d3d9::gpu::flush();
    crate::d3d9::gpu::pump();
    no_model_errors();
    release_all();
}
