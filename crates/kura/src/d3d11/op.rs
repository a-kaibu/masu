//! What a D3D11 context call does, as one value: [`ContextOp`], read once from the call's guest
//! arguments (its arrays and structures copied, its objects named by id) and applied to a
//! context's state. The immediate context applies it at once; a deferred context applies the
//! ones that set state to its own and keeps every one in the command list, which applies the
//! same values when it is executed (deferred.rs). One reading and one meaning for both: a
//! recorded call is not put back into guest memory and read again, and what a list holds is
//! what its operations name ([`ContextOp::objects`]), not a table of method numbers beside it.
//!
//! A stage is an index into the state's per-stage arrays: VS PS GS HS DS CS.

use kome::num::*;
use std::borrow::Cow;
use super::*;

/// The most a recorded call copies from one pointer (an UpdateSubresource of 256 MB)
pub(super) const MAX_BLOB: u64 = 1 << 28;

/// What OMSetRenderTargetsAndUnorderedAccessViews does to the output merger's UAVs.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Uavs {
    /// D3D11_KEEP_UNORDERED_ACCESS_VIEWS
    Keep,
    /// OMSetRenderTargets: none
    Clear,
    /// all unbound, then `views` from `start`, their hidden counters from `counts` (-1: kept)
    Set { start: u32, views: Vec<u32>, counts: Option<Vec<u32>> },
}

/// A D3D11 context call that changes state or the device's resources. `'a`: an update's bytes
/// as the call gives them (the guest's, applied at once); a recorded one owns its copy
/// ([`ContextOp::owned`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum ContextOp<'a> {
    /// *SetConstantBuffers(start, n, buffers) (`ranges` None) and *SetConstantBuffers1 (each
    /// slot's (first, count) constants)
    ConstantBuffers { stage: usize, start: u32, bufs: Vec<u32>, ranges: Option<Vec<(u32, u32)>> },
    ShaderResources { stage: usize, start: u32, views: Vec<u32> },
    Samplers { stage: usize, start: u32, samplers: Vec<u32> },
    /// *SetShader (its class instances: none are made, CreateClassLinkage gives no instances)
    Shader { stage: usize, shader: u32 },
    InputLayout(u32),
    /// (buffer, stride, offset) from `start`
    VertexBuffers { start: u32, vbs: Vec<(u32, u32, u32)> },
    IndexBuffer { buf: u32, fmt: u32, off: u32 },
    Topology(u32),
    Draw { indexed: bool, count: u32, instances: u32, first: u32, base: i32, first_instance: u32 },
    /// Draw(Indexed)InstancedIndirect(args, offset)
    DrawIndirect { indexed: bool, args: u32, off: u32 },
    DrawAuto,
    Dispatch([u32; 3]),
    DispatchIndirect { args: u32, off: u32 },
    /// Begin / End
    Query { query: u32, begin: bool },
    Predication { predicate: u32, value: u32 },
    /// OMSetRenderTargets (`uavs` Clear) and ...AndUnorderedAccessViews: the render targets and
    /// the depth view (None: D3D11_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL), the UAVs
    Targets { rtvs: Option<(Vec<u32>, u32)>, uavs: Uavs },
    /// CSSetUnorderedAccessViews(start, n, views, initial counts)
    ComputeUavs { start: u32, views: Vec<u32>, counts: Option<Vec<u32>> },
    Blend { state: u32, factor: [f32; 4], mask: u32 },
    DepthStencil { state: u32, stencil_ref: u32 },
    Raster(u32),
    /// SOSetTargets(n, buffers, offsets)
    StreamOutput { bufs: Vec<u32>, offsets: Option<Vec<u32>> },
    /// RSSetViewports (None: a NULL array, none set)
    Viewports(Option<Vec<[f32; 6]>>),
    Scissors(Option<Vec<[i32; 4]>>),
    /// CopySubresourceRegion(1)(dst, dst sub, x, y, z, src, src sub, box)
    CopyRegion { dst: u32, dsub: u32, at: [u32; 3], src: u32, ssub: u32, bx: Option<[u32; 6]> },
    CopyResource { dst: u32, src: u32 },
    /// UpdateSubresource(1)(resource, sub, box, data, row pitch, depth pitch): `data` the bytes
    /// it reads ([`update_bytes`])
    Update { res: u32, sub: u32, bx: Option<[u32; 6]>, data: Cow<'a, [u8]>, pitch: u32, slice: u32 },
    CopyStructureCount { dst: u32, at: u32, uav: u32 },
    ClearTarget { view: u32, color: [u32; 4] },
    /// ClearUnorderedAccessViewUint / ...Float
    ClearUav { view: u32, values: [u32; 4], float: bool },
    ClearDepth { view: u32, flags: u32, depth: u32, stencil: u32 },
    GenerateMips(u32),
    Resolve { dst: u32, dsub: u32, src: u32, ssub: u32, fmt: u32 },
    /// ClearView(view, color, rects, n)
    ClearView { view: u32, color: [u32; 4], rects: Vec<[i32; 4]> },
    /// ExecuteCommandList(list, restore)
    Execute { list: u32, restore: bool },
    ClearState,
    /// SetResourceMinLOD, Discard*, a ClearView of a NULL array with a count, an update whose
    /// bytes wrap the address space: what Windows does with them changes nothing here
    Nothing,
}

/// D3D11_KEEP_RENDER_TARGETS_AND_DEPTH_STENCIL, D3D11_KEEP_UNORDERED_ACCESS_VIEWS
const KEEP: u32 = 0xffff_ffff;

/// The most a slot array of a call is read for (the stages' largest, 128 resource views).
const SLOTS_MAX: u32 = 128;

