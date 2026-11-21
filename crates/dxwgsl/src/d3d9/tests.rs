use super::*;

use crate::valid_wgsl as valid;

/// The dimension `tex<n>` is declared with.
fn dim_of(m: &naga::Module, n: u32) -> Option<naga::ImageDimension> {
    let name = format!("tex{n}");
    m.global_variables.iter().find(|(_, g)| g.name.as_deref() == Some(name.as_str())).and_then(|(_, g)| match m.types[g.ty].inner {
        naga::TypeInner::Image { dim, arrayed: false, .. } => Some(dim),
        _ => None,
    })
}

fn opts() -> Options {
    Options { link: 0x3ff, vs_inputs: Vec::new(), alpha_test: None, fog: false, addr: Vec::new(), targets: 1, flat: false }
}

/// The interpolation of each @location of an entry point's inputs (pixel) or outputs (vertex).
fn interp(m: &naga::Module, pixel: bool) -> Vec<(u32, Option<naga::Interpolation>)> {
    let ep = &m.entry_points[0];
    let mut out = Vec::new();
    let mut take = |ty: naga::Handle<naga::Type>, b: &Option<naga::Binding>| {
        let mut one = |b: &Option<naga::Binding>| {
            if let Some(naga::Binding::Location { location, interpolation, .. }) = b {
                out.push((*location, *interpolation));
            }
        };
        match &m.types[ty].inner {
            naga::TypeInner::Struct { members, .. } => members.iter().for_each(|x| one(&x.binding)),
            _ => one(b),
        }
    };
    if pixel {
        for a in &ep.function.arguments {
            take(a.ty, &a.binding);
        }
    } else if let Some(r) = &ep.function.result {
        take(r.ty, &r.binding);
    }
    out.sort_by_key(|x| x.0);
    out
}

/// Shaders of 幻聖神姫LINEAGE and tests/winref/probes/d3d9flat.c: ps_2_0 texture * v0, ps_2_0
/// v0 + v1, ps_1_1 mov r0, v0, vs_2_0 writing oPos and oD0, vs_3_0 writing COLOR0/1 + TEXCOORD0.
const FLAT_PS: [&[u32]; 3] = [
    &[0xffff0200, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x80000000, 0xb0030000, 0x0200001f, 0x90000000, 0xa00f0800, 0x03000042, 0x800f0000, 0xb0e40000, 0xa0e40800, 0x03000005, 0x80070000, 0x80e40000, 0x90e40000, 0x02000001, 0x80080000, 0x90ff0000, 0x02000001, 0x800f0800, 0x80e40000, 0x0000ffff],
    &[0xffff0200, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x80000000, 0x900f0001, 0x03000002, 0x800f0000, 0x90e40000, 0x90e40001, 0x02000001, 0x800f0800, 0x80e40000, 0x0000ffff],
    &[0xffff0101, 0x00000001, 0x800f0000, 0x90e40000, 0x0000ffff],
];
const FLAT_VS: [&[u32]; 2] = [
    &[0xfffe0200, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x8000000a, 0x900f0001, 0x02000001, 0xc00f0000, 0x90e40000, 0x02000001, 0xd00f0000, 0x90e40001, 0x0000ffff],
    &[0xfffe0300, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x80000000, 0xe00f0000, 0x0200001f, 0x8000000a, 0xe00f0001, 0x0200001f, 0x8001000a, 0xe00f0002, 0x0200001f, 0x80000005, 0xe0030003,
      0x02000001, 0xe00f0000, 0x90e40000, 0x02000001, 0xe00f0001, 0x90e40000, 0x02000001, 0xe00f0002, 0x90e40000, 0x02000001, 0xe0030003, 0x90e40000, 0x0000ffff],
];

