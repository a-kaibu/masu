//! Draws WebGPU's render pipeline cannot do alone: with a geometry shader, with stream output,
//! and while a query counts what the clipper does (dxwgsl::expand has the passes). A draw that
//! needs one runs as compute passes on the page:
//!
//! 1. the vertex shader once per vertex the draw reads (instances too) into `vout`;
//! 2. the geometry shader once per input primitive and GS instance (the primitives as
//!    [`assemble`] makes them from the topology, strip cuts of the indices, adjacency), what
//!    it emits into `gout` as list primitives (`glist`, `gcount`);
//! 3. stream output of each stream in primitive order into the bound buffers, from the fill
//!    table (bytes each buffer holds, kept on the page with the counters), counting
//!    SO_STATISTICS;
//! 4. while a PIPELINE_STATISTICS query is open, the clipper's counts (CInvocations,
//!    CPrimitives) of the rasterized stream;
//! 5. the rasterized stream drawn: a geometry shader's by the render pipeline from its store
//!    (dxwgsl::expand::store_vertex), anything else as a draw of its own (draw_with).
//!
//! The counters (u32 words, dxwgsl::expand::counter) and the fill table (words counter::N +
//! slot) are one page buffer; queries read them with COUNT_BEGIN / COUNT_END tickets. Buffers
//! stream output wrote are the page's newer copy: `pull` reads them back before the guest's
//! side reads or changes their bytes.

use super::*;
use crate::render::d3d::DATA;
use dxwgsl::expand::{self as x, Attr, SoEntry, Store};
use dxwgsl::Binding;
use tess::Tess;

#[path = "expand_tess.rs"]
pub mod tess;

/// CreateGeometryShaderWithStreamOutput's declarations: entries, buffer strides (bytes a
/// vertex, 0 for slots it names none), the rasterized stream (None: NO_RASTERIZED_STREAM).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SoDesc {
    pub decls: Vec<SoEntry>,
    pub strides: [u32; 4],
    pub rasterized: Option<u32>,
}

/// D3D11_SO_NO_RASTERIZED_STREAM
pub const NO_RASTERIZED_STREAM: u32 = 0xffff_ffff;

const E_INVALIDARG: u32 = 0x8007_0057;

/// A D3D11_SO_DECLARATION_ENTRY as the guest gave it (semantic None: a gap).
pub struct RawEntry {
    pub stream: u32,
    pub name: Option<String>,
    pub index: u32,
    pub start: u32,
    pub count: u32,
    pub slot: u32,
}

/// CreateGeometryShaderWithStreamOutput's checks of its declarations (D3D11 runtime: entries,
/// D3D11_SO_STREAM_COUNT streams and buffer slots, D3D11_SO_BUFFER_MAX_STRIDE_IN_BYTES; winref
/// d3d11gs "errors"): the declarations, or E_INVALIDARG. `osgn`: the outputs of the shader
/// (a geometry shader's, `gs`, or a vertex shader's).
pub fn so_desc(osgn: &[dxwgsl::dxbc::SigElement], gs: bool, entries: &[RawEntry], strides: &[u32], raster: u32) -> Result<SoDesc, u32> {
    if entries.is_empty() || entries.len() > 512 || strides.len() > 4 {
        return Err(E_INVALIDARG);
    }
    let rasterized = match raster {
        NO_RASTERIZED_STREAM => None,
        // (a vertex shader's has only stream 0: another rasterized stream is accepted, winref
        // d3d11gs "rasterized stream 1" S_OK, and has nothing in it)
        r if r < 4 && (gs || r == 0) => Some(r),
        r if r < 4 => None,
        _ => return Err(E_INVALIDARG),
    };
    let mut bytes = [0u32; 4];
    let mut stream_of: [Option<u32>; 4] = [None; 4];
    let mut decls = Vec::with_capacity(entries.len());
    for e in entries {
        let slot = us(e.slot);
        if e.stream >= 4 || (!gs && e.stream != 0) || e.slot >= 4 || slot >= strides.len() || e.count == 0 || e.count > 4 {
            return Err(E_INVALIDARG);
        }
        // (one slot takes one stream's)
        let (Some(of), Some(b)) = (stream_of.get_mut(slot), bytes.get_mut(slot)) else { return Err(E_INVALIDARG) };
        match *of {
            Some(s) if s != e.stream => return Err(E_INVALIDARG),
            Some(_) | None => *of = Some(e.stream),
        }
        if let Some(n) = &e.name
            && (e.start.saturating_add(e.count) > 4 || !osgn.iter().any(|o| o.stream == e.stream && o.name.eq_ignore_ascii_case(n) && o.index == e.index))
        {
            return Err(E_INVALIDARG);
        }
        *b = b.saturating_add(e.count.saturating_mul(4));
        decls.push(SoEntry { stream: e.stream, name: e.name.clone(), index: e.index, start: e.start, count: e.count, slot: e.slot });
    }
    let mut st = [0u32; 4];
    for ((&w, s), &need) in strides.iter().zip(st.iter_mut()).zip(&bytes) {
        if !w.is_multiple_of(4) || w > 2048 || w < need {
            return Err(E_INVALIDARG);
        }
        *s = w;
    }
    Ok(SoDesc { decls, strides: st, rasterized })
}

/// Fill table slots a page buffer is made with (more: a bigger one, the old one copied).
const SLOTS: u32 = 1024;

/// The page shaders of the passes (and the stores' vertex shaders) by what they are made of.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Vertex { vs: u32, attrs: Vec<Attr> },
    Geometry { gs: u32, vs: u32 },
    StoreVertex { src: u32, stream: u32, corners: u32, ps: u32 },
    So { gs: u32, vs: u32, stream: u32, corners: u32 },
    Clip { src: u32, stream: u32, corners: u32 },
    Hull { hs: u32, vs: u32 },
    Domain { ds: u32, hs: u32 },
}

/// A page shader and what it binds.
type PageShader = (u32, Vec<(u32, Binding)>);

/// What the device keeps for the expanded draws.
#[derive(Default)]
pub struct Gpu {
    /// the counters and the fill table (0: not made yet), its fill slots
    counters: u32,
    slots_made: u32,
    /// stream output buffers' fill slots, and slots free again
    slots: HashMap<u32, u32>,
    free: Vec<u32>,
    /// 256 bytes of zeros (a binding nothing is bound to)
    zero: u32,
    /// vout, gout, glist, gcount, hcp, hpc, dout, dlist: (page buffer, bytes), made bigger
    /// when a draw needs it
    scratch: [(u32, u32); 8],
    shaders: HashMap<Key, Option<PageShader>>,
    /// PIPELINE_STATISTICS queries open (their ids): draws count the clipper's work
    pub open: Vec<u32>,
}

