//! Direct3D 11.1 logic operations and integer targets' stores on the page (dxwgsl::logic has
//! the shader's part and WARP's rules): which targets of a draw are integer ones and how the
//! shader writes them, and the draw cut into its primitives, each drawn after a copy of the
//! targets its op reads, so a primitive reads what the ones before it in the draw wrote
//! (Direct3D's output merger applies each primitive's pixels in order; the page's shader
//! reads its target's copy, not the target).
//!
//! A multisampled target with an op that reads it, under a pixel-rate shader: the shader runs
//! once a pixel, as Direct3D's does (winref d3d11logic: SV_Position, interpolation, SV_Coverage
//! and the shader's own UAV writes those of a draw without the op), writing the source's bits
//! to a stash of the target's format and 1 to a mark of the samples it covers; a resolve pass
//! (gpu::LOGIC_RESOLVE, dxwgsl::logic::resolve_wgsl) then gives each marked sample the op over
//! its own bits.

use dxwgsl::logic::{Logic, Op, Sign, Store};

use super::format as f;
use super::{gpu, target_texture, Data, State, D11};
use crate::guest::gc;
use kome::num::lo32;

/// The bits of each channel and the sign of a page format with integer channels (the formats
/// Direct3D 11.1 applies logic ops to, and whose plain writes saturate); None for the rest.
pub fn int_bits(page: u32) -> Option<([u32; 4], Sign)> {
    let u = Sign::Unsigned;
    let s = Sign::Signed;
    Some(match page {
        f::P_RGBA8UI => ([8, 8, 8, 8], u),
        f::P_RG8UI => ([8, 8, 0, 0], u),
        f::P_R8UI => ([8, 0, 0, 0], u),
        f::P_RGBA16UI => ([16, 16, 16, 16], u),
        f::P_RG16UI => ([16, 16, 0, 0], u),
        f::P_R16UI => ([16, 0, 0, 0], u),
        f::P_RGBA32UI => ([32, 32, 32, 32], u),
        f::P_RG32UI => ([32, 32, 0, 0], u),
        f::P_R32UI => ([32, 0, 0, 0], u),
        f::P_RGB10A2UI => ([10, 10, 10, 2], u),
        f::P_RGBA8SI => ([8, 8, 8, 8], s),
        f::P_RG8SI => ([8, 8, 0, 0], s),
        f::P_R8SI => ([8, 0, 0, 0], s),
        f::P_RGBA16SI => ([16, 16, 16, 16], s),
        f::P_RG16SI => ([16, 16, 0, 0], s),
        f::P_R16SI => ([16, 0, 0, 0], s),
        f::P_RGBA32SI => ([32, 32, 32, 32], s),
        f::P_RG32SI => ([32, 32, 0, 0], s),
        f::P_R32SI => ([32, 0, 0, 0], s),
        _ => return None,
    })
}

/// What the blend state (kura's words: AlphaToCoverage, IndependentBlend, 8 targets of 8
/// words, then (LogicOpEnable, LogicOp) per target) asks of targets of these page formats
/// (multisampled or not) under a pixel shader of sample rate or not: how the shader writes
/// each integer target, and the targets left as they are (NOOP: their write mask 0). A
/// multisampled target with an op that reads it under a pixel-rate shader is a stash, its
/// mark at the first location no target takes (`formats.len()`).
pub fn targets(blend: &[u32], formats: &[u32], multisampled: &[bool], sample_rate: bool) -> (Vec<Logic>, Vec<u32>) {
    let independent = blend.get(1).copied().unwrap_or(0) != 0;
    let mark = lo32(formats.len());
    let (mut out, mut kept) = (Vec::new(), Vec::new());
    for (i, &page) in formats.iter().enumerate() {
        let Some((bits, sign)) = int_bits(page) else { continue };
        let rt = if independent { i } else { 0 };
        let at = |k: usize| blend.get(66usize.saturating_add(rt.saturating_mul(2)).saturating_add(k)).copied().unwrap_or(0);
        let target = lo32(i);
        let ms = multisampled.get(i).copied().unwrap_or(false);
        let store = match Op::of(at(1)).filter(|_| at(0) != 0) {
            None => Store::Plain,
            Some(Op::Noop) => {
                kept.push(target);
                continue;
            }
            Some(op) if ms && !sample_rate && op.reads_dst() => Store::Stash { op, mark },
            Some(op) => Store::Direct(op),
        };
        out.push(Logic { target, store, bits, sign, multisampled: ms });
    }
    (out, kept)
}

/// One primitive of a draw as a draw of its own: its first vertex (from the draw's start), its
/// vertices, and whether it is a strip's odd triangle (wound the other way when drawn alone:
/// the page flips its front face for it).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Prim {
    pub first: u32,
    pub count: u32,
    pub odd: bool,
}

