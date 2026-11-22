//! Draws WebGPU's render pipeline cannot do in one: a geometry shader, stream output, and the
//! pipeline statistics that need the vertices' positions. Compute passes do the stages and
//! keep what they give in storage buffers ("stores"); kura's d3d11 expand.rs runs them:
//!
//! 1. [`vertex_pass`]: the vertex shader once per vertex the draw reads (index buffer, base
//!    vertex, instances), fetching its inputs itself; outputs into `vout`.
//! 2. [`gs_pass`]: the geometry shader once per input primitive (and GS instance), its
//!    vertices from `vout` by the primitive list kura made (`prims`); what it emits into
//!    `gout`, made into list primitives in `glist` (`gcount` of them per invocation and stream).
//! 3. [`store_vertex`]: the render pipeline's vertex shader for what a store holds (the
//!    rasterized stream), one list primitive's corner per vertex.
//! 4. [`so_pass`]: stream output, in primitive order: the declarations' components into the
//!    stream output buffers while they have room, the counts (written, needed) into counters.
//! 5. [`clip_pass`]: what the clipper makes of each primitive (CInvocations, CPrimitives).
//!
//! Bindings (group 0 of the compute passes, the vertex group of `store_vertex`): the shader's
//! own as in a compute shader (b<n> n, t<n> 16 + n, s<n> 160 + n), and
//! 400 + k vertex buffer k (storage, read), 410 the index buffer, 420 the uniform `xd`
//! (Xd), 430 vout, 431 prims, 440 gout, 441 glist, 442 gcount, 450 + k stream output buffer
//! slot k, 455 the counters: `counter::N` of them, then the stream output fill table (bytes
//! in each buffer, by kura's slots). (At most 8 storage buffers a pass: WebGPU's
//! maxStorageBuffersPerShaderStage.)

/// Appends a formatted line (`wln!`) or text (`w!`) to a String (no fmt::Result to drop).
macro_rules! wln {
    ($m:expr, $($t:tt)*) => {{
        $m.push_str(&format!($($t)*));
        $m.push('\n');
    }};
}
macro_rules! w {
    ($m:expr, $($t:tt)*) => {
        $m.push_str(&format!($($t)*))
    };
}

use super::{bits, interp_attr, module_decls, parse, vty, W, K, XYZW};

/// Component `c` (0..4) of a register: x, y, z or w.
fn xyzw(c: u8) -> char {
    XYZW.get(usize::from(c)).copied().unwrap_or('x')
}
use crate::dxbc::SigElement;
use crate::tpf::{self, Opcode};
use crate::{Binding, Wgsl};

pub const B_VB: u32 = 400;
pub const B_IB: u32 = 410;
pub const B_XD: u32 = 420;
pub const B_VOUT: u32 = 430;
pub const B_PRIMS: u32 = 431;
pub const B_GOUT: u32 = 440;
pub const B_GLIST: u32 = 441;
pub const B_GCOUNT: u32 = 442;
pub const B_SO: u32 = 450;
pub const B_COUNTERS: u32 = 455;
/// vec4s of the uniform `xd`
pub const XD_VECS: u32 = 15;

/// The uniform the passes read (`xd`, XD_VECS vec4<u32>), word by word.
pub mod xd {
    /// vertices (or indices) the draw reads per instance
    pub const COUNT: usize = 0;
    /// first vertex (or index)
    pub const START: usize = 1;
    /// base vertex (i32 bits)
    pub const BASE: usize = 2;
    pub const INSTANCES: usize = 3;
    pub const FIRST_INSTANCE: usize = 4;
    /// 0 not indexed, 16 or 32 bit indices
    pub const INDEX_BITS: usize = 5;
    /// the index buffer's offset in bytes
    pub const INDEX_OFFSET: usize = 6;
    /// what the pass runs over: vertices (count * instances), invocations, primitives
    pub const ITEMS: usize = 7;
    /// vertex buffer k's offset in bytes (8 of them)
    pub const VB_OFFSET: usize = 8;
    pub const VB_STRIDE: usize = 16;
    /// bytes of vertex buffer k after its offset (fetches past: zeros)
    pub const VB_SIZE: usize = 24;
    /// the stream output buffers' slots in the fill table (4: words counter::N + slot of the
    /// counters)
    pub const SO_SLOT: usize = 32;
    /// the stream output buffers' sizes in bytes (4; 0 none bound)
    pub const SO_SIZE: usize = 36;
    /// the index buffer's size in bytes after its offset
    pub const INDEX_SIZE: usize = 40;
    /// stream output: the list's first primitive (vertex stream output: kura's list)
    pub const FIRST: usize = 41;
    /// the rasterizer's viewport (4 f32: TopLeftX, TopLeftY, Width, Height) for PSInvocations
    pub const VIEWPORT: usize = 48;
    /// bits 0..2 CullMode (ps::Cull::word), 4 FrontCounterClockwise, 8 DepthClipEnable, 16 a
    /// pixel shader is bound
    pub const RASTER: usize = 52;
    /// the pixels that can be drawn (4 i32: ps::Raster::rect)
    pub const RECT: usize = 53;
    /// the viewport's MinDepth and MaxDepth (2 f32: the setup pass's depth plane, setup.rs)
    pub const DEPTH: usize = 44;
    /// the words
    pub const WORDS: usize = 60;
    /// The words' names in the passes' WGSL (`const XD_<name>`), so the shaders say which word
    /// they read
    pub const NAMED: [(&str, usize); 19] = [
        ("COUNT", COUNT),
        ("START", START),
        ("BASE", BASE),
        ("INSTANCES", INSTANCES),
        ("FIRST_INSTANCE", FIRST_INSTANCE),
        ("INDEX_BITS", INDEX_BITS),
        ("INDEX_OFFSET", INDEX_OFFSET),
        ("ITEMS", ITEMS),
        ("VB_OFFSET", VB_OFFSET),
        ("VB_STRIDE", VB_STRIDE),
        ("VB_SIZE", VB_SIZE),
        ("SO_SLOT", SO_SLOT),
        ("SO_SIZE", SO_SIZE),
        ("INDEX_SIZE", INDEX_SIZE),
        ("FIRST", FIRST),
        ("VIEWPORT", VIEWPORT),
        ("RASTER", RASTER),
        ("RECT", RECT),
        ("DEPTH", DEPTH),
    ];
}