const VOUT: usize = 0;
const GOUT: usize = 1;
const GLIST: usize = 2;
const GCOUNT: usize = 3;
/// a hull shader pass's output control points and patch constants, the domain shader pass's
/// outputs, the tessellator's primitives (a store draw's list)
const HCP: usize = 4;
const HPC: usize = 5;
const DOUT: usize = 6;
const DLIST: usize = 7;

/// The page buffer of the counters and the fill table (made, zeros, the first time).
pub fn counters(d: &mut D11) -> u32 {
    if d.expand.counters == 0 {
        let g = page_id();
        let bytes = word_at(SLOTS);
        gpu::record(|q| q.cmd(&[gpu::CREATE_BUFFER, g, bytes]));
        d.expand.counters = g;
        d.expand.slots_made = SLOTS;
    }
    d.expand.counters
}

fn zero(d: &mut D11) -> u32 {
    if d.expand.zero == 0 {
        let g = page_id();
        gpu::record(|q| q.cmd(&[gpu::CREATE_BUFFER, g, 256]));
        d.expand.zero = g;
    }
    d.expand.zero
}

/// Scratch buffer `k` with room for `bytes`.
fn scratch(d: &mut D11, k: usize, bytes: u32) -> u32 {
    let want = bytes.max(16).next_multiple_of(256);
    let Some(&(g, have)) = d.expand.scratch.get(k) else { return 0 };
    if g != 0 && have >= want {
        return g;
    }
    let size = want.max(have.saturating_mul(2));
    let n = page_id();
    gpu::record(|q| {
        if g != 0 {
            q.cmd(&[gpu::DESTROY_BUFFER, g]);
        }
        q.cmd(&[gpu::CREATE_BUFFER, n, size]);
    });
    if let Some(e) = d.expand.scratch.get_mut(k) {
        *e = (n, size);
    }
    n
}

/// The byte of the counters' buffer where fill slot `s` is (the first past the counters: their
/// buffer's size for `s` slots).
fn word_at(s: u32) -> u32 {
    x::counter::N.saturating_add(s).saturating_mul(4)
}

/// Stream output buffer `buf`'s fill slot (made the first time: 0 bytes).
pub fn fill_slot(d: &mut D11, buf: u32) -> u32 {
    if let Some(&s) = d.expand.slots.get(&buf) {
        return s;
    }
    let c = counters(d);
    let s = match d.expand.free.pop() {
        Some(s) => s,
        None => lo32(d.expand.slots.len()),
    };
    if s >= d.expand.slots_made {
        // a bigger table, the old one's words copied
        let made = d.expand.slots_made;
        let more = made.saturating_mul(2);
        let n = page_id();
        gpu::record(|q| {
            q.cmd(&[gpu::CREATE_BUFFER, n, word_at(more)]);
            q.cmd(&[gpu::COPY_BUFFER, c, 0, n, 0, word_at(made)]);
            q.cmd(&[gpu::DESTROY_BUFFER, c]);
        });
        d.expand.counters = n;
        d.expand.slots_made = more;
    }
    d.expand.slots.insert(buf, s);
    set_fill(d, buf, 0);
    s
}

/// The fill of buffer `buf` is `bytes` from here on (SOSetTargets with an offset).
pub fn set_fill(d: &mut D11, buf: u32, bytes: u32) {
    let s = fill_slot(d, buf);
    let c = counters(d);
    gpu::record(|q| {
        let off = q.blob(&bytes.to_le_bytes());
        q.cmd(&[gpu::WRITE_BUFFER, c, word_at(s), off, 4]);
    });
}

/// A buffer is gone: its fill slot is free.
pub fn forget_buffer(d: &mut D11, buf: u32) {
    if let Some(s) = d.expand.slots.remove(&buf) {
        d.expand.free.push(s);
    }
}

/// The fill of buffer `buf` (what stream output wrote to it), waited for; None: it has no fill
/// slot (never a stream output target) or the page gave up.
pub fn fill_of(d: &mut D11, buf: u32) -> Option<u32> {
    let s = *d.expand.slots.get(&buf)?;
    let c = counters(d);
    // (guest memory: what the page writes into; not after the wait gave up)
    let dst = d.guest_alloc(4);
    d.next_gpu = d.next_gpu.wrapping_add(1);
    let req = 0x4000_0000 | d.next_gpu;
    gpu::record(|q| q.cmd(&[gpu::READ_BUFFER, req, c, word_at(s), 4, dst]));
    // SAFETY: the 4 bytes just allocated, written by the page once the readback is done
    let fill = gpu::wait_readback(req).then(|| unsafe { rd(dst, 0) });
    d.guest_free(dst);
    fill
}

/// Buffer `id`'s bytes made the page's again if the GPU wrote them since (stream output): before
/// anything on the guest's side reads or changes them.
pub fn pull(d: &mut D11, id: u32) {
    let Some(Data::Buffer(b)) = d.objs.get(&id).map(|o| &o.data) else { return };
    if !b.gpu_written || b.gpu == 0 || b.mem == 0 {
        return;
    }
    let (g, mem, size) = (b.gpu, b.mem, b.size);
    d.next_gpu = d.next_gpu.wrapping_add(1);
    let req = 0x4000_0000 | d.next_gpu;
    let words = size & !3;
    gpu::record(|q| q.cmd(&[gpu::READ_BUFFER, req, g, 0, words, mem]));
    let ok = gpu::wait_readback(req);
    if let Some(Data::Buffer(b)) = d.objs.get_mut(&id).map(|o| &mut o.data) {
        b.gpu_written = false;
        if ok && b.usage == 2 {
            b.shadow.clear();
            // SAFETY: the buffer's guest memory, `size` bytes
            b.shadow.extend((0..size).map(|i| unsafe { *gc::<u8>(mem.wrapping_add(i)) }));
        }
    }
}

