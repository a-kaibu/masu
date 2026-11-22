//! The ops against their truth tables, the WGSL against the Rust rule (naga reads it), and the
//! translation of a pixel shader with a logic op.
use proptest::prelude::*;

use super::*;

/// D3D11_LOGIC_OP's table (d3d11_1.h): the result's bit for source bit s and destination bit d,
/// as 4 bits (s d) = 00, 01, 10, 11
const TABLE: [(Op, [u32; 4]); 16] = [
    (Op::Clear, [0, 0, 0, 0]),
    (Op::Set, [1, 1, 1, 1]),
    (Op::Copy, [0, 0, 1, 1]),
    (Op::CopyInverted, [1, 1, 0, 0]),
    (Op::Noop, [0, 1, 0, 1]),
    (Op::Invert, [1, 0, 1, 0]),
    (Op::And, [0, 0, 0, 1]),
    (Op::Nand, [1, 1, 1, 0]),
    (Op::Or, [0, 1, 1, 1]),
    (Op::Nor, [1, 0, 0, 0]),
    (Op::Xor, [0, 1, 1, 0]),
    (Op::Equiv, [1, 0, 0, 1]),
    (Op::AndReverse, [0, 0, 1, 0]),
    (Op::AndInverted, [0, 1, 0, 0]),
    (Op::OrReverse, [1, 0, 1, 1]),
    (Op::OrInverted, [1, 1, 0, 1]),
];

#[test]
fn ops_are_d3d11s_table() {
    for (i, (op, t)) in TABLE.iter().enumerate() {
        assert_eq!(Op::of(u32::try_from(i).unwrap()), Some(*op));
        for (k, want) in t.iter().enumerate() {
            let (s, d) = (u32::from(k & 2 != 0), u32::from(k & 1 != 0));
            assert_eq!(op.apply(s, d) & 1, *want, "{op:?} s {s} d {d}");
        }
        assert_eq!(op.reads_dst(), t[0] != t[1] || t[2] != t[3], "{op:?} reads d");
        assert_eq!(op.reads_src(), t[0] != t[2] || t[1] != t[3], "{op:?} reads s");
    }
    assert_eq!(Op::of(16), None);
}

fn target(store: Store, bits: [u32; 4], sign: Sign) -> Logic {
    Logic { target: 0, store, bits, sign, multisampled: false }
}

const WIDTHS: [u32; 7] = [0, 2, 8, 10, 16, 31, 32];

proptest! {
    /// bitwise: each bit of the result is the table's for that bit of s and d; the channel's
    /// width kept, the source's low bits (WARP's, winref d3d11logic "a source wider than the
    /// channels": rgba8_uint copy of 0x1ff is 0xff, of 0x100 is 0), signed or not
    #[test]
    fn result_is_bitwise_in_the_channel(op in 0u32..16, s in any::<[u32; 4]>(), d in any::<[u32; 4]>(), bits in prop::array::uniform4(prop::sample::select(WIDTHS.to_vec())), signed in any::<bool>()) {
        let op = Op::of(op).unwrap();
        let l = target(Store::Direct(op), bits, if signed { Sign::Signed } else { Sign::Unsigned });
        let r = result(&l, s, d);
        for c in 0..4 {
            let max = channel_max(bits[c]);
            let (sv, dv) = (s[c] & max, d[c] & max);
            prop_assert_eq!(r[c] & !max, 0);
            for b in 0..bits[c].min(32) {
                let t = TABLE[op as usize].1[usize::try_from((sv >> b & 1) << 1 | (dv >> b & 1)).unwrap()];
                prop_assert_eq!(r[c] >> b & 1, t);
            }
        }
    }

    /// without a logic op: UINT saturates to 0..max, SINT to its range (the value kept when
    /// it fits, the nearest end otherwise), whatever the destination
    #[test]
    fn plain_saturates(s in any::<[u32; 4]>(), d in any::<[u32; 4]>(), bits in prop::array::uniform4(prop::sample::select(WIDTHS.to_vec())), signed in any::<bool>()) {
        let sign = if signed { Sign::Signed } else { Sign::Unsigned };
        let r = result(&target(Store::Plain, bits, sign), s, d);
        for c in 0..4 {
            let max = channel_max(bits[c]);
            prop_assert_eq!(r[c] & !max, 0);
            if bits[c] == 0 {
                prop_assert_eq!(r[c], 0);
            } else if signed {
                let (lo, hi) = (-(i64::from(max >> 1)) - 1, i64::from(max >> 1));
                let v = i64::from(s[c].cast_signed());
                // the channel's bits read back as a signed value of its width
                let back = { let x = i64::from(r[c]); if x > hi { x - i64::from(max) - 1 } else { x } };
                prop_assert_eq!(back, v.clamp(lo, hi));
            } else {
                prop_assert_eq!(r[c], s[c].min(max));
            }
        }
    }
}