proptest::proptest! {
    /// SHADEMODE flat: the colors (locations 0, 1) and only they are flat in what every
    /// vertex stage (shader or fixed function) writes and every pixel shader reads, so the two
    /// stages always agree (WebGPU refuses a pipeline whose stages disagree), and the WGSL is valid.
    #[test]
    fn flat_colors_agree_between_stages(flat: bool, vi in 0usize..3, pi in 0usize..3, rhw: bool) {
        let check = |l: &[(u32, Option<naga::Interpolation>)]| {
            for &(loc, i) in l {
                let is_flat = i == Some(naga::Interpolation::Flat);
                assert_eq!(is_flat, flat && loc < 2, "location {loc}: {i:?}");
            }
        };
        let ps = Shader::parse(FLAT_PS[pi]).unwrap();
        let pm = valid(&Gen::new(&ps).pixel(&Options { flat, ..opts() }));
        let pin = interp(&pm, true);
        check(&pin);
        proptest::prop_assert!(pin.iter().any(|x| x.0 == 0), "the pixel shader reads v0");
        let vout = if vi < 2 {
            let vs = Shader::parse(FLAT_VS[vi]).unwrap();
            interp(&valid(&Gen::new(&vs).vertex(&Options { flat, link: 0, vs_inputs: vec![(0, 0, 0), (1, 1, 3)], ..opts() })), false)
        } else {
            let k = crate::d3d9ff::VsKey { inputs: vec![(USAGE_POSITIONT, 0, 0, 0), (USAGE_COLOR, 0, 1, 3)], rhw, tci: [(0, 0); 8], flat, light: None };
            interp(&valid(&crate::d3d9ff::vertex(&k)), false)
        };
        check(&vout);
        for &(loc, i) in &pin {
            if let Some(&(_, o)) = vout.iter().find(|x| x.0 == loc) {
                proptest::prop_assert_eq!(i == Some(naga::Interpolation::Flat), o == Some(naga::Interpolation::Flat), "location {}", loc);
            }
        }
    }
}

/// Pixel fog (Options::fog: the fog color in u.x[2] as f32 bits) is valid WGSL for every
/// pixel shader version (it mixed the vec4u words themselves: invalid; found by the
/// d3d9_shader fuzz target).
#[test]
fn pixel_fog_is_valid_wgsl() {
    // ps_1_1: mov r0, t0? (tex t0; mov r0, t0) / ps_2_0: mov oC0, c0 / ps_3_0: mov oC0, c0
    let shaders: [&[u32]; 3] = [
        &[0xffff0101, 0x00000042, 0xb00f0000, 0x00000001, 0x800f0000, 0xb0e40000, 0x0000ffff],
        &[0xffff0200, 0x02000001, 0x800f0800, 0xa0e40000, 0x0000ffff],
        &[0xffff0300, 0x02000001, 0x800f0800, 0xa0e40000, 0x0000ffff],
    ];
    for c in shaders {
        let s = Shader::parse(c).unwrap();
        valid(&Gen::new(&s).pixel(&Options { fog: true, ..opts() }));
    }
}

/// texldd in a ps_1_x shader is refused (shader model 2 and later only; it sampled an
/// undeclared texture: invalid WGSL, found by the d3d9_shader fuzz target).
#[test]
fn texldd_needs_shader_model_2() {
    assert!(Shader::parse(&[0xffff0101, 0x5d, 0x80052001]).is_none());
    assert!(Shader::parse(&[0xffff0104, 0x0400005d, 0x800f0000, 0xb0e40000, 0xa0e40800, 0xb0e40000, 0xb0e40000, 0x0000ffff]).is_none());
}

/// ps_1_1..1_3 texture ops write texture registers only (texcoord r9 read texcoord 9, which
/// no shader has: invalid WGSL, found by the d3d9_shader fuzz target); tN still goes.
#[test]
fn ps1_texture_ops_write_texture_registers() {
    assert!(Shader::parse(&[0xffff0101, 0x1000040, 0x80142009]).is_none());
    let s = Shader::parse(&[0xffff0101, 0x00000040, 0xb00f0000, 0x00000001, 0x800f0000, 0xb0e40000, 0x0000ffff]).unwrap();
    valid(&Gen::new(&s).pixel(&opts()));
}

/// A vertex shader with relative addressing and a constant named past 256: the constant
/// array holds it (found by the d3d9_shader fuzz target).
#[test]
fn direct_constants_past_the_relative_range_are_declared() {
    // vs_2_0: mova a0.x, c0.x; mov oPos, c[a0.x + 1]; mov oD0, c300
    let c = [0xfffe0200u32, 0x0200002e, 0xb0010000, 0xa0000000, 0x03000001, 0xc00f0000, 0xa0e42001, 0xb0000000, 0x02000001, 0xd00f0000, 0xa0e4012c, 0x0000ffff];
    let s = Shader::parse(&c).unwrap();
    assert!(s.uses_rel && s.max_const > 300, "{}", s.max_const);
    valid(&Gen::new(&s).vertex(&opts()));
}

/// A matrix op's matrix is registers that all exist (m3x2 of a0 read a1: invalid WGSL,
/// found by the d3d9_shader fuzz target); constants and temps go.
#[test]
fn matrix_ops_take_whole_matrices() {
    // vs_1_1: m3x2 r0.xy, v0, a0 / m3x2 r0.xy, v0, c4; mov oPos, r0
    let m = |src1: u32| [0xfffe0101u32, 0x0000001f, 0x80000000, 0x900f0000, 0x00000018, 0x80030000, 0x90e40000, src1, 0x00000001, 0xc00f0000, 0x80e40000, 0x0000ffff];
    assert!(Shader::parse(&m(0xb0e40000)).is_none());
    let s = Shader::parse(&m(0xa0e40004)).unwrap();
    valid(&Gen::new(&s).vertex(&opts()));
}

