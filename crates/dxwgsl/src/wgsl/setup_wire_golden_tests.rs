//! FillMode WIREFRAME in the setup model against Windows' answers of winref d3d11wire (WARP):
//! which triangles are drawn under each CullMode and winding, each edge an aliased line with
//! its own depth bias, the pixel shader's SV_Position and the depth written, every pixel.
use std::collections::BTreeMap;

use super::*;

const GOLDEN: &str = include_str!("../../../../tests/winref/golden/d3d11wire-64/stdout.txt");

/// A pixel's record: SV_Position's words, the times the pixel shader ran, the depth buffer's word
type Px = ([u32; 4], u32, u32);

fn golden() -> BTreeMap<String, BTreeMap<(u32, u32), Px>> {
    let mut out: BTreeMap<String, BTreeMap<(u32, u32), Px>> = BTreeMap::new();
    let mut cur = String::new();
    for l in GOLDEN.lines() {
        if let Some(name) = l.strip_suffix(':').filter(|_| !l.starts_with(' ')) {
            cur = name.to_owned();
            out.entry(cur.clone()).or_default();
            continue;
        }
        let f: Vec<&str> = l.split_whitespace().collect();
        let [at, a, b, c, d, n, "depth", dw] = f[..] else { continue };
        let Some((x, y)) = at.trim_end_matches(':').split_once(',') else { continue };
        let hex = |v: &str| u32::from_str_radix(v, 16).unwrap_or(0);
        let (Ok(x), Ok(y), Ok(n)) = (x.parse(), y.parse(), n.trim_start_matches('x').parse()) else { continue };
        out.entry(cur.clone()).or_default().insert((x, y), ([hex(a), hex(b), hex(c), hex(d)], n, hex(dw)));
    }
    out
}

/// The probe's corners of each case (clip space, the vertex shader passes them on)
fn corners(name: &str) -> (Vec<[f32; 4]>, bool) {
    let list = |v: &[[f32; 4]]| (v.to_vec(), false);
    match name.split(',').next().unwrap_or("") {
        "cw left" => list(&[[-0.9, -0.7, 0.5, 1.0], [-0.5, 0.8, 0.5, 1.0], [-0.1, -0.7, 0.5, 1.0], [0.1, -0.7, 0.5, 1.0], [0.9, -0.7, 0.5, 1.0], [0.5, 0.8, 0.5, 1.0]]),
        "strip" => (vec![[-0.9, -0.7, 0.5, 1.0], [-0.6, 0.8, 0.5, 1.0], [0.2, -0.7, 0.5, 1.0], [0.6, 0.8, 0.5, 1.0]], true),
        "depth" => list(&[[-0.9, -0.8, 0.2, 1.0], [-0.2, 0.9, 0.4, 1.0], [0.9, -0.6, 0.7, 1.0]]),
        "clipped near" => list(&[[-0.8, -0.8, -0.5, 1.0], [0.0, 1.35, 0.75, 1.5], [0.9, -0.6, 0.9, 1.0]]),
        "clipped far" => list(&[[-0.8, -0.8, 0.3, 1.0], [0.0, 0.9, 1.6, 1.0], [0.9, -0.6, 0.6, 1.0]]),
        "on a line" => list(&[[-0.5, -0.5, 0.5, 1.0], [0.0, 0.0, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]]),
        "no area" => list(&[[-0.9, -0.6, 0.5, 1.0], [-0.4, 0.1, 0.5, 1.0], [0.1, 0.8, 0.5, 1.0], [0.5, 0.25, 0.5, 1.0], [0.5, 0.25, 0.5, 1.0], [0.5, 0.25, 0.5, 1.0]]),
        other => panic!("no corners for {other}"),
    }
}

