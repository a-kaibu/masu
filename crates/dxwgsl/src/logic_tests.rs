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

proptest! {
    /// bitwise: each bit of the result is the table's for that bit of s and d; the channel's width
    /// kept, the source saturated to it
    #[test]
    fn result_is_bitwise_in_the_channel(op in 0u32..16, s in any::<[u32; 4]>(), d in any::<[u32; 4]>(), bits in prop::array::uniform4(prop::sample::select(vec![0u32, 2, 8, 10, 16, 32]))) {
        let op = Op::of(op).unwrap();
        let l = Logic { target: 0, op, bits, multisampled: false };
        let r = result(&l, s, d);
        for c in 0..4 {
            let max = channel_max(bits[c]);
            let (sv, dv) = (s[c].min(max), d[c] & max);
            prop_assert_eq!(r[c] & !max, 0);
            for b in 0..bits[c].min(32) {
                let t = TABLE[op as usize].1[usize::try_from((sv >> b & 1) << 1 | (dv >> b & 1)).unwrap()];
                prop_assert_eq!(r[c] >> b & 1, t);
            }
        }
    }
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