/// The primitives of a draw of `count` vertices of page topology `topo` (0 points, 1 lines, 2
/// line strip, 3 triangles, 4 triangle strip); `cut[k]`: vertex k is a strip's restart index
/// (an indexed strip; a primitive with one is none, and the triangle after it is even again).
pub fn primitives(topo: u32, count: u32, cut: &[bool]) -> Vec<Prim> {
    let cut_at = |k: u32| cut.get(usize::try_from(k).unwrap_or(usize::MAX)).copied().unwrap_or(false);
    let list = |n: u32| (0..count.checked_div(n).unwrap_or(0)).map(|i| Prim { first: i.saturating_mul(n), count: n, odd: false }).collect();
    let strip = |n: u32| {
        let mut out = Vec::new();
        // (the primitive's place since the last restart: a triangle strip's parity)
        let mut since = 0u32;
        for k in 0..count.saturating_sub(n.saturating_sub(1)) {
            if (k..k.saturating_add(n)).any(cut_at) {
                if cut_at(k) {
                    since = 0;
                }
                continue;
            }
            out.push(Prim { first: k, count: n, odd: n == 3 && since % 2 == 1 });
            since = since.saturating_add(1);
        }
        out
    };
    match topo {
        0 => list(1),
        1 => list(2),
        2 => strip(2),
        4 => strip(3),
        _ => list(3),
    }
}

/// A target's copy for its logic op: the page's COPY of the target into the scratch texture,
/// and the pixel shader's LogicDst entry of it.
#[derive(Clone, Copy, Debug)]
pub struct Copy {
    pub cmd: [u32; 14],
    pub entry: [u32; 8],
}

/// A page texture of the draw's own (made the first time, kept by what it is for: the target's
/// slot, the level's size, the format, multisampled): one level and layer.
fn scratch(d: &mut D11, key: (u32, u32, u32, u32, bool)) -> u32 {
    let (_, w, h, page, ms) = key;
    *d.logic_scratch.entry(key).or_insert_with(|| {
        let id = super::page_id();
        gpu::record(|q| q.cmd(&[gpu::CREATE_TEXTURE2, id, w, h, 1, 1, page, if ms { 0x200 } else { 0 }]));
        id
    })
}

/// A LogicDst binding `b`'s texture entry with no texture: an unsigned (4) or signed (5)
/// integer one, 2D or multisampled 2D, level 0 of 1, layer 0.
pub fn unbound(b: u32, multisampled: bool, sign: Sign) -> [u32; 8] {
    let kind = match sign {
        Sign::Unsigned => 4,
        Sign::Signed => 5,
    };
    [b, kind, 0, if multisampled { 6 } else { 1 }, 0, 1, 0, 1]
}

/// The kinds of the draw's own textures (the first word of `logic_scratch`'s key).
const COPY_OF: u32 = 0;
const STASH_OF: u32 = 0x100;
const MARK: u32 = 0x200;

/// The copy for target view `rtv` with logic op `l` (its scratch texture by the target's slot:
/// two targets of one size and format have a copy each).
pub fn copy_of(d: &mut D11, rtv: u32, l: &Logic) -> Option<Copy> {
    let (tex, level, layer, ms, w, h, page) = target_texture(d, rtv)?;
    let copy = scratch(d, (COPY_OF | l.target, w, h, page, ms));
    let b = dxwgsl::logic::LOGIC_DST.saturating_add(l.target);
    let mut entry = unbound(b, ms, l.sign);
    entry[2] = copy;
    let cmd = [gpu::COPY, tex, level | layer << 16, 0, 0, w, h, copy, 0, 0, 0, w, h, 0];
    Some(Copy { cmd, entry })
}

/// A stash target's two passes (crate doc): the targets the shader's pass draws to (the stash
/// in the target's place, the mark after the last), the mark's clear before it, the resolve
/// after it.
#[derive(Clone, Debug)]
pub struct Stash {
    pub targets: Vec<u32>,
    pub clear: [u32; 11],
    pub resolves: Vec<[u32; 12]>,
}