/// A context call's arguments as the guest gave them, and what its pointers point to: read
/// through it by argument (`p`: the argument that is the pointer), each array at most
/// SLOTS_MAX long.
struct Guest<'a> {
    d: &'a D11,
    a: &'a dyn Fn(usize) -> u32,
}

impl Guest<'_> {
    /// # Safety
    /// `a` are a context method's arguments as the guest gave them: each pointer argument is
    /// NULL or points to what the method reads (the arrays as long as their counts).
    unsafe fn new<'a>(d: &'a D11, a: &'a dyn Fn(usize) -> u32) -> Guest<'a> {
        Guest { d, a }
    }
    fn arg(&self, i: usize) -> u32 {
        (self.a)(i)
    }
    /// The object argument `i` names (0: none).
    fn id(&self, i: usize) -> u32 {
        self.d.id_of(self.arg(i))
    }
    /// `n` objects of the array of pointers at argument `p` (NULL: `n` empty slots).
    fn ids(&self, p: usize, n: u32) -> Vec<u32> {
        let at = self.arg(p);
        (0..n.min(SLOTS_MAX))
            .map(|i| {
                if at == 0 {
                    return 0;
                }
                // SAFETY: Guest::new's contract: the array of `n` pointers at `at`
                let o = unsafe { rdp(at, us(i)) };
                self.d.id_of(o)
            })
            .collect()
    }
    /// `n` dwords of the array at argument `p` (None: NULL).
    fn words(&self, p: usize, n: u32) -> Option<Vec<u32>> {
        let at = self.arg(p);
        // SAFETY: Guest::new's contract: the array of `n` dwords at `at`
        (at != 0).then(|| (0..n.min(SLOTS_MAX)).map(|i| unsafe { rd(at, us(i)) }).collect())
    }
    /// `n` rows of `N` dwords at argument `p` (None: NULL): viewports, rectangles.
    fn rows<const N: usize>(&self, p: usize, n: u32) -> Option<Vec<[u32; N]>> {
        let at = self.arg(p);
        // SAFETY: Guest::new's contract: the array of `n` structures of `N` dwords at `at`
        (at != 0).then(|| (0..us(n)).map(|k| std::array::from_fn(|i| unsafe { rd(at, k.saturating_mul(N).saturating_add(i)) })).collect())
    }
    /// The `N` dwords at argument `p` (None: NULL): a color's four, a D3D11_BOX's six.
    fn fixed<const N: usize>(&self, p: usize) -> Option<[u32; N]> {
        let at = self.arg(p);
        // SAFETY: Guest::new's contract: the structure of `N` dwords at `at`
        (at != 0).then(|| std::array::from_fn(|i| unsafe { rd(at, i) }))
    }
    /// The `n` bytes at argument `p` (empty: NULL), as long as the guest's memory is (the call
    /// is applied before the guest runs again; [`ContextOp::owned`] copies them).
    fn bytes(&self, p: usize, n: u64) -> &'static [u8] {
        let at = self.arg(p);
        if at == 0 || n == 0 || u64::from(at).saturating_add(n) > 1 << 32 {
            return &[];
        }
        // SAFETY: Guest::new's contract: the `n` bytes the method reads at `at` (inside the
        // address space: checked above)
        unsafe { std::slice::from_raw_parts(gc::<u8>(at), us(lo32(n))) }
    }
}

/// The bytes UpdateSubresource reads of `res`'s subresource `sub` for `bx` with these pitches:
/// as update_subresource and upload_texture read them (a texture's box from its block's corner).
pub(super) fn update_bytes(d: &D11, res: u32, sub: u32, bx: Option<[u32; 6]>, pitch: u32, slice: u32) -> u64 {
    match d.objs.get(&res).map(|o| &o.data) {
        Some(Data::Buffer(b)) => u64::from(bx.map_or(b.size, |x| x[3].min(b.size).saturating_sub(x[0]))),
        Some(Data::Texture(t)) => {
            let level = sub.checked_rem(t.mips.max(1)).unwrap_or(0);
            let (lw, lh) = ((t.w >> level).max(1), (t.h >> level).max(1));
            let ld = if t.dim == 3 { (t.layers >> level).max(1) } else { 1 };
            let block = format::info(t.fmt).map_or(1, |f| f.block.max(1));
            let [x0, y0, z0, x1, y1, z1] = bx.unwrap_or([0, 0, 0, lw, lh, ld]);
            let on_block = |v: u32| v.checked_div(block).unwrap_or(0).saturating_mul(block);
            let (x0, y0) = (on_block(x0), on_block(y0));
            let (w, h, dp) = (x1.min(lw).saturating_sub(x0), y1.min(lh).saturating_sub(y0), z1.min(ld).saturating_sub(z0));
            if w == 0 || h == 0 || dp == 0 {
                return 0;
            }
            let (row, rows) = format::pitch_rows(t.fmt, w, h);
            // (in 64 bits: the pitches are the application's)
            u64::from(dp.saturating_sub(1)).saturating_mul(u64::from(slice)).saturating_add(u64::from(rows.saturating_sub(1)).saturating_mul(u64::from(pitch))).saturating_add(u64::from(row))
        }
        // (Data has many kinds; only buffers and textures take updates)
        Some(_) | None => 0,
    }
}