/// The counters a pass adds to (u32 each, kura reads them for the queries)
pub mod counter {
    /// SO_STATISTICS of stream s: primitives written (2 s), needed (2 s + 1)
    pub const SO: u32 = 0;
    pub const GS_PRIMITIVES: u32 = 8;
    pub const C_INVOCATIONS: u32 = 9;
    pub const C_PRIMITIVES: u32 = 10;
    /// PSInvocations / 2: the pairs of pixel shader invocations (ps::invocations)
    pub const PS_PAIRS: u32 = 11;
    pub const N: u32 = 16;
}

/// Most storage buffers a pass binds (WebGPU's default maxStorageBuffersPerShaderStage).
pub const MAX_STORES: usize = 8;

/// A vertex shader input the vertex pass fetches: register, DXGI format, byte offset in the
/// vertex, its input slot's place in xd (VB_OFFSET + slot ..: 0..8), the vertex buffer it reads
/// (binding 400 + buf: input slots of one buffer share it), instance step rate (0: per vertex,
/// u32::MAX: every instance the first's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Attr {
    pub reg: u32,
    pub fmt: u32,
    pub offset: u32,
    pub slot: u32,
    pub buf: u32,
    pub step: u32,
}

/// A DXGI vertex format: (components, bits of each: 8 16 32 or 0 packed, kind: 0 float 1 unorm
/// 2 snorm 3 uint 4 sint, BGRA order). None: not a vertex format.
fn vformat(f: u32) -> Option<(u32, u32, u32, bool)> {
    Some(match f {
        2 => (4, 32, 0, false),
        3 => (4, 32, 3, false),
        4 => (4, 32, 4, false),
        6 => (3, 32, 0, false),
        7 => (3, 32, 3, false),
        8 => (3, 32, 4, false),
        10 => (4, 16, 0, false),
        11 => (4, 16, 1, false),
        12 => (4, 16, 3, false),
        13 => (4, 16, 2, false),
        14 => (4, 16, 4, false),
        16 => (2, 32, 0, false),
        17 => (2, 32, 3, false),
        18 => (2, 32, 4, false),
        // (R10G10B10A2_UNORM / UINT)
        24 => (4, 0, 1, false),
        25 => (4, 0, 3, false),
        28 | 29 => (4, 8, 1, false),
        30 => (4, 8, 3, false),
        31 => (4, 8, 2, false),
        32 => (4, 8, 4, false),
        34 => (2, 16, 0, false),
        35 => (2, 16, 1, false),
        36 => (2, 16, 3, false),
        37 => (2, 16, 2, false),
        38 => (2, 16, 4, false),
        41 => (1, 32, 0, false),
        42 => (1, 32, 3, false),
        43 => (1, 32, 4, false),
        49 => (2, 8, 1, false),
        50 => (2, 8, 3, false),
        51 => (2, 8, 2, false),
        52 => (2, 8, 4, false),
        54 => (1, 16, 0, false),
        56 => (1, 16, 1, false),
        57 => (1, 16, 3, false),
        58 => (1, 16, 2, false),
        59 => (1, 16, 4, false),
        61 => (1, 8, 1, false),
        62 => (1, 8, 3, false),
        63 => (1, 8, 2, false),
        64 => (1, 8, 4, false),
        87 | 88 => (4, 8, 1, true),
        _ => return None,
    })
}

/// Bytes of a vertex format's element.
pub fn vformat_bytes(f: u32) -> Option<u32> {
    let (n, b, _, _) = vformat(f)?;
    Some(if b == 0 { 4 } else { n.saturating_mul(b) / 8 })
}

/// WGSL reading attribute `a` of vertex buffer `k` at byte `at` as a vec4<u32> of bits (missing
/// components 0, 0, 0, 1; past the buffer's end all zeros, as Direct3D fetches).
fn fetch(a: &Attr, at: &str) -> Result<String, String> {
    let (n, b, kind, bgra) = vformat(a.fmt).ok_or(format!("vertex format {}", a.fmt))?;
    let k = a.buf;
    let one = if kind >= 3 { "1u" } else { "0x3f800000u" };
    let comp = |i: u32| -> String {
        let word = |byte: u32| format!("vb{k}_u({at} + {byte}u)");
        match b {
            32 => word(i.saturating_mul(4)),
            0 => {
                // (10 10 10 2)
                let (sh, w) = if i < 3 { (i.saturating_mul(10), 10) } else { (30, 2) };
                let v = format!("extractBits({}, {sh}u, {w}u)", word(0));
                match kind {
                    1 => format!("bitcast<u32>(f32({v}) / {}.0f)", (1u32 << w).wrapping_sub(1)),
                    _ => v,
                }
            }
            _ => {
                let byte = i.saturating_mul(b) / 8;
                let v = format!("extractBits({}, 0u, {b}u)", word(byte));
                let max = 1u32.checked_shl(b).unwrap_or(0).wrapping_sub(1);
                let smax = 1u32.checked_shl(b.saturating_sub(1)).unwrap_or(0).wrapping_sub(1);
                match kind {
                    0 => format!("bitcast<u32>(unpack2x16float({v}).x)"),
                    1 => format!("bitcast<u32>(f32({v}) / {max}.0f)"),
                    // (snorm: -2^(b-1) is -1 too)
                    2 => format!("bitcast<u32>(max(f32(extractBits(bitcast<i32>({v}), 0u, {b}u)) / {smax}.0f, -1.0f))"),
                    3 => v,
                    _ => format!("bitcast<u32>(extractBits(bitcast<i32>({v}), 0u, {b}u))"),
                }
            }
        }
    };
    let mut c = [String::from("0u"), "0u".into(), "0u".into(), one.into()];
    for i in 0..n {
        let dst = if bgra && i < 3 { 2u32.saturating_sub(i) } else { i };
        if let Some(c) = c.get_mut(kome::num::us(dst)) {
            *c = comp(i);
        }
    }
    Ok(format!("vec4<u32>({})", c.join(", ")))
}

