//! ID3D11DeviceContext::SetResourceMinLOD / GetResourceMinLOD through the real entry point:
//! the clamp is the texture's own, given back bit for bit (winref d3d11minlod measures what
//! Windows gives back and what shaders read under it).

use super::query_tests::{cc, dc, device, made, mem, put, rd1, release_all};
use super::*;
use crate::d3d9::trace_tests::SERIAL;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    /// Each texture keeps the last value set on it, and only it.
    #[test]
    fn min_lod_is_each_textures_own(sets in prop::collection::vec((0usize..2, any::<u32>()), 0..12)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (dev, ctx) = device();
        let tex = || {
            let d = mem(11);
            put(d, &[8, 8, 4, 1, 28, 1, 0, 0, 8, 0, 0]);
            let o = mem(2);
            assert_eq!(dc(dev, 5, &[d, 0, o]), 0);
            made(rd1(o))
        };
        let t = [tex(), tex()];
        let mut want = [0u32; 2];
        for (k, bits) in sets {
            cc(ctx, 55, &[t[k], bits]);
            want[k] = bits;
            prop_assert_eq!([cc(ctx, 56, &[t[0]]), cc(ctx, 56, &[t[1]])], want);
        }
        release_all();
    }
}