/// A register declared twice is refused (two `var v0`: invalid WGSL, found by the
/// d3d9_shader fuzz target), as D3D's validator refuses it.
#[test]
fn registers_are_declared_once() {
    // vs_2_0: dcl_position v0; dcl_texcoord v0; mov oPos, v0
    let c = [0xfffe0200u32, 0x0200001f, 0x80000000, 0x900f0000, 0x0200001f, 0x80000005, 0x900f0000, 0x02000001, 0xc00f0000, 0x90e40000, 0x0000ffff];
    assert!(Shader::parse(&c).is_none());
    let mut ok = c.to_vec();
    ok.drain(4..7);
    valid(&Gen::new(&Shader::parse(&ok).unwrap()).vertex(&opts()));
}

/// Cube and volume samplers (dcl_cube, dcl_volume) are textures of their kind sampled at
/// .xyz with every texld form (#107: they sampled as black, undeclared), 2D ones beside them
/// still 2D.
#[test]
fn cube_and_volume_samplers_are_sampled() {
    use naga::ImageDimension as D;
    // dcl_2d / dcl_cube / dcl_volume: texture type in bits 27-30 of the usage token
    let dcl = |s: u32, ty: u32| [0x0200001f, 0x8000_0000 | ty << 27, 0xa00f0800 | s];
    // texld r0, t0, sN; texldp r1, t0, sN; texldb r2, t0, sN (ps_2_0)
    for (ty, want) in [(3, D::Cube), (4, D::D3)] {
        let mut c = vec![0xffff0200u32, 0x0200001f, 0x80000000, 0xb00f0000];
        c.extend(dcl(0, 2));
        c.extend(dcl(1, ty));
        c.extend([0x03000042, 0x800f0000, 0xb0e40000, 0xa0e40801]);
        c.extend([0x03010042, 0x800f0001, 0xb0e40000, 0xa0e40801]);
        c.extend([0x03020042, 0x800f0002, 0xb0e40000, 0xa0e40801]);
        c.extend([0x03000042, 0x800f0003, 0xb0e40000, 0xa0e40800]);
        // oC0 = r0 + r1 + r2 + r3
        c.extend([0x03000002, 0x800f0000, 0x80e40000, 0x80e40001]);
        c.extend([0x03000002, 0x800f0000, 0x80e40000, 0x80e40002]);
        c.extend([0x03000002, 0x800f0000, 0x80e40000, 0x80e40003]);
        c.extend([0x02000001, 0x800f0800, 0x80e40000, 0x0000ffff]);
        let s = Shader::parse(&c).unwrap();
        let w = Gen::new(&s).pixel(&opts());
        let m = valid(&w);
        assert_eq!(dim_of(&m, 1), Some(want), "{w}");
        assert_eq!(dim_of(&m, 0), Some(D::D2), "{w}");
        assert!(!w.contains("vec4f(0.0, 0.0, 0.0, 1.0)"), "no black stand-in: {w}");
        assert_eq!(w.matches("(tex1, smp1, ").count(), 3, "{w}");
        assert!(w.contains("textureSampleBias(tex1"), "{w}");
    }
    // ps_3_0: dcl_texcoord0 v0; texldl r0, v0, s1; texldd r1, v0, s1, v0, v0
    for (ty, want) in [(3, D::Cube), (4, D::D3)] {
        let mut c = vec![0xffff0300u32, 0x0200001f, 0x80000005, 0x900f0000];
        c.extend(dcl(1, ty));
        c.extend([0x0300005f, 0x800f0000, 0x90e40000, 0xa0e40801]);
        c.extend([0x0500005d, 0x800f0001, 0x90e40000, 0xa0e40801, 0x90e40000, 0x90e40000]);
        c.extend([0x03000002, 0x800f0000, 0x80e40000, 0x80e40001]);
        c.extend([0x02000001, 0x800f0800, 0x80e40000, 0x0000ffff]);
        let s = Shader::parse(&c).unwrap();
        let w = Gen::new(&s).pixel(&opts());
        let m = valid(&w);
        assert_eq!(dim_of(&m, 1), Some(want), "{w}");
        assert!(w.contains("textureSampleLevel(tex1") && w.contains("textureSampleGrad(tex1"), "{w}");
    }
}
