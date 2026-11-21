//! ID3D11DeviceContext::SetResourceMinLOD / GetResourceMinLOD through the real entry point:
//! Windows' device keeps no clamp, so Get gives 0 after any sequence of Sets, on 2D textures,
//! 3D textures and buffers alike (winref d3d11minlod).

use super::query_tests::{cc, dc, device, made, mem, put, rd1, release_all};
use super::*;
use crate::d3d9::trace_tests::SERIAL;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]
    #[test]
    fn min_lod_reads_zero_whatever_was_set(sets in prop::collection::vec((0usize..3, any::<u32>()), 0..12)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (dev, ctx) = device();
        let tex2 = {
            let d = mem(11);
            put(d, &[8, 8, 4, 1, 28, 1, 0, 0, 8, 0, 0]);
            let o = mem(2);
            assert_eq!(dc(dev, 5, &[d, 0, o]), 0);
            made(rd1(o))
        };
        let tex3 = {
            let d = mem(10);
            put(d, &[4, 4, 4, 1, 28, 0, 8, 0, 0, 0]);
            let o = mem(2);
            assert_eq!(dc(dev, 6, &[d, 0, o]), 0);
            made(rd1(o))
        };
        let buf = {
            let d = mem(6);
            put(d, &[64, 0, 8, 0, 0, 0]);
            let o = mem(2);
            assert_eq!(dc(dev, 3, &[d, 0, o]), 0);
            made(rd1(o))
        };
        let r = [tex2, tex3, buf];
        prop_assert_eq!(r.map(|x| cc(ctx, 56, &[x])), [0; 3]);
        for (k, bits) in sets {
            cc(ctx, 55, &[r[k], bits]);
            prop_assert_eq!(r.map(|x| cc(ctx, 56, &[x])), [0; 3]);
        }
        release_all();
    }
}