/// The helpers reading vertex buffer `k` (`vb<k>_u(byte)`: 32 bits from any byte, 0 past the end).
fn vb_helpers(m: &mut String, k: u32) {
    wln!(m, "@group(0) @binding({}) var<storage, read> vb{k}: array<u32>;", B_VB.saturating_add(k));
    wln!(
        m,
        "fn vb{k}_u(a: u32) -> u32 {{\n  let i = a >> 2u;\n  let s = (a & 3u) * 8u;\n  let n = arrayLength(&vb{k});\n  let lo = select(0u, vb{k}[min(i, n - 1u)], i < n);\n  if (s == 0u) {{ return lo; }}\n  let hi = select(0u, vb{k}[min(i + 1u, n - 1u)], i + 1u < n);\n  return (lo >> s) | (hi << (32u - s));\n}}"
    );
}

fn xd_decl(m: &mut String, bindings: &mut Vec<(u32, Binding)>) {
    wln!(m, "@group(0) @binding({B_XD}) var<uniform> xd: array<vec4<u32>, {XD_VECS}>;");
    m.push_str("fn xw(i: u32) -> u32 { return xd[i / 4u][i % 4u]; }\n");
    for (name, w) in xd::NAMED {
        wln!(m, "const XD_{name} = {w}u;");
    }
    bindings.push((B_XD, Binding::Uniform(XD_VECS * 16)));
}

/// The invocation's index across a dispatch of more than 65535 groups (64 each, rows of x).
const ITEM: &str = "let item = gid.x + gid.y * nwg.x * 64u;\n    if (item >= xw(XD_ITEMS)) { return; }\n";

/// A program's body translated (its declarations read): the W, and the program.
fn body(code: &[u8], kind: tpf::ProgramType) -> Result<(W, tpf::Program), String> {
    let (c, p) = parse(code)?;
    if p.kind != kind {
        return Err(format!("program type {} where {} is wanted", p.kind, kind));
    }
    let mut w = W::new(false, c.signature(false), c.signature(true));
    w.split = true;
    super::declarations(&mut w, &p)?;
    for i in &p.insts {
        if matches!(i.op, Opcode::DerivRtx | Opcode::DerivRty | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine | Opcode::Discard) {
            return Err(format!("instruction {:#x} outside a pixel shader", i.code));
        }
        w.inst(i)?;
    }
    if !w.blocks.is_empty() {
        return Err("unclosed block".into());
    }
    Ok((w, p))
}

/// Registers a store keeps per vertex for a shader with these outputs: the highest + 1.
pub fn store_regs(osgn: &[SigElement], stream: u32) -> u32 {
    osgn.iter().filter(|e| e.stream == stream && e.reg != u32::MAX).map(|e| e.reg.saturating_add(1)).max().unwrap_or(0).max(1)
}

/// The vertex pass: vertex shader `vs` once per vertex the draw reads, `attrs` fetched from the
/// vertex buffers; its output registers (0 .. store_regs) into vout[item * regs + r].
pub fn vertex_pass(vs: &[u8], attrs: &[Attr]) -> Result<Wgsl, String> {
    let (mut w, _) = body(vs, tpf::VERTEX)?;
    let mut m = String::new();
    let mut bindings = Vec::new();
    let mut temps = String::new();
    module_decls(&mut w, &mut m, &mut bindings, 0, Some(&mut temps))?;
    xd_decl(&mut m, &mut bindings);
    if attrs.iter().any(|a| a.slot >= 8) {
        return Err("an input slot past xd's 8".into());
    }
    let mut bufs: Vec<u32> = attrs.iter().map(|a| a.buf).collect();
    bufs.sort_unstable();
    bufs.dedup();
    // (and the index buffer and vout)
    if bufs.len().saturating_add(2) > MAX_STORES || bufs.iter().any(|&b| b >= 8) {
        return Err(format!("vertex buffers {bufs:?}: more than {} storage buffers", MAX_STORES - 2));
    }
    for &k in &bufs {
        vb_helpers(&mut m, k);
        bindings.push((B_VB.saturating_add(k), Binding::StorageBuffer(false)));
    }
    wln!(m, "@group(0) @binding({B_IB}) var<storage, read> ib: array<u32>;");
    bindings.push((B_IB, Binding::StorageBuffer(false)));
    wln!(m, "@group(0) @binding({B_VOUT}) var<storage, read_write> vout: array<vec4<u32>>;");
    bindings.push((B_VOUT, Binding::StorageBuffer(true)));
    let regs = store_regs(&w.osgn, 0);
    w!(m, "\nfn shader_main() {{\n{temps}{}}}\n\n", w.body);
    wln!(m, "@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n    {ITEM}");
    m.push_str("    let count = max(xw(XD_COUNT), 1u);\n    let inst = item / count;\n    let k = item % count;\n");
    // (the vertex: from the index buffer (16 or 32 bits, past its end 0) plus the base vertex)
    m.push_str(
        "    var vid = xw(XD_START) + k;\n    if (xw(XD_INDEX_BITS) != 0u) {\n        let ia = xw(XD_INDEX_OFFSET) + vid * (xw(XD_INDEX_BITS) / 8u);\n        var ix = 0u;\n        if (vid * (xw(XD_INDEX_BITS) / 8u) + xw(XD_INDEX_BITS) / 8u <= xw(XD_INDEX_SIZE)) {\n            let wd = ib[min(ia / 4u, arrayLength(&ib) - 1u)];\n            ix = select(wd, extractBits(wd, (ia & 2u) * 8u, 16u), xw(XD_INDEX_BITS) == 16u);\n        }\n        vid = ix + xw(XD_BASE);\n    }\n",
    );
    for (&r, d) in &w.inputs {
        let val = match d.sysval {
            0 => match attrs.iter().find(|a| a.reg == r) {
                Some(a) => {
                    let el = if a.step == 0 { "vid".to_string() } else { format!("(xw(XD_FIRST_INSTANCE) + inst / {}u)", a.step) };
                    let bytes = vformat_bytes(a.fmt).ok_or(format!("vertex format {}", a.fmt))?;
                    let k = a.slot;
                    let at = format!("(xw({}u) + {el} * xw({}u) + {}u)", xd::VB_OFFSET.saturating_add(kome::num::us(k)), xd::VB_STRIDE.saturating_add(kome::num::us(k)), a.offset);
                    let f = fetch(a, &format!("a{r}"))?;
                    wln!(m, "    let a{r} = {at};");
                    format!("select(vec4<u32>(0u), {f}, {el} * xw({}u) + {}u <= xw({}u))", xd::VB_STRIDE.saturating_add(kome::num::us(k)), a.offset.saturating_add(bytes), xd::VB_SIZE.saturating_add(kome::num::us(k)))
                }
                None => "vec4<u32>(0u)".into(),
            },
            6 => "vec4<u32>(vid)".into(),
            8 => "vec4<u32>(inst)".into(),
            s => return Err(format!("vertex shader input system value {s}")),
        };
        wln!(m, "    v{r} = {val};");
    }
    m.push_str("    shader_main();\n");
    for r in 0..regs {
        let v = if w.outputs.contains_key(&r) { format!("o{r}") } else { "vec4<u32>(0u)".into() };
        wln!(m, "    vout[item * {regs}u + {r}u] = {v};");
    }
    m.push_str("}\n");
    Ok(Wgsl { code: m, bindings })
}

