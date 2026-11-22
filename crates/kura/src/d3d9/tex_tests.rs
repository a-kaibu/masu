//! Direct3D 9 texture bookkeeping as Windows has it (Wine's tests test_resource_priority,
//! test_lod, test_mipmap_autogen; winref d3d9tex): SetPriority/SetLOD of each pool,
//! D3DUSAGE_AUTOGENMIPMAP (CheckDeviceFormat's D3DOK_NOAUTOGEN, the refusals, one level shown,
//! the page making the others from the top before a draw reads them), and dirty regions
//! (UpdateTexture copies only what a lock without D3DLOCK_NO_DIRTY_UPDATE or AddDirtyRect
//! marked, then nothing is dirty): a model (PBT) of any sequence of locks, AddDirtyRect and
//! UpdateTexture.

use super::draw_tests::{call, device_with, out, put_bytes, rd, words};
use super::trace_tests::{alloc, SERIAL};
use super::*;
use crate::render::model;
use proptest::prelude::*;

use super::device::Mips;
use super::format as f;

const INVALIDCALL: u32 = 0x8876_086c;

fn device() -> u32 {
    device_with([8, 8, 22, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 0])
}

fn init() {
    unsafe {
        let vt: Vec<u32> = (1..=64).map(|i| 0x1000 * i).collect();
        vnhost_d3d9_init(vt.as_ptr(), vt.len() as u32);
    }
}

fn pump() {
    gpu::flush();
    gpu::pump();
}

fn errors() -> Vec<String> {
    model::with(|m| std::mem::take(&mut m.errors))
}

fn texture(dev: u32, w: u32, levels: u32, usage: u32, fmt: u32, pool: u32) -> Result<u32, u32> {
    let o = out();
    match call(IF_DEVICE, 23, &[dev, w, w, levels, usage, fmt, pool, o, 0]) {
        0 => Ok(rd(o)),
        hr => Err(hr),
    }
}

/// The page's 2D textures of `w`x`w` made since `from`: texels per mip level.
fn page_textures(from: usize, w: u32) -> Vec<Vec<Vec<u64>>> {
    model::with(|m| {
        m.objs[from..]
            .iter()
            .filter_map(|o| match o {
                model::Obj::Texture(t) if t.dim == "2d" && t.w == w && t.h == w && t.layers == 1 => Some(t.texels.clone()),
                _ => None,
            })
            .collect()
    })
}

/// Rectangle `r` of level `level` of `t` locked with `flags` and filled with `px`.
fn lock_fill(t: u32, level: u32, r: [u32; 4], flags: u32, px: impl Fn(u32, u32) -> u32) {
    let lr = unsafe { alloc(8) as u32 };
    assert_eq!(call(IF_TEXTURE, 19, &[t, level, lr, words(&r), flags]), 0);
    let (pitch, bits) = (rd(lr), rd(lr + 4));
    for y in r[1]..r[3] {
        let row: Vec<u8> = (r[0]..r[2]).flat_map(|x| px(x, y).to_le_bytes()).collect();
        put_bytes(bits + (y - r[1]) * pitch, &row);
    }
    assert_eq!(call(IF_TEXTURE, 20, &[t, level]), 0);
}

#[derive(Clone, Copy, Debug)]
enum Op {
    SetPriority(u32),
    SetLod(u32),
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 32, ..ProptestConfig::default() })]
    /// Any sequence of SetPriority and SetLOD on a texture of any pool, its top surface and a
    /// vertex buffer of that pool: a managed texture or buffer keeps its priority, a managed
    /// texture its LOD (at most its last level), the rest answer 0 (Windows: Wine's tests).
    #[test]
    fn priority_and_lod_only_of_managed(pool in 0u32..4, ops in proptest::collection::vec(prop_oneof![any::<u8>().prop_map(|v| Op::SetPriority(v.into())), (0u32..6).prop_map(Op::SetLod)], 1..10)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        init();
        let dev = device();
        let t = texture(dev, 16, 3, 0, f::A8R8G8B8, pool).map_err(|hr| TestCaseError::fail(format!("CreateTexture {hr:#x}")))?;
        let o = out();
        prop_assert_eq!(call(IF_TEXTURE, 18, &[t, 0, o]), 0);
        let s = rd(o);
        let vb = (pool != 3).then(|| {
            let o = out();
            assert_eq!(call(IF_DEVICE, 26, &[dev, 64, 0, 0, pool, o, 0]), 0);
            rd(o)
        });
        let managed = pool == 1;
        let (mut prio, mut lod) = (0u32, 0u32);
        for op in ops {
            match op {
                Op::SetPriority(v) => {
                    prop_assert_eq!(call(IF_TEXTURE, 7, &[t, v]), prio);
                    if let Some(vb) = vb {
                        prop_assert_eq!(call(IF_VB, 7, &[vb, v]), prio);
                    }
                    prop_assert_eq!(call(IF_SURFACE, 7, &[s, v]), 0);
                    if managed {
                        prio = v;
                    }
                }
                Op::SetLod(v) => {
                    prop_assert_eq!(call(IF_TEXTURE, 11, &[t, v]), lod);
                    if managed {
                        lod = v.min(2);
                    }
                }
            }
            prop_assert_eq!(call(IF_TEXTURE, 8, &[t]), prio);
            prop_assert_eq!(call(IF_TEXTURE, 12, &[t]), lod);
            prop_assert_eq!(call(IF_SURFACE, 8, &[s]), 0);
            if let Some(vb) = vb {
                prop_assert_eq!(call(IF_VB, 8, &[vb]), prio);
            }
        }
        call(IF_SURFACE, 2, &[s]);
        call(IF_TEXTURE, 2, &[t]);
        if let Some(vb) = vb {
            call(IF_VB, 2, &[vb]);
        }
        gpu::set_pump(None);
    }
}

