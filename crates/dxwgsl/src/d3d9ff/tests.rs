use super::*;
use proptest::prelude::*;

fn valid(w: &str) {
    let m = naga::front::wgsl::parse_str(w).unwrap_or_else(|e| panic!("{}\n{w}", e.emit_to_string(w)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&m)
        .unwrap_or_else(|e| panic!("{e:?}\n{w}"));
}

/// The stage lines of the shader: what each stage assigns ("current" / "temp").
fn stage_dsts(w: &str) -> Vec<&str> {
    w.lines().filter_map(|l| l.trim().strip_suffix(" = vec4f(c.xyz, a); }")).collect()
}

// D3DTA_TEMP (5) and D3DTSS_RESULTARG = D3DTA_TEMP: what kura reports with
// D3DPMISCCAPS_TSSARGTEMP. A stage that targets temp leaves current as it was for the next
// stage, and a later stage reads temp (ダンジョンズ＆シネステジア quits without the cap).
#[test]
fn resultarg_temp_goes_to_temp_and_back() {
    // stage 0: temp = tex0 (SELECTARG1 TEXTURE, result TEMP); stage 1: current = diffuse * temp
    let k = PsKey {
        stages: vec![[2, 2, 0, 0, 2, 2, 0, 0, 5, 0], [4, 0, 5, 0, 4, 0, 5, 0, 1, 0]],
        tex: [2, 0, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    };
    let w = pixel(&k);
    valid(&w);
    assert_eq!(stage_dsts(&w), ["temp", "current"], "{w}");
    assert!(w.contains("(diffuse * temp)"), "{w}");
    assert!(w.contains("var temp = vec4f(0.0);"), "temp starts at 0 (as D3D's)\n{w}");
}

fn stage() -> impl Strategy<Value = [u32; 10]> {
    // ops 2-26 (not 1, DISABLE, which ends the stages), args 0-6 with COMPLEMENT /
    // ALPHAREPLICATE, RESULTARG CURRENT or TEMP
    let op = 2u32..=26;
    let a = (0u32..=6, any::<bool>(), any::<bool>()).prop_map(|(r, c, ar)| r | if c { 0x10 } else { 0 } | if ar { 0x20 } else { 0 });
    (op.clone(), a.clone(), a.clone(), a.clone(), op, a.clone(), a.clone(), a, prop::bool::ANY, prop::bool::ANY)
        .prop_map(|(co, c1, c2, c0, ao, a1, a2, a0, temp, proj)| [co, c1, c2, c0, ao, a1, a2, a0, if temp { 5 } else { 1 }, proj as u32])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    // Every stage setup a game can make (any op, any arg incl. TEMP, either result register)
    // gives a valid shader, and each stage writes the register its RESULTARG names.
    #[test]
    fn any_stages_valid_and_write_their_resultarg(stages in prop::collection::vec(stage(), 1..=8), tex in prop::array::uniform8(prop_oneof![Just(0u8), Just(2u8)]), spec: bool) {
        let k = PsKey { stages: stages.clone(), tex, specular: spec, ..Default::default() };
        let w = pixel(&k);
        valid(&w);
        let want: Vec<&str> = stages.iter().map(|s| if s[8] & 0xf == 5 { "temp" } else { "current" }).collect();
        prop_assert_eq!(stage_dsts(&w), want);
    }

    // Behind a programmable vertex shader, whatever it writes: a valid shader whose inputs
    // are only locations it writes (WebGPU rejects the pipeline otherwise), the same stage
    // results, and an unwritten input reads as a pixel shader's would (diffuse 1, others 0).
    #[test]
    fn inputs_only_what_the_vertex_shader_writes(stages in prop::collection::vec(stage(), 1..=8), tex in prop::array::uniform8(prop_oneof![Just(0u8), Just(2u8)]), writes in 0u32..0x400, flat: bool) {
        let k = PsKey { stages: stages.clone(), tex, flat, vs_writes: Some(writes), ..Default::default() };
        let w = pixel(&k);
        valid(&w);
        for l in 0..10 {
            let declared = w.contains(&format!(" l{l}: vec4f,"));
            prop_assert_eq!(declared, writes & (1 << l) != 0, "location {} with writes {:#x}\n{}", l, writes, w);
            if writes & (1 << l) == 0 {
                let def = if l == 0 { "vec4f(1.0)" } else { "vec4f(0.0)" };
                prop_assert!(w.contains(&format!("let l{l} = {def};")), "{}", w);
            }
        }
        let want: Vec<&str> = stages.iter().map(|s| if s[8] & 0xf == 5 { "temp" } else { "current" }).collect();
        prop_assert_eq!(stage_dsts(&w), want);
    }
}

/// the stage lines as written: (color, alpha) of each stage
fn stage_exprs(w: &str) -> Vec<(String, String)> {
    w.lines()
        .filter_map(|l| l.trim().strip_prefix("{ let c = clamp("))
        .map(|l| {
            let (c, rest) = l.split_once(", vec4f(-1.0), vec4f(1.0)); let a = clamp(").unwrap();
            (c.to_string(), rest.strip_suffix(", -1.0, 1.0);").unwrap().to_string())
        })
        .collect()
}

// 正しい性奴○の使い方: stage 0 MODULATE TEXTURE DIFFUSE with its texture, stage 1 the same
// with none bound. Windows passes stage 0's result through stage 1 (a stand-in white texture
// gave stage 1 the diffuse: white sprites, a white screen).
#[test]
fn unbound_stage_passes_current_through() {
    let k = PsKey { stages: vec![[4, 2, 0, 1, 4, 2, 0, 1, 1, 0], [4, 2, 0, 1, 4, 2, 0, 1, 1, 0]], tex: [2, 0, 0, 0, 0, 0, 0, 0], ..Default::default() };
    let w = pixel(&k);
    valid(&w);
    assert_eq!(stage_exprs(&w), [("(t0 * diffuse)".into(), "((t0 * diffuse)).w".into()), ("current".into(), "(current).w".into())], "{w}");
    // (a bound texture of another kind, a cube under fixed function: sampled as white)
    let w = pixel(&PsKey { tex: [2, 1, 0, 0, 0, 0, 0, 0], ..k });
    assert_eq!(stage_exprs(&w)[1].0, "(t1 * diffuse)", "{w}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    // Any stages, any texture bound or not: a stage with no texture whose color (alpha) op
    // reads the texture is exactly SELECTARG1 CURRENT for color (alpha), and every other
    // stage is written as its op and args say (the same line as with a texture bound).
    #[test]
    fn unbound_texture_reads_become_current(stages in prop::collection::vec(stage(), 1..=8), tex in prop::array::uniform8(prop_oneof![Just(0u8), Just(1u8), Just(2u8)])) {
        let k = PsKey { stages: stages.clone(), tex, ..Default::default() };
        let w = pixel(&k);
        valid(&w);
        let got = stage_exprs(&w);
        let bound = stage_exprs(&pixel(&PsKey { tex: [2; 8], ..k.clone() }));
        prop_assert_eq!(got.len(), stages.len());
        for (t, s) in stages.iter().enumerate() {
            let none = tex[t] == 0;
            let (c, a) = &got[t];
            if none && no_texture(s[0], s[1], s[2], s[3]) {
                prop_assert_eq!(c, "current", "stage {} {:?}\n{}", t, s, w);
            } else {
                prop_assert_eq!(c, &bound[t].0);
            }
            if none && no_texture(s[4], s[5], s[6], s[7]) {
                prop_assert_eq!(a, "(current).w", "stage {} {:?}\n{}", t, s, w);
            } else {
                prop_assert_eq!(a, &bound[t].1);
            }
        }
    }
}

// F&C's C4 trial: vs_1_1 writing oPos, oD0 and oT0 (locations 0 and 2), two texture stages
// with no pixel shader. Stage 1's coordinates (location 3) and beyond read 0.
#[test]
fn vs11_with_two_stages_links() {
    let k = PsKey {
        stages: vec![[4, 2, 0, 1, 4, 2, 0, 1, 1, 0], [4, 2, 1, 1, 4, 2, 1, 1, 1, 0]],
        tex: [2, 2, 0, 0, 0, 0, 0, 0],
        vs_writes: Some(0b101),
        ..Default::default()
    };
    let w = pixel(&k);
    valid(&w);
    assert!(w.contains("@location(0) l0") && w.contains("@location(2) l2"), "{w}");
    assert!(!w.contains(" l1: vec4f,") && !w.contains(" l3: vec4f,") && !w.contains(" l4: vec4f,"), "{w}");
    assert!(w.contains("let l3 = vec4f(0.0);") && w.contains("let l1 = vec4f(0.0);"), "{w}");
    // (the fixed-function vertex shader: all ten, as before)
    let all = pixel(&PsKey { vs_writes: None, ..k });
    assert!((0..10).all(|l| all.contains(&format!(" l{l}: vec4f,"))), "{all}");
}

fn light_key() -> impl Strategy<Value = (LightKey, [bool; 3])> {
    (0u8..=8, any::<bool>(), any::<bool>(), any::<bool>(), prop::array::uniform4(0u32..=2), any::<bool>(), prop::array::uniform3(any::<bool>())).prop_map(
        |(count, specular, local_viewer, normalize, rs, cv, has)| {
            (LightKey { count, specular, local_viewer, normalize, sources: sources(rs, cv, [has[0], has[1]]) }, has)
        },
    )
}

proptest! {
    /// Lit vertex shaders are valid WGSL for any lights, sources and vertex layout (normal,
    /// diffuse, specular present or not), and read the vertex color only where a source
    /// takes it.
    #[test]
    fn lit_vertex_shaders_are_valid((lk, has) in light_key()) {
        let mut inputs = vec![(USAGE_POSITION, 0, 0, 0)];
        if has[2] { inputs.push((USAGE_NORMAL, 0, 1, 0)); }
        if has[0] { inputs.push((USAGE_COLOR, 0, 2, 3)); }
        if has[1] { inputs.push((USAGE_COLOR, 1, 3, 3)); }
        let k = VsKey { inputs, rhw: false, tci: [(0, 0); 8], flat: false, light: Some(lk) };
        let w = vertex(&k);
        valid(&w);
        prop_assert!(w.contains("ff_light("), "{}", w);
        prop_assert_eq!(w.contains("i.i2.zyxw"), lk.sources.contains(&1), "{}", w);
        prop_assert_eq!(w.contains("i.i3.zyxw"), lk.sources.contains(&2), "{}", w);
        prop_assert_eq!(w.contains("let hv ="), lk.specular, "{}", w);
        // (pretransformed vertices are never lit)
        let rhw = vertex(&VsKey { rhw: true, ..k });
        valid(&rhw);
        prop_assert!(!rhw.contains("ff_light"), "{}", rhw);
    }

    /// The material sources: a vertex color only when COLORVERTEX is on and the vertex has it.
    #[test]
    fn sources_follow_colorvertex(rs in prop::array::uniform4(0u32..=4), cv in any::<bool>(), has in prop::array::uniform2(any::<bool>())) {
        let s = sources(rs, cv, has);
        for i in 0..4 {
            let want = match rs[i] { 1 if cv && has[0] => 1, 2 if cv && has[1] => 2, _ => 0 };
            prop_assert_eq!(s[i], want);
        }
    }

    /// The normals' matrix keeps normals perpendicular to the surface: for a world * view
    /// with any rotation, scale (also uneven) and translation, a normal n of a tangent t
    /// (n . t = 0) stays perpendicular to the transformed tangent t * WV.
    #[test]
    fn normals_stay_perpendicular(a in -3.0f32..3.0, b in -3.0f32..3.0, sx in 0.2f32..4.0, sy in 0.2f32..4.0, sz in 0.2f32..4.0,
                                  t in prop::array::uniform3(-5.0f32..5.0), n in prop::array::uniform3(-1.0f32..1.0), u in prop::array::uniform3(-1.0f32..1.0)) {
        let (ca, sa, cb, sb) = (a.cos(), a.sin(), b.cos(), b.sin());
        // world: scale then rotate about z; view: rotate about x and move
        let world = [sx * ca, sx * sa, 0.0, 0.0, -sy * sa, sy * ca, 0.0, 0.0, 0.0, 0.0, sz, 0.0, 0.0, 0.0, 0.0, 1.0];
        let view = [1.0, 0.0, 0.0, 0.0, 0.0, cb, sb, 0.0, 0.0, -sb, cb, 0.0, t[0], t[1], t[2], 1.0];
        let mut regs = [[0f32; 4]; VS_CONSTS as usize];
        lighting_regs(&mut regs, &[0.0; 17], 0, &world, &view, &[]);
        // a tangent: u minus its part along n
        let nn = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
        prop_assume!(nn > 0.01);
        let k = (u[0] * n[0] + u[1] * n[1] + u[2] * n[2]) / nn;
        let tan = [u[0] - k * n[0], u[1] - k * n[1], u[2] - k * n[2]];
        let wv = mat_mul(&world, &view);
        let tt = row_mul([tan[0], tan[1], tan[2], 0.0], &wv);
        // WGSL's mat3x3f(c22, c23, c24) * n
        let nt: Vec<f32> = (0..3).map(|r| (0..3).map(|c| regs[22 + c][r] * n[c]).sum()).collect();
        let d: f32 = (0..3).map(|i| nt[i] * tt[i]).sum();
        let scale = (nt.iter().map(|x| x * x).sum::<f32>() * tt.iter().take(3).map(|x| x * x).sum::<f32>()).sqrt();
        prop_assert!(d.abs() <= 1e-3 * scale.max(1e-3), "{} {:?} {:?}", d, nt, tt);
    }

    /// D3DLIGHT9's bytes, field by field.
    #[test]
    fn light_from_bytes(v in prop::array::uniform25(-100.0f32..100.0), ty in 1u32..=3) {
        let mut b = ty.to_le_bytes().to_vec();
        for x in v { b.extend_from_slice(&x.to_le_bytes()); }
        prop_assert_eq!(b.len(), 104);
        let l = Light::from_bytes(&b);
        prop_assert_eq!(l.ty, ty);
        prop_assert_eq!(l.diffuse, [v[0], v[1], v[2], v[3]]);
        prop_assert_eq!(l.specular, [v[4], v[5], v[6], v[7]]);
        prop_assert_eq!(l.ambient, [v[8], v[9], v[10], v[11]]);
        prop_assert_eq!(l.position, [v[12], v[13], v[14]]);
        prop_assert_eq!(l.direction, [v[15], v[16], v[17]]);
        prop_assert_eq!((l.range, l.falloff), (v[18], v[19]));
        prop_assert_eq!(l.attenuation, [v[20], v[21], v[22]]);
        prop_assert_eq!((l.theta, l.phi), (v[23], v[24]));
    }
}

// 小悪魔ディーラーのキャンディ体験版 (Ags5 engine) draws its sprites lit: material diffuse
// 0 with the fade in its alpha, emissive white, one directional light. The vertex color is
// the emissive one, not the material's diffuse (black before lighting was done).
#[test]
fn emissive_lights_an_unlit_sprite() {
    let lk = LightKey { count: 1, specular: false, local_viewer: true, normalize: false, sources: sources([1, 2, 0, 0], true, [false, false]) };
    let k = VsKey { inputs: vec![(USAGE_POSITION, 0, 0, 0), (USAGE_NORMAL, 0, 1, 0), (5, 0, 2, 0)], rhw: false, tci: [(0, 0); 8], flat: false, light: Some(lk) };
    let w = vertex(&k);
    valid(&w);
    assert!(w.contains("ff_light(vp.xyz, vn, u.c[12], u.c[13], u.c[14], u.c[15])"), "{w}");
    assert!(w.contains("let d = ce.xyz + ca.xyz * (u.c[16].xyz + amb) + cd.xyz * dif;"), "{w}");
    let mut regs = [[0f32; 4]; VS_CONSTS as usize];
    let mut m = [0f32; 17];
    m[3] = 0.5;
    m[12..15].copy_from_slice(&[1.0; 3]);
    let id: [f32; 16] = std::array::from_fn(|i| if i % 5 == 0 { 1.0 } else { 0.0 });
    let l = Light { ty: 3, diffuse: [1.0; 4], direction: [0.6667, -0.6667, 0.3333], ..Default::default() };
    lighting_regs(&mut regs, &m, 0xff50_5050, &id, &id, &[l]);
    assert_eq!(regs[12], [0.0, 0.0, 0.0, 0.5]);
    assert_eq!(regs[15], [1.0, 1.0, 1.0, 0.0]);
    assert_eq!(regs[16], [80.0 / 255.0, 80.0 / 255.0, 80.0 / 255.0, 1.0]);
    assert_eq!(regs[LIGHT_BASE as usize + 5][3], 3.0);
}