/// What a geometry shader declares (None: not one).
pub fn gs_decl(code: &[u8]) -> Result<super::GsDecl, String> {
    let (c, p) = parse(code)?;
    if p.kind != tpf::GEOMETRY {
        return Err(format!("program type {} for a geometry shader", p.kind));
    }
    let mut w = W::new(false, c.signature(false), c.signature(true));
    super::declarations(&mut w, &p)?;
    w.gs.ok_or("a geometry shader without its declarations".into())
}

/// Each store vertex's registers as a geometry shader writes them, the same for every stream
/// (its streams' most), and one more: x 1 where a strip starts.
pub fn gs_regs(gs: &[u8]) -> u32 {
    let osgn = crate::signature(gs, true);
    (0..4).map(|s| store_regs(&osgn, s)).max().unwrap_or(1).saturating_add(1)
}

/// How a strip's odd triangles are made into list triangles: the vertices of triangle `j`
/// (strip vertices `j`, `j + 1`, `j + 2`). Direct3D keeps the winding and the leading vertex:
/// odd ones are `j`, `j + 2`, `j + 1`.
pub const ODD: [u32; 3] = [0, 2, 1];

/// The geometry shader pass: one invocation per input primitive (prims: `verts` vertex items
/// of vout then the primitive id) and GS instance. What it emits on stream s goes to
/// gout[((item * streams + s) * maxout + e) * regs + r] (regs: gs_regs), its list primitives to
/// glist[((item * streams + s) * maxprims + t) * corners + c] (the store vertex, u32::MAX for
/// none), their count to gcount[item * streams + s]. `vs_osgn`: the vertex shader's outputs
/// (vout's registers, matched to the geometry shader's inputs by semantic), `vs_regs` of them.
pub fn gs_pass(gs: &[u8], vs_osgn: &[SigElement], vs_regs: u32) -> Result<Wgsl, String> {
    let (mut w, _) = body(gs, tpf::GEOMETRY)?;
    let g = w.gs.clone().ok_or("no geometry shader declarations")?;
    let mut m = String::new();
    let mut bindings = Vec::new();
    let mut temps = String::new();
    module_decls(&mut w, &mut m, &mut bindings, 0, Some(&mut temps))?;
    xd_decl(&mut m, &mut bindings);
    wln!(m, "@group(0) @binding({B_VOUT}) var<storage, read> vout: array<vec4<u32>>;");
    wln!(m, "@group(0) @binding({B_PRIMS}) var<storage, read> prims: array<u32>;");
    wln!(m, "@group(0) @binding({B_GOUT}) var<storage, read_write> gout: array<vec4<u32>>;");
    wln!(m, "@group(0) @binding({B_GLIST}) var<storage, read_write> glist: array<u32>;");
    wln!(m, "@group(0) @binding({B_GCOUNT}) var<storage, read_write> gcount: array<u32>;");
    wln!(m, "@group(0) @binding({B_COUNTERS}) var<storage, read_write> counters: array<atomic<u32>, {}>;", counter::N);
    for (b, rw) in [(B_VOUT, false), (B_PRIMS, false), (B_GOUT, true), (B_GLIST, true), (B_GCOUNT, true), (B_COUNTERS, true)] {
        bindings.push((b, Binding::StorageBuffer(rw)));
    }
    let nin = w.inputs.keys().max().map_or(1, |r| r.saturating_add(1));
    let ns = kome::num::u32_of(g.streams.len());
    wln!(m, "var<private> gv: array<array<vec4<u32>, {nin}>, {}>;", g.verts);
    m.push_str("var<private> vprim: vec4<u32>;\nvar<private> vgsi: vec4<u32>;\nvar<private> gs_item: u32;\n");
    wln!(m, "var<private> gs_n: array<u32, {ns}>;\nvar<private> gs_cut: array<u32, {ns}>;");
    // emit: the stream's output registers into its next vertex (past maxout: dropped)
    for (s, _) in (0u32..).zip(&g.streams) {
        let regs = gs_regs(gs);
        wln!(m, "fn gs_emit{s}() {{\n  if (gs_n[{s}] < {}u) {{\n    let b = ((gs_item * {ns}u + {s}u) * {}u + gs_n[{s}]) * {regs}u;", g.maxout, g.maxout);
        for r in 0..regs.saturating_sub(1) {
            let key = s << 8 | r;
            let v = if w.gs_outputs.contains_key(&key) { format!("o{r}") } else { "vec4<u32>(0u)".into() };
            wln!(m, "    gout[b + {r}u] = {v};");
        }
        wln!(m, "    gout[b + {}u] = vec4<u32>(gs_cut[{s}], 0u, 0u, 0u);\n    gs_n[{s}] += 1u;\n  }}\n  gs_cut[{s}] = 0u;\n}}", regs.saturating_sub(1));
    }
    w!(m, "\nfn shader_main() {{\n{temps}{}}}\n\n", w.body);
    wln!(m, "@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n    {ITEM}");
    let per = g.verts.saturating_add(1);
    wln!(m, "    gs_item = item;\n    let p = item / {}u;\n    vgsi = vec4<u32>(item % {}u);\n    vprim = vec4<u32>(prims[p * {per}u + {}u]);", g.instances, g.instances, g.verts);
    for k in 0..g.verts {
        wln!(m, "    let vi{k} = prims[p * {per}u + {k}u] * {vs_regs}u;");
        // (each input element from the vertex shader's output of its semantic)
        for e in &w.isgn {
            if !w.inputs.contains_key(&e.reg) {
                continue;
            }
            let src = vs_osgn.iter().find(|f| f.name.eq_ignore_ascii_case(&e.name) && f.index == e.index && f.stream == 0);
            for (gc, vc) in bits(e.mask).into_iter().zip(src.map_or_default(|f| bits(f.mask).into_iter().map(|c| (f.reg, c)).collect::<Vec<_>>())) {
                wln!(m, "    gv[{k}][{}].{} = vout[vi{k} + {}u].{};", e.reg, xyzw(gc), vc.0, xyzw(vc.1));
            }
        }
    }
    for s in 0..ns {
        wln!(m, "    gs_cut[{s}] = 1u;");
    }
    m.push_str("    shader_main();\n");
    // the strips as list primitives
    for (s, &topo) in (0u32..).zip(&g.streams) {
        let regs = gs_regs(gs);
        let corners = super::GsDecl::prim_verts(topo);
        let maxp = g.max_prims(topo);
        wln!(m, "    {{\n        let base = (item * {ns}u + {s}u) * {}u;\n        var run = 0u;\n        var np = 0u;\n        for (var e = 0u; e < gs_n[{s}]; e += 1u) {{", g.maxout);
        wln!(m, "            if (gout[(base + e) * {regs}u + {}u].x != 0u) {{ run = 0u; }}\n            run += 1u;", regs.saturating_sub(1));
        wln!(m, "            if (run >= {corners}u) {{\n                let at = ((item * {ns}u + {s}u) * {maxp}u + np) * {corners}u;");
        match corners {
            1 => m.push_str("                glist[at] = base + e;\n"),
            2 => m.push_str("                glist[at] = base + e - 1u;\n                glist[at + 1u] = base + e;\n"),
            _ => {
                m.push_str("                let j0 = base + e - 2u;\n                if ((run & 1u) == 1u) {\n");
                m.push_str("                    glist[at] = j0;\n                    glist[at + 1u] = j0 + 1u;\n                    glist[at + 2u] = j0 + 2u;\n                } else {\n");
                for (c, o) in (0u32..).zip(ODD) {
                    wln!(m, "                    glist[at + {c}u] = j0 + {o}u;");
                }
                m.push_str("                }\n");
            }
        }
        m.push_str("                np += 1u;\n            }\n        }\n");
        wln!(m, "        for (var t = np; t < {maxp}u; t += 1u) {{\n            for (var c = 0u; c < {corners}u; c += 1u) {{ glist[((item * {ns}u + {s}u) * {maxp}u + t) * {corners}u + c] = 0xffffffffu; }}\n        }}");
        wln!(m, "        gcount[item * {ns}u + {s}u] = np;\n        atomicAdd(&counters[{}u], np);\n    }}", counter::GS_PRIMITIVES);
    }
    m.push_str("}\n");
    Ok(Wgsl { code: m, bindings })
}

