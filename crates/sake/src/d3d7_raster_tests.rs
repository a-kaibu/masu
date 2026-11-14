//! Direct3D 7's software rasterizer pinned before its states become types: seeded random draws
//! with every texture operation and argument (and values past them), every compare, blend,
//! address mode, cull mode, color keys, alpha tests, fog, flat and specular shading, one or two
//! stages, point and bilinear sampling, depth tests and writes in 16 and 32 bits, onto targets
//! and from textures of X8R8G8B8, A8R8G8B8, R5G6B5 and A1R5G5B5. Each draw's target and depth
//! bytes go into an FNV hash line of d3d7_raster_golden.txt. MASU_UPDATE_GOLDEN=1 rewrites it.
use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
    fn f(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * (self.next() % 10001) as f32 / 10000.
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.below(v.len() as u32) as usize]
    }
    fn c4(&mut self) -> C4 {
        std::array::from_fn(|_| if self.below(4) == 0 { self.pick(&[0., 1., 128. / 255.]) } else { self.f(0., 1.) })
    }
    /// a state value: mostly the defined ones, sometimes past them
    fn state(&mut self, n: u32) -> u32 {
        if self.below(12) == 0 { self.next() } else { self.below(n) }
    }
}

const MASKS: [[u32; 4]; 4] = [[0xff_0000, 0xff00, 0xff, 0], [0xff_0000, 0xff00, 0xff, 0xff00_0000], [0xf800, 0x7e0, 0x1f, 0], [0x7c00, 0x3e0, 0x1f, 0x8000]];

fn fmt_of(m: [u32; 4]) -> Fmt {
    Fmt { ch: m.map(|m| if m == 0 { (0, 0) } else { (m.trailing_zeros(), m >> m.trailing_zeros()) }) }
}

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h = (*h ^ x as u64).wrapping_mul(0x100_0000_01b3);
    }
}

const W: i32 = 19;
const H: i32 = 15;

fn draw(rng: &mut Rng, out: &mut Vec<String>) {
    let rm = rng.pick(&MASKS);
    let rbytes = if rm[0] > 0xffff { 4 } else { 2 };
    let mut rt: Vec<u8> = (0..W as usize * H as usize * rbytes).map(|_| rng.next() as u8).collect();
    let zbytes = rng.pick(&[2usize, 4]);
    let zmax: f64 = if zbytes == 2 { 65535. } else { rng.pick(&[16_777_215., 4_294_967_295.]) };
    // depths the triangles have exactly (so that equal depths meet every compare), and others
    let zs = [0f32, 0.25, 0.5, 1.];
    let mut zb: Vec<u8> = Vec::new();
    for _ in 0..W * H {
        let v = if rng.below(2) == 0 { (f64::from(rng.pick(&zs)) * zmax) as u32 } else { rng.next() };
        zb.extend_from_slice(&v.to_le_bytes()[..zbytes]);
    }
    let mut texes: Vec<(Vec<u8>, i32, i32, usize, Fmt, Option<(u32, u32)>)> = Vec::new();
    let nst = 1 + rng.below(2) as usize;
    for _ in 0..nst {
        let m = rng.pick(&MASKS);
        let b = if m[0] > 0xffff { 4 } else { 2 };
        let (tw, th) = (rng.pick(&[1, 2, 4, 7, 8]), rng.pick(&[1, 3, 4, 8]));
        // (few distinct texels, so that color keys catch some)
        let px: Vec<u8> = (0..tw as usize * th as usize * b).map(|_| rng.next() as u8 & rng.pick(&[0xff, 0x81, 0xf0])).collect();
        let key = (rng.below(3) == 0).then(|| {
            let k = rng.next() & rng.pick(&[0x8181_8181, 0xffff, 0]);
            (k, if rng.below(2) == 0 { k } else { k | rng.below(256) })
        });
        texes.push((px, tw, th, b, fmt_of(m), key));
    }
    let stages: Vec<Stage> = texes
        .iter()
        .map(|(px, tw, th, b, f, key)| Stage {
            cop: rng.state(27),
            ca: [rng.state(6) | rng.pick(&[0, 0, 0x10, 0x20, 0x30]), rng.state(6) | rng.pick(&[0, 0, 0x10, 0x20])],
            aop: rng.state(27),
            aa: [rng.state(6) | rng.pick(&[0, 0, 0x10]), rng.state(6)],
            tex: (rng.below(8) != 0).then(|| Tex { bits: px.as_ptr(), w: *tw, h: *th, pitch: *tw as usize * b, bpp: *b, fmt: *f, pal: None, key: *key }),
            tci: rng.below(2) as usize,
            addr: [Addr::of(rng.state(6)), Addr::of(rng.state(6))],
            border: rng.c4(),
            linear: rng.below(2) == 0,
        })
        .collect();
    let (rt0, zb0) = (rt.clone(), zb.clone());
    let use_z = rng.below(2) == 0;
    let r = Raster {
        rt: rt.as_mut_ptr(),
        pitch: W as usize * rbytes,
        bpp: rbytes,
        fmt: fmt_of(rm),
        z: use_z.then(|| (zb.as_mut_ptr(), W as usize * zbytes, zbytes, zmax as f32)),
        clip: [rng.below(4) as i32, rng.below(4) as i32, W - rng.below(4) as i32, H - rng.below(4) as i32],
        ztest: use_z.then(|| Cmp::of(rng.state(10))),
        zwrite: use_z && rng.below(2) == 0,
        blend: (rng.below(2) == 0).then(|| (Blend::of(rng.state(15)), Blend::of(rng.state(15)))),
        atest: (rng.below(3) == 0).then(|| { let any = rng.below(256); (Cmp::of(rng.state(10)), rng.pick(&[0, 255, 128, any]) as f32 / 255.) }),
        ck: rng.below(3) == 0,
        flat: rng.below(4) == 0,
        spec: rng.below(3) == 0,
        fog: (rng.below(4) == 0).then(|| [rng.f(0., 1.), rng.f(0., 1.), rng.f(0., 1.)]),
        cull: rng.state(5),
        stages,
        tfactor: rng.c4(),
        ntc: 1 + rng.below(2) as usize,
        copy: rng.below(2) == 0,
    };
    for _ in 0..1 + rng.below(3) {
        let flat_z = (rng.below(2) == 0).then(|| rng.pick(&zs));
        let v: Vec<([f32; 4], Vtx)> = (0..3)
            .map(|_| {
                let (x, y, rhw) = (rng.f(-4., W as f32 + 4.), rng.f(-4., H as f32 + 4.), rng.f(0.2, 2.));
                let z = flat_z.unwrap_or_else(|| rng.f(-0.1, 1.1));
                let mut tex = [[0.; 2]; 8];
                for t in tex.iter_mut().take(2) {
                    *t = [rng.f(-1.5, 2.5), rng.f(-1.5, 2.5)];
                }
                let (diff, spec) = (rng.c4(), rng.c4());
                ([x, y, z, rhw], Vtx { clip: [x, y, z, 1. / rhw], screen: true, diff, spec, tex })
            })
            .collect();
        r.tri([(&v[0].0, &v[0].1), (&v[1].0, &v[1].1), (&v[2].0, &v[2].1)]);
    }
    drop(r);
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    fnv(&mut h, &rt);
    let mut z = 0xcbf2_9ce4_8422_2325u64;
    fnv(&mut z, &zb);
    let changed = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(x, y)| x != y).count();
    out.push(format!("{:016x} {:016x} {} {}", h, z, changed(&rt, &rt0), changed(&zb, &zb0)));
}

