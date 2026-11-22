//! The setup model against Windows' answers of winref d3d11interp (WARP): every record the
//! probe's pixel shaders wrote, each case's corners as the probe gives them.
use std::collections::BTreeMap;

use super::*;

const GOLDEN: &str = include_str!("../../../../tests/winref/golden/d3d11interp-64/stdout.txt");

type Rec = ((u32, u32, u32), [u32; 4], u32);

/// The golden's sections: name -> records.
fn golden() -> BTreeMap<String, Vec<Rec>> {
    let mut out: BTreeMap<String, Vec<Rec>> = BTreeMap::new();
    let mut cur = String::new();
    for l in GOLDEN.lines() {
        if let Some(name) = l.strip_suffix(':').filter(|_| !l.starts_with(' ')) {
            cur = name.to_owned();
            out.entry(cur.clone()).or_default();
            continue;
        }
        let Some((at, rest)) = l.trim().split_once(": ") else { continue };
        let Some((xy, s)) = at.split_once('[') else { continue };
        let Some((x, y)) = xy.split_once(',') else { continue };
        let w: Vec<u32> = rest.split(' ').filter_map(|v| u32::from_str_radix(v, 16).ok()).collect();
        let (Ok(x), Ok(y), Ok(s)) = (x.parse(), y.parse(), s.trim_end_matches(']').parse()) else { continue };
        if let [a, b, c, d, cov] = w[..] {
            out.entry(cur.clone()).or_default().push(((x, y, s), [a, b, c, d], cov));
        }
    }
    out
}