/// Where a store's vertices come from for [`store_vertex`], [`so_pass`] and [`clip_pass`].
#[derive(Clone, Debug)]
pub enum Store {
    /// a vertex shader's (vout, registers per vertex), its list primitives in prims as kura
    /// made them: `corners` vertex items each
    Vertex { regs: u32, corners: u32 },
    /// a geometry shader's stream `stream` (gs_pass's layout)
    Geometry { decl: super::GsDecl, stream: u32, regs: u32 },
}

impl Store {
    pub fn corners(&self) -> u32 {
        match self {
            Store::Vertex { corners, .. } => *corners,
            Store::Geometry { decl, stream, .. } => super::GsDecl::prim_verts(decl.streams.get(kome::num::us(*stream)).copied().unwrap_or(1)),
        }
    }
    fn regs(&self) -> u32 {
        match self {
            Store::Vertex { regs, .. } | Store::Geometry { regs, .. } => *regs,
        }
    }
    fn decls(&self, m: &mut String, bindings: &mut Vec<(u32, Binding)>) {
        match self {
            Store::Vertex { .. } => {
                wln!(m, "@group(0) @binding({B_VOUT}) var<storage, read> store: array<vec4<u32>>;");
                wln!(m, "@group(0) @binding({B_PRIMS}) var<storage, read> list: array<u32>;");
                bindings.push((B_VOUT, Binding::StorageBuffer(false)));
                bindings.push((B_PRIMS, Binding::StorageBuffer(false)));
            }
            Store::Geometry { .. } => {
                wln!(m, "@group(0) @binding({B_GOUT}) var<storage, read> store: array<vec4<u32>>;");
                wln!(m, "@group(0) @binding({B_GLIST}) var<storage, read> list: array<u32>;");
                wln!(m, "@group(0) @binding({B_GCOUNT}) var<storage, read> gcount: array<u32>;");
                bindings.push((B_GOUT, Binding::StorageBuffer(false)));
                bindings.push((B_GLIST, Binding::StorageBuffer(false)));
                bindings.push((B_GCOUNT, Binding::StorageBuffer(false)));
            }
        }
    }
    /// WGSL: the list slot of primitive `t` (the rasterized order: every invocation's maxprims)
    fn slot(&self, t: &str) -> String {
        match self {
            Store::Vertex { .. } => t.to_string(),
            Store::Geometry { decl, stream, .. } => {
                let topo = decl.streams.get(kome::num::us(*stream)).copied().unwrap_or(1);
                let maxp = decl.max_prims(topo);
                let ns = decl.streams.len();
                format!("(({t} / {maxp}u) * {ns}u + {stream}u) * {maxp}u + {t} % {maxp}u")
            }
        }
    }
}

