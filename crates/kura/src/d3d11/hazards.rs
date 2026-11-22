//! What the Direct3D 11 runtime does with a resource bound for reading and writing at once
//! (formal/tla/Binds11.tla, winref probe d3d11bind): binding it for output (a render target, a
//! writable depth view, an unordered access view, a stream output target) unbinds the shader
//! resource views (every stage) and vertex buffers that read it; binding it for input while it
//! is bound for output binds NULL instead. Per subresource: views of other mips or array
//! slices of the same texture do not conflict. (On WebGPU a texture sampled and rendered in
//! the same pass is a validation error that loses the pass.)

use kome::num::*;
use super::{Data, State, View, D11};

/// What a view covers: (resource, first mip, mips, first slice, slices); a buffer view covers
/// its whole buffer.
fn span(d: &D11, view: u32) -> Option<(u32, u32, u32, u32, u32)> {
    match d.objs.get(&view).map(|o| &o.data) {
        Some(Data::View(View { buf: Some(b), .. })) => Some((b.buf, 0, u32::MAX, 0, u32::MAX)),
        Some(Data::BufferUav(u)) => Some((u.buf, 0, u32::MAX, 0, u32::MAX)),
        Some(Data::View(v)) => Some((v.res, v.mip, v.mips.max(1), v.layer, v.layers.max(1))),
        _ => None,
    }
}

fn overlap(a: (u32, u32, u32, u32, u32), b: (u32, u32, u32, u32, u32)) -> bool {
    let meet = |s0: u32, n0: u32, s1: u32, n1: u32| s0 < s1.saturating_add(n1) && s1 < s0.saturating_add(n0);
    a.0 == b.0 && meet(a.1, a.2, b.1, b.2) && meet(a.3, a.4, b.3, b.4)
}

/// What is bound for output: render targets, the depth view unless read-only for depth, the
/// output merger's and the compute shader's UAVs, the stream output targets (whole buffers).
fn outputs(d: &D11, st: &State) -> Vec<(u32, u32, u32, u32, u32)> {
    let mut v: Vec<u32> = st.rtvs.iter().chain(st.uavs[0].iter()).chain(st.uavs[1].iter()).copied().collect();
    let dsv_ro = match d.objs.get(&st.dsv).map(|o| &o.data) {
        Some(Data::View(x)) => x.desc.get(2).is_some_and(|f| f & 1 != 0),
        _ => false,
    };
    if !dsv_ro {
        v.push(st.dsv);
    }
    let mut out: Vec<_> = v.into_iter().filter(|&x| x != 0).filter_map(|x| span(d, x)).collect();
    out.extend(st.so.iter().filter(|&&b| b != 0).map(|&b| (b, 0, u32::MAX, 0, u32::MAX)));
    out
}

/// An input view that must not be bound now (it reads what is bound for output).
fn refused(d: &D11, st: &State, srv: u32) -> bool {
    span(d, srv).is_some_and(|s| outputs(d, st).into_iter().any(|o| overlap(o, s)))
}

/// After SRVs were set in `stage` from `start` (`n` slots): the ones that read an output are
/// NULL.
pub fn after_srvs(d: &mut D11, st: &mut State, stage: usize, start: u32, n: u32) {
    let Some(slots) = st.srvs.get(stage).map(|x| x.len()) else { return };
    for s in us(start)..us(start.saturating_add(n)).min(slots) {
        let v = st.srvs[stage][s];
        if v != 0 && refused(d, st, v) {
            st.srvs[stage][s] = 0;
            d.unbind(v);
        }
    }
}

/// After outputs were set (render targets, depth, either stage's UAVs, stream output targets,
/// each alone or together, KEEP or not): the SRVs of every stage and the vertex buffers that
/// read them go. Every setter of an output calls this, whichever of them it changed.
pub fn after_outputs(d: &mut D11, st: &mut State) {
    after_buffers(d, st);
    let outs = outputs(d, st);
    if outs.is_empty() {
        return;
    }
    for stage in 0..st.srvs.len() {
        for s in 0..st.srvs[stage].len() {
            let v = st.srvs[stage][s];
            if v != 0 && span(d, v).is_some_and(|x| outs.iter().any(|o| overlap(*o, x))) {
                st.srvs[stage][s] = 0;
                d.unbind(v);
            }
        }
    }
}

/// A vertex buffer (any of the 32 slots) of a buffer bound for output (a stream output target,
/// a buffer UAV or render target view): the vertex buffer goes (set first, it is unbound; set
/// after, it is refused).
pub fn after_buffers(d: &mut D11, st: &mut State) {
    let outs = outputs(d, st);
    for s in 0..st.vbs.len() {
        let vb = st.vbs[s].0;
        if vb != 0 && outs.iter().any(|o| o.0 == vb) {
            st.vbs[s].0 = 0;
            d.unbind(vb);
        }
    }
}
