use super::*;

fn raster(w: f32, h: f32) -> Raster {
    Raster { viewport: [0.0, 0.0, w, h, 0.0, 1.0], cull: Cull::None, front_ccw: false, depth_clip: true }
}

/// The corners of winref d3d11logic's vs_attr: (x, y) of a half-target triangle, its T =
/// x * 0.37 + y * 0.11 + 0.5 as the vertex shader computes it (two products and two adds).
fn attr_corners() -> [Corner; 3] {
    let p = [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0)];
    p.map(|(x, y)| {
        let t = x * 0.37 + y * 0.11 + 0.5;
        Corner { pos: [x, y, 0.5, 1.0], attrs: vec![t.to_bits(), 0, 0, 0], ..Corner::default() }
    })
}

/// Windows' T of every pixel the triangle covers in winref d3d11logic's "position, value,
/// coverage" rows (WARP, 4 x 4, pixel centres): (x, y, bits).
const LOGIC_T: [(u32, u32, u32); 10] = [
    (0, 0, 0x3e9c28f6),
    (0, 1, 0x3e800000),
    (1, 1, 0x3edeb852),
    (0, 2, 0x3e47ae14),
    (1, 2, 0x3ec28f5c),
    (2, 2, 0x3f10a3d7),
    (0, 3, 0x3e0f5c28),
    (1, 3, 0x3ea66666),
    (2, 3, 0x3f028f5c),
    (3, 3, 0x3f31eb86),
];

#[test]
fn perspective_input_as_warp_gives_it() {
    let fans = triangle(&attr_corners(), &[Mode::Perspective(At::Center)], &raster(4.0, 4.0));
    assert_eq!(fans.len(), 1);
    for (x, y, want) in LOGIC_T {
        let got = eval(&fans[0], 0, Mode::Perspective(At::Center), x as f32 + 0.5, y as f32 + 0.5);
        assert_eq!(got, want, "pixel {x},{y}: {got:08x}");
    }
}

/// The golden itself holds those values (the test reads the file, not a copy of it).
#[test]
fn logic_golden_holds_the_values() {
    let g = include_str!("../../../../tests/winref/golden/d3d11logic-64/stdout.txt");
    for (_, _, want) in LOGIC_T {
        assert!(g.contains(&format!("{want:08x}")), "{want:08x}");
    }
}

#[test]
fn exact_conversions_round_to_even() {
    for v in [0i128, 1, -1, (1 << 24) + 1, (1 << 24) + 3, (1 << 25) + 2, (1 << 25) + 6, i128::from(i64::MAX), -(1i128 << 40) - 12345, 0x7fff_ffff_ffff] {
        // (f64 is exact to 2^53; above, compare with the double rounding avoided by hand)
        if v.unsigned_abs() < 1 << 53 {
            assert_eq!(f32_of(v), v as f64 as f32, "{v}");
        }
    }
    assert_eq!(f32_of(i128::from(i64::MAX)), 9.223372e18);
}

#[test]
fn a_triangle_outside_one_plane_is_dropped_and_one_across_is_fanned() {
    let c = |x: f32, y: f32| Corner { pos: [x, y, 0.5, 1.0], attrs: vec![x.to_bits(); 4], ..Corner::default() };
    assert!(triangle(&[c(2.0, 0.0), c(3.0, 1.0), c(2.5, -1.0)], &[Mode::Perspective(At::Center)], &raster(8.0, 8.0)).is_empty());
    let fans = triangle(&[c(-1.7, -0.3), c(0.6, 1.9), c(1.4, -1.2)], &[Mode::Perspective(At::Center)], &raster(8.0, 8.0));
    assert!(fans.len() >= 3, "{}", fans.len());
    // (every corner drawn inside the clip volume)
    for f in &fans {
        for p in f.pos {
            assert!(p[0].abs() <= p[3] && p[1].abs() <= p[3], "{p:?}");
        }
    }
}