/// The vertex shader that draws a store: vertex `i` is corner i % corners of list primitive
/// i / corners (none: a point outside the clip volume), its registers (`osgn`, the shader's
/// that wrote the store) to the pixel shader `ps`'s inputs by semantic.
pub fn store_vertex(store: &Store, osgn: &[SigElement], ps: Option<&[u8]>) -> Result<Wgsl, String> {
    let mut m = String::new();
    let mut bindings = Vec::new();
    store.decls(&mut m, &mut bindings);
    let stream = match store {
        Store::Vertex { .. } => 0,
        Store::Geometry { stream, .. } => *stream,
    };
    let osgn: Vec<SigElement> = osgn.iter().filter(|e| e.stream == stream).cloned().collect();
    let corners = store.corners();
    let regs = store.regs();
    let mut fields = vec!["@builtin(position) pos: vec4<f32>".to_string()];
    let mut post = Vec::new();
    let reg_of = |r: u32| format!("store[e * {regs}u + {r}u]");
    let pos = osgn.iter().find(|e| e.sysval == 1).map(|e| e.reg);
    post.push(match pos {
        Some(r) => format!("out.pos = bitcast<vec4<f32>>({});", reg_of(r)),
        None => "out.pos = vec4<f32>(0.0f);".into(),
    });
    match ps.map(super::varyings) {
        Some(Ok(vary)) => {
            for v in vary {
                let mut comp = ["0u".to_string(), "0u".to_string(), "0u".to_string(), "0u".to_string()];
                for e in &v.elems {
                    let Some(f) = osgn.iter().find(|f| f.sysval == 0 && f.name.eq_ignore_ascii_case(&e.name) && f.index == e.index) else { continue };
                    for (pc, vc) in bits(e.mask).into_iter().zip(bits(f.mask)) {
                        if let Some(c) = comp.get_mut(usize::from(pc)) {
                            *c = format!("{}.{}", reg_of(f.reg), xyzw(vc));
                        }
                    }
                }
                let r = v.reg;
                fields.push(format!("@location({r}) {} l{r}: {}", v.interp, vty(v.k, 4)));
                let u = format!("vec4<u32>({})", comp.join(", "));
                post.push(if v.k == K::U { format!("out.l{r} = {u};") } else { format!("out.l{r} = bitcast<vec4<{}>>({u});", v.k.s()) });
            }
        }
        Some(Err(e)) => return Err(format!("its pixel shader: {e}")),
        None => {
            for e in osgn.iter().filter(|e| e.sysval == 0) {
                let k = K::of_sig(e.component_type);
                let r = e.reg;
                if fields.iter().any(|f| f.starts_with(&format!("@location({r})"))) {
                    continue;
                }
                fields.push(format!("@location({r}) {} l{r}: {}", interp_attr(2, k), vty(k, 4)));
                post.push(if k == K::U { format!("out.l{r} = {};", reg_of(r)) } else { format!("out.l{r} = bitcast<vec4<{}>>({});", k.s(), reg_of(r)) });
            }
        }
    }
    wln!(m, "struct Out {{\n    {},\n}}\n", fields.join(",\n    "));
    wln!(m, "@vertex\nfn main(@builtin(vertex_index) vi: u32) -> Out {{\n    var out: Out;\n    let t = vi / {corners}u;\n    let e = list[({}) * {corners}u + vi % {corners}u];", store.slot("t"));
    m.push_str("    if (e == 0xffffffffu) {\n        out.pos = vec4<f32>(2.0f, 2.0f, 0.5f, 1.0f);\n        return out;\n    }\n");
    for s in &post {
        wln!(m, "    {s}");
    }
    m.push_str("    return out;\n}\n");
    Ok(Wgsl { code: m, bindings })
}

/// A D3D11_SO_DECLARATION_ENTRY: stream, semantic (None: a gap of `count` components),
/// semantic index, start component, component count, output slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SoEntry {
    pub stream: u32,
    pub name: Option<String>,
    pub index: u32,
    pub start: u32,
    pub count: u32,
    pub slot: u32,
}

