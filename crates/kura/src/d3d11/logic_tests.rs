//! Which targets get logic ops, and a draw's primitives as draws of their own.
use super::*;
use crate::d3d11::PageTopo;
use dxwgsl::logic::{Logic, Mark, Op, Sign, Store};

/// targets 0..2 of R8G8B8A8_UINT, R32_UINT, R8G8B8A8_UNORM; blend state words with logic ops
fn blend(independent: bool, ops: [(u32, u32); 3]) -> Vec<u32> {
    let mut b = vec![0, u32::from(independent)];
    for _ in 0..8 {
        b.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
    }
    for k in 0..8 {
        b.extend_from_slice(&ops.get(k).map_or([0, 4], |&(e, o)| [e, o]));
    }
    b
}

#[test]
fn targets_of_integer_formats_only() {
    let fmts = [f::P_RGBA8UI, f::P_R32UI, f::P_RGBA8];
    let (ops, kept) = targets(&blend(true, [(1, 10), (1, 4), (1, 10)]), &fmts, &[30, 42, 28], &[false, true, false], false);
    assert_eq!(ops, vec![Logic { target: 0, store: Store::Direct(Op::Xor), bits: [8, 8, 8, 8], sign: Sign::Unsigned, multisampled: false }]);
    assert_eq!(kept, vec![1]);
    // (without IndependentBlendEnable every target has target 0's; a multisampled one under a
    // pixel-rate shader a stash, its mark after the targets)
    let (ops, _) = targets(&blend(false, [(1, 6), (0, 0), (0, 0)]), &fmts, &[30, 42, 28], &[false, true, false], false);
    assert_eq!(ops.iter().map(|l| (l.target, l.store, l.multisampled)).collect::<Vec<_>>(), vec![(0, Store::Direct(Op::And), false), (1, Store::Stash { op: Op::And, mark: Mark::Slot(3) }, true)]);
    // (a shader of sample rate: each sample its own invocation, the op in the shader)
    let (ops, _) = targets(&blend(false, [(1, 6), (0, 0), (0, 0)]), &fmts, &[30, 42, 28], &[false, true, false], true);
    assert_eq!(ops.iter().map(|l| l.store).collect::<Vec<_>>(), vec![Store::Direct(Op::And), Store::Direct(Op::And)]);
    // (LogicOpEnable off: the integer targets written plainly, saturated)
    let (ops, kept) = targets(&blend(true, [(0, 10), (0, 10), (0, 10)]), &fmts, &[30, 42, 28], &[false; 3], false);
    assert_eq!(ops.iter().map(|l| (l.target, l.store)).collect::<Vec<_>>(), vec![(0, Store::Plain), (1, Store::Plain)]);
    assert!(kept.is_empty());
    // (SINT: the op too (winref d3d11logic), signed)
    let (ops, _) = targets(&blend(false, [(1, 8), (0, 0), (0, 0)]), &[f::P_RGBA8SI, f::P_R32SI], &[32, 43], &[false; 2], false);
    assert_eq!(ops.iter().map(|l| (l.store, l.sign, l.bits)).collect::<Vec<_>>(), vec![(Store::Direct(Op::Or), Sign::Signed, [8, 8, 8, 8]), (Store::Direct(Op::Or), Sign::Signed, [32, 0, 0, 0])]);
}

/// blend state words: target 0 blending (BlendEnable, SrcBlend, DestBlend, BlendOp,
/// SrcBlendAlpha, DestBlendAlpha, BlendOpAlpha) with a logic op too
fn blending(on: bool, w: [u32; 6]) -> Vec<u32> {
    let mut b = blend(false, [(1, 10), (0, 0), (0, 0)]);
    b[2] = u32::from(on);
    b[3..9].copy_from_slice(&w);
    b
}

