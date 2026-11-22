//! Direct3D 11.1 logic operations on the page (dxwgsl::logic has the shader's part): which
//! targets of a draw have one, and the draw cut into its primitives, each drawn after a copy of
//! those targets, so a primitive reads what the ones before it in the draw wrote (Direct3D's
//! output merger applies each primitive's pixels in order; the page's shader reads its
//! target's copy, not the target).

use dxwgsl::logic::{Logic, Op};

use super::format as f;
use super::{gpu, target_texture, Data, State, D11};
use crate::guest::gc;
use kome::num::lo32;

/// The bits of each channel of a page format with unsigned integer channels (the formats
/// Direct3D 11.1 applies logic ops to); None for the rest.
pub fn uint_bits(page: u32) -> Option<[u32; 4]> {
    Some(match page {
        f::P_RGBA8UI => [8, 8, 8, 8],
        f::P_RG8UI => [8, 8, 0, 0],
        f::P_R8UI => [8, 0, 0, 0],
        f::P_RGBA16UI => [16, 16, 16, 16],
        f::P_RG16UI => [16, 16, 0, 0],
        f::P_R16UI => [16, 0, 0, 0],
        f::P_RGBA32UI => [32, 32, 32, 32],
        f::P_RG32UI => [32, 32, 0, 0],
        f::P_R32UI => [32, 0, 0, 0],
        f::P_RGB10A2UI => [10, 10, 10, 2],
        _ => return None,
    })
}

/// What the blend state (kura's words: AlphaToCoverage, IndependentBlend, 8 targets of 8
/// words, then (LogicOpEnable, LogicOp) per target) asks of targets of these page formats
/// (multisampled or not): the logic ops the shader applies, and the targets left as they are
/// (NOOP: their write mask 0).
pub fn targets(blend: &[u32], formats: &[u32], multisampled: &[bool]) -> (Vec<Logic>, Vec<u32>) {
    let independent = blend.get(1).copied().unwrap_or(0) != 0;
    let (mut ops, mut kept) = (Vec::new(), Vec::new());
    for (i, &page) in formats.iter().enumerate() {
        let rt = if independent { i } else { 0 };
        let at = |k: usize| blend.get(66usize.saturating_add(rt.saturating_mul(2)).saturating_add(k)).copied().unwrap_or(0);
        let (Some(bits), Some(op)) = (uint_bits(page), Op::of(at(1))) else { continue };
        if at(0) == 0 {
            continue;
        }
        let target = u32::try_from(i).unwrap_or(u32::MAX);
        match op {
            Op::Noop => kept.push(target),
            Op::Clear | Op::Set | Op::Copy | Op::CopyInverted | Op::Invert | Op::And | Op::Nand | Op::Or | Op::Nor | Op::Xor | Op::Equiv | Op::AndReverse | Op::AndInverted | Op::OrReverse | Op::OrInverted => {
                ops.push(Logic { target, op, bits, multisampled: multisampled.get(i).copied().unwrap_or(false) });
            }
        }
    }
    (ops, kept)
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

/// The copy for target view `rtv` with logic op `l` (its scratch texture made the first time:
/// the level's size and format, one level and layer).
pub fn copy_of(d: &mut D11, rtv: u32, l: &Logic) -> Option<Copy> {
    let (tex, level, layer, ms, w, h, page) = target_texture(d, rtv)?;
    let scratch = *d.logic_scratch.entry((w, h, page, ms)).or_insert_with(|| {
        let id = super::page_id();
        gpu::record(|q| q.cmd(&[gpu::CREATE_TEXTURE2, id, w, h, 1, 1, page, if ms { 0x200 } else { 0 }]));
        id
    });
    let b = dxwgsl::logic::LOGIC_DST.saturating_add(l.target);
    // (a texture entry: an unsigned integer one (4), 2D or multisampled 2D, level 0 of 1, layer 0)
    let entry = [b, 4, scratch, if ms { 6 } else { 1 }, 0, 1, 0, 1];
    let cmd = [gpu::COPY, tex, level | layer << 16, 0, 0, w, h, scratch, 0, 0, 0, w, h, 0];
    Some(Copy { cmd, entry })
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
/// [`primitives`]), each after `copies`; a strip's odd triangle with pipeline `flipped`.
pub fn split(w: &[u32], copies: &[Copy], topo: u32, cuts: &[bool], flipped: u32) {
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
            }
        }
    });
}

#[cfg(test)]
#[path = "logic_tests.rs"]
mod tests;