/// Shader object `id` is gone: the page shaders made of it (forgotten here; the caller
/// destroys them on the page).
pub fn forget_shader(d: &mut D11, id: u32) -> Vec<u32> {
    let mut gone = Vec::new();
    d.expand.shaders.retain(|k, v| {
        let of = match k {
            Key::Vertex { vs, .. } => *vs == id,
            Key::Geometry { gs, vs } | Key::So { gs, vs, .. } => *gs == id || *vs == id,
            Key::StoreVertex { src, ps, .. } => *src == id || *ps == id,
            Key::Clip { src, .. } => *src == id,
            Key::Hull { hs, vs } => *hs == id || *vs == id,
            Key::Domain { ds, hs } => *ds == id || *hs == id,
        };
        if of && let Some((m, _)) = v {
            gone.push(*m);
        }
        !of
    });
    gone
}

/// D3D11_PRIMITIVE_TOPOLOGY of the draws the passes run (no patches).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Topo {
    Points,
    Lines,
    LineStrip,
    Triangles,
    TriangleStrip,
    LinesAdj,
    LineStripAdj,
    TrianglesAdj,
    TriangleStripAdj,
}

impl Topo {
    pub fn of(t: u32) -> Option<Topo> {
        Some(match t {
            1 => Topo::Points,
            2 => Topo::Lines,
            3 => Topo::LineStrip,
            4 => Topo::Triangles,
            5 => Topo::TriangleStrip,
            10 => Topo::LinesAdj,
            11 => Topo::LineStripAdj,
            12 => Topo::TrianglesAdj,
            13 => Topo::TriangleStripAdj,
            _ => return None,
        })
    }

    /// The vertices a geometry shader gets of each primitive.
    pub fn input(self) -> u32 {
        match self {
            Topo::Points => 1,
            Topo::Lines | Topo::LineStrip => 2,
            Topo::Triangles | Topo::TriangleStrip => 3,
            Topo::LinesAdj | Topo::LineStripAdj => 4,
            Topo::TrianglesAdj | Topo::TriangleStripAdj => 6,
        }
    }

    /// Of those, the primitive's own (without a geometry shader: what goes on).
    pub fn corners(self) -> &'static [usize] {
        match self {
            Topo::Points => &[0],
            Topo::Lines | Topo::LineStrip => &[0, 1],
            Topo::Triangles | Topo::TriangleStrip => &[0, 1, 2],
            Topo::LinesAdj | Topo::LineStripAdj => &[1, 2],
            Topo::TrianglesAdj | Topo::TriangleStripAdj => &[0, 2, 4],
        }
    }

    fn strip(self) -> bool {
        match self {
            Topo::LineStrip | Topo::TriangleStrip | Topo::LineStripAdj | Topo::TriangleStripAdj => true,
            Topo::Points | Topo::Lines | Topo::Triangles | Topo::LinesAdj | Topo::TrianglesAdj => false,
        }
    }
}

/// The primitives `n` vertices of topology `t` make, each its geometry shader input's vertex
/// numbers (0 .. n) in order; `cut(k)`: vertex k is a strip cut (the index 0xffff or
/// 0xffffffff of an indexed strip), which ends the strip there. Strips' odd triangles keep
/// their winding (dxwgsl::expand::ODD); a triangle strip with adjacency is OpenGL's and
/// Direct3D's table of its vertices.
pub fn assemble(t: Topo, n: u32, cut: impl Fn(u32) -> bool) -> Vec<Vec<u32>> {
    let mut out = Vec::new();
    if !t.strip() {
        let per = t.input();
        let all: Vec<u32> = (0..n).collect();
        return all.chunks_exact(us(per)).map(<[u32]>::to_vec).collect();
    }
    let mut run: Vec<u32> = Vec::new();
    let mut flush = |run: &mut Vec<u32>| {
        strip_prims(t, run, &mut out);
        run.clear();
    };
    for k in 0..n {
        if cut(k) {
            flush(&mut run);
        } else {
            run.push(k);
        }
    }
    flush(&mut run);
    out
}

fn strip_prims(t: Topo, r: &[u32], out: &mut Vec<Vec<u32>>) {
    let at = |i: usize| r.get(i).copied().unwrap_or(0);
    match t {
        Topo::LineStrip | Topo::LineStripAdj => out.extend(r.windows(us(t.input())).map(<[u32]>::to_vec)),
        Topo::TriangleStrip => {
            for (j, w) in r.windows(3).enumerate() {
                out.push(if j % 2 == 0 { w.to_vec() } else { x::ODD.iter().map(|&o| w.get(us(o)).copied().unwrap_or(0)).collect() });
            }
        }
        Topo::TriangleStripAdj => {
            // (2 (m + 2) vertices: m triangles, as Direct3D gives them to a geometry shader:
            // corner, adjacent, corner, adjacent, corner, adjacent, the odd triangles' corners
            // in the order that keeps the strip's winding; winref d3d11gs "the vertices a
            // geometry shader is given", strip adjacency of 6, 8 and 10)
            let m = (r.len() / 2).saturating_sub(2);
            for i in 0..m {
                let p = |o: usize| i.saturating_mul(2).saturating_add(o);
                let far = if i.saturating_add(1) == m { p(5) } else { p(6) };
                let k: [usize; 6] = if i % 2 == 0 {
                    [p(0), if i == 0 { 1 } else { p(0).saturating_sub(2) }, p(2), far, p(4), p(3)]
                } else {
                    [p(0), p(3), p(4), far, p(2), p(0).saturating_sub(2)]
                };
                out.push(k.iter().map(|&k| at(k)).collect());
            }
        }
        Topo::Points | Topo::Lines | Topo::Triangles | Topo::LinesAdj | Topo::TrianglesAdj => {}
    }
}

/// The restart index of the bound index buffer's format (strips are cut at it).
fn cut_value(st: &State) -> u32 {
    if st.ib.1 == 42 { u32::MAX } else { 0xffff }
}

/// Index `k` of a draw from index `start` of the bound index buffer: 0 past its end or without
/// one (Direct3D 11 reads 0 there: winref d3d11gs "indexed strip" reading past the end).
fn index_at(d: &D11, st: &State, start: u32, k: u32) -> u32 {
    let Some(Data::Buffer(b)) = d.objs.get(&st.ib.0).map(|o| &o.data) else { return 0 };
    let isz: u32 = if st.ib.1 == 42 { 4 } else { 2 };
    let at = u64::from(st.ib.2).saturating_add(u64::from(start).saturating_add(u64::from(k)).saturating_mul(u64::from(isz)));
    if at.saturating_add(u64::from(isz)) > u64::from(b.size) {
        return 0;
    }
    let p = b.mem.wrapping_add(lo32(at));
    if isz == 4 {
        // SAFETY: inside the buffer's guest memory (checked above)
        unsafe { rd(p, 0) }
    } else {
        // SAFETY: inside the buffer's guest memory (checked above)
        u32::from(unsafe { *gc::<u16>(p) })
    }
}