/// Stream output of a store's primitives on `stream`: in primitive order, each primitive's
/// vertices by the declarations `decls` into stream output buffer slot k (binding 450 + k,
/// `strides[k]` bytes a vertex) from the fill table's bytes on, while every slot the stream
/// writes has room for the whole primitive (xd SO_SIZE); counters SO + 2 stream (written) and
/// SO + 2 stream + 1 (needed). One invocation: the order is the draw's. `osgn`: the outputs of the shader
/// that wrote the store.
pub fn so_pass(store: &Store, osgn: &[SigElement], decls: &[SoEntry], strides: &[u32; 4]) -> Result<Wgsl, String> {
    let stride = |k: u32| strides.get(kome::num::us(k)).copied().unwrap_or(0);
    let stream = match store {
        Store::Vertex { .. } => 0,
        Store::Geometry { stream, .. } => *stream,
    };
    let mut m = String::new();
    let mut bindings = Vec::new();
    xd_decl(&mut m, &mut bindings);
    store.decls(&mut m, &mut bindings);
    let mine: Vec<&SoEntry> = decls.iter().filter(|e| e.stream == stream).collect();
    let mut slots: Vec<u32> = mine.iter().map(|e| e.slot).collect();
    slots.sort_unstable();
    slots.dedup();
    for &k in &slots {
        if k >= 4 {
            return Err(format!("stream output slot {k}"));
        }
        wln!(m, "@group(0) @binding({}) var<storage, read_write> so{k}: array<u32>;", B_SO.saturating_add(k));
        bindings.push((B_SO.saturating_add(k), Binding::StorageBuffer(true)));
    }
    wln!(m, "@group(0) @binding({B_COUNTERS}) var<storage, read_write> counters: array<atomic<u32>>;");
    bindings.push((B_COUNTERS, Binding::StorageBuffer(true)));
    if bindings.iter().filter(|b| matches!(b.1, Binding::StorageBuffer(_))).count() > MAX_STORES {
        return Err(format!("stream output to slots {slots:?}: more than {MAX_STORES} storage buffers"));
    }
    let corners = store.corners();
    let regs = store.regs();
    let osgn: Vec<&SigElement> = osgn.iter().filter(|e| e.stream == stream).collect();
    m.push_str("@compute @workgroup_size(1)\nfn main() {\n");
    for &k in &slots {
        wln!(m, "    var f{k} = atomicLoad(&counters[{}u + xw({}u)]);", counter::N, xd::SO_SLOT.saturating_add(kome::num::us(k)));
    }
    m.push_str("    var written = 0u;\n    var needed = 0u;\n");
    // the primitives in order: (invocation, its primitives) of a geometry shader, the list of a vertex shader
    let (outer, inner) = match store {
        Store::Vertex { .. } => ("for (var q = 0u; q < 1u; q += 1u) {".to_string(), "let np = xw(XD_ITEMS);".to_string()),
        Store::Geometry { decl, stream, .. } => ("for (var q = 0u; q < xw(XD_ITEMS); q += 1u) {".to_string(), format!("let np = gcount[q * {}u + {stream}u];", decl.streams.len())),
    };
    wln!(m, "    {outer}\n        {inner}\n        for (var t = 0u; t < np; t += 1u) {{\n            needed += 1u;");
    let fits: Vec<String> = slots.iter().map(|&k| format!("f{k} + {}u <= xw({}u)", corners.saturating_mul(stride(k)), xd::SO_SIZE.saturating_add(kome::num::us(k)))).collect();
    wln!(m, "            if ({}) {{", if fits.is_empty() { "false".to_string() } else { fits.join(" && ") });
    let slot = match store {
        Store::Vertex { .. } => "xw(XD_FIRST) + t".to_string(),
        Store::Geometry { decl, stream, .. } => {
            let topo = decl.streams.get(kome::num::us(*stream)).copied().unwrap_or(1);
            let maxp = decl.max_prims(topo);
            format!("(q * {}u + {stream}u) * {maxp}u + t", decl.streams.len())
        }
    };
    wln!(m, "                for (var c = 0u; c < {corners}u; c += 1u) {{\n                    let e = list[({slot}) * {corners}u + c];");
    let mut at = [0u32; 4];
    for d in &mine {
        let k = kome::num::us(d.slot);
        match &d.name {
            None => {
                if let Some(a) = at.get_mut(k) {
                    *a = a.saturating_add(d.count.saturating_mul(4));
                }
            }
            Some(name) => {
                let el = osgn.iter().find(|e| e.name.eq_ignore_ascii_case(name) && e.index == d.index).ok_or(format!("stream output of {name}{} the shader does not write", d.index))?;
                let comps = bits(el.mask);
                for i in d.start..d.start.saturating_add(d.count) {
                    let v = match comps.get(kome::num::us(i)) {
                        Some(&c) => format!("store[e * {regs}u + {}u].{}", el.reg, xyzw(c)),
                        None => "0u".into(),
                    };
                    let a = at.get(k).copied().unwrap_or(0);
                    wln!(m, "                    so{}[(f{} + c * {}u + {}u) / 4u] = {v};", d.slot, d.slot, stride(d.slot), a);
                    if let Some(a) = at.get_mut(k) {
                        *a = a.saturating_add(4);
                    }
                }
            }
        }
    }
    m.push_str("                }\n");
    for &k in &slots {
        wln!(m, "                f{k} += {}u;", corners.saturating_mul(stride(k)));
    }
    m.push_str("                written += 1u;\n            }\n        }\n    }\n");
    for &k in &slots {
        wln!(m, "    atomicStore(&counters[{}u + xw({}u)], f{k});", counter::N, xd::SO_SLOT.saturating_add(kome::num::us(k)));
    }
    wln!(m, "    atomicAdd(&counters[{}u], written);\n    atomicAdd(&counters[{}u], needed);\n}}", counter::SO.saturating_add(stream.saturating_mul(2)), counter::SO.saturating_add(stream.saturating_mul(2)).saturating_add(1));
    Ok(Wgsl { code: m, bindings })
}

/// The clip planes in the order the clipper takes them (near, far, left, right, bottom, top):
/// a clip-space position's distance inside each (inside: >= 0).
pub const CLIP_PLANES: [fn(&[f32; 4]) -> f32; 6] = [|&[_, _, z, _]| z, |&[_, _, z, w]| w - z, |&[x, _, _, w]| w + x, |&[x, _, _, w]| w - x, |&[_, y, _, w]| w + y, |&[_, y, _, w]| w - y];