impl ContextOp<'_> {
    /// Context method `m` with arguments `a` (a(0) the context), read: None when it is not
    /// one that changes state or resources (a Get*, Map, a query's data, ...).
    ///
    /// # Safety
    /// `a` are the method's arguments as the guest gave them: its pointers point to what the
    /// method reads.
    pub(super) unsafe fn read(d: &D11, m: u32, a: &dyn Fn(usize) -> u32) -> Option<ContextOp<'static>> {
        // SAFETY: read's own contract
        let g = unsafe { Guest::new(d, a) };
        let (a, id) = (|i: usize| g.arg(i), |i: usize| g.id(i));
        Some(match m {
            7 | 16 | 22 | 62 | 66 | 71 => ContextOp::ConstantBuffers { stage: cb_stage(m), start: a(1), bufs: g.ids(3, a(2)), ranges: None },
            119..=124 => {
                let stage = CB1_STAGE.get(us(m.saturating_sub(119))).copied().unwrap_or(0);
                let ranges = match (g.words(4, a(2)), g.words(5, a(2))) {
                    (Some(f), Some(c)) => Some(f.into_iter().zip(c).collect()),
                    // (either array NULL: whole buffers, winref d3d11cb1)
                    (Some(_) | None, Some(_) | None) => None,
                };
                ContextOp::ConstantBuffers { stage, start: a(1), bufs: g.ids(3, a(2)), ranges }
            }
            8 | 25 | 31 | 59 | 63 | 67 => ContextOp::ShaderResources { stage: slot_stage(m), start: a(1), views: g.ids(3, a(2)) },
            10 | 26 | 32 | 61 | 65 | 70 => ContextOp::Samplers { stage: slot_stage(m), start: a(1), samplers: g.ids(3, a(2)) },
            9 | 11 | 23 | 60 | 64 | 69 => ContextOp::Shader { stage: shader_stage(m), shader: id(1) },
            17 => ContextOp::InputLayout(id(1)),
            18 => {
                let n = a(2).min(32);
                let (bufs, strides, offsets) = (g.ids(3, n), g.words(4, n), g.words(5, n));
                let at = |w: &Option<Vec<u32>>, i: usize| w.as_ref().and_then(|w| w.get(i).copied()).unwrap_or(0);
                ContextOp::VertexBuffers { start: a(1), vbs: bufs.iter().enumerate().map(|(i, &b)| (b, at(&strides, i), at(&offsets, i))).collect() }
            }
            19 => ContextOp::IndexBuffer { buf: id(1), fmt: a(2), off: a(3) },
            24 => ContextOp::Topology(a(1)),
            12 => ContextOp::Draw { indexed: true, count: a(1), instances: 1, first: a(2), base: a(3).cast_signed(), first_instance: 0 },
            13 => ContextOp::Draw { indexed: false, count: a(1), instances: 1, first: a(2), base: 0, first_instance: 0 },
            20 => ContextOp::Draw { indexed: true, count: a(1), instances: a(2), first: a(3), base: a(4).cast_signed(), first_instance: a(5) },
            21 => ContextOp::Draw { indexed: false, count: a(1), instances: a(2), first: a(3), base: 0, first_instance: a(4) },
            39 | 40 => ContextOp::DrawIndirect { indexed: m == 39, args: id(1), off: a(2) },
            38 => ContextOp::DrawAuto,
            41 => ContextOp::Dispatch([a(1), a(2), a(3)]),
            42 => ContextOp::DispatchIndirect { args: id(1), off: a(2) },
            27 | 28 => ContextOp::Query { query: id(1), begin: m == 27 },
            30 => ContextOp::Predication { predicate: id(1), value: a(2) },
            33 => ContextOp::Targets { rtvs: Some((g.ids(2, a(1).min(8)), id(3))), uavs: Uavs::Clear },
            34 => {
                let rtvs = (a(1) != KEEP).then(|| (g.ids(2, a(1).min(8)), id(3)));
                let uavs = if a(5) == KEEP {
                    Uavs::Keep
                } else {
                    let n = a(5).min(8);
                    Uavs::Set { start: a(4), views: g.ids(6, n), counts: if a(6) == 0 { None } else { g.words(7, n) } }
                };
                ContextOp::Targets { rtvs, uavs }
            }
            68 => ContextOp::ComputeUavs { start: a(1), views: g.ids(3, a(2)), counts: if a(3) == 0 { None } else { g.words(4, a(2)) } },
            35 => ContextOp::Blend { state: id(1), factor: g.fixed::<4>(2).map_or([1.0; 4], |f| f.map(f32::from_bits)), mask: a(3) },
            36 => ContextOp::DepthStencil { state: id(1), stencil_ref: a(2) },
            43 => ContextOp::Raster(id(1)),
            37 => {
                let n = a(1).min(4);
                ContextOp::StreamOutput { bufs: g.ids(2, n), offsets: g.words(3, n) }
            }
            44 => ContextOp::Viewports(g.rows::<6>(2, a(1).min(16)).map(|v| v.into_iter().map(|r| r.map(f32::from_bits)).collect())),
            45 => ContextOp::Scissors(g.rows::<4>(2, a(1).min(16)).map(|v| v.into_iter().map(|r| r.map(u32::cast_signed)).collect())),
            46 | 115 => ContextOp::CopyRegion { dst: id(1), dsub: a(2), at: [a(3), a(4), a(5)], src: id(6), ssub: a(7), bx: g.fixed::<6>(8) },
            47 => ContextOp::CopyResource { dst: id(1), src: id(2) },
            48 | 116 => {
                let (res, bx) = (id(1), g.fixed::<6>(3));
                let n = update_bytes(d, res, a(2), bx, a(5), a(6));
                // (bytes past the address space: nothing Windows would read)
                if u64::from(a(4)).saturating_add(n) > 1 << 32 {
                    return Some(ContextOp::Nothing);
                }
                ContextOp::Update { res, sub: a(2), bx, data: Cow::Borrowed(g.bytes(4, n)), pitch: a(5), slice: a(6) }
            }
            49 => ContextOp::CopyStructureCount { dst: id(1), at: a(2), uav: id(3) },
            50 => g.fixed::<4>(2).map_or(ContextOp::Nothing, |color| ContextOp::ClearTarget { view: id(1), color }),
            51 | 52 => g.fixed::<4>(2).map_or(ContextOp::Nothing, |values| ContextOp::ClearUav { view: id(1), values, float: m == 52 }),
            53 => ContextOp::ClearDepth { view: id(1), flags: a(2), depth: a(3), stencil: a(4) },
            54 => ContextOp::GenerateMips(id(1)),
            57 => ContextOp::Resolve { dst: id(1), dsub: a(2), src: id(3), ssub: a(4), fmt: a(5) },
            132 => {
                // (NULL rects with a count: nothing, as Windows does, winref d3d11clear)
                if a(3) == 0 && a(4) != 0 {
                    return Some(ContextOp::Nothing);
                }
                let Some(color) = g.fixed::<4>(2) else { return Some(ContextOp::Nothing) };
                let rects = g.rows::<4>(3, a(4).min(4096)).map_or_else(Vec::new, |v| v.into_iter().map(|r| r.map(u32::cast_signed)).collect());
                ContextOp::ClearView { view: id(1), color, rects }
            }
            58 => ContextOp::Execute { list: id(1), restore: a(2) != 0 },
            110 => ContextOp::ClearState,
            55 | 117 | 118 | 133 => ContextOp::Nothing,
            _ => return None,
        })
    }

    /// The same operation owning all it holds (to be kept in a command list); None when its
    /// bytes are more than a recorded call copies ([`MAX_BLOB`]).
    pub(super) fn owned(self) -> Option<ContextOp<'static>> {
        Some(match self {
            ContextOp::Update { res, sub, bx, data, pitch, slice } => {
                if lo64(data.len()) > MAX_BLOB {
                    return None;
                }
                ContextOp::Update { res, sub, bx, data: Cow::Owned(data.into_owned()), pitch, slice }
            }
            ContextOp::ConstantBuffers { stage, start, bufs, ranges } => ContextOp::ConstantBuffers { stage, start, bufs, ranges },
            ContextOp::ShaderResources { stage, start, views } => ContextOp::ShaderResources { stage, start, views },
            ContextOp::Samplers { stage, start, samplers } => ContextOp::Samplers { stage, start, samplers },
            ContextOp::Shader { stage, shader } => ContextOp::Shader { stage, shader },
            ContextOp::InputLayout(l) => ContextOp::InputLayout(l),
            ContextOp::VertexBuffers { start, vbs } => ContextOp::VertexBuffers { start, vbs },
            ContextOp::IndexBuffer { buf, fmt, off } => ContextOp::IndexBuffer { buf, fmt, off },
            ContextOp::Topology(t) => ContextOp::Topology(t),
            ContextOp::Draw { indexed, count, instances, first, base, first_instance } => ContextOp::Draw { indexed, count, instances, first, base, first_instance },
            ContextOp::DrawIndirect { indexed, args, off } => ContextOp::DrawIndirect { indexed, args, off },
            ContextOp::DrawAuto => ContextOp::DrawAuto,
            ContextOp::Dispatch(g) => ContextOp::Dispatch(g),
            ContextOp::DispatchIndirect { args, off } => ContextOp::DispatchIndirect { args, off },
            ContextOp::Query { query, begin } => ContextOp::Query { query, begin },
            ContextOp::Predication { predicate, value } => ContextOp::Predication { predicate, value },
            ContextOp::Targets { rtvs, uavs } => ContextOp::Targets { rtvs, uavs },
            ContextOp::ComputeUavs { start, views, counts } => ContextOp::ComputeUavs { start, views, counts },
            ContextOp::Blend { state, factor, mask } => ContextOp::Blend { state, factor, mask },
            ContextOp::DepthStencil { state, stencil_ref } => ContextOp::DepthStencil { state, stencil_ref },
            ContextOp::Raster(r) => ContextOp::Raster(r),
            ContextOp::StreamOutput { bufs, offsets } => ContextOp::StreamOutput { bufs, offsets },
            ContextOp::Viewports(v) => ContextOp::Viewports(v),
            ContextOp::Scissors(s) => ContextOp::Scissors(s),
            ContextOp::CopyRegion { dst, dsub, at, src, ssub, bx } => ContextOp::CopyRegion { dst, dsub, at, src, ssub, bx },
            ContextOp::CopyResource { dst, src } => ContextOp::CopyResource { dst, src },
            ContextOp::CopyStructureCount { dst, at, uav } => ContextOp::CopyStructureCount { dst, at, uav },
            ContextOp::ClearTarget { view, color } => ContextOp::ClearTarget { view, color },
            ContextOp::ClearUav { view, values, float } => ContextOp::ClearUav { view, values, float },
            ContextOp::ClearDepth { view, flags, depth, stencil } => ContextOp::ClearDepth { view, flags, depth, stencil },
            ContextOp::GenerateMips(v) => ContextOp::GenerateMips(v),
            ContextOp::Resolve { dst, dsub, src, ssub, fmt } => ContextOp::Resolve { dst, dsub, src, ssub, fmt },
            ContextOp::ClearView { view, color, rects } => ContextOp::ClearView { view, color, rects },
            ContextOp::Execute { list, restore } => ContextOp::Execute { list, restore },
            ContextOp::ClearState => ContextOp::ClearState,
            ContextOp::Nothing => ContextOp::Nothing,
        })
    }

    /// Whether it changes the context's state (a deferred context applies it to its own as
    /// well as recording it); the others only record there.
    pub(super) fn sets_state(&self) -> bool {
        match self {
            ContextOp::ConstantBuffers { .. }
            | ContextOp::ShaderResources { .. }
            | ContextOp::Samplers { .. }
            | ContextOp::Shader { .. }
            | ContextOp::InputLayout(_)
            | ContextOp::VertexBuffers { .. }
            | ContextOp::IndexBuffer { .. }
            | ContextOp::Topology(_)
            | ContextOp::Predication { .. }
            | ContextOp::Targets { .. }
            | ContextOp::ComputeUavs { .. }
            | ContextOp::Blend { .. }
            | ContextOp::DepthStencil { .. }
            | ContextOp::Raster(_)
            | ContextOp::StreamOutput { .. }
            | ContextOp::Viewports(_)
            | ContextOp::Scissors(_)
            | ContextOp::ClearState => true,
            ContextOp::Draw { .. }
            | ContextOp::DrawIndirect { .. }
            | ContextOp::DrawAuto
            | ContextOp::Dispatch(_)
            | ContextOp::DispatchIndirect { .. }
            | ContextOp::Query { .. }
            | ContextOp::CopyRegion { .. }
            | ContextOp::CopyResource { .. }
            | ContextOp::Update { .. }
            | ContextOp::CopyStructureCount { .. }
            | ContextOp::ClearTarget { .. }
            | ContextOp::ClearUav { .. }
            | ContextOp::ClearDepth { .. }
            | ContextOp::GenerateMips(_)
            | ContextOp::Resolve { .. }
            | ContextOp::ClearView { .. }
            | ContextOp::Execute { .. }
            | ContextOp::Nothing => false,
        }
    }

    /// Whether SetPredication's predicate turns it off (draws, dispatches, copies, updates,
    /// clears, resolves, GenerateMips).
    fn predicated(&self) -> bool {
        match self {
            ContextOp::Draw { .. }
            | ContextOp::DrawIndirect { .. }
            | ContextOp::DrawAuto
            | ContextOp::Dispatch(_)
            | ContextOp::DispatchIndirect { .. }
            | ContextOp::CopyRegion { .. }
            | ContextOp::CopyResource { .. }
            | ContextOp::Update { .. }
            | ContextOp::ClearTarget { .. }
            | ContextOp::ClearUav { .. }
            | ContextOp::ClearDepth { .. }
            | ContextOp::GenerateMips(_)
            | ContextOp::Resolve { .. }
            | ContextOp::ClearView { .. } => true,
            ContextOp::ConstantBuffers { .. }
            | ContextOp::ShaderResources { .. }
            | ContextOp::Samplers { .. }
            | ContextOp::Shader { .. }
            | ContextOp::InputLayout(_)
            | ContextOp::VertexBuffers { .. }
            | ContextOp::IndexBuffer { .. }
            | ContextOp::Topology(_)
            | ContextOp::Query { .. }
            | ContextOp::Predication { .. }
            | ContextOp::Targets { .. }
            | ContextOp::ComputeUavs { .. }
            | ContextOp::Blend { .. }
            | ContextOp::DepthStencil { .. }
            | ContextOp::Raster(_)
            | ContextOp::StreamOutput { .. }
            | ContextOp::Viewports(_)
            | ContextOp::Scissors(_)
            | ContextOp::CopyStructureCount { .. }
            | ContextOp::Execute { .. }
            | ContextOp::ClearState
            | ContextOp::Nothing => false,
        }
    }

    /// The objects it names (0s left out): what a command list holding it holds.
    pub(super) fn objects(&self) -> Vec<u32> {
        let mut v: Vec<u32> = match self {
            ContextOp::ConstantBuffers { bufs, .. } => bufs.clone(),
            ContextOp::ShaderResources { views, .. } => views.clone(),
            ContextOp::Samplers { samplers, .. } => samplers.clone(),
            ContextOp::Shader { shader, .. } => vec![*shader],
            ContextOp::InputLayout(o) | ContextOp::Raster(o) | ContextOp::GenerateMips(o) => vec![*o],
            ContextOp::VertexBuffers { vbs, .. } => vbs.iter().map(|v| v.0).collect(),
            ContextOp::IndexBuffer { buf, .. } => vec![*buf],
            ContextOp::DrawIndirect { args, .. } | ContextOp::DispatchIndirect { args, .. } => vec![*args],
            ContextOp::Query { query, .. } => vec![*query],
            ContextOp::Predication { predicate, .. } => vec![*predicate],
            ContextOp::Targets { rtvs, uavs } => {
                let mut v = rtvs.as_ref().map_or_else(Vec::new, |(r, dsv)| r.iter().copied().chain([*dsv]).collect());
                if let Uavs::Set { views, .. } = uavs {
                    v.extend(views);
                }
                v
            }
            ContextOp::ComputeUavs { views, .. } => views.clone(),
            ContextOp::Blend { state, .. } | ContextOp::DepthStencil { state, .. } => vec![*state],
            ContextOp::StreamOutput { bufs, .. } => bufs.clone(),
            ContextOp::CopyRegion { dst, src, .. } | ContextOp::CopyResource { dst, src } | ContextOp::Resolve { dst, src, .. } => vec![*dst, *src],
            ContextOp::Update { res, .. } => vec![*res],
            ContextOp::CopyStructureCount { dst, uav, .. } => vec![*dst, *uav],
            ContextOp::ClearTarget { view, .. } | ContextOp::ClearUav { view, .. } | ContextOp::ClearDepth { view, .. } | ContextOp::ClearView { view, .. } => vec![*view],
            ContextOp::Execute { list, .. } => vec![*list],
            ContextOp::Topology(_) | ContextOp::Draw { .. } | ContextOp::DrawAuto | ContextOp::Dispatch(_) | ContextOp::Viewports(_) | ContextOp::Scissors(_) | ContextOp::ClearState | ContextOp::Nothing => Vec::new(),
        };
        v.retain(|&o| o != 0);
        v
    }

    /// Applies it to `st` (the immediate context's state, a deferred context's, or a command
    /// list's as it runs).
    ///
    /// # Safety
    /// The objects it names are this device's; the guest memory of its buffers is theirs.
    pub(super) unsafe fn apply(&self, d: &mut D11, st: &mut State, after: &mut After) {
        if self.predicated() && predicated_off(d, st) {
            return;
        }
        // SAFETY: apply's own contract: the objects are this device's, their memory theirs
        unsafe { self.apply_to(d, st, after) }
    }

    /// [`ContextOp::apply`] past the predicate.
    ///
    /// # Safety
    /// As [`ContextOp::apply`].
    unsafe fn apply_to(&self, d: &mut D11, st: &mut State, after: &mut After) {
        match self {
            ContextOp::ConstantBuffers { stage, start, bufs, ranges } => set_cbs(d, st, *stage, *start, bufs, ranges.as_deref()),
            ContextOp::ShaderResources { stage, start, views } => {
                if let Some(slots) = st.srvs.get_mut(*stage) {
                    set_slots(d, slots, *start, views);
                }
                hazards::after_srvs(d, st, *stage, *start, lo32(views.len()));
            }
            ContextOp::Samplers { stage, start, samplers } => {
                if let Some(slots) = st.samplers.get_mut(*stage) {
                    set_slots(d, slots, *start, samplers);
                }
            }
            ContextOp::Shader { stage, shader } => {
                let slot = match stage {
                    0 => Some(&mut st.vs),
                    1 => Some(&mut st.ps),
                    s => st.shaders.get_mut(s.saturating_sub(2)),
                };
                if let Some(slot) = slot {
                    set_one(d, slot, *shader);
                }
            }
            ContextOp::InputLayout(l) => set_one(d, &mut st.layout, *l),
            ContextOp::VertexBuffers { start, vbs } => {
                for (i, &v) in vbs.iter().enumerate() {
                    let Some(slot) = st.vbs.get_mut(us(start.saturating_add(lo32(i)))) else { break };
                    let old = slot.0;
                    *slot = v;
                    d.rebind(v.0, old);
                }
                hazards::after_buffers(d, st);
            }
            ContextOp::IndexBuffer { buf, fmt, off } => {
                let old = st.ib.0;
                st.ib = (*buf, *fmt, *off);
                d.rebind(*buf, old);
            }
            ContextOp::Topology(t) => st.topology = *t,
            // SAFETY: draw's own contract: this context's state, of this device
            ContextOp::Draw { indexed, count, instances, first, base, first_instance } => unsafe { draw(d, st, *indexed, *count, *instances, *first, *base, *first_instance) },
            ContextOp::DrawIndirect { indexed, args, off } => {
                // (the counts from the buffer)
                let n: u32 = if *indexed { 5 } else { 4 };
                expand::pull(d, *args);
                if let Some(Data::Buffer(b)) = d.objs.get(args).map(|o| &o.data) && off.is_multiple_of(4) && u64::from(*off).saturating_add(u64::from(n) * 4) <= u64::from(b.size) {
                    let p = b.mem.saturating_add(*off);
                    let word = |i: usize| {
                        if i >= us(n) {
                            return 0;
                        }
                        // SAFETY: the buffer's guest memory, `n` words inside it (checked above)
                        unsafe { rd(p, i) }
                    };
                    let [c, inst, first, base, first_inst]: [u32; 5] = std::array::from_fn(word);
                    if *indexed {
                        // SAFETY: draw's own contract: this context's state, of this device
                        unsafe { draw(d, st, true, c, inst, first, base.cast_signed(), first_inst) };
                    } else {
                        // SAFETY: (the same)
                        unsafe { draw(d, st, false, c, inst, first, 0, base) };
                    }
                }
            }
            ContextOp::DrawAuto => {
                // the vertices stream output wrote to the buffer in input slot 0 (its fill, from
                // the offset it is bound at, by its stride)
                let (buf, stride, off) = st.vbs.first().copied().unwrap_or_default();
                if buf != 0 && let Some(fill) = expand::fill_of(d, buf) && let Some(n) = fill.saturating_sub(off).checked_div(stride) {
                    // SAFETY: draw's own contract: this context's state, of this device
                    unsafe { draw(d, st, false, n, 1, 0, 0, 0) };
                }
            }
            // SAFETY: dispatch's own contract: this context's state, of this device
            ContextOp::Dispatch([x, y, z]) => unsafe { dispatch(d, st, *x, *y, *z) },
            ContextOp::DispatchIndirect { args, off } => {
                expand::pull(d, *args);
                if let Some(Data::Buffer(b)) = d.objs.get(args).map(|o| &o.data) && u64::from(*off).saturating_add(12) <= u64::from(b.size) {
                    let p = b.mem.saturating_add(*off);
                    // SAFETY: the buffer's guest memory, three words inside it (checked above)
                    let [x, y, z]: [u32; 3] = std::array::from_fn(|i| unsafe { rd(p, i) });
                    // SAFETY: dispatch's own contract: this context's state, of this device
                    unsafe { dispatch(d, st, x, y, z) };
                }
            }
            ContextOp::Query { query, begin } => {
                let q = *query;
                let now = d.stats;
                let on_page = matches!(d.objs.get(&q).map(|o| &o.data), Some(Data::Query(x)) if x.counts_on_page());
                let counters = if on_page { expand::counters(d) } else { 0 };
                // (PIPELINE_STATISTICS counts the clipper's work on the page: the draws while one
                // is open are expanded, expand.rs)
                if matches!(d.objs.get(&q).map(|o| &o.data), Some(Data::Query(x)) if x.kind == query::Kind::PipelineStatistics) {
                    d.expand.open.retain(|&x| x != q);
                    if *begin {
                        d.expand.open.push(q);
                    }
                }
                if let Some(Data::Query(x)) = d.objs.get_mut(&q).map(|o| &mut o.data) {
                    if *begin { x.begin(&now, counters) } else { x.end(&now) }
                }
            }
            ContextOp::Predication { predicate, value } => {
                let mut p = st.predicate.0;
                set_one(d, &mut p, *predicate);
                st.predicate = (p, *value);
            }
            ContextOp::Targets { rtvs, uavs } => {
                if let Uavs::Set { start, views, counts } = uavs {
                    let mut u = st.uavs[0];
                    set_slots(d, &mut u, 0, &[0; 8]);
                    set_slots(d, &mut u, *start, views);
                    st.uavs[0] = u;
                    initial_counts(d, views, counts.as_deref());
                }
                if let Some((r, dsv)) = rtvs {
                    let mut rt = st.rtvs;
                    set_slots(d, &mut rt, 0, &[0; 8]);
                    set_slots(d, &mut rt, 0, r);
                    st.rtvs = rt;
                    set_one(d, &mut st.dsv, *dsv);
                    if *uavs == Uavs::Clear {
                        let mut u = st.uavs[0];
                        set_slots(d, &mut u, 0, &[0; 8]);
                        st.uavs[0] = u;
                    }
                    // the viewport follows the first target when the game never set one
                    if st.viewport[0][2] == 0.0 && let Some((w, h)) = view_size(d, st.rtvs[0]).or_else(|| view_size(d, st.dsv)) {
                        st.viewport[0] = [0.0, 0.0, to_f32(w), to_f32(h), 0.0, 1.0];
                    }
                }
                // (KEEP render targets with new UAVs is an output change too)
                hazards::after_outputs(d, st);
            }
            ContextOp::ComputeUavs { start, views, counts } => {
                set_slots(d, &mut st.uavs[1], *start, views);
                initial_counts(d, views, counts.as_deref());
                hazards::after_outputs(d, st);
            }
            ContextOp::Blend { state, factor, mask } => {
                set_one(d, &mut st.blend, *state);
                st.blend_factor = *factor;
                st.sample_mask = *mask;
            }
            ContextOp::DepthStencil { state, stencil_ref } => {
                set_one(d, &mut st.depth, *state);
                st.stencil_ref = *stencil_ref;
            }
            ContextOp::Raster(r) => set_one(d, &mut st.raster, *r),
            ContextOp::StreamOutput { bufs, offsets } => {
                // the buffers stream output writes (expand.rs), each from its offset (-1: on from
                // what it holds; no offsets: 0)
                let mut so = st.so;
                set_slots(d, &mut so, 0, &[0; 4]);
                set_slots(d, &mut so, 0, bufs);
                st.so = so;
                for (k, &buf) in so.iter().enumerate().take(bufs.len()) {
                    let off = offsets.as_ref().and_then(|o| o.get(k).copied()).unwrap_or(0);
                    if buf != 0 && off != u32::MAX {
                        expand::set_fill(d, buf, off);
                    }
                }
                hazards::after_outputs(d, st);
            }
            ContextOp::Viewports(v) => {
                st.nviewports = v.as_ref().map_or(0, |v| lo32(v.len()));
                for (slot, v) in st.viewport.iter_mut().zip(v.iter().flatten()) {
                    *slot = *v;
                }
                if d.trace && v.is_some() {
                    eprintln!("d3d11: viewport {:?}", st.viewport[0]);
                }
            }
            ContextOp::Scissors(s) => {
                st.nscissors = s.as_ref().map_or(0, |s| lo32(s.len()));
                for (slot, s) in st.scissor.iter_mut().zip(s.iter().flatten()) {
                    *slot = *s;
                }
            }
            // SAFETY: copy_region's own contract: resources of this device
            ContextOp::CopyRegion { dst, dsub, at, src, ssub, bx } => unsafe { copy_region(d, *dst, *dsub, *at, *src, *ssub, *bx) },
            // SAFETY: copy_resource's own contract: resources of this device
            ContextOp::CopyResource { dst, src } => unsafe { copy_resource(d, *dst, *src) },
            ContextOp::Update { res, sub, bx, data, pitch, slice } => {
                // (it reads no more than it has: a recorded one's box is its copy's)
                if lo64(data.len()) >= update_bytes(d, *res, *sub, *bx, *pitch, *slice) {
                    // SAFETY: update_subresource's own contract: `data` holds what it reads
                    unsafe { update_subresource(d, *res, *sub, *bx, data, *pitch, *slice) };
                }
            }
            // SAFETY: copy_structure_count's own contract: objects of this device
            ContextOp::CopyStructureCount { dst, at, uav } => unsafe { copy_structure_count(d, *dst, *at, *uav) },
            // SAFETY: clear_target's own contract: a view of this device
            ContextOp::ClearTarget { view, color } => unsafe { clear_target(d, *view, *color) },
            ContextOp::ClearUav { view, values, float } => {
                let vals = if *float { clear::Values::Float(values.map(f32::from_bits)) } else { clear::Values::Uint(*values) };
                if let Some(Data::BufferUav(u)) = d.objs.get(view).map(|o| &o.data) {
                    let (buf, fmt, offset, bytes) = (u.buf, u.view.fmt, u.view.offset, u.view.bytes);
                    // SAFETY: clear_buffer_view's own contract: the view's buffer, of this device
                    unsafe { clear_buffer_view(d, buf, fmt, offset, bytes, vals) };
                } else {
                    // SAFETY: clear_view_texels' own contract: a view of this device
                    unsafe { clear_view_texels(d, *view, vals, &[]) };
                }
            }
            ContextOp::ClearDepth { view, flags, depth, stencil } => {
                if let Some((tex, level, layer)) = view_target(d, *view) {
                    for l in layer..layer.saturating_add(view_layers(d, *view)) {
                        gpu::record(|q| q.cmd(&[gpu::CLEAR_VIEW, tex, level, l, (flags & 3) << 1, 0, 0, 0, 0, *depth, stencil & 0xff]));
                    }
                    d.targets = None;
                }
            }
            ContextOp::GenerateMips(view) => {
                if let Some(Data::View(v)) = d.objs.get(view).map(|o| &o.data) && let Some(Data::Texture(t)) = d.objs.get(&v.res).map(|o| &o.data) {
                    let g = t.gpu;
                    gpu::record(|q| q.cmd(&[gpu::GEN_MIPS, g]));
                }
            }
            // SAFETY: resolve's own contract: textures of this device
            ContextOp::Resolve { dst, dsub, src, ssub, fmt } => unsafe { resolve(d, *dst, *dsub, *src, *ssub, *fmt) },
            // ClearView: a render target or unordered access view, in rectangles of the view
            // (none: all of it), the color as a Float clear's, but truncated on an integer format
            // (D3D11.1 functional spec, ClearView; clear::Values::Color)
            ContextOp::ClearView { view, color, rects } => {
                if rects.is_empty() && matches!(d.objs.get(view).map(|o| o.iface), Some(IF_RTV)) {
                    // SAFETY: clear_target's own contract: a view of this device
                    unsafe { clear_target(d, *view, *color) };
                } else {
                    // SAFETY: clear_view_texels' own contract: a view of this device
                    unsafe { clear_view_texels(d, *view, clear::Values::Color(color.map(f32::from_bits)), rects) };
                }
            }
            // SAFETY: execute's own contract: a list of this device
            ContextOp::Execute { list, restore } => unsafe { deferred::execute(d, st, *list, *restore, after) },
            ContextOp::ClearState => {
                let old = std::mem::replace(st, default_state());
                unbind_state(d, &old);
            }
            ContextOp::Nothing => {}
        }
    }
}

