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

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]
    /// Without a geometry shader an adjacency topology rasterizes what its plain topology does
    /// of the primitives' own vertices, winding kept: a line list of vertices 1 2 of each 4, a
    /// line strip of a run's inner vertices, a triangle list of 0 2 4 of each 6, a triangle
    /// strip of a run's even vertices (winref d3d11gs "triangle list adjacency of 6": the
    /// triangle 0 2 4, culled as Windows culls it).
    #[test]
    fn adjacency_rasterizes_its_own_vertices(cuts in prop::collection::vec(prop::bool::weighted(0.15), 0..48), t in prop::sample::select(vec![Topo::LinesAdj, Topo::LineStripAdj, Topo::TrianglesAdj, Topo::TriangleStripAdj])) {
        let n = lo32(cuts.len());
        let strip = matches!(t, Topo::LineStripAdj | Topo::TriangleStripAdj);
        let cut = |k: u32| strip && cuts[us(k)];
        let got = rasterized(t, n, cut);
        let mut want = Vec::new();
        if strip {
            let mut runs: Vec<Vec<u32>> = vec![Vec::new()];
            for k in 0..n {
                if cut(k) {
                    runs.push(Vec::new());
                } else if let Some(r) = runs.last_mut() {
                    r.push(k);
                }
            }
            for r in runs {
                match t {
                    Topo::LineStripAdj if r.len() >= 4 => {
                        let inner = &r[1..r.len() - 1];
                        want.extend(rasterized(Topo::LineStrip, lo32(inner.len()), |_| false).into_iter().map(|k| inner[us(k)]));
                    }
                    Topo::TriangleStripAdj if r.len() >= 6 => {
                        // (the last odd vertex of an odd run is no corner)
                        let even: Vec<u32> = r.iter().step_by(2).copied().take(r.len() / 2).collect();
                        want.extend(rasterized(Topo::TriangleStrip, lo32(even.len()), |_| false).into_iter().map(|k| even[us(k)]));
                    }
                    _ => {}
                }
            }
        } else {
            let (per, own): (u32, &[u32]) = if t == Topo::LinesAdj { (4, &[1, 2]) } else { (6, &[0, 2, 4]) };
            for p in 0..n / per {
                want.extend(own.iter().map(|&c| p * per + c));
            }
        }
        prop_assert_eq!(got, want);
    }
}

/// Triangle strips with adjacency as Windows gives them to a geometry shader (winref d3d11gs,
/// "triangle strip adjacency" of 10, 8 and 6 vertices), vertex for vertex.
#[test]
fn triangle_strip_adjacency_table() {
    assert_eq!(assemble(Topo::TriangleStripAdj, 10, |_| false), vec![vec![0, 1, 2, 6, 4, 3], vec![2, 5, 6, 8, 4, 0], vec![4, 2, 6, 9, 8, 7]]);
    assert_eq!(assemble(Topo::TriangleStripAdj, 8, |_| false), vec![vec![0, 1, 2, 6, 4, 3], vec![2, 5, 6, 7, 4, 0]]);
    assert_eq!(assemble(Topo::TriangleStripAdj, 6, |_| false), vec![vec![0, 1, 2, 5, 4, 3]]);
}

/// The other topologies' primitives as d3d11gs's stream output shows them: a triangle
/// strip's odd triangles (1 3 2), a cut strip, line strips with adjacency.
#[test]
fn strips_as_windows_gives_them() {
    assert_eq!(assemble(Topo::TriangleStrip, 6, |_| false), vec![vec![0, 1, 2], vec![1, 3, 2], vec![2, 3, 4], vec![3, 5, 4]]);
    assert_eq!(assemble(Topo::TriangleStrip, 9, |k| k == 4), vec![vec![0, 1, 2], vec![1, 3, 2], vec![5, 6, 7], vec![6, 8, 7]]);
    assert_eq!(assemble(Topo::LineStripAdj, 6, |_| false), vec![vec![0, 1, 2, 3], vec![1, 2, 3, 4], vec![2, 3, 4, 5]]);
    assert_eq!(assemble(Topo::LinesAdj, 8, |_| false), vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]);
}