#[test]
fn snorm_views_are_twins_with_their_blend() {
    use dxwgsl::snorm::{Blend, BlendOp, Equation, Factor};
    // (R8G8B8A8_SNORM 31, R8G8_SNORM 51, R8_SNORM 63; the texture's page format whatever it is:
    // a TYPELESS texture's view of SNORM is drawn as SNORM)
    for (view, page, bits) in [(31, f::P_RGBA8SN, [8, 8, 8, 8]), (51, f::P_RG8SN, [8, 8, 0, 0]), (63, f::P_R8SN, [8, 0, 0, 0]), (31, f::P_RGBA8, [8, 8, 8, 8])] {
        let (ops, kept) = targets(&blending(false, [2, 1, 1, 2, 1, 1]), &[page], &[view], &[false], false);
        assert_eq!(ops, vec![Logic { target: 0, store: Store::Snorm(None), bits, sign: Sign::Signed, multisampled: false }]);
        assert!(kept.is_empty());
    }
    let (ops, _) = targets(&blending(true, [5, 6, 1, 2, 7, 3]), &[f::P_RGBA8SN], &[31], &[false], false);
    let b = Blend { color: Equation { src: Factor::SrcAlpha, dst: Factor::InvSrcAlpha, op: BlendOp::Add }, alpha: Equation { src: Factor::One, dst: Factor::DestAlpha, op: BlendOp::RevSubtract } };
    assert_eq!(ops.iter().map(|l| l.store).collect::<Vec<_>>(), vec![Store::Snorm(Some(b))]);
    assert!(ops[0].reads_copy());
    // (UNORM views: not twins)
    let (ops, _) = targets(&blending(true, [5, 6, 1, 2, 7, 3]), &[f::P_RGBA8], &[28], &[false], false);
    assert!(ops.is_empty());
}

#[test]
fn eight_rgba16_targets_buffer_one_source() {
    let mut b = vec![0, 0];
    for _ in 0..8 {
        b.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
    }
    b.extend_from_slice(&[1, 10]);
    let (out, _) = targets(&b, &[f::P_RGBA16UI; 8], &[0; 8], &[true; 8], false);
    assert_eq!(out.len(), 8);
    for l in &out {
        assert_eq!(l.store, Store::Stash { op: Op::Xor, mark: Mark::Slot(0) });
        assert_eq!(l.buffered(), l.target == 0);
    }
}

proptest::proptest! {
    /// Every target's store follows WARP's rules: none for a format without integer channels,
    /// plain without the op, kept for NOOP, a stash only for a multisampled target with an op
    /// that reads it under a pixel-rate shader.
    #[test]
    fn stores_follow_warp(page in 0u32..64, op in 0u32..16, on: bool, ms: bool, rate: bool) {
        let b = blend(false, [(u32::from(on), op), (0, 0), (0, 0)]);
        let (ops, kept) = targets(&b, &[page], &[0], &[ms], rate);
        let op = Op::of(op);
        match (int_bits(page), op.filter(|_| on)) {
            (None, _) => proptest::prop_assert!(ops.is_empty() && kept.is_empty()),
            (Some(_), None) => proptest::prop_assert_eq!(ops.iter().map(|l| l.store).collect::<Vec<_>>(), vec![Store::Plain]),
            (Some(_), Some(Op::Noop)) => proptest::prop_assert!(ops.is_empty() && kept == vec![0]),
            (Some((bits, sign)), Some(o)) => {
                let store = if ms && !rate && o.reads_dst() { Store::Stash { op: o, mark: Mark::Slot(1) } } else { Store::Direct(o) };
                proptest::prop_assert_eq!(ops, vec![Logic { target: 0, store, bits, sign, multisampled: ms }]);
            }
        }
    }
}

#[test]
fn copies_are_by_slot() {
    // (two targets of one size and format: two scratch keys, not one; the first word of the key)
    for (a, b) in [(COPY_OF, STASH_OF), (STASH_OF, MARK), (COPY_OF, MARK)] {
        assert_eq!(a & 0xff, 0);
        assert_ne!(a, b);
    }
    assert_ne!(COPY_OF | 1, COPY_OF);
    assert_eq!(unbound(961, true, Sign::Signed), [961, 5, 0, 6, 0, 1, 0, 1]);
    assert_eq!(unbound(960, false, Sign::Unsigned), [960, 4, 0, 1, 0, 1, 0, 1]);
}