/// What Direct3D's clipper makes of one primitive (clip-space positions of its 1, 2 or 3
/// corners): how many primitives come out (CPrimitives). A triangle is clipped plane by plane
/// (CLIP_PLANES, Sutherland-Hodgman: a corner on a plane is inside, a crossing adds the
/// crossing point, so a polygon can hold one point twice) and its polygon of n points counts
/// n - 2 (a fan); a line counts 1 when a piece of it is left, a point when it is inside every
/// plane. Culling comes after (a back face still counts), and so do zero-area triangles.
/// winref d3d11gs "pipeline statistics of triangles in clip space" and "of indexed draws,
/// strips, lines and points". The same rule as [`clip_pass`]'s WGSL (clip_count).
pub fn clip_count(p: &[[f32; 4]]) -> u32 {
    let mut poly: Vec<[f32; 4]> = p.to_vec();
    let closed = p.len() >= 3;
    for plane in CLIP_PLANES {
        let mut out = Vec::with_capacity(poly.len().saturating_add(1));
        let n = poly.len();
        // (a polygon's edges wrap around; a line's one edge does not)
        let edges = if closed { n } else { n.saturating_sub(1) };
        if n == 1 {
            out.extend(poly.iter().filter(|v| plane(v) >= 0.0));
        }
        for k in 0..edges {
            let (Some(a), Some(b)) = (poly.get(k), poly.get(k.saturating_add(1).checked_rem(n).unwrap_or(0))) else { continue };
            let (da, db) = (plane(a), plane(b));
            if da >= 0.0 {
                out.push(*a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                out.push(std::array::from_fn(|c| a.get(c).copied().unwrap_or(0.0) + t * (b.get(c).copied().unwrap_or(0.0) - a.get(c).copied().unwrap_or(0.0))));
            }
            // (a line's far end, when it is inside)
            if !closed && db >= 0.0 {
                out.push(*b);
            }
        }
        poly = out;
        if poly.is_empty() {
            return 0;
        }
    }
    match p.len() {
        0 => 0,
        1 | 2 => 1,
        _ => kome::num::u32_of(poly.len().saturating_sub(2)),
    }
}

/// clip_count in WGSL: `fn clip_count(p: array<vec4<f32>, 3>, n: u32) -> u32` (n corners).
const CLIP_COUNT: &str = "fn clip_d(v: vec4<f32>, k: u32) -> f32 {
    switch k {
        case 0u: { return v.z; }
        case 1u: { return v.w - v.z; }
        case 2u: { return v.w + v.x; }
        case 3u: { return v.w - v.x; }
        case 4u: { return v.w + v.y; }
        default: { return v.w - v.y; }
    }
}
fn clip_count(p: array<vec4<f32>, 3>, n: u32) -> u32 {
    var poly: array<vec4<f32>, 12>;
    var next: array<vec4<f32>, 12>;
    var m = n;
    for (var c = 0u; c < n; c += 1u) { poly[c] = p[c]; }
    let closed = n >= 3u;
    for (var k = 0u; k < 6u; k += 1u) {
        var o = 0u;
        if (m == 1u && clip_d(poly[0], k) >= 0.0f) { next[0] = poly[0]; o = 1u; }
        let edges = select(m - 1u, m, closed);
        for (var e = 0u; e < edges && m > 1u; e += 1u) {
            let a = poly[e];
            let b = poly[(e + 1u) % m];
            let da = clip_d(a, k);
            let db = clip_d(b, k);
            if (da >= 0.0f && o < 12u) { next[o] = a; o += 1u; }
            if ((da >= 0.0f) != (db >= 0.0f) && o < 12u) { next[o] = a + ps_r4((b - a) * ps_div(da, da - db)); o += 1u; }
            if (!closed && db >= 0.0f && o < 12u) { next[o] = b; o += 1u; }
        }
        m = o;
        if (m == 0u) { return 0u; }
        for (var c = 0u; c < m; c += 1u) { poly[c] = next[c]; }
    }
    if (n < 3u) { return 1u; }
    return m - 2u;
}
";

/// Each list primitive of a store through the clipper: counters C_INVOCATIONS (+1 each),
/// C_PRIMITIVES (clip_count) and PS_PAIRS (ps::WGSL's ps_pairs, by the uniform's raster words).
/// `osgn`: the shader's outputs (its SV_Position).
pub fn clip_pass(store: &Store, osgn: &[SigElement]) -> Result<Wgsl, String> {
    let stream = match store {
        Store::Vertex { .. } => 0,
        Store::Geometry { stream, .. } => *stream,
    };
    let mut m = String::new();
    let mut bindings = Vec::new();
    xd_decl(&mut m, &mut bindings);
    store.decls(&mut m, &mut bindings);
    wln!(m, "@group(0) @binding({B_COUNTERS}) var<storage, read_write> counters: array<atomic<u32>, {}>;", counter::N);
    bindings.push((B_COUNTERS, Binding::StorageBuffer(true)));
    m.push_str(CLIP_COUNT);
    m.push_str(ps::WGSL);
    m.push_str(ps::DIV);
    let corners = store.corners();
    let regs = store.regs();
    let pos = osgn.iter().find(|e| e.stream == stream && e.sysval == 1).map(|e| e.reg);
    wln!(m, "@compute @workgroup_size(64)\nfn main(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) nwg: vec3<u32>) {{\n    {ITEM}");
    m.push_str(ps::ENTRY);
    wln!(m, "    let t = item;\n    let first = list[({}) * {corners}u];\n    if (first == 0xffffffffu) {{ return; }}", store.slot("t"));
    m.push_str("    var p: array<vec4<f32>, 3>;\n");
    wln!(m, "    for (var c = 0u; c < {corners}u; c += 1u) {{\n        let e = list[({}) * {corners}u + c];", store.slot("t"));
    match pos {
        Some(r) => {
            wln!(m, "        p[c] = bitcast<vec4<f32>>(store[e * {regs}u + {r}u]);");
        }
        None => m.push_str("        p[c] = vec4<f32>(0.0f);\n"),
    }
    m.push_str("    }\n");
    wln!(m, "    atomicAdd(&counters[{}u], 1u);\n    atomicAdd(&counters[{}u], clip_count(p, {corners}u));\n    atomicAdd(&counters[{}u], ps_pairs(p, {corners}u));\n}}", counter::C_INVOCATIONS, counter::C_PRIMITIVES, counter::PS_PAIRS);
    Ok(Wgsl { code: m, bindings })
}

/// Is `code` a geometry shader's bytecode?
pub fn is_gs(code: &[u8]) -> bool {
    parse(code).is_ok_and(|(_, p)| p.kind == tpf::GEOMETRY)
}

#[path = "expand_tess.rs"]
mod tess;
pub use tess::{ds_pass, hs_cp_regs, hs_pass, tess_decl, HsOut, B_DPTS, B_HCP, B_HPC, DPT};

#[path = "ps.rs"]
pub mod ps;

#[path = "setup.rs"]
pub mod setup;

#[cfg(test)]
#[path = "expand_tests.rs"]
mod tests;