fn col(stream: u32) -> dxwgsl::dxbc::SigElement {
    dxwgsl::dxbc::SigElement { name: "COL".into(), index: 0, sysval: 0, component_type: 3, reg: 1, mask: 0xf, stream }
}

fn entry(stream: u32, name: Option<&str>, start: u32, count: u32, slot: u32) -> RawEntry {
    RawEntry { stream, name: name.map(String::from), index: 0, start, count, slot }
}

/// The declarations d3d11gs's "errors" give a vertex shader writing COL, with Windows' answers.
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
        ("rasterized stream 1", entry(0, Some("COL"), 0, 4, 0), 16, 1, Ok(())),
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
    no_model_errors();
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

/// Indexed draws' input assembly while PIPELINE_STATISTICS is open: the vertex shader runs once
/// per distinct index of whole primitives, a strip is cut at 0xffff, indices past the buffer's
/// end read 0 (winref d3d11gs "pipeline statistics of indexed draws", Windows' answers).
#[test]
fn indexed_statistics_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    let o = mem(2);
    let vs = hlslc::compile(SO_VS, &[], &mut |_, _| None, "main", "vs_4_0", 0).expect("vs");
    let (p, n) = code(&vs);
    assert_eq!(dc(dev, 12, &[p, n, 0, o]), 0);
    cc(ctx, 11, &[made(rd1(o)), 0, 0]);
    // indices 0 1 2 2 1 3 0 1 2 0 1 2 3 3 3 0 ffff 1 2 3 (R16_UINT, 40 bytes)
    let idx: [u16; 20] = [0, 1, 2, 2, 1, 3, 0, 1, 2, 0, 1, 2, 3, 3, 3, 0, 0xffff, 1, 2, 3];
    let words: Vec<u32> = idx.chunks_exact(2).map(|c| u32::from(c[0]) | u32::from(c[1]) << 16).collect();
    let data = mem(10);
    put(data, &words);
    let init = mem(3);
    put(init, &[data, 0, 0]);
    let desc = mem(6);
    put(desc, &[40, 0, 2, 0, 0, 0]);
    assert_eq!(dc(dev, 3, &[desc, init, o]), 0);
    let ib = made(rd1(o));
    cc(ctx, 19, &[ib, 57, 0]);
    let stats = query(dev, 4);
    // (topology, count, start, instances) -> IA vertices, IA primitives, VS invocations
    let cases: [((u32, u32, u32, u32), (u64, u64, u64)); 6] = [
        ((4, 6, 0, 1), (6, 2, 4)),
        ((4, 12, 0, 1), (12, 4, 4)),
        ((4, 3, 12, 1), (3, 1, 1)),
        ((4, 6, 0, 2), (12, 4, 8)),
        ((5, 5, 15, 1), (5, 1, 3)),
        ((5, 6, 18, 1), (6, 4, 3)),
    ];
    for ((topo, count, start, inst), want) in cases {
        cc(ctx, 24, &[topo]);
        cc(ctx, 27, &[stats]);
        // DrawIndexedInstanced(count, instances, start, base, first instance)
        cc(ctx, 20, &[count, inst, start, 0, 0]);
        cc(ctx, 28, &[stats]);
        let s = u64s(&wait(ctx, stats, 88));
        assert_eq!((s[0], s[1], s[2]), want, "topology {topo}, {count} from {start}, {inst} instances");
    }
    no_model_errors();
    release_all();
}