/// Whether a fan triangle covers the point (1/256 pixels), Direct3D's top-left rule.
fn covers(f: &Fan, px: i64, py: i64) -> bool {
    let mut v = f.scr.map(|(x, y)| (i64::from(x), i64::from(y)));
    let cross = (v[1].0 - v[0].0) * (v[2].1 - v[0].1) - (v[2].0 - v[0].0) * (v[1].1 - v[0].1);
    if cross == 0 {
        return false;
    }
    if cross < 0 {
        v.swap(1, 2);
    }
    (0..3).all(|i| {
        let (a, b) = (v[i], v[(i + 1) % 3]);
        let e = (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        e > 0 || (e == 0 && ((dy == 0 && dx > 0) || dy < 0))
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Ps {
    /// the attribute in this mode
    Attr(Mode),
    /// a.xyz and the uint k, nointerpolation
    Flat,
    /// SV_Position at the pixel (or sample / centroid as the mode's At)
    Pos(At),
    /// a.xyz linear and the clip distance
    Clip,
}

struct Case {
    corners: Vec<Corner>,
    tris: Vec<[usize; 3]>,
    ps: Ps,
    samples: u32,
    viewport: [f32; 6],
}

const OFFS: [(i64, i64); 4] = [(-2, -6), (6, -2), (-6, 2), (2, 6)];

fn modes(ps: Ps) -> Vec<Mode> {
    match ps {
        Ps::Attr(m) => vec![m],
        Ps::Flat => vec![Mode::Constant, Mode::Constant],
        Ps::Pos(_) => vec![Mode::Undefined],
        Ps::Clip => vec![Mode::Perspective(At::Center)],
    }
}

/// The records the model gives a case.
fn model(c: &Case) -> Vec<Rec> {
    let r = Raster { viewport: c.viewport, cull: Cull::None, front_ccw: false, depth_clip: true };
    let ms = modes(c.ps);
    let mut out: BTreeMap<(u32, u32, u32), ([u32; 4], u32)> = BTreeMap::new();
    for t in &c.tris {
        let tri = [c.corners[t[0]].clone(), c.corners[t[1]].clone(), c.corners[t[2]].clone()];
        for f in triangle(&tri, &ms, &r) {
            for y in 0..8u32 {
                for x in 0..8u32 {
                    let (cx, cy) = (i64::from(x) * 256 + 128, i64::from(y) * 256 + 128);
                    let pts: Vec<(i64, i64)> = if c.samples == 1 { vec![(cx, cy)] } else { OFFS.iter().map(|(dx, dy)| (cx + dx * 16, cy + dy * 16)).collect() };
                    let cov: u32 = pts.iter().enumerate().filter(|(_, p)| covers(&f, p.0, p.1)).map(|(i, _)| 1 << i).sum();
                    if cov == 0 {
                        continue;
                    }
                    let per_sample = matches!(c.ps, Ps::Attr(Mode::Perspective(At::Sample) | Mode::NoPerspective(At::Sample)) | Ps::Pos(At::Sample));
                    let at = |s: Option<usize>| -> (f32, f32) {
                        let (px, py) = match s {
                            Some(i) => pts[i],
                            None => (cx, cy),
                        };
                        (px as f32 / 256.0, py as f32 / 256.0)
                    };
                    let centroid = |cov: u32| -> Option<usize> { if cov == (1 << pts.len()) - 1 { None } else { (0..pts.len()).find(|i| cov >> i & 1 == 1) } };
                    let mut put = |s: u32, p: (f32, f32), cv: u32| {
                        let v = match c.ps {
                            Ps::Attr(m) => std::array::from_fn(|k| eval(&f, k, m, p.0, p.1)),
                            Ps::Flat => [eval(&f, 0, Mode::Constant, 0.0, 0.0), eval(&f, 1, Mode::Constant, 0.0, 0.0), eval(&f, 2, Mode::Constant, 0.0, 0.0), eval(&f, 4, Mode::Constant, 0.0, 0.0)],
                            Ps::Pos(_) => [p.0.to_bits(), p.1.to_bits(), f.z.at(p.0, p.1).to_bits(), (1.0 / f.rhw.at(p.0, p.1)).to_bits()],
                            Ps::Clip => [eval(&f, 0, Mode::Perspective(At::Center), p.0, p.1), eval(&f, 1, Mode::Perspective(At::Center), p.0, p.1), eval(&f, 2, Mode::Perspective(At::Center), p.0, p.1), eval(&f, 4, Mode::Undefined, p.0, p.1)],
                        };
                        let e = out.entry((x, y, s)).or_insert(([0; 4], 0));
                        e.0 = v;
                        e.1 = cv;
                    };
                    if per_sample {
                        for i in 0..pts.len() {
                            if cov >> i & 1 == 1 {
                                put(i as u32, at(Some(i)), cov);
                            }
                        }
                    } else {
                        let cen = matches!(c.ps, Ps::Attr(Mode::Perspective(At::Centroid) | Mode::NoPerspective(At::Centroid)) | Ps::Pos(At::Centroid));
                        let p = if cen { at(centroid(cov)) } else { at(None) };
                        put(0, p, cov);
                    }
                }
            }
        }
    }
    out.into_iter().map(|(k, (v, c))| (k, v, c)).collect()
}

fn corners(p: &[[f32; 4]], a: &[[f32; 4]], d: Option<&[f32]>) -> Vec<Corner> {
    p.iter()
        .zip(a)
        .enumerate()
        .map(|(i, (p, a))| {
            let k = u32::try_from(i).unwrap_or(0) * 0x101 + 7;
            let mut attrs: Vec<u32> = a.iter().map(|v| v.to_bits()).collect();
            attrs.extend([k, 0, 0, 0]);
            Corner { pos: *p, attrs, clip: d.map(|d| vec![d[i]]).unwrap_or_default(), cull: vec![] }
        })
        .collect()
}

const TP: [[f32; 4]; 3] = [[-0.9, -0.8, 0.3, 1.0], [0.4, 1.8, 0.2, 2.0], [0.4, -0.35, 0.3, 0.5]];
const TA: [[f32; 4]; 3] = [[0.1, 1.7, -3.25, 100.0], [0.73, -0.2, 7.5, 0.001], [0.31, 0.9, 0.0, -42.5]];
const FULL: [f32; 6] = [0.0, 0.0, 8.0, 8.0, 0.0, 1.0];

fn cases() -> Vec<(&'static str, Case)> {
    let lin = Ps::Attr(Mode::Perspective(At::Center));
    let nop = Ps::Attr(Mode::NoPerspective(At::Center));
    let t = |ps, samples| Case { corners: corners(&TP, &TA, None), tris: vec![[0, 1, 2]], ps, samples, viewport: FULL };
    let rp = [TP[1], TP[2], TP[0]];
    let ra = [TA[1], TA[2], TA[0]];
    let qp = [[-1.0, 1.0, 0.5, 1.0], [1.0, 1.0, 0.5, 1.0], [-1.0, -1.0, 0.5, 1.0], [1.0, -1.0, 0.5, 1.0]];
    let qa = [[0.0, 0.0, 0.25, 1.0], [1.0, 0.0, 0.75, 1.0], [0.0, 1.0, 0.5, 0.0], [1.0, 1.0, 0.125, 0.0]];
    let q = |ps, samples, viewport| Case { corners: corners(&qp, &qa, None), tris: vec![[0, 1, 2], [1, 3, 2]], ps, samples, viewport };
    let cp = [[-1.7, -0.3, 0.5, 1.0], [0.6, 1.9, 0.5, 1.0], [1.4, -1.2, 0.5, 1.0]];
    let ca = [[0.2, 3.0, -1.5, 0.6], [0.9, -2.0, 0.25, 0.1], [0.45, 0.7, 5.5, 0.35]];
    let np = [[-0.8, -0.8, -0.5, 1.0], [0.0, 1.35, 0.75, 1.5], [0.9, -0.6, 0.9, 1.0]];
    let cl = |p: &[[f32; 4]], ps| Case { corners: corners(p, &ca, None), tris: vec![[0, 1, 2]], ps, samples: 1, viewport: FULL };
    let vp = [0.5, 1.25, 6.5, 5.75, 0.0, 1.0];
    vec![
        ("triangle ps_lin", t(lin, 1)),
        ("triangle ps_nop", t(nop, 1)),
        ("triangle ps_cen", t(Ps::Attr(Mode::Perspective(At::Centroid)), 1)),
        ("triangle ps_smp", t(Ps::Attr(Mode::Perspective(At::Sample)), 1)),
        ("triangle ps_flat", t(Ps::Flat, 1)),
        ("triangle ps_pos", t(Ps::Pos(At::Center), 1)),
        ("triangle, geometry shader ps_lin", t(lin, 1)),
        ("triangle, geometry shader ps_flat", t(Ps::Flat, 1)),
        ("triangle ps_lin 4x", t(lin, 4)),
        ("triangle ps_cen 4x", t(Ps::Attr(Mode::Perspective(At::Centroid)), 4)),
        ("triangle ps_nopcen 4x", t(Ps::Attr(Mode::NoPerspective(At::Centroid)), 4)),
        ("triangle ps_smp 4x", t(Ps::Attr(Mode::Perspective(At::Sample)), 4)),
        ("triangle ps_nopsmp 4x", t(Ps::Attr(Mode::NoPerspective(At::Sample)), 4)),
        ("triangle ps_pos 4x", t(Ps::Pos(At::Center), 4)),
        ("triangle ps_possmp 4x", t(Ps::Pos(At::Sample), 4)),
        ("triangle ps_poscen 4x", t(Ps::Pos(At::Centroid), 4)),
        ("triangle turned ps_lin", Case { corners: corners(&rp, &ra, None), tris: vec![[0, 1, 2]], ps: lin, samples: 1, viewport: FULL }),
        ("triangle turned ps_flat", Case { corners: corners(&rp, &ra, None), tris: vec![[0, 1, 2]], ps: Ps::Flat, samples: 1, viewport: FULL }),
        ("quad ps_lin", q(lin, 1, FULL)),
        ("quad ps_nop", q(nop, 1, FULL)),
        ("quad, viewport 0.5 1.25 6.5 5.75 ps_lin", q(lin, 1, vp)),
        ("quad, viewport 0.5 1.25 6.5 5.75 ps_pos", q(Ps::Pos(At::Center), 1, vp)),
        ("quad ps_lin 4x", q(lin, 4, FULL)),
        ("clipped left top right ps_lin", cl(&cp, lin)),
        ("clipped left top right ps_nop", cl(&cp, nop)),
        ("clipped left top right ps_flat", cl(&cp, Ps::Flat)),
        ("clipped left top right ps_pos", cl(&cp, Ps::Pos(At::Center))),
        ("clipped near ps_lin", cl(&np, lin)),
        ("clipped near ps_nop", cl(&np, nop)),
        ("clipped near ps_pos", cl(&np, Ps::Pos(At::Center))),
        ("clip distance ps_clip", Case { corners: corners(&TP, &TA, Some(&[-0.5, 1.0, 0.75])), tris: vec![[0, 1, 2]], ps: Ps::Clip, samples: 1, viewport: FULL }),
    ]
}

#[test]
fn every_triangle_record_as_warp_wrote_it() {
    let g = golden();
    let mut bad = Vec::new();
    for (name, c) in cases() {
        let mut want = g.get(name).cloned().unwrap_or_default();
        assert!(!want.is_empty(), "{name}: no records in the golden");
        let mut got = model(&c);
        // (both in the golden's order: rows, then columns, then samples)
        want.sort_by_key(|r| (r.0.1, r.0.0, r.0.2));
        got.sort_by_key(|r| (r.0.1, r.0.0, r.0.2));
        let (wk, gk): (Vec<_>, Vec<_>) = (want.iter().map(|r| r.0).collect(), got.iter().map(|r| r.0).collect());
        if wk != gk {
            bad.push(format!("{name}: covered {:?} vs model {:?}", wk, gk));
            continue;
        }
        let mut words = 0;
        let mut first = None;
        for (w, m) in want.iter().zip(&got) {
            for k in 0..4 {
                if w.1[k] != m.1[k] {
                    words += 1;
                    first.get_or_insert(format!("{:?} word {k}: {:08x} vs model {:08x}", w.0, w.1[k], m.1[k]));
                }
            }
            if w.2 != m.2 {
                words += 1;
                first.get_or_insert(format!("{:?} coverage {:x} vs model {:x}", w.0, w.2, m.2));
            }
        }
        if words > 0 {
            bad.push(format!("{name}: {words} words differ, first {}", first.unwrap_or_default()));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

const LP: [[f32; 4]; 4] = [[-0.95, -0.6, 0.5, 1.0], [1.6, 0.9, 0.25, 2.0], [-0.3, 0.95, 0.5, 0.5], [0.1, -0.9, 0.5, 1.0]];
const LA: [[f32; 4]; 4] = [[0.1, 1.7, -3.25, 100.0], [0.73, -0.2, 7.5, 0.001], [0.31, 0.9, 0.0, -42.5], [0.6, 0.15, 2.0, 9.0]];

/// The records the model gives lines drawn in order (the later one's record where two light a
/// pixel).
fn model_lines(corners: &[Corner], lines: &[[usize; 2]], ps: Ps) -> Vec<Rec> {
    let r = Raster { viewport: FULL, cull: Cull::None, front_ccw: false, depth_clip: true };
    let ms = modes(ps);
    let mut out: BTreeMap<(u32, u32, u32), ([u32; 4], u32)> = BTreeMap::new();
    for l in lines {
        let Some(s) = line(&[corners[l[0]].clone(), corners[l[1]].clone()], &ms, &r) else { continue };
        for y in 0..8u32 {
            for x in 0..8u32 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                if !line_covers(&s, px, py) {
                    continue;
                }
                let w = 1.0 / line_at(s.rhw, px, py);
                let v = |k: usize, m: Mode| -> u32 {
                    let p = s.attrs.get(k).copied().unwrap_or_default();
                    match m {
                        Mode::Undefined => 0,
                        Mode::Constant => p.c.to_bits(),
                        Mode::NoPerspective(_) => line_at(p, px, py).to_bits(),
                        Mode::Perspective(_) => (line_at(p, px, py) * w).to_bits(),
                    }
                };
                let rec = match ps {
                    Ps::Attr(m) => std::array::from_fn(|k| v(k, m)),
                    Ps::Flat => [v(0, Mode::Constant), v(1, Mode::Constant), v(2, Mode::Constant), v(4, Mode::Constant)],
                    Ps::Pos(_) => [px.to_bits(), py.to_bits(), line_at(s.z, px, py).to_bits(), w.to_bits()],
                    Ps::Clip => [0; 4],
                };
                out.insert((x, y, 0), (rec, 1));
            }
        }
    }
    out.into_iter().map(|(k, (v, c))| (k, v, c)).collect()
}

fn compare(name: &str, mut got: Vec<Rec>, g: &BTreeMap<String, Vec<Rec>>, bad: &mut Vec<String>) {
    let mut want = g.get(name).cloned().unwrap_or_default();
    assert!(!want.is_empty(), "{name}: no records in the golden");
    want.sort_by_key(|r| (r.0.1, r.0.0, r.0.2));
    got.sort_by_key(|r| (r.0.1, r.0.0, r.0.2));
    let (wk, gk): (Vec<_>, Vec<_>) = (want.iter().map(|r| r.0).collect(), got.iter().map(|r| r.0).collect());
    if wk != gk {
        bad.push(format!("{name}: covered {:?} vs model {:?}", wk, gk));
        return;
    }
    let mut words = 0;
    let mut first = None;
    for (w, m) in want.iter().zip(&got) {
        for k in 0..4 {
            if w.1[k] != m.1[k] {
                words += 1;
                first.get_or_insert(format!("{:?} word {k}: {:08x} vs model {:08x}", w.0, w.1[k], m.1[k]));
            }
        }
    }
    if words > 0 {
        bad.push(format!("{name}: {words} words differ, first {}", first.unwrap_or_default()));
    }
}

#[test]
fn every_line_record_as_warp_wrote_it() {
    let g = golden();
    let mut bad = Vec::new();
    let c = corners(&LP, &LA, None);
    let lines = [[0, 1], [2, 3]];
    compare("lines ps_lin", model_lines(&c, &lines, Ps::Attr(Mode::Perspective(At::Center))), &g, &mut bad);
    compare("lines ps_nop", model_lines(&c, &lines, Ps::Attr(Mode::NoPerspective(At::Center))), &g, &mut bad);
    compare("lines ps_flat", model_lines(&c, &lines, Ps::Flat), &g, &mut bad);
    compare("lines ps_pos", model_lines(&c, &lines, Ps::Pos(At::Center)), &g, &mut bad);
    let t = corners(&TP, &TA, None);
    compare("triangle, wireframe ps_lin", model_lines(&t, &[[0, 1], [1, 2], [2, 0]], Ps::Attr(Mode::Perspective(At::Center))), &g, &mut bad);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn every_point_record_as_warp_wrote_it() {
    let g = golden();
    let mut bad = Vec::new();
    let r = Raster { viewport: FULL, cull: Cull::None, front_ccw: false, depth_clip: true };
    let c = corners(&LP, &LA, None);
    for (name, ps) in [("points ps_lin", Ps::Attr(Mode::Perspective(At::Center))), ("points ps_pos", Ps::Pos(At::Center))] {
        let ms = modes(ps);
        let mut got = Vec::new();
        for v in &c {
            let Some(d) = point(v, &ms, &r) else { continue };
            let (Ok(x), Ok(y)) = (u32::try_from(d.pixel.0), u32::try_from(d.pixel.1)) else { continue };
            if x >= 8 || y >= 8 {
                continue;
            }
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let rec = match ps {
                Ps::Attr(m) => std::array::from_fn(|k| dot_eval(&d, k, m)),
                _ => [px.to_bits(), py.to_bits(), d.z.to_bits(), (1.0 / d.rhw).to_bits()],
            };
            got.push(((x, y, 0), rec, 1));
        }
        compare(name, got, &g, &mut bad);
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