#[test]
fn autogen_mipmaps_as_windows() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device();
    errors();
    let o = out();
    assert_eq!(call(IF_DEVICE, 6, &[dev, o]), 0);
    let d3d = rd(o);
    // CheckDeviceFormat(AUTOGENMIPMAP) of textures and cube textures, DYNAMIC or not: Windows'
    // answers (winref d3d9tex), D3DOK_NOAUTOGEN MAKE_D3DSTATUS(2159)
    let golden = [
        (f::A8R8G8B8, 0), (f::X8R8G8B8, 0x0876_086f), (f::R5G6B5, 0x0876_086f), (f::A8, 0), (f::L8, 0), (f::A8L8, 0),
        (f::V8U8, 0x0876_086f), (f::DXT1, 0x0876_086f), (f::A16B16G16R16F, 0), (f::R32F, 0), (f::D16, 0x8876_086a),
    ];
    for (fmt, want) in golden {
        for usage in [0x400, 0x400 | 0x200] {
            for rtype in [3, 5] {
                assert_eq!(call(IF_D3D9, 10, &[d3d, 0, 1, f::X8R8G8B8, usage, rtype, fmt]), want, "format {fmt} usage {usage:#x} type {rtype}");
            }
        }
    }
    // refused: levels other than 0 or 1, system or scratch memory, a depth format, a volume
    assert_eq!(texture(dev, 64, 0, 0x400, f::D16, 0), Err(INVALIDCALL), "D16");
    for (levels, pool) in [(2, 0), (6, 1), (0, 2), (0, 3)] {
        assert_eq!(texture(dev, 64, levels, 0x400, f::A8R8G8B8, pool), Err(INVALIDCALL), "levels {levels} pool {pool}");
    }
    let o = out();
    assert_eq!(call(IF_DEVICE, 24, &[dev, 8, 8, 8, 0, 0x400, f::A8R8G8B8, 0, o, 0]), INVALIDCALL, "a volume");
    // one level shown, LINEAR at first, NONE refused
    for (fmt, pool) in [(f::A8R8G8B8, 0), (f::A8R8G8B8, 1), (f::DXT1, 1)] {
        let t = texture(dev, 64, 0, 0x400, fmt, pool).expect("CreateTexture");
        assert_eq!(call(IF_TEXTURE, 13, &[t]), 1, "GetLevelCount {fmt}");
        assert_eq!(call(IF_TEXTURE, 15, &[t]), 2);
        assert_eq!(call(IF_TEXTURE, 14, &[t, 0]), INVALIDCALL);
        assert_eq!(call(IF_TEXTURE, 14, &[t, 1]), 0);
        assert_eq!(call(IF_TEXTURE, 15, &[t]), 1);
        let o = out();
        assert_eq!(call(IF_TEXTURE, 18, &[t, 1, o]), INVALIDCALL, "no level 1");
        call(IF_TEXTURE, 2, &[t]);
    }
    // the page: all levels, made from the top before a draw reads them, again only after the
    // top changed (or GenerateMipSubLevels)
    let from = model::with(|m| m.objs.len());
    let t = texture(dev, 4, 0, 0x400, f::A8R8G8B8, 1).expect("CreateTexture");
    let mips = |from: usize| {
        pump();
        let p = page_textures(from, 4);
        assert_eq!(p.len(), 1, "one 4x4 page texture");
        assert_eq!(p[0].len(), 3, "its three levels");
        (p[0][1].clone(), p[0][2].clone())
    };
    let zero = mips(from);
    lock_fill(t, 0, [0, 0, 4, 4], 0, |x, y| [0xffff_0000, 0xff00_ff00, 0xff00_00ff, 0xffff_ffff][usize::from(y >= 2) * 2 + usize::from(x >= 2)]);
    assert_eq!(mips(from), zero, "nothing made until a draw reads it");
    draw(dev, t);
    let first = mips(from);
    assert_ne!(first.0, zero.0, "level 1 made");
    assert_ne!(first.1, zero.1, "level 2 made");
    let stale = || {
        let d = D3D.lock().unwrap_or_else(|e| e.into_inner());
        let d = d.as_ref().expect("D3D");
        match d.objs.get(&d.id_of(t)).map(|o| &o.data) {
            Some(Data::Texture(tx)) => tx.mips,
            _ => panic!("no texture"),
        }
    };
    assert_eq!(stale(), Mips::Auto { stale: false });
    lock_fill(t, 0, [0, 0, 2, 2], 0, |_, _| 0xff80_8000);
    assert_eq!(stale(), Mips::Auto { stale: true }, "the top changed");
    assert_eq!(call(IF_TEXTURE, 16, &[t]), 0, "GenerateMipSubLevels");
    assert_eq!(stale(), Mips::Auto { stale: false });
    assert_ne!(mips(from), first, "made again from the new top");
    // a filter change makes them again
    assert_eq!(call(IF_TEXTURE, 14, &[t, 1]), 0);
    assert_eq!(stale(), Mips::Auto { stale: true });
    let e = errors();
    assert!(e.is_empty(), "{:?}", &e[..e.len().min(8)]);
    call(IF_TEXTURE, 2, &[t]);
    gpu::set_pump(None);
}