/// The case's rasterizer state from its name: "<case>, wireframe cull <mode>[ ccw], bias <n>
/// clamp <f> slope <f>, <format>"
fn state(name: &str) -> (Raster, Bias) {
    let parts: Vec<&str> = name.split(", ").collect();
    let mode = parts.iter().find_map(|p| p.strip_prefix("wireframe cull ")).unwrap_or("none");
    let cull = match mode.trim_end_matches(" ccw") {
        "front" => Cull::Front,
        "back" => Cull::Back,
        _ => Cull::None,
    };
    let nums: Vec<&str> = parts.iter().find(|p| p.starts_with("bias ")).map_or_default(|p| p.split(' ').collect());
    let num = |k: &str| nums.iter().position(|w| *w == k).and_then(|i| nums.get(i + 1)).copied().unwrap_or("0");
    let unit = match parts.last().copied().unwrap_or("") {
        "D24_UNORM_S8_UINT" => DepthUnit::Unorm(24),
        "D16_UNORM" => DepthUnit::Unorm(16),
        _ => DepthUnit::Float,
    };
    let r = Raster { viewport: [0.0, 0.0, 16.0, 8.0, 0.0, 1.0], cull, front_ccw: mode.ends_with(" ccw"), depth_clip: true };
    (r, Bias { constant: num("bias").parse().unwrap_or(0), clamp: num("clamp").parse().unwrap_or(0.0), slope: num("slope").parse().unwrap_or(0.0), unit })
}

/// The depth buffer's word of a depth (clamped to the viewport's range); the probe's 0 with no
/// depth stencil view
fn depth_word(z: f32, unit: DepthUnit, view: bool) -> u32 {
    if !view {
        return 0;
    }
    let z = z.clamp(0.0, 1.0);
    match unit {
        DepthUnit::Float => z.to_bits(),
        DepthUnit::Unorm(n) => {
            // (the product in f32, as WARP converts: z · (2^n - 1) rounded to even)
            let max = ((1u32 << n) - 1) as f32;
            (z * max).round_ties_even() as u32
        }
    }
}

/// What the model draws of a case: each triangle's lines in order, the later record where two
/// light a pixel, the pixel shader's runs counted.
fn model(name: &str) -> BTreeMap<(u32, u32), Px> {
    let (p, strip) = corners(name);
    let (r, b) = state(name);
    let c: Vec<Corner> = p.iter().map(|&pos| Corner { pos, ..Corner::default() }).collect();
    let tris: Vec<[usize; 3]> = if strip {
        (0..c.len() - 2).map(|j| if j % 2 == 0 { [j, j + 1, j + 2] } else { [j, j + 2, j + 1] }).collect()
    } else {
        (0..c.len() / 3).map(|t| [3 * t, 3 * t + 1, 3 * t + 2]).collect()
    };
    let mut out: BTreeMap<(u32, u32), Px> = BTreeMap::new();
    for t in tris {
        for l in wire(&t.map(|k| c[k].clone()), &[], &r, &b) {
            for y in 0..8u32 {
                for x in 0..16u32 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    if !line_covers(&l, px, py) {
                        continue;
                    }
                    let z = line_at(l.z, px, py);
                    let w = 1.0 / line_at(l.rhw, px, py);
                    let e = out.entry((x, y)).or_insert(([0; 4], 0, 0));
                    *e = ([px.to_bits(), py.to_bits(), z.to_bits(), w.to_bits()], e.1 + 1, depth_word(z, b.unit, !name.ends_with(", no depth view")));
                }
            }
        }
    }
    out
}

#[test]
fn every_wireframe_pixel_as_warp_drew_it() {
    let g = golden();
    let mut bad = Vec::new();
    let mut cases = 0;
    for (name, want) in g.iter().filter(|(n, _)| n.contains(", wireframe cull ")) {
        cases += 1;
        let got = model(name);
        let (wk, gk): (Vec<_>, Vec<_>) = (want.keys().collect(), got.keys().collect());
        if wk != gk {
            bad.push(format!("{name}: covered {wk:?}\n    model {gk:?}"));
            continue;
        }
        if let Some((at, (w, m))) = want.iter().zip(got.values()).map(|((at, w), m)| (at, (w, m))).find(|(_, (w, m))| w != m) {
            let n = want.iter().zip(got.values()).filter(|((_, w), m)| w != m).count();
            bad.push(format!("{name}: {n} pixels differ, first {at:?}: {w:08x?} vs model {m:08x?}"));
        }
    }
    assert!(cases >= 40, "{cases} wireframe cases in the golden");
    assert!(bad.is_empty(), "{} of {cases} cases:\n{}", bad.len(), bad.join("\n"));
}