/// An indexed draw's input assembly per instance (count indices from `start`): its primitives
/// (a strip cut at the restart index) and the distinct indices of their vertices, the vertex
/// shader's runs (Direct3D shades each once: winref d3d11gs "indexed 0 1 2 2 1 3" VS 4).
/// None for a topology the input assembler does not make primitives of here (patch lists).
pub fn assembled_indexed(d: &mut D11, st: &State, count: u32, start: u32) -> Option<query::Assembled> {
    let topo = Topo::of(st.topology)?;
    pull(d, st.ib.0);
    let prims = assemble(topo, count, |k| topo.strip() && index_at(d, st, start, k) == cut_value(st));
    let mut seen = std::collections::HashSet::new();
    for k in prims.iter().flatten() {
        seen.insert(index_at(d, st, start, *k));
    }
    Some(query::Assembled { primitives: u64::try_from(prims.len()).unwrap_or(u64::MAX), shaded: u64::try_from(seen.len()).unwrap_or(u64::MAX) })
}

/// What the rasterizer takes of a draw of an adjacency topology without a geometry shader: each
/// primitive's own corners (`Topo::corners`), as a line list (lines with adjacency) or a
/// triangle list, the vertex numbers of a draw without indices or the index values of one
/// with them (strips cut at the cut value, indices past the buffer's end 0). None: not an
/// adjacency topology.
pub fn adjacency_list(d: &mut D11, st: &State, indexed: bool, count: u32, start: u32) -> Option<(Vec<u32>, bool)> {
    let topo = Topo::of(st.topology)?;
    let lines = match topo {
        Topo::LinesAdj | Topo::LineStripAdj => true,
        Topo::TrianglesAdj | Topo::TriangleStripAdj => false,
        Topo::Points | Topo::Lines | Topo::LineStrip | Topo::Triangles | Topo::TriangleStrip => return None,
    };
    if indexed {
        pull(d, st.ib.0);
    }
    let corners = rasterized(topo, count, |k| indexed && topo.strip() && index_at(d, st, start, k) == cut_value(st));
    let list = corners.into_iter().map(|k| if indexed { index_at(d, st, start, k) } else { start.saturating_add(k) }).collect();
    Some((list, lines))
}

/// The vertex numbers (0 .. n) of the corners the rasterizer takes of `n` vertices of topology
/// `t` (`assemble`, `Topo::corners`), primitive after primitive.
pub fn rasterized(t: Topo, n: u32, cut: impl Fn(u32) -> bool) -> Vec<u32> {
    assemble(t, n, cut).iter().flat_map(|p| t.corners().iter().map(|&c| p.get(c).copied().unwrap_or(0))).collect()
}

/// A pass that could not be made: without a geometry shader the draw goes on as it is, with one
/// it draws nothing (its output never reached the rasterizer).
fn failed(gs: bool) -> After {
    if gs { After::Done } else { After::Draw }
}

/// What the draw goes on to do after its passes.
pub enum After {
    /// the draw as it is (the render pipeline draws it)
    Draw,
    /// nothing more (drawn from a store, or nothing rasterized)
    Done,
}

/// Word `k` of the xd array that starts at word `at` (VB_OFFSET, SO_SLOT, ...).
fn put(xw: &mut [u32; x::xd::WORDS], at: usize, k: usize, v: u32) {
    if let Some(w) = xw.get_mut(at.saturating_add(k)) {
        *w = v;
    }
}

/// The page shader of `key`, made of `make` the first time.
fn shader(d: &mut D11, key: Key, make: impl FnOnce(&D11) -> Result<dxwgsl::Wgsl, String>) -> Option<PageShader> {
    if let Some(r) = d.expand.shaders.get(&key) {
        return r.clone();
    }
    let r = match make(d) {
        Ok(w) => {
            let id = page_id();
            gpu::record(|q| {
                let off = q.blob(w.code.as_bytes());
                q.cmd(&[gpu::CREATE_SHADER, id, off, lo32(w.code.len())]);
            });
            if d.trace {
                eprintln!("d3d11: expanded draw shader {key:?} -> {id} bindings {:?}", w.bindings);
                if trace2() {
                    eprintln!("{}", w.code);
                }
            }
            Some((id, w.bindings))
        }
        Err(e) => {
            eprintln!("d3d11: expanded draw {key:?}: {e}");
            None
        }
    };
    d.expand.shaders.insert(key, r.clone());
    r
}

fn code_of(d: &D11, id: u32) -> Option<&[u8]> {
    match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::Shader(s)) => Some(&s.code),
        _ => None,
    }
}

/// What a pass binds besides its shader's own resources: a page buffer's entry, or bytes the
/// batch's data block holds for it (a uniform, kind 0, or a read-only store, 9), put there in
/// the same batch as the pass.
pub enum Over {
    Entry([u32; 8]),
    Data { binding: u32, kind: u32, bytes: Vec<u8> },
}

/// The uniform `xd` of a pass.
fn xd(words: &[u32; x::xd::WORDS]) -> Over {
    Over::Data { binding: x::B_XD, kind: 0, bytes: words.iter().flat_map(|w| w.to_le_bytes()).collect() }
}

/// A read-only store of `words` (at least one).
fn data_store(binding: u32, words: &[u32]) -> Over {
    let mut bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
    if bytes.is_empty() {
        bytes.resize(4, 0);
    }
    Over::Data { binding, kind: 9, bytes }
}

fn store(binding: u32, buf: u32, rw: bool) -> Over {
    Over::Entry(store_entry(binding, buf, rw))
}

/// A page buffer's whole bytes as a store (read-write or read-only).
fn store_entry(binding: u32, buf: u32, rw: bool) -> [u32; 8] {
    [binding, if rw { 10 } else { 9 }, buf, 0, 0, 0, 0, 0]
}

/// The entries of `over` with the data block's placed at offset 0 (`place` puts them).
fn entries_of(over: &[Over]) -> Vec<[u32; 8]> {
    over.iter()
        .map(|o| match o {
            Over::Entry(e) => *e,
            Over::Data { binding, kind, bytes } => {
                let n = lo32(bytes.len());
                [*binding, *kind, DATA, 0, n, if *kind == 0 { n } else { 0 }, 0, 0]
            }
        })
        .collect()
}