/// A pre-transformed quad over the 8x8 target textured with `t`.
fn draw(dev: u32, t: u32) {
    let v = unsafe { alloc(96) as u32 };
    let mut b = Vec::new();
    for (x, y) in [(0f32, 0f32), (8.0, 0.0), (0.0, 8.0), (8.0, 8.0)] {
        for f in [x - 0.5, y - 0.5, 0.5, 1.0, x / 8.0, y / 8.0] {
            b.extend(f.to_le_bytes());
        }
    }
    put_bytes(v, &b);
    call(IF_DEVICE, 65, &[dev, 0, t]);
    call(IF_DEVICE, 89, &[dev, 0x4 | 0x100]);
    call(IF_DEVICE, 41, &[dev]);
    assert_eq!(call(IF_DEVICE, 83, &[dev, 5, 2, v, 24]), 0);
    call(IF_DEVICE, 42, &[dev]);
    call(IF_DEVICE, 65, &[dev, 0, 0]);
    pump();
}

#[derive(Clone, Copy, Debug)]
enum Dirty {
    /// a lock of (level, rect seed) with D3DLOCK_NO_DIRTY_UPDATE or not, its texels from a seed
    Lock(u32, [u8; 4], bool, u32),
    /// AddDirtyRect (None: NULL)
    Add(Option<[u8; 4]>),
    Update,
}

