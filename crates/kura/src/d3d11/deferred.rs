//! Deferred contexts and command lists (CreateDeferredContext, FinishCommandList,
//! ExecuteCommandList), as Windows has them:
//!
//! - a deferred context has its own state, which its Set* calls change and its Get* calls
//!   read; drawing, clearing, copying and updating on it only record;
//! - a command list starts from the state the deferred context had when the list began (the
//!   default one, or what FinishCommandList(TRUE) kept), then replays its calls in order;
//! - ExecuteCommandList(list, restore) runs it on the immediate context from that starting state
//!   (nothing of the immediate context's state comes in), then gives back the immediate
//!   context's state (restore) or clears it; on a deferred context it is recorded, and the
//!   deferred context's state is cleared unless restore;
//! - Map on a deferred context: only WRITE_DISCARD of dynamic resources (then WRITE_NO_OVERWRITE
//!   of a buffer already discarded in the list); what is written is recorded at Unmap.
//!
//! A recorded call is the [`ContextOp`] the immediate context applies (op.rs): it owns copies
//! of what its pointers pointed to, and the list holds private references to the objects it
//! names ([`ContextOp::objects`]) until it goes.

use kome::num::*;
use crate::guest::gc;
use super::*;
use super::op::MAX_BLOB;

/// D3D11_ERROR_DEFERRED_CONTEXT_MAP_WITHOUT_INITIAL_DISCARD
const MAP_WITHOUT_DISCARD: u32 = 0x887c_0004;

pub struct CmdList {
    /// the state the list starts from (held privately)
    start: State,
    /// (shared: a list executed runs its calls where they are, without copying them)
    recs: std::sync::Arc<Vec<ContextOp<'static>>>,
    /// objects the calls name (held privately)
    held: Vec<u32>,
}

pub struct Deferred {
    st: State,
    list: CmdList,
    /// mapped here: (resource, subresource, scratch memory, size, row pitch, depth pitch)
    maps: Vec<(u32, u32, u32, u32, u32, u32)>,
}

/// The objects a state holds.
pub(super) fn state_objects(s: &State) -> Vec<u32> {
    let mut held: Vec<u32> = Vec::new();
    held.extend(s.vbs.iter().map(|v| v.0));
    held.extend([s.ib.0, s.layout, s.vs, s.ps, s.dsv, s.blend, s.depth, s.raster, s.predicate.0]);
    held.extend(s.shaders);
    held.extend(s.uavs.iter().flatten());
    held.extend(s.so);
    for k in 0..6 {
        held.extend(s.cbs[k]);
        held.extend(s.srvs[k]);
        held.extend(s.samplers[k]);
    }
    held.extend(s.rtvs);
    held.retain(|&h| h != 0);
    held
}

/// A copy of `s` holding its own references.
fn clone_state(d: &mut D11, s: &State) -> State {
    for h in state_objects(s) {
        d.bind(h);
    }
    s.clone()
}

fn new_list(d: &mut D11, start: &State) -> CmdList {
    CmdList { start: clone_state(d, start), recs: Default::default(), held: Vec::new() }
}

/// Lets go of what a list holds.
pub(super) fn drop_list(d: &mut D11, l: CmdList) {
    unbind_state(d, &l.start);
    for h in l.held {
        d.unbind(h);
    }
}

pub(super) fn drop_deferred(d: &mut D11, x: Deferred) {
    unbind_state(d, &x.st);
    drop_list(d, x.list);
    for m in x.maps {
        d.guest_free(m.2);
    }
}

/// CreateDeferredContext(flags, out) (and ...1, ...2)
pub(super) unsafe fn create(d: &mut D11, flags: u32, out: u32) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    if flags != 0 {
        wrp(out, 0, 0);
        return E_INVALIDARG;
    }
    let st = default_state();
    let list = new_list(d, &st);
    let o = d.new_obj(IF_CONTEXT, Data::Deferred(Box::new(Deferred { st, list, maps: Vec::new() })));
    // (a device child: it holds the device while it is held)
    let dev = d.device;
    d.add_ref(dev);
    wrp(out, 0, o);
    S_OK
}

/// A call onto the list being recorded (nobody else has it yet).
fn push(l: &mut CmdList, op: ContextOp<'static>) {
    if let Some(v) = std::sync::Arc::get_mut(&mut l.recs) {
        v.push(op);
    }
}

/// Records `op` (it owns its copies; the list holds what it names).
fn record(d: &mut D11, this: u32, op: ContextOp<'_>) {
    // (what changes nothing is not kept)
    if op == ContextOp::Nothing {
        return;
    }
    let Some(op) = op.owned() else {
        // (data past what any resource takes: not recorded)
        if d.trace {
            eprintln!("d3d11: deferred call with data too large to record, dropped");
        }
        return;
    };
    let mut ids = op.objects();
    ids.retain(|&i| i != this);
    for &i in &ids {
        d.bind(i);
    }
    if let Some(Data::Deferred(x)) = d.objs.get_mut(&this).map(|o| &mut o.data) {
        x.list.held.extend(ids);
        push(&mut x.list, op);
    }
}