/// Puts `over`'s bytes into the batch's data block, their entries' offsets into `entries`.
pub fn place(q: &mut gpu::Batch, over: &[Over], entries: &mut [[u32; 8]]) {
    for o in over {
        if let Over::Data { binding, bytes, .. } = o {
            let at = q.data(bytes, 256);
            for e in entries.iter_mut().filter(|e| e[0] == *binding && e[2] == DATA) {
                e[3] = at;
            }
        }
    }
}

/// Runs a compute pass: shader `sh` with stage `stage`'s resources and `over`, `items`
/// invocations in groups of 64 (None: one invocation, the stream output pass).
fn pass(d: &mut D11, st: &State, stage: usize, sh: &PageShader, over: &[Over], items: Option<u32>) {
    let mut cbs = Vec::new();
    let mut entries = bind_entries(d, st, stage, &sh.1, &entries_of(over), &mut cbs);
    let (gx, gy) = match items {
        None => (1, 1),
        Some(n) => {
            let groups = n.div_ceil(64).max(1);
            if groups <= 65535 { (groups, 1) } else { (65535, groups.div_ceil(65535)) }
        }
    };
    gpu::record(|q| {
        place(q, over, &mut entries);
        let mut w = vec![gpu::DISPATCH, sh.0, gx, gy, 1, lo32(entries.len())];
        for e in &entries {
            w.extend_from_slice(e);
        }
        q.cmd(&w);
    });
    d.targets = None;
}