/// WARP's answers (winref d3d11logic "a source wider than the channels"): the copy and xor of
/// the low bits, the plain write saturated
#[test]
fn wide_sources_as_warp() {
    let wide = [0x1234_5601, 0x1ff, 0x100, 0xffff_ffff];
    let pack8 = |c: [u32; 4]| c[0] | c[1] << 8 | c[2] << 16 | c[3] << 24;
    let rgba8 = [8, 8, 8, 8];
    let dst8 = [0x3c, 0x5a, 0xa5, 0xc3];
    assert_eq!(pack8(result(&target(Store::Plain, rgba8, Sign::Unsigned), wide, dst8)), 0xffff_ffff);
    assert_eq!(pack8(result(&target(Store::Direct(Op::Copy), rgba8, Sign::Unsigned), wide, dst8)), 0xff00_ff01);
    assert_eq!(pack8(result(&target(Store::Direct(Op::Xor), rgba8, Sign::Unsigned), wide, dst8)), 0x3ca5_a53d);
    let pack10 = |c: [u32; 4]| c[0] | c[1] << 10 | c[2] << 20 | c[3] << 30;
    let rgb10a2 = [10, 10, 10, 2];
    let d10 = [0xf0 | 0x2 << 8, 0x37, 0x1ab, 2];
    assert_eq!(pack10(result(&target(Store::Plain, rgb10a2, Sign::Unsigned), wide, d10)), 0xd007_ffff);
    assert_eq!(pack10(result(&target(Store::Direct(Op::Copy), rgb10a2, Sign::Unsigned), wide, d10)), 0xd007_fe01);
}

/// every store's WGSL parses: the resolve pass of a stash, and the helpers' expressions
#[test]
fn store_wgsl_parses() {
    for sign in [Sign::Unsigned, Sign::Signed] {
        for bits in [[8, 8, 8, 8], [16, 16, 0, 0], [32, 0, 0, 0], [10, 10, 10, 2]] {
            for op in Op::ALL {
                let l = Logic { target: 3, store: Store::Stash { op, mark: 4 }, bits, sign, multisampled: true };
                let src = resolve_wgsl(&l).unwrap();
                naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{op:?} {sign:?} {bits:?}: {e}\n{src}"));
            }
            let l = target(Store::Plain, bits, sign);
            let k = l.scalar();
            let src = format!("fn f(o: vec4<u32>, v: vec4<{k}>) -> vec4<{k}> {{ let b = {}; let c = {}; return {}; }}\n", l.bits_in_wgsl("v"), l.saturate_wgsl("o"), l.bits_out_wgsl("b"));
            naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{sign:?} {bits:?}: {e}\n{src}"));
        }
    }
    assert!(resolve_wgsl(&target(Store::Plain, [8; 4], Sign::Unsigned)).is_none());
}

/// every op's WGSL is valid and names what it reads
#[test]
fn wgsl_of_every_op() {
    for op in Op::ALL {
        let e = op.wgsl("s", "d");
        let src = format!("fn f(s: vec4<u32>, d: vec4<u32>) -> vec4<u32> {{ return {e}; }}\n");
        naga::front::wgsl::parse_str(&src).unwrap_or_else(|err| panic!("{op:?}: {err}"));
        assert_eq!(e.contains('s'), op.reads_src(), "{op:?}: {e}");
        assert_eq!(e.contains('d'), op.reads_dst(), "{op:?}: {e}");
    }
}