/// Takes the deferred context's data out (to work on it with `d`), and puts it back.
fn with_deferred<R>(d: &mut D11, this: u32, f: impl FnOnce(&mut D11, &mut Deferred) -> R) -> Option<R> {
    let o = d.objs.get_mut(&this)?;
    let Data::Deferred(x) = std::mem::replace(&mut o.data, Data::Other) else {
        return None;
    };
    let mut x = x;
    let r = f(d, &mut x);
    if let Some(o) = d.objs.get_mut(&this) {
        o.data = Data::Deferred(x);
    }
    Some(r)
}

/// A method of a deferred context: what sets state applied to its own and recorded, drawing,
/// clearing, copying, updating, queries' Begin/End and ExecuteCommandList recorded only.
pub(super) unsafe fn call(d: &mut D11, this: u32, m: u32, a: &dyn Fn(usize) -> u32, after: &mut After) -> u32 {
    // SAFETY: call's own contract: the method's arguments as the guest gave them
    if let Some(op) = unsafe { ContextOp::read(d, m, a) } {
        match &op {
            // ExecuteCommandList(list, restore): of a list only; the state is cleared unless
            // restore
            ContextOp::Execute { list, restore } => {
                if !matches!(d.objs.get(list).map(|o| &o.data), Some(Data::CmdList(_))) {
                    return S_OK;
                }
                let restore = *restore;
                record(d, this, op);
                if !restore {
                    with_deferred(d, this, |d, x| {
                        let old = std::mem::replace(&mut x.st, default_state());
                        unbind_state(d, &old);
                    });
                }
            }
            ContextOp::ConstantBuffers { .. }
            | ContextOp::ShaderResources { .. }
            | ContextOp::Samplers { .. }
            | ContextOp::Shader { .. }
            | ContextOp::InputLayout(_)
            | ContextOp::VertexBuffers { .. }
            | ContextOp::IndexBuffer { .. }
            | ContextOp::Topology(_)
            | ContextOp::Draw { .. }
            | ContextOp::DrawIndirect { .. }
            | ContextOp::DrawAuto
            | ContextOp::Dispatch(_)
            | ContextOp::DispatchIndirect { .. }
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
            | ContextOp::ClearState
            | ContextOp::Nothing => {
                if op.sets_state() {
                    // SAFETY: the objects it names are this device's (read just now)
                    with_deferred(d, this, |d, x| unsafe { op.apply(d, &mut x.st, after) });
                }
                record(d, this, op);
            }
        }
        return S_OK;
    }
    match m {
        3 => {
            let dev = d.device;
            d.add_ref(dev);
            wrp(a(1), 0, d.addr_of(dev));
            S_OK
        }
        4..=6 => private_data(d, m, this, a),
        // reads of the state
        72..=109 | 125..=130 => with_deferred(d, this, |d, x| context_call2(d, &mut x.st, m, a, after)).unwrap_or(E_FAIL),
        14 => map(d, this, a(1), a(2), a(3), a(5)),
        15 => {
            unmap(d, this, a(1), a(2));
            S_OK
        }
        // queries' data is the immediate context's to give
        29 => DXGI_ERROR_INVALID_CALL,
        111 => S_OK,
        112 => 1, // D3D11_DEVICE_CONTEXT_DEFERRED
        113 => 0,
        114 => finish(d, this, a(1), a(2)),
        131 => {
            // SwapDeviceContextState: not on a deferred context
            if a(2) != 0 {
                wrp(a(2), 0, 0);
            }
            S_OK
        }
        // GetResourceMinLOD (Windows' device keeps no MinLOD: 0, as on the immediate context)
        56 => 0,
        134..=139 | 141..=143 => S_OK,
        140 => 0,
        // Flush1 / hardware protection on a deferred context: nothing to do
        144 | 145 => 0,
        146 => {
            if a(1) != 0 {
                wr(a(1), 0, 0);
            }
            0
        }
        147 | 148 => E_INVALIDARG,
        _ => E_NOTIMPL,
    }
}

/// FinishCommandList(restore, out): the list so far; the state kept (restore) or cleared, and
/// the next list starting from it.
unsafe fn finish(d: &mut D11, this: u32, restore: u32, out: u32) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    let list = with_deferred(d, this, |d, x| {
        // (what a map left open is not in it)
        for m in std::mem::take(&mut x.maps) {
            d.guest_free(m.2);
        }
        if restore == 0 {
            let old = std::mem::replace(&mut x.st, default_state());
            unbind_state(d, &old);
        }
        let next = new_list(d, &x.st);
        std::mem::replace(&mut x.list, next)
    });
    let Some(list) = list else { return E_FAIL };
    let o = d.new_obj(IF_CMDLIST, Data::CmdList(Box::new(list)));
    wrp(out, 0, o);
    S_OK
}