/// The draw's passes (when it needs them: a geometry shader, stream output, or a query that
/// counts the clipper's work), what it goes on to do.
#[allow(clippy::too_many_arguments)]
pub unsafe fn run(d: &mut D11, st: &State, indexed: bool, count: u32, instances: u32, start: u32, base: i32, first_instance: u32) -> After {
    let gobj = st.shaders[0];
    let (gs_code, so) = match d.objs.get(&gobj).map(|o| &o.data) {
        Some(Data::Shader(s)) => (x::is_gs(&s.code).then(|| s.code.clone()), s.so.clone()),
        _ => (None, None),
    };
    let stats = !d.expand.open.is_empty();
    let ts = match tess::stages(d, st) {
        Tess::Nothing => return After::Done,
        Tess::Off => None,
        Tess::On(s) => Some(s),
    };
    if gs_code.is_none() && so.is_none() && !stats && ts.is_none() {
        return After::Draw;
    }
    // (tessellating: the tessellator's primitives, after the hull and domain shader passes)
    let topo = match &ts {
        Some(s) => tess::topo(s.output),
        None => match Topo::of(st.topology) {
            Some(t) => t,
            None => return After::Draw,
        },
    };
    let stored = gs_code.is_some() || ts.is_some();
    let Some(vs_code) = code_of(d, st.vs).map(<[u8]>::to_vec) else { return After::Draw };
    let vs_osgn = dxwgsl::signature(&vs_code, true);
    let vregs = x::store_regs(&vs_osgn, 0);
    // the vertices: index values (cuts of strips), the vertex buffers' attributes
    let ib = if indexed { st.ib.0 } else { 0 };
    if ib != 0 {
        pull(d, ib);
    }
    let prims = if ts.is_some() { Vec::new() } else { assemble(topo, count, |k| indexed && topo.strip() && index_at(d, st, start, k) == cut_value(st)) };
    let attrs = attributes(d, st, &vs_code);
    let Some((attrs, vbs)) = attrs else { return After::Draw };
    // the uniform's vertex words
    let mut xw = [0u32; x::xd::WORDS];
    put(&mut xw, x::xd::COUNT, 0, count);
    put(&mut xw, x::xd::START, 0, start);
    put(&mut xw, x::xd::BASE, 0, base.cast_unsigned());
    put(&mut xw, x::xd::INSTANCES, 0, instances);
    put(&mut xw, x::xd::FIRST_INSTANCE, 0, first_instance);
    if indexed {
        let size = match d.objs.get(&ib).map(|o| &o.data) {
            Some(Data::Buffer(b)) => b.size,
            _ => 0,
        };
        put(&mut xw, x::xd::INDEX_BITS, 0, if st.ib.1 == 42 { 32 } else { 16 });
        put(&mut xw, x::xd::INDEX_OFFSET, 0, st.ib.2);
        put(&mut xw, x::xd::INDEX_SIZE, 0, size.saturating_sub(st.ib.2));
    }
    if vbs.slots.len() > 8 {
        eprintln!("d3d11: expanded draw: {} input slots (the vertex pass reads 8)", vbs.slots.len());
        return failed(stored);
    }
    for (k, &(_, stride, off, size)) in vbs.slots.iter().enumerate() {
        put(&mut xw, x::xd::VB_OFFSET, k, off);
        put(&mut xw, x::xd::VB_STRIDE, k, stride);
        put(&mut xw, x::xd::VB_SIZE, k, size.saturating_sub(off));
    }
    let items = count.saturating_mul(instances);
    put(&mut xw, x::xd::ITEMS, 0, items);
    // 1. the vertex pass
    let vs_obj = st.vs;
    let Some(vsh) = shader(d, Key::Vertex { vs: vs_obj, attrs: attrs.clone() }, |_| x::vertex_pass(&vs_code, &attrs)) else { return failed(stored) };
    let vout = scratch(d, VOUT, items.saturating_mul(vregs).saturating_mul(16));
    let mut over = vec![xd(&xw), store(x::B_VOUT, vout, true)];
    for (k, &buf) in (0u32..).zip(&vbs.bufs) {
        // (a slot nothing is bound to: zeros, its size 0)
        let g = if buf != 0 { buffer_gpu(d, buf) } else { zero(d) };
        over.push(store(x::B_VB.saturating_add(k), g, false));
    }
    let ibg = if ib != 0 { buffer_gpu(d, ib) } else { zero(d) };
    over.push(store(x::B_IB, ibg, false));
    pass(d, st, 0, &vsh, &over, Some(items));
    // tessellation: the hull shader pass, the tessellator, the domain shader pass; what the
    // domain shader makes is the draw's vertices from here on (one "instance" of them)
    let (vout, vs_osgn, vregs, vs_obj, count, instances, prims, ids) = match &ts {
        None => (vout, vs_osgn, vregs, vs_obj, count, instances, prims, None),
        Some(s) => {
            let Some(t) = tessellate(d, st, s, &xw, vout, &vs_osgn, vregs, count, instances) else { return After::Done };
            let ds_osgn = dxwgsl::signature(&s.ds_code, true);
            let dregs = x::store_regs(&ds_osgn, 0);
            (t.0, ds_osgn, dregs, s.ds, t.1.count(), 1, t.1.prims, Some(t.1.ids))
        }
    };
    // the primitives of every instance: their vertex items
    let per_inst = lo32(prims.len());
    let nprims = per_inst.saturating_mul(instances);
    let corners_of = |p: &Vec<u32>| -> Vec<u32> { topo.corners().iter().map(|&c| p.get(c).copied().unwrap_or(0)).collect() };
    let counters = counters(d);
    // stream output's buffers (bound, with declarations)
    let so_bufs: [u32; 4] = st.so;
    let mut gs_decl = None;
    let mut gs_items = 0;
    if let Some(gs) = &gs_code {
        // 2. the geometry shader pass
        let Ok(decl) = x::gs_decl(gs) else { return After::Done };
        let inst = decl.instances.max(1);
        gs_items = nprims.saturating_mul(inst);
        let verts = topo.input();
        if verts != decl.verts {
            // (a geometry shader of another input primitive: Direct3D draws nothing)
            return After::Done;
        }
        let mut list = Vec::with_capacity(us(nprims.saturating_mul(verts.saturating_add(1))));
        for i in 0..instances {
            for (id, p) in (0u32..).zip(&prims) {
                list.extend(p.iter().map(|&k| i.saturating_mul(count).saturating_add(k)));
                // (after tessellation: the patch's SV_PrimitiveID)
                list.push(ids.as_ref().and_then(|v| v.get(us(id))).copied().unwrap_or(id));
            }
        }
        let gregs = x::gs_regs(gs);
        let ns = lo32(decl.streams.len()).max(1);
        let gsh = {
            let gs = gs.clone();
            let vo = vs_osgn.clone();
            shader(d, Key::Geometry { gs: gobj, vs: vs_obj }, move |_| x::gs_pass(&gs, &vo, vregs))
        };
        let Some(gsh) = gsh else { return After::Done };
        let slots = gs_items.saturating_mul(ns);
        let gout = scratch(d, GOUT, slots.saturating_mul(decl.maxout).saturating_mul(gregs).saturating_mul(16));
        let maxp = decl.streams.iter().map(|&t| decl.max_prims(t).saturating_mul(dxwgsl::GsDecl::prim_verts(t))).max().unwrap_or(1);
        let glist = scratch(d, GLIST, slots.saturating_mul(maxp).saturating_mul(4));
        let gcount = scratch(d, GCOUNT, slots.saturating_mul(4));
        let mut gx = xw;
        put(&mut gx, x::xd::ITEMS, 0, gs_items);
        let over = [xd(&gx), store(x::B_VOUT, vout, false), data_store(x::B_PRIMS, &list), store(x::B_GOUT, gout, true), store(x::B_GLIST, glist, true), store(x::B_GCOUNT, gcount, true), store(x::B_COUNTERS, counters, true)];
        pass(d, st, 2, &gsh, &over, Some(gs_items));
        gs_decl = Some((decl, gregs, gout, glist, gcount));
    }
    // the list primitives of the vertex shader's (no geometry shader)
    let vlist: Vec<u32> = if gs_decl.is_none() {
        let mut l = Vec::new();
        for i in 0..instances {
            for p in &prims {
                l.extend(corners_of(p).into_iter().map(|k| i.saturating_mul(count).saturating_add(k)));
            }
        }
        l
    } else {
        Vec::new()
    };
    let vcorners = lo32(topo.corners().len());
    // the store of stream s: (Store, its source shader object, its outputs, entries, list
    // primitives)
    let store_of = |s: u32| -> (Store, Vec<dxwgsl::dxbc::SigElement>, u32) {
        match &gs_decl {
            Some((decl, gregs, ..)) => {
                let t = decl.streams.get(us(s)).copied().unwrap_or(1);
                let osgn = gs_code.as_deref().map_or_default(|g| dxwgsl::signature(g, true));
                (Store::Geometry { decl: decl.clone(), stream: s, regs: *gregs }, osgn, gs_items.saturating_mul(decl.max_prims(t)))
            }
            None => (Store::Vertex { regs: vregs, corners: vcorners }, vs_osgn.clone(), nprims),
        }
    };
    let store_entries = || -> Vec<Over> {
        match &gs_decl {
            Some((_, _, gout, glist, gcount)) => vec![store(x::B_GOUT, *gout, false), store(x::B_GLIST, *glist, false), store(x::B_GCOUNT, *gcount, false)],
            None => vec![store(x::B_VOUT, vout, false), data_store(x::B_PRIMS, &vlist)],
        }
    };
    let src = if gs_decl.is_some() { gobj } else { vs_obj };
    // 3. stream output, stream by stream
    if let Some(so) = &so {
        let streams = gs_decl.as_ref().map_or(1, |g| lo32(g.0.streams.len()).max(1));
        for s in 0..streams {
            if !so.decls.iter().any(|e| e.stream == s) {
                continue;
            }
            let (stv, osgn, _) = store_of(s);
            let corners = stv.corners();
            let decls = so.decls.clone();
            let strides = so.strides;
            let Some(ssh) = shader(d, Key::So { gs: gobj, vs: vs_obj, stream: s, corners }, move |_| x::so_pass(&stv, &osgn, &decls, &strides)) else { continue };
            let mut sx = xw;
            put(&mut sx, x::xd::ITEMS, 0, if gs_decl.is_some() { gs_items } else { nprims });
            put(&mut sx, x::xd::FIRST, 0, 0);
            let mut over = vec![store(x::B_COUNTERS, counters, true)];
            for k in 0..4u32 {
                if !so.decls.iter().any(|e| e.stream == s && e.slot == k) {
                    continue;
                }
                let buf = so_bufs.get(us(k)).copied().unwrap_or(0);
                let size = match d.objs.get(&buf).map(|o| &o.data) {
                    Some(Data::Buffer(b)) => Some(b.size),
                    _ => None,
                };
                match size {
                    Some(size) => {
                        let slot = fill_slot(d, buf);
                        let g = buffer_gpu(d, buf);
                        put(&mut sx, x::xd::SO_SLOT, us(k), slot);
                        put(&mut sx, x::xd::SO_SIZE, us(k), size);
                        over.push(store(x::B_SO.saturating_add(k), g, true));
                        if let Some(Data::Buffer(b)) = d.objs.get_mut(&buf).map(|o| &mut o.data) {
                            b.gpu_written = true;
                        }
                    }
                    None => {
                        let z = zero(d);
                        over.push(store(x::B_SO.saturating_add(k), z, true));
                    }
                }
            }
            over.push(xd(&sx));
            over.extend(store_entries());
            pass(d, st, 0, &ssh, &over, None);
        }
    }
    // the rasterized stream: a geometry shader's (0 without declarations), the vertex
    // shader's unless stream output says none
    let raster = match &so {
        Some(so) => so.rasterized,
        None => Some(0),
    };
    // 4. the clipper's counts
    if stats && let Some(r) = raster {
        let (stv, osgn, slots) = store_of(r);
        let corners = stv.corners();
        if let Some(csh) = shader(d, Key::Clip { src, stream: r, corners }, move |_| x::clip_pass(&stv, &osgn)) {
            let mut cx = xw;
            put(&mut cx, x::xd::ITEMS, 0, slots);
            raster_words(d, st, &mut cx);
            let mut over = vec![xd(&cx), store(x::B_COUNTERS, counters, true)];
            over.extend(store_entries());
            pass(d, st, 0, &csh, &over, Some(slots));
        }
    }
    // 5. drawn
    match (&gs_decl, raster) {
        (_, None) => After::Done,
        (None, Some(_)) if ts.is_none() => After::Draw,
        (None, Some(r)) => {
            // (the domain shader's vertices, the tessellator's primitives: a store's draw)
            let (stv, osgn, slots) = store_of(r);
            let corners = stv.corners();
            let ps_obj = st.ps;
            let ps_code = code_of(d, ps_obj).map(<[u8]>::to_vec);
            let Some(vsh) = shader(d, Key::StoreVertex { src, stream: r, corners, ps: ps_obj }, move |_| x::store_vertex(&stv, &osgn, ps_code.as_deref())) else { return After::Done };
            let bytes = vlist.len().saturating_mul(4);
            let dlist = scratch(d, DLIST, lo32(bytes));
            gpu::record(|q| {
                let off = q.blob(&vlist.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>());
                q.cmd(&[gpu::WRITE_BUFFER, dlist, 0, off, lo32(bytes)]);
            });
            let topo = match corners {
                1 => 0,
                2 => 1,
                _ => 3,
            };
            let sd = StoreDraw { vs: vsh, over: vec![store_entry(x::B_VOUT, vout, false), store_entry(x::B_PRIMS, dlist, false)], topo, count: slots.saturating_mul(corners) };
            // SAFETY: run's own contract (a store draw reads no guest memory of its own)
            unsafe { draw_with(d, st, false, sd.count, 1, 0, 0, 0, None, Some(&sd)) };
            After::Done
        }
        (Some((decl, ..)), Some(r)) => {
            let (stv, osgn, slots) = store_of(r);
            let corners = stv.corners();
            let ps_obj = st.ps;
            let ps_code = code_of(d, ps_obj).map(<[u8]>::to_vec);
            let Some(vsh) = shader(d, Key::StoreVertex { src, stream: r, corners, ps: ps_obj }, move |_| x::store_vertex(&stv, &osgn, ps_code.as_deref())) else { return After::Done };
            let topo = match decl.streams.get(us(r)).map(|&t| dxwgsl::GsDecl::prim_verts(t)) {
                Some(1) => 0,
                Some(2) => 1,
                _ => 3,
            };
            let Some((_, _, gout, glist, gcount)) = &gs_decl else { return After::Done };
            let over = vec![store_entry(x::B_GOUT, *gout, false), store_entry(x::B_GLIST, *glist, false), store_entry(x::B_GCOUNT, *gcount, false)];
            let sd = StoreDraw { vs: vsh, over, topo, count: slots.saturating_mul(corners) };
            // SAFETY: run's own contract (a store draw reads no guest memory of its own)
            unsafe { draw_with(d, st, false, sd.count, 1, 0, 0, 0, None, Some(&sd)) };
            After::Done
        }
    }
}