#[test]
fn primitives_of_each_topology() {
    let p = |first, count, odd| Prim { first, count, odd };
    assert_eq!(primitives(PageTopo::Points, 3, &[]), vec![p(0, 1, false), p(1, 1, false), p(2, 1, false)]);
    assert_eq!(primitives(PageTopo::Lines, 5, &[]), vec![p(0, 2, false), p(2, 2, false)]);
    assert_eq!(primitives(PageTopo::LineStrip, 3, &[]), vec![p(0, 2, false), p(1, 2, false)]);
    assert_eq!(primitives(PageTopo::Triangles, 7, &[]), vec![p(0, 3, false), p(3, 3, false)]);
    assert_eq!(primitives(PageTopo::TriangleStrip, 5, &[]), vec![p(0, 3, false), p(1, 3, true), p(2, 3, false)]);
    // (a restart at 2: triangles 3 4 5 and 4 5 6 after it, the first even again)
    let cut = [false, false, true, false, false, false, false];
    assert_eq!(primitives(PageTopo::TriangleStrip, 7, &cut), vec![p(3, 3, false), p(4, 3, true)]);
    let cut = [false, false, false, true, false, false, false];
    assert_eq!(primitives(PageTopo::TriangleStrip, 7, &cut), vec![p(0, 3, false), p(4, 3, false)]);
    assert!(primitives(PageTopo::TriangleStrip, 2, &[]).is_empty());
}

const INTS: [u32; 9] = [f::P_RGBA8UI, f::P_RG8UI, f::P_R8UI, f::P_RGBA16UI, f::P_RG16UI, f::P_RGB10A2UI, f::P_RGBA8SI, f::P_RGBA16UI, f::P_RGBA16SI];

proptest::proptest! {
    /// Eight targets: no location free, the mark carried by the first stash that can carry it,
    /// every stash pointing at that one, its stash format one the page multisamples; with no
    /// stash that can carry it, at the first stash's location, that one's source buffered.
    #[test]
    fn eight_targets_carry_the_mark(pages in proptest::collection::vec(0usize..9, 8), ops in proptest::collection::vec(0u32..16, 8)) {
        let mut b = vec![0, 1];
        for _ in 0..8 {
            b.extend_from_slice(&[0, 2, 1, 1, 2, 1, 1, 0xf]);
        }
        for o in &ops {
            b.extend_from_slice(&[1, *o]);
        }
        let fmts: Vec<u32> = pages.iter().map(|&p| INTS[p]).collect();
        let (out, _) = targets(&b, &fmts, &[0; 8], &[true; 8], false);
        let stashes: Vec<&Logic> = out.iter().filter(|l| matches!(l.store, Store::Stash { .. })).collect();
        let wanted: Vec<&Logic> = out.iter().filter(|l| Op::of(ops[usize::try_from(l.target).unwrap()]).is_some_and(Op::reads_dst)).collect();
        // (every op that reads its target a stash, the shader once a pixel)
        proptest::prop_assert_eq!(stashes.len(), wanted.len());
        let carrier = wanted.iter().find(|l| dxwgsl::logic::carry(l.bits).is_some());
        for l in &stashes {
            match (carrier, l.store) {
                (Some(c), Store::Stash { mark: Mark::Carried { by, .. }, .. }) => {
                    proptest::prop_assert_eq!(by, c.target);
                    let page = uint_page(l.stash_bits());
                    proptest::prop_assert!(page.is_some_and(crate::render::d3d::page_multisamples), "{:?}", l);
                }
                // (none can carry it: the first stash's location the mark's, its source buffered)
                (None, Store::Stash { mark: Mark::Slot(at), .. }) => {
                    proptest::prop_assert_eq!(at, stashes[0].target);
                    proptest::prop_assert_eq!(l.buffered(), l.target == at);
                    proptest::prop_assert!(uint_page(l.stash_bits()).is_some_and(crate::render::d3d::page_multisamples), "{:?}", l);
                }
                (_, store) => return Err(proptest::test_runner::TestCaseError::fail(format!("{store:?} with carrier {carrier:?}"))),
            }
        }
    }
}