/// The stage of *SetConstantBuffers `m`.
fn cb_stage(m: u32) -> usize {
    match m {
        7 => 0,
        16 => 1,
        _ => stage_of(m),
    }
}

/// The stage of *SetShaderResources and *SetSamplers `m`.
fn slot_stage(m: u32) -> usize {
    match m {
        8 | 10 => 1,
        25 | 26 => 0,
        _ => stage_of(m),
    }
}

/// The stage of *SetShader `m`.
fn shader_stage(m: u32) -> usize {
    match m {
        9 => 1,
        11 => 0,
        _ => stage_of(m),
    }
}

/// Sets slots from `start` of a slot array to `objs` (those past its end left out).
fn set_slots(d: &mut D11, slots: &mut [u32], start: u32, objs: &[u32]) {
    for (i, &new) in objs.iter().enumerate() {
        let Some(slot) = slots.get_mut(us(start.saturating_add(lo32(i)))) else { break };
        let old = *slot;
        *slot = new;
        d.rebind(new, old);
    }
}

/// Puts object `new` in `slot`.
fn set_one(d: &mut D11, slot: &mut u32, new: u32) {
    let old = *slot;
    *slot = new;
    d.rebind(new, old);
}

/// The hidden counters of `views` set from `counts` (None: kept; -1: that one kept), as
/// *SetUnorderedAccessViews take them.
fn initial_counts(d: &mut D11, views: &[u32], counts: Option<&[u32]>) {
    let Some(counts) = counts else { return };
    for (&v, &c) in views.iter().zip(counts) {
        if c == u32::MAX {
            continue;
        }
        if let Some(Data::BufferUav(u)) = d.objs.get_mut(&v).map(|o| &mut o.data) && u.counted() {
            u.counter = Count::Host(c);
        }
    }
}