/// The hull shader pass over the draw's patches (the vertex pass's `vout`), the tessellator
/// over their patch constants (read back), the domain shader pass over its points: the domain
/// shader's outputs (a page buffer as vout) and the points. None: nothing to draw (no whole
/// patch, every one culled, a pass that could not be made).
#[allow(clippy::too_many_arguments)]
fn tessellate(d: &mut D11, st: &State, s: &tess::Stages, xw: &[u32; x::xd::WORDS], vout: u32, vs_osgn: &[dxwgsl::dxbc::SigElement], vregs: u32, count: u32, instances: u32) -> Option<(u32, tess::Points)> {
    let per_inst = count.checked_div(s.decl.in_cps).unwrap_or(0);
    let patches = per_inst.saturating_mul(instances);
    if patches == 0 {
        return None;
    }
    let out = match x::HsOut::of(&s.hs_code) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("d3d11: hull shader: {e}");
            return None;
        }
    };
    let (hs_obj, ds_obj) = (s.hs, s.ds);
    let hsh = {
        let code = s.hs_code.clone();
        let vo = vs_osgn.to_vec();
        shader(d, Key::Hull { hs: hs_obj, vs: st.vs }, move |_| x::hs_pass(&code, &vo, vregs))?
    };
    let hcp = scratch(d, HCP, patches.saturating_mul(out.cps).saturating_mul(out.cp_regs).saturating_mul(16));
    let pc_words = patches.saturating_mul(out.pc_regs).saturating_mul(4);
    let hpc = scratch(d, HPC, pc_words.saturating_mul(4));
    let mut hx = *xw;
    put(&mut hx, x::xd::ITEMS, 0, patches);
    pass(d, st, 3, &hsh, &[xd(&hx), store(x::B_VOUT, vout, false), store(x::B_HCP, hcp, true), store(x::B_HPC, hpc, true)], Some(patches));
    // the patch constants, read back for the tessellator
    let words = read_back(d, hpc, pc_words)?;
    #[cfg(test)]
    let words = tess::MODEL_PATCH_CONSTANTS.with(|m| m.borrow_mut().take()).unwrap_or(words);
    let regs: Vec<[u32; 4]> = words.chunks_exact(4).map(|c| <[u32; 4]>::try_from(c).unwrap_or([0; 4])).collect();
    let pcr = us(out.pc_regs);
    let pts = tess::points(s, patches, per_inst, |k| regs.iter().skip(us(k).saturating_mul(pcr)).take(pcr).copied().collect());
    // DSInvocations: a domain shader run per point
    if let Some(c) = d.stats.get_mut(9) {
        *c = c.saturating_add(u64::from(pts.count()));
    }
    let n = pts.count();
    if n == 0 {
        return None;
    }
    let dsh = {
        let code = s.ds_code.clone();
        let o = out.clone();
        shader(d, Key::Domain { ds: ds_obj, hs: hs_obj }, move |_| x::ds_pass(&code, &o))?
    };
    let dregs = x::store_regs(&dxwgsl::signature(&s.ds_code, true), 0);
    let dout = scratch(d, DOUT, n.saturating_mul(dregs).saturating_mul(16));
    let mut dx = *xw;
    put(&mut dx, x::xd::ITEMS, 0, n);
    pass(d, st, 4, &dsh, &[xd(&dx), store(x::B_HCP, hcp, false), store(x::B_HPC, hpc, false), data_store(x::B_DPTS, &pts.dpts), store(x::B_VOUT, dout, true)], Some(n));
    Some((dout, pts))
}