/// rect seed -> a rectangle of an `e`x`e` level
fn rect_of(r: [u8; 4], e: u32) -> [u32; 4] {
    let (x0, y0) = (u32::from(r[0]) % e, u32::from(r[1]) % e);
    [x0, y0, x0 + 1 + u32::from(r[2]) % (e - x0), y0 + 1 + u32::from(r[3]) % (e - y0)]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]
    /// Any sequence of locks of a system memory 8x8 two-level texture (some
    /// D3DLOCK_NO_DIRTY_UPDATE), AddDirtyRect and UpdateTexture into a default pool one leaves
    /// the target's levels holding what a model of dirty regions holds: the union of what was
    /// marked since the last UpdateTexture (a level's in level 0's texels, the sublevel copying
    /// the texels it touches), all of it at first.
    #[test]
    fn update_texture_copies_the_dirty_part(ops in proptest::collection::vec(prop_oneof![
        (0u32..2, any::<[u8; 4]>(), any::<bool>(), any::<u32>()).prop_map(|(l, r, n, s)| Dirty::Lock(l, r, n, s)),
        proptest::option::of(any::<[u8; 4]>()).prop_map(Dirty::Add),
        Just(Dirty::Update),
    ], 1..14)) {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        init();
        let dev = device();
        errors();
        let sys = texture(dev, 8, 2, 0, f::A8R8G8B8, 2).map_err(|hr| TestCaseError::fail(format!("{hr:#x}")))?;
        let from = model::with(|m| m.objs.len());
        let def = texture(dev, 8, 2, 0, f::A8R8G8B8, 0).map_err(|hr| TestCaseError::fail(format!("{hr:#x}")))?;
        // the model: each level's texels in the source and the target, and the dirty box in
        // level 0's texels (all of a new texture)
        let mut src: Vec<Vec<u32>> = vec![vec![0; 64], vec![0; 16]];
        let mut dst = src.clone();
        let mut dirty: Option<[u32; 4]> = Some([0, 0, 8, 8]);
        let grow = |d: &mut Option<[u32; 4]>, r: [u32; 4]| {
            *d = Some(d.map_or(r, |o| [o[0].min(r[0]), o[1].min(r[1]), o[2].max(r[2]), o[3].max(r[3])]));
        };
        for op in ops.into_iter().chain([Dirty::Update]) {
            match op {
                Dirty::Lock(l, r, no_dirty, seed) => {
                    let e = 8 >> l;
                    let r = rect_of(r, e);
                    let px = |x: u32, y: u32| seed.wrapping_mul(2654435761).wrapping_add(x * 977 + y * 131 + l);
                    lock_fill(sys, l, r, if no_dirty { 0x8000 } else { 0 }, px);
                    for y in r[1]..r[3] {
                        for x in r[0]..r[2] {
                            src[l as usize][(y * e + x) as usize] = px(x, y);
                        }
                    }
                    if !no_dirty {
                        grow(&mut dirty, r.map(|v| v << l));
                    }
                }
                Dirty::Add(r) => {
                    let r = r.map(|r| rect_of(r, 8));
                    prop_assert_eq!(call(IF_TEXTURE, 21, &[sys, r.map_or(0, |r| words(&r))]), 0);
                    grow(&mut dirty, r.unwrap_or([0, 0, 8, 8]));
                }
                Dirty::Update => {
                    prop_assert_eq!(call(IF_DEVICE, 31, &[dev, sys, def]), 0);
                    if let Some(b) = dirty.take() {
                        for l in 0..2u32 {
                            let e = 8 >> l;
                            let (x0, y0, x1, y1) = (b[0] >> l, b[1] >> l, b[2].div_ceil(1 << l).min(e), b[3].div_ceil(1 << l).min(e));
                            for y in y0..y1 {
                                for x in x0..x1 {
                                    dst[l as usize][(y * e + x) as usize] = src[l as usize][(y * e + x) as usize];
                                }
                            }
                        }
                    }
                    pump();
                    // (the target: the 8x8 page texture of two levels)
                    let p = page_textures(from, 8);
                    let p: Vec<_> = p.into_iter().filter(|t| t.len() == 2).collect();
                    prop_assert_eq!(p.len(), 1);
                    for l in 0..2usize {
                        let got: Vec<u32> = p[0][l].iter().map(|&v| v as u32).collect();
                        prop_assert_eq!(&got, &dst[l], "level {}", l);
                    }
                }
            }
        }
        prop_assert!(errors().is_empty());
        call(IF_TEXTURE, 2, &[sys]);
        call(IF_TEXTURE, 2, &[def]);
        gpu::set_pump(None);
    }
}

/// The levels AUTOGENMIPMAP makes of four colored quadrants (winref d3d9tex): D3DTEXF_POINT
/// takes each texel's (2x + 1, 2y + 1) above, so the 1x1 level is the bottom right quadrant's;
/// D3DTEXF_LINEAR the four's mean (a blit through a linear sampler). And a draw with D3DSAMP_MAXMIPLEVEL asks the page for a
/// sampler whose lodMinClamp is that level.
#[test]
fn autogen_levels_by_filter_and_max_mip_level() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    init();
    let dev = device();
    errors();
    for (filter, one) in [(1u32, None), (2, Some(()))] {
        let from = model::with(|m| m.objs.len());
        let t = texture(dev, 4, 0, 0x400, f::A8R8G8B8, 1).expect("CreateTexture");
        assert_eq!(call(IF_TEXTURE, 14, &[t, filter]), 0);
        lock_fill(t, 0, [0, 0, 4, 4], 0, |x, y| [0xffff_0000, 0xff00_ff00, 0xff00_00ff, 0xffff_ffff][usize::from(y >= 2) * 2 + usize::from(x >= 2)]);
        draw(dev, t);
        pump();
        let p = page_textures(from, 4);
        assert_eq!(p.len(), 1);
        let (l1, l2) = (&p[0][1], &p[0][2]);
        let top = &p[0][0];
        // (level 1 is the quadrants' colors either way: what each 2x2 block of the top holds)
        assert_eq!(l1, &vec![top[0], top[2], top[8], top[10]], "filter {filter}: level 1");
        // (the model samples a blit's source at the nearest texel, its texels being hashes: the
        // linear mean shows as the blit's linear sampler)
        let linear = model::with(|m| m.objs[from..].iter().filter(|o| matches!(o, model::Obj::Sampler(k) if k & 3 == 1)).count());
        match one {
            None => {
                assert_eq!(l2, &vec![top[15]], "point: the bottom right texel");
                assert_eq!(linear, 0, "point: no linear sampler");
            }
            Some(()) => assert_eq!(linear, 1, "linear: the blit's one linear sampler"),
        }
        call(IF_TEXTURE, 2, &[t]);
    }
    gpu::set_pump(None);
}