#[test]
fn raster_draws_as_before() {
    let mut out = Vec::new();
    for seed in [0x9e37_79b9_7f4a_7c15u64, 0xdead_beef_cafe_f00d, 0x0123_4567_89ab_cdef] {
        let mut rng = Rng(seed);
        out.push(format!("seed {seed:#x}"));
        for _ in 0..1500 {
            draw(&mut rng, &mut out);
        }
    }
    let out = out.join("\n") + "\n";
    let golden = concat!(env!("CARGO_MANIFEST_DIR"), "/src/d3d7_raster_golden.txt");
    if std::env::var_os("MASU_UPDATE_GOLDEN").is_some() {
        std::fs::write(golden, &out).unwrap();
        return;
    }
    let want = std::fs::read_to_string(golden).unwrap_or_default();
    if want != out {
        let (w, o): (Vec<&str>, Vec<&str>) = (want.lines().collect(), out.lines().collect());
        let i = w.iter().zip(&o).position(|(a, b)| a != b).unwrap_or(w.len().min(o.len()));
        panic!("d3d7: d3d7_raster_golden.txt differs at line {} (MASU_UPDATE_GOLDEN=1 rewrites it):\n  want {:?}\n  got  {:?}", i + 1, w.get(i), o.get(i));
    }
}

/// SRCBLEND/DESTBLEND as Raster::new read them before Blend (the pair, then each factor by its
/// number), for every value up to past the defined ones and a few far ones
#[test]
fn blend_pair_as_before() {
    fn old(b: u32, c: C4, d: C4) -> C4 {
        match b {
            1 => [0.; 4],
            3 => c,
            4 => c.map(|x| 1. - x),
            5 => [c[3]; 4],
            6 => [1. - c[3]; 4],
            7 => [d[3]; 4],
            8 => [1. - d[3]; 4],
            9 => d,
            10 => d.map(|x| 1. - x),
            11 => {
                let s = c[3].min(1. - d[3]);
                [s, s, s, 1.]
            }
            _ => [1.; 4],
        }
    }
    let (c, d) = ([0.1, 0.4, 0.7, 0.3], [0.9, 0.2, 0.6, 0.8]);
    for s in (0..20).chain([u32::MAX, 0x8000_0000, 255]) {
        for t in (0..20).chain([u32::MAX, 256]) {
            let (os, od) = match s {
                12 => (5, 6),
                13 => (6, 5),
                s => (s, t),
            };
            let (ns, nd) = Blend::pair(s, t);
            assert_eq!((ns.factor(c, d), nd.factor(c, d)), (old(os, c, d), old(od, c, d)), "{s} {t}");
        }
    }
    for v in (0..20).chain([u32::MAX]) {
        let old_cmp = |a: f32, b: f32| match v {
            1 => false,
            2 => a < b,
            3 => a == b,
            4 => a <= b,
            5 => a > b,
            6 => a != b,
            7 => a >= b,
            _ => true,
        };
        for (a, b) in [(0., 0.), (0., 1.), (1., 0.), (0.5, 0.5)] {
            assert_eq!(Cmp::of(v).test(a, b), old_cmp(a, b), "{v}");
        }
    }
}