/// CreateBlendState's description (D3D11_BLEND_DESC as kura reads it) takes a blend exactly
/// when dxwgsl's equations do: MIN / MAX with a dual-source factor refused (winref d3d11snorm),
/// whatever the factors, on color and on alpha
#[test]
fn min_max_refuse_dual_source_factors() {
    let colors = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 14, 15, 16, 17, 18, 19];
    let alphas = [1, 2, 5, 6, 7, 8, 11, 14, 15, 18, 19];
    for op in 1..=5u32 {
        for s in colors {
            for d in colors {
                let mut desc = blending(true, [s, d, op, 2, 1, 1]);
                desc.truncate(66);
                let took = super::super::normalize_state(20, &mut desc).is_some();
                let dual = [16, 17, 18, 19].contains(&s) || [16, 17, 18, 19].contains(&d);
                assert_eq!(took, op < 4 || !dual, "color {s} {d} op {op}");
            }
        }
        for s in alphas {
            for d in alphas {
                let mut desc = blending(true, [2, 1, 1, s, d, op]);
                desc.truncate(66);
                let took = super::super::normalize_state(20, &mut desc).is_some();
                let dual = [18, 19].contains(&s) || [18, 19].contains(&d);
                assert_eq!(took, op < 4 || !dual, "alpha {s} {d} op {op}");
            }
        }
    }
}

/// a multisampled SNORM target whose blend reads it is a stash under a pixel-rate shader (the
/// mark at its own location, every stash's), drawn directly under one of sample rate; a blend
/// that reads no target is the shader's store either way
#[test]
fn multisampled_snorm_blends_in_the_resolve_pass() {
    use dxwgsl::logic::Mark;
    let reads = blending(true, [5, 6, 1, 2, 7, 3]);
    let (ops, _) = targets(&reads, &[f::P_RGBA8SI], &[31], &[true], false);
    assert!(matches!(ops[0].store, Store::SnormStash { mark: Mark::Slot(0), .. }), "{ops:?}");
    assert!(ops[0].stashed() && ops[0].reads_copy());
    let (ops, _) = targets(&reads, &[f::P_RGBA8SI], &[31], &[true], true);
    assert!(matches!(ops[0].store, Store::Snorm(Some(_))), "{ops:?}");
    let (ops, _) = targets(&blending(true, [5, 1, 1, 2, 1, 1]), &[f::P_RGBA8SI], &[31], &[true], false);
    assert!(matches!(ops[0].store, Store::Snorm(Some(_))), "{ops:?}");
    // (an integer stash beside it: the SNORM stash's location the mark's for both)
    let mut b = blend(true, [(0, 0), (1, 6), (0, 0)]);
    b[2..10].copy_from_slice(&[1, 5, 6, 1, 2, 7, 3, 0xf]);
    let (ops, _) = targets(&b, &[f::P_RGBA8SI, f::P_RGBA8UI], &[31, 30], &[true, true], false);
    assert_eq!(ops.iter().map(|l| l.mark()).collect::<Vec<_>>(), vec![Some(Mark::Slot(0)), Some(Mark::Slot(0))]);
}

/// a multisampled 8-bit SNORM texture is its SINT twin on the page where the page multisamples
/// it (2D, one level and layer, no UAV), its own format otherwise
#[test]
fn multisampled_snorm_textures_are_twins() {
    assert_eq!(snorm_ms_page(31, 2, 1, 1, 4, 0x20), Some(f::P_RGBA8SI));
    assert_eq!(snorm_ms_page(51, 2, 1, 1, 8, 0x20), Some(f::P_RG8SI));
    assert_eq!(snorm_ms_page(63, 2, 1, 1, 2, 0x20), Some(f::P_R8SI));
    assert_eq!(snorm_ms_page(31, 2, 1, 1, 1, 0x20), None);
    assert_eq!(snorm_ms_page(31, 2, 2, 1, 4, 0x20), None);
    assert_eq!(snorm_ms_page(31, 2, 1, 2, 4, 0x20), None);
    assert_eq!(snorm_ms_page(31, 2, 1, 1, 4, 0xa0), None);
    assert_eq!(snorm_ms_page(28, 2, 1, 1, 4, 0x20), None);
}