/// The stash passes of a draw to targets `colors` (SET_TARGETS2's (tex level layer) words)
/// and depth `depth` ((tex level layer)), its stash targets `stashes` (multisampled ones) with
/// their write masks `masks` and copies `copies`.
pub fn stash_of(d: &mut D11, rtvs: &[u32], colors: &[u32], depth: [u32; 3], stashes: &[&Logic], masks: &[u32], copies: &[Copy]) -> Option<Stash> {
    let first = stashes.first()?;
    let (_, _, _, _, w, h, _) = target_texture(d, rtvs.get(usize::try_from(first.target).ok()?).copied()?)?;
    let mark = scratch(d, (MARK, w, h, f::P_R8UI, true));
    let mut targets = colors.to_vec();
    let mut resolves = Vec::new();
    for l in stashes {
        let Store::Stash { op, .. } = l.store else { continue };
        let rtv = rtvs.get(usize::try_from(l.target).ok()?).copied()?;
        let (tex, _, _, _, tw, th, page) = target_texture(d, rtv)?;
        let stash = scratch(d, (STASH_OF | l.target, tw, th, page, true));
        let at = usize::try_from(l.target).ok()?.checked_mul(3)?;
        targets.get_mut(at..at.checked_add(3)?)?.copy_from_slice(&[stash, 0, 0]);
        let copy = copies.iter().find(|c| c.entry[0] == dxwgsl::logic::LOGIC_DST.saturating_add(l.target)).map_or(0, |c| c.entry[2]);
        let mask = masks.get(usize::try_from(l.target).ok()?).copied().unwrap_or(0xf);
        let sign = match l.sign {
            Sign::Unsigned => 0,
            Sign::Signed => 1,
        };
        let op_word = Op::ALL.iter().position(|&o| o == op).map_or(0, lo32);
        resolves.push([gpu::LOGIC_RESOLVE, tex, copy, stash, mark, mask, op_word, l.bits[0], l.bits[1], l.bits[2], l.bits[3], sign]);
    }
    targets.extend_from_slice(&[mark, 0, 0]);
    let mut set = vec![gpu::SET_TARGETS2, lo32(targets.len() / 3)];
    set.extend_from_slice(&targets);
    set.extend_from_slice(&depth);
    Some(Stash { targets: set, clear: [gpu::CLEAR_VIEW, mark, 0, 0, 1, 0, 0, 0, 0, 0, 0], resolves })
}

/// Which of the `count` indices from `start` of the bound index buffer are a strip's restart
/// index (0xffff, 0xffffffff by the index format); the buffer's GPU-written bytes read back
/// first. An index past the buffer: none.
///
/// # Safety
/// The index buffer's guest memory is mapped (draw's contract).
pub unsafe fn cuts(d: &mut D11, st: &State, start: u32, count: u32) -> Vec<bool> {
    super::expand::pull(d, st.ib.0);
    let Some(Data::Buffer(b)) = d.objs.get(&st.ib.0).map(|o| &o.data) else { return Vec::new() };
    let wide = st.ib.1 == 42;
    let size: u32 = if wide { 4 } else { 2 };
    (0..count)
        .map(|k| {
            let at = u64::from(st.ib.2).saturating_add(u64::from(start.saturating_add(k)).saturating_mul(u64::from(size)));
            if at.saturating_add(u64::from(size)) > u64::from(b.size) {
                return false;
            }
            let p = b.mem.saturating_add(lo32(at));
            if wide {
                // SAFETY: the caller's contract; `at` is inside the buffer
                unsafe { super::rd(p, 0) == u32::MAX }
            } else {
                // SAFETY: the caller's contract; `at` is inside the buffer
                unsafe { *gc::<u16>(p) == u16::MAX }
            }
        })
        .collect()
}

/// Records DRAW2 command `w` (its last words: indexed, count, instances, start, base, first
/// instance, the hash) as one draw per instance and primitive (`topo`, `cuts` as for
/// [`primitives`]), each after `copies` (and, with a stash, the mark's clear and the stash's
/// targets before it, the resolves after it); a strip's odd triangle with pipeline `flipped`.
pub fn split(w: &[u32], copies: &[Copy], stash: Option<&Stash>, topo: u32, cuts: &[bool], flipped: u32) {
    let n = w.len();
    let Some(tail) = w.get(n.saturating_sub(7)..) else { return };
    let [_, count, instances, start, _, first_instance, _] = <[u32; 7]>::try_from(tail).unwrap_or([0; 7]);
    let prims = primitives(topo, count, cuts);
    gpu::record(|q| {
        for k in 0..instances {
            for p in &prims {
                for c in copies {
                    q.cmd(&c.cmd);
                }
                if let Some(s) = stash {
                    q.cmd(&s.clear);
                    q.cmd(&s.targets);
                }
                let mut x = w.to_vec();
                let mut put = |i: usize, v: u32| {
                    if let Some(s) = x.get_mut(i) {
                        *s = v;
                    }
                };
                if p.odd {
                    put(1, flipped);
                }
                put(n.saturating_sub(6), p.count);
                put(n.saturating_sub(5), 1);
                put(n.saturating_sub(4), start.saturating_add(p.first));
                put(n.saturating_sub(2), first_instance.saturating_add(k));
                put(n.saturating_sub(1), 0);
                q.cmd(&x);
                if let Some(s) = stash {
                    for r in &s.resolves {
                        q.cmd(r);
                    }
                }
            }
        }
    });
}

#[cfg(test)]
#[path = "logic_tests.rs"]
mod tests;