/// Runs list `id` on `st`: from the list's start state, its operations in order. The state `st`
/// had, given back (None: no list, `st` as it was).
pub(super) unsafe fn run(d: &mut D11, st: &mut State, id: u32, after: &mut After) -> Option<State> {
    let Some(Data::CmdList(l)) = d.objs.get(&id).map(|o| &o.data) else { return None };
    let (start, recs) = (l.start.clone(), l.recs.clone());
    // (the list is held while it runs: a call in it may let go of the last reference)
    d.bind(id);
    let start = clone_state(d, &start);
    let saved = std::mem::replace(st, start);
    for op in recs.iter() {
        // SAFETY: the objects it names are this device's, held by the list
        unsafe { op.apply(d, st, after) };
    }
    d.unbind(id);
    Some(saved)
}

/// ExecuteCommandList(list, restore) on the immediate context (`st` its state).
pub(super) unsafe fn execute(d: &mut D11, st: &mut State, id: u32, restore: bool, after: &mut After) {
    // SAFETY: execute's own contract
    let Some(saved) = (unsafe { run(d, st, id, after) }) else { return };
    if restore {
        let ran = std::mem::replace(st, saved);
        unbind_state(d, &ran);
    } else {
        let ran = std::mem::replace(st, default_state());
        unbind_state(d, &ran);
        unbind_state(d, &saved);
    }
}

/// Map(resource, sub, type, flags, out) on a deferred context.
unsafe fn map(d: &mut D11, this: u32, res: u32, sub: u32, ty: u32, out: u32) -> u32 {
    let id = d.id_of(res);
    let (usage, buffer, size, pitch, depth) = match d.objs.get(&id).map(|o| &o.data) {
        Some(Data::Buffer(b)) => (b.usage, true, b.size, b.size, b.size),
        Some(Data::Texture(t)) => {
            let level = sub % t.mips.max(1);
            let (lw, lh) = ((t.w >> level).max(1), (t.h >> level).max(1));
            let (pitch, rows) = format::pitch_rows(t.fmt, lw, lh);
            let ld = if t.dim == 3 { (t.layers >> level).max(1) } else { 1 };
            // (in 64 bits: a 2048^3 texture's level 0 does not fit in 32)
            let size = u64::from(pitch) * u64::from(rows) * u64::from(ld);
            if size > MAX_BLOB {
                if out != 0 {
                    wrp(out, 0, 0);
                }
                return E_OUTOFMEMORY;
            }
            (t.usage, false, lo32(size), pitch, pitch * rows)
        }
        _ => (0, false, 0, 0, 0),
    };
    let fail = |hr: u32| {
        if out != 0 {
            wrp(out, 0, 0);
        }
        hr
    };
    if usage != 2 || !(ty == 4 || ty == 5 && buffer) || out == 0 {
        return fail(E_INVALIDARG);
    }
    let r = with_deferred(d, this, |d, x| {
        let have = x.maps.iter().find(|m| m.0 == id && m.1 == sub).map(|m| m.2);
        match (ty, have) {
            (5, None) => Err(MAP_WITHOUT_DISCARD),
            (_, Some(mem)) => Ok(mem),
            (_, None) => {
                let mem = d.guest_alloc(size);
                if mem == 0 {
                    return Err(E_OUTOFMEMORY);
                }
                x.maps.push((id, sub, mem, size, pitch, depth));
                Ok(mem)
            }
        }
    });
    match r {
        Some(Ok(mem)) => {
            put_mapped(out, mem, pitch, depth, false);
            S_OK
        }
        Some(Err(hr)) => fail(hr),
        None => fail(E_FAIL),
    }
}

/// Unmap on a deferred context: what was written, recorded as an update of the subresource.
unsafe fn unmap(d: &mut D11, this: u32, res: u32, sub: u32) {
    let id = d.id_of(res);
    let Some(Some((mem, size, pitch, depth))) = with_deferred(d, this, |_, x| x.maps.iter().find(|m| m.0 == id && m.1 == sub).map(|m| (m.2, m.3, m.4, m.5))) else {
        // (not mapped here: nothing, as Windows does)
        return;
    };
    // SAFETY: the scratch memory map gave for it, `size` bytes
    let data = unsafe { std::slice::from_raw_parts(gc::<u8>(mem), us(size)) };
    record(d, this, ContextOp::Update { res: id, sub, bx: None, data: std::borrow::Cow::Borrowed(data), pitch, slice: depth });
}

/// (tests: the calls recorded so far on a deferred context, the bytes of updates they copied)
#[cfg(test)]
pub(super) fn recorded(d: &D11, this: u32) -> (usize, usize) {
    match d.objs.get(&this).map(|o| &o.data) {
        Some(Data::Deferred(x)) => (x.list.recs.len(), x.list.recs.iter().map(|op| if let ContextOp::Update { data, .. } = op { data.len() } else { 0 }).sum()),
        _ => (0, 0),
    }
}

/// (tests: a deferred context's own state)
#[cfg(test)]
pub(super) fn state_of(d: &D11, this: u32) -> Option<&State> {
    match d.objs.get(&this).map(|o| &o.data) {
        Some(Data::Deferred(x)) => Some(&x.st),
        _ => None,
    }
}

/// (tests: what a list holds: its start state's objects and what its operations name)
#[cfg(test)]
pub(super) fn list_objects(l: &CmdList) -> Vec<u32> {
    let mut v = state_objects(&l.start);
    v.extend(l.recs.iter().flat_map(ContextOp::objects));
    v
}