/// `words` words of page buffer `buf` from its start, waited for (None: the page gave up).
fn read_back(d: &mut D11, buf: u32, words: u32) -> Option<Vec<u32>> {
    let bytes = words.saturating_mul(4);
    let dst = d.guest_alloc(bytes);
    if dst == 0 {
        return None;
    }
    d.next_gpu = d.next_gpu.wrapping_add(1);
    let req = 0x4000_0000 | d.next_gpu;
    gpu::record(|q| q.cmd(&[gpu::READ_BUFFER, req, buf, 0, bytes, dst]));
    // SAFETY: word i of the `bytes` just allocated, written by the page once the readback is
    // done
    let v = gpu::wait_readback(req).then(|| (0..us(words)).map(|i| unsafe { rd(dst, i) }).collect());
    d.guest_free(dst);
    v
}

/// The clip pass's raster words (xd VIEWPORT, RASTER, RECT) for PSInvocations: the first
/// viewport, the rasterizer state's cull mode, winding and depth clip (D3D11's defaults without
/// one), whether a pixel shader is bound, and the pixels that can be drawn: the viewport's
/// (centres inside it), the first render target's (or the depth view's) and, with ScissorEnable,
/// the first scissor rectangle's.
fn raster_words(d: &D11, st: &State, w: &mut [u32; x::xd::WORDS]) {
    let vp = st.viewport[0];
    for (k, v) in vp.iter().take(4).enumerate() {
        put(w, x::xd::VIEWPORT, k, v.to_bits());
    }
    let r = state_desc(d, st.raster).cloned().unwrap_or_else(|| vec![3, 3, 0, 0, 0, 0, 1, 0, 0, 0]);
    let at = |i: usize| r.get(i).copied().unwrap_or(0);
    let cull = x::ps::Cull::of(at(1)).unwrap_or(x::ps::Cull::Back);
    let bits = cull.word() | u32::from(at(2) != 0) << 2 | u32::from(at(6) != 0) << 3 | u32::from(st.ps != 0) << 4;
    put(w, x::xd::RASTER, 0, bits);
    for (k, v) in x::ps::rect(&vp, view_size(d, st.rtvs[0]).or_else(|| view_size(d, st.dsv)), (at(7) != 0).then_some(st.scissor[0])).iter().enumerate() {
        put(w, x::xd::RECT, k, v.cast_unsigned());
    }
}

/// A draw from a store: its vertex shader, the stores it reads, the page topology (0 points, 1
/// lines, 3 triangles), vertices.
pub struct StoreDraw {
    pub vs: PageShader,
    pub over: Vec<[u32; 8]>,
    pub topo: u32,
    pub count: u32,
}

/// The vertex buffers the vertex pass reads: input slots (buffer, stride, offset, size) by xd
/// place, the distinct buffers by binding.
pub struct Vbs {
    pub slots: Vec<(u32, u32, u32, u32)>,
    pub bufs: Vec<u32>,
}

/// The vertex shader's inputs from the input layout and the vertex buffers (as `draw`'s
/// pipeline: by semantic).
fn attributes(d: &mut D11, st: &State, vs: &[u8]) -> Option<(Vec<Attr>, Vbs)> {
    let layout: Vec<(String, u32, u32, u32, u32, bool, u32)> = match d.objs.get(&st.layout).map(|o| &o.data) {
        Some(Data::Layout(l)) => l.iter().map(|e| (e.name.clone(), e.index, e.fmt, e.slot, e.offset, e.instance, e.step)).collect(),
        _ => Vec::new(),
    };
    let mut attrs = Vec::new();
    let mut vbs = Vbs { slots: Vec::new(), bufs: Vec::new() };
    let mut d3d_slots: Vec<u32> = Vec::new();
    for inp in dxwgsl::signature(vs, false) {
        if inp.sysval != 0 {
            continue;
        }
        let Some(e) = layout.iter().find(|e| e.0.eq_ignore_ascii_case(inp.name()) && e.1 == inp.index) else { continue };
        let (buf, stride, off) = st.vbs.get(us(e.3)).copied().unwrap_or((0, 0, 0));
        let slot = match d3d_slots.iter().position(|&s| s == e.3) {
            Some(i) => lo32(i),
            None => {
                d3d_slots.push(e.3);
                let size = match d.objs.get(&buf).map(|o| &o.data) {
                    Some(Data::Buffer(b)) => b.size,
                    _ => 0,
                };
                vbs.slots.push((buf, stride, off, size));
                lo32(d3d_slots.len().saturating_sub(1))
            }
        };
        let b = match vbs.bufs.iter().position(|&x| x == buf) {
            Some(i) => lo32(i),
            None => {
                vbs.bufs.push(buf);
                lo32(vbs.bufs.len().saturating_sub(1))
            }
        };
        let step = match (e.5, e.6) {
            (false, _) => 0,
            (true, 0) => u32::MAX,
            (true, r) => r,
        };
        attrs.push(Attr { reg: inp.reg, fmt: e.2, offset: e.4, slot, buf: b, step });
    }
    for &b in &vbs.bufs {
        pull(d, b);
    }
    // (a slot without a buffer reads zeros: its size 0)
    Some((attrs, vbs))
}

#[cfg(test)]
#[path = "expand_tests.rs"]
mod tests;