/// a multisampled 16-bit SNORM texture is its SINT twin too; 16-bit SNORM views have twins of
/// 16-bit channels, drawn with the shader's 16-bit store
#[test]
fn sixteen_bit_snorm_has_twins() {
    assert_eq!(snorm_twin(13), Some((f::P_RGBA16SI, [16, 16, 16, 16])));
    assert_eq!(snorm_twin(37), Some((f::P_RG16SI, [16, 16, 0, 0])));
    assert_eq!(snorm_twin(58), Some((f::P_R16SI, [16, 0, 0, 0])));
    assert_eq!(snorm_ms_page(13, 2, 1, 1, 4, 0x20), Some(f::P_RGBA16SI));
    assert_eq!(snorm_ms_page(37, 2, 1, 1, 4, 0x20), Some(f::P_RG16SI));
    assert_eq!(snorm_ms_page(58, 2, 1, 1, 4, 0x20), Some(f::P_R16SI));
    let (ops, _) = targets(&blending(true, [5, 6, 1, 2, 7, 3]), &[f::P_RGBA32F], &[13], &[false], false);
    assert!(matches!(ops[0].store, Store::Snorm(Some(_))), "{ops:?}");
    assert_eq!(ops[0].snorm_width(), dxwgsl::snorm::Width::Bits16);
}

/// the twins' copies: bits between 8-bit pages (COPY's filter 2 single-sampled, CONVERT's bytes
/// multisampled), a 16-bit SNORM page's floats to and from its twin's values, nothing else
#[test]
fn twin_copies() {
    let c = |from, to, ms| twin_copy(1, 2 | 3 << 16, 4, 0, 8, 6, from, to, ms);
    assert_eq!(c(f::P_RGBA8SN, f::P_RGBA8SI, false), Some(vec![gpu::COPY, 1, 2 | 3 << 16, 0, 0, 8, 6, 4, 0, 0, 0, 8, 6, 2]));
    assert_eq!(c(f::P_RGBA8, f::P_RGBA8SI, false).map(|v| v[13]), Some(2));
    let conv = |k: gpu::Convert| Some(vec![gpu::CONVERT, 1, 2 | 3 << 16, 4, 0, 8, 6, k.word()]);
    assert_eq!(c(f::P_RGBA8, f::P_RGBA8SI, true), conv(gpu::Convert::Unorm8ToSint8));
    assert_eq!(c(f::P_RG8SI, f::P_RG8, true), conv(gpu::Convert::Sint8ToUnorm8));
    assert_eq!(c(f::P_RGBA32F, f::P_RGBA16SI, false), conv(gpu::Convert::Snorm16ToSint16));
    assert_eq!(c(f::P_R16SI, f::P_R32F, false), conv(gpu::Convert::Sint16ToSnorm16));
    assert_eq!(c(f::P_RGBA16F, f::P_RGBA16SI, false), None);
    assert_eq!(c(f::P_RGBA32F, f::P_RGBA16SI, true), None);
    assert_eq!(c(f::P_BGRA8, f::P_RGBA8SI, true), None);
}

proptest::proptest! {
    /// an SNORM texel's values: each channel's bytes as the signed value (8 or 16 bits), a
    /// channel the format has not 0
    #[test]
    fn snorm_values_of_texels(b in proptest::prelude::any::<[u8; 8]>()) {
        let s8 = |i: usize| i32::from(b[i].cast_signed());
        let s16 = |i: usize| i32::from(i16::from_le_bytes([b[i], b[i + 1]]));
        proptest::prop_assert_eq!(snorm_values([8, 8, 8, 8], &b), [s8(0), s8(1), s8(2), s8(3)]);
        proptest::prop_assert_eq!(snorm_values([8, 8, 0, 0], &b[..2]), [s8(0), s8(1), 0, 0]);
        proptest::prop_assert_eq!(snorm_values([16, 16, 16, 16], &b), [s16(0), s16(2), s16(4), s16(6)]);
        proptest::prop_assert_eq!(snorm_values([16, 0, 0, 0], &b[..2]), [s16(0), 0, 0, 0]);
    }
}