/// FillMode WIREFRAME with and without a geometry shader, of triangle lists and strips: each
/// triangle the solid draw rasterizes is three edges of the wireframe one, a line list (the
/// geometry shader's output skipped the wireframe before: review e18bf44), the pipeline made
/// for it kept apart from the solid one's (PipelineKey's fill), the model's checks passed.
#[test]
fn wireframe_of_the_drawn_triangles_and_of_a_geometry_shaders_output() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (dev, ctx) = device();
    let o = mem(2);
    let vs = hlslc::compile(SO_VS, &[], &mut |_, _| None, "main", "vs_4_0", 0).expect("vs");
    let (p, n) = code(&vs);
    assert_eq!(dc(dev, 12, &[p, n, 0, o]), 0);
    cc(ctx, 11, &[made(rd1(o)), 0, 0]);
    let gs = hlslc::compile(SO_GS, &[], &mut |_, _| None, "main", "gs_4_0", 0).expect("gs");
    let (p, n) = code(&gs);
    assert_eq!(dc(dev, 13, &[p, n, 0, o]), 0);
    let g = made(rd1(o));
    // rasterizer states: FillMode, CullMode NONE, the rest D3D11's defaults
    let state = |fill: u32| {
        let desc = mem(10);
        put(desc, &[fill, 1, 0, 0, 0, 0, 1, 0, 0, 0]);
        assert_eq!(dc(dev, 22, &[desc, o]), 0);
        made(rd1(o))
    };
    let (solid, wire) = (state(3), state(2));
    let drawn = |fill: u32, gs: u32, topology: u32| {
        cc(ctx, 23, &[gs, 0, 0]);
        cc(ctx, 43, &[fill]);
        cc(ctx, 24, &[topology]);
        crate::render::model::with(|m| m.prims = Some(Vec::new()));
        cc(ctx, 13, &[6, 0]);
        crate::d3d9::gpu::flush();
        crate::d3d9::gpu::pump();
        let prims = crate::render::model::with(|m| m.prims.take()).unwrap_or_default();
        no_model_errors();
        prims.into_iter().flatten().map(|p| p.len()).collect::<Vec<usize>>()
    };
    for gs in [0, g] {
        for topology in [4, 5] {
            let s = drawn(solid, gs, topology);
            let w = drawn(wire, gs, topology);
            let what = format!("geometry shader {}, topology {topology}", gs != 0);
            assert!(!s.is_empty() && s.iter().all(|&k| k == 3), "{what}: solid {s:?}");
            assert!(w.iter().all(|&k| k == 2), "{what}: wireframe {w:?}");
            assert_eq!(w.len(), 3 * s.len(), "{what}: three edges a triangle");
        }
    }
    let keys: Vec<(Fill, u32)> = {
        let gd = kome::locks::Lock::locked(&D11S);
        let d = gd.as_ref().unwrap_or_else(|| panic!("no device"));
        d.pipelines.keys().map(|k| (k.fill, k.desc[2])).collect()
    };
    assert!(keys.iter().any(|&(f, t)| f == Fill::Wireframe && t == 1), "{keys:?}");
    assert!(keys.iter().all(|&(f, t)| f == Fill::Solid || t == 1), "a wireframe pipeline of triangles: {keys:?}");
    // (the context is the tests' shared one: its default rasterizer state and no geometry
    // shader again for the next test)
    cc(ctx, 43, &[0]);
    cc(ctx, 23, &[0, 0, 0]);
    release_all();
}

proptest! {
    /// A store's (a geometry shader's output's) edges, as wire_edges gives a draw's: three a
    /// triangle, each its own corners, in winding order.
    #[test]
    fn a_stores_edges_are_its_triangles(n in 0u32..60) {
        let idx: Vec<u32> = (0..n).collect();
        let e = triangle_edges(&idx, false, None);
        prop_assert_eq!(e.len(), us(n / 3) * 6);
        for (t, c) in e.chunks_exact(6).enumerate() {
            let b = lo32(t) * 3;
            prop_assert_eq!(c, &[b, b + 1, b + 1, b + 2, b + 2, b][..]);
        }
    }
}