/// *SetConstantBuffers(start, n, buffers) (`ranges` None) and *SetConstantBuffers1(start, n,
/// buffers, firsts, counts): each slot's buffer and range. A range is given in 16-byte
/// constants: the first a multiple of 16, the count a multiple of 16 up to 4096 (D3D11.1
/// functional spec, ConstantBufferOffsetting); either array NULL binds whole buffers (winref
/// d3d11cb1).
fn set_cbs(d: &mut D11, st: &mut State, stage: usize, start: u32, bufs: &[u32], ranges: Option<&[(u32, u32)]>) {
    let (Some(slots), Some(slot_ranges)) = (st.cbs.get_mut(stage), st.cb_ranges.get_mut(stage)) else { return };
    // a range not of whole 16-constant blocks, or of more than 4096 constants, makes the call do
    // nothing: the slots keep what they had (winref d3d11cb1 "first8-num16", "first16-num8",
    // "first16-num4112"; 0 constants is a range)
    if ranges.is_some_and(|r| r.iter().any(|&(first, count)| !cb_range_taken(first, count))) {
        return;
    }
    set_slots(d, slots, start, bufs);
    for i in 0..bufs.len() {
        let Some(slot) = slot_ranges.get_mut(us(start.saturating_add(lo32(i)))) else { break };
        *slot = match ranges {
            None => CbRange::WHOLE,
            Some(r) => r.get(i).map_or(CbRange::WHOLE, |&(first, count)| CbRange { first, count }),
        };
    }
}
