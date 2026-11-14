use super::*;

/// (vertex WGSL, fragment WGSL, samplers, attributes)
fn translate(vs: &str, ps: &str) -> (String, String, Vec<String>, Vec<(String, u32)>) {
    let p = link(vs, ps, &HashMap::new()).unwrap_or_else(|e| panic!("{e}"));
    let mut samp = p.vs.samplers;
    samp.extend(p.ps.samplers);
    (p.vs.wgsl, p.ps.wgsl, samp, p.attribs)
}

#[test]
fn fixed_function_builtins() {
    // GLSL 1.10 the old way: built-in attributes, matrices and varyings
    let (vw, pw, samp, _) = translate(
        "void main() { gl_TexCoord[0] = gl_MultiTexCoord0; gl_FrontColor = gl_Color;\n\
         gl_Position = gl_ModelViewProjectionMatrix * gl_Vertex; }",
        "uniform sampler2D tex; uniform vec4 tint;\n\
         void main() { gl_FragColor = texture2D(tex, gl_TexCoord[0].st) * gl_Color * tint; }",
    );
    assert!(vw.contains("@vertex") && pw.contains("@fragment"));
    assert_eq!(samp, ["tex"]);
}

#[test]
fn attributes_varyings_uniforms() {
    // what gltest.c and engines with their own shaders write (Ren'Py's gl renderer looks like this)
    let (vw, pw, samp, attribs) = translate(
        "#ifdef GL_ES\nprecision highp float;\n#endif\n\
         uniform mat4 Projection; attribute vec4 Vertex; attribute vec2 VertexTexCoord0;\n\
         varying vec2 vTexCoord0; // comment\n\
         void main() { vTexCoord0 = VertexTexCoord0; gl_Position = Projection * Vertex; }",
        "#ifdef GL_ES\nprecision mediump float;\n#endif\n\
         uniform sampler2D tex0; uniform float Alpha; varying vec2 vTexCoord0;\n\
         /* block\n comment */ void main() { vec4 c = texture2D(tex0, vTexCoord0); gl_FragColor = c * Alpha; }",
    );
    assert!(vw.contains("@vertex") && pw.contains("@fragment"));
    assert_eq!(samp, ["tex0"]);
    let names: Vec<&str> = attribs.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"Vertex") && names.contains(&"VertexTexCoord0"), "{attribs:?}");
}

#[test]
fn glsl_130_in_out() {
    let (vw, pw, _, attribs) = translate(
        "#version 130\nin vec2 pos; in vec3 col; out vec3 v_col; uniform mat4 rot;\n\
         void main() { v_col = col; gl_Position = rot * vec4(pos, 0.0, 1.0); }",
        "#version 130\nin vec3 v_col; uniform vec4 tint; out vec4 color;\n\
         void main() { color = vec4(v_col, 1.0) * tint; }",
    );
    assert!(vw.contains("@vertex") && pw.contains("@fragment"));
    assert_eq!(attribs.len(), 2, "{attribs:?}");
}

#[test]
fn block_layout() {
    let p = link(
        "uniform mat4 m; uniform vec2 offs[3]; attribute vec4 v; void main() { gl_Position = m * v + vec4(offs[1], 0.0, 0.0); }",
        "uniform float a; void main() { gl_FragColor = vec4(a); }",
        &HashMap::new(),
    )
    .unwrap();
    let b = p.vs.block.as_ref().unwrap();
    let offs = b.members.iter().find(|m| m.name == "offs").unwrap();
    assert_eq!((offs.cols, offs.rows, offs.count, offs.stride), (1, 2, Some(3), 16));
    assert_eq!(p.sizes["m"], 16);
    let mut out = vec![0u8; b.size as usize];
    offs.put(&mut out, &[1, 2, 3, 4, 5, 6]);
    let at = |o: u32| u32::from_le_bytes(out[o as usize..o as usize + 4].try_into().unwrap());
    assert_eq!((at(offs.offset + 16), at(offs.offset + 20)), (3, 4));
}
#[test]
fn check_reports_errors_with_the_rewritten_source() {
    assert!(check("void main() { gl_FragColor = vec4(1.0); }", true).is_ok());
    let (msg, src) = check("void main() { gl_FragColor = undefined_thing; }", true).unwrap_err();
    assert!(!msg.is_empty());
    assert!(src.starts_with("#version 450\n") && src.contains("vn_FragColor = undefined_thing"), "{src}");
}

#[test]
fn identifiers() {
    assert!(is_ident('a') && is_ident('Z') && is_ident('7') && is_ident('_'));
    assert!(!is_ident('.') && !is_ident(' ') && !is_ident('['));
}

#[test]
fn comments() {
    assert_eq!(strip_comments("a // b\nc"), "a \nc");
    assert_eq!(strip_comments("a /* b\nc */ d"), "a \n d");
    assert_eq!(strip_comments("a /* b */"), "a ");
    assert_eq!(strip_comments("/**/x"), "x");
    assert_eq!(strip_comments("/* * / */y"), "y");
    assert_eq!(strip_comments("x/y/"), "x/y/");
    assert_eq!(strip_comments("p /* unclosed"), "p ");
    assert_eq!(strip_comments("a/"), "a/");
    assert_eq!(strip_comments("x // tail"), "x ");
    assert_eq!(strip_comments("p /* x*"), "p ");
}

#[test]
fn identifier_mapping() {
    let mut f = |id: &str, rest: &str| (id == "ab").then(|| format!("<{rest}>"));
    assert_eq!(map_idents("ab+ab1 xab _ab ab", &mut f), "<+ab1>+ab1 xab _ab <>");
    assert_eq!(map_idents("1ab ab", &mut f), "1ab <>");
    // gl_TexCoord[n] and friends take their index with them
    let mut g = |id: &str, _: &str| (id == "gl_TexCoord").then(|| "T".to_string());
    assert_eq!(map_idents("gl_TexCoord[1].x", &mut g), "T.x");
    assert_eq!(map_idents("gl_TexCoord", &mut g), "T");
    let mut h = |id: &str, _: &str| (id == "gl_FragData" || id == "gl_TextureMatrix" || id == "gl_MultiTexCoord").then(|| "F".to_string());
    assert_eq!(map_idents("gl_FragData[0] gl_TextureMatrix[2] gl_MultiTexCoord[3]", &mut h), "F F F");
}

#[test]
fn declarations() {
    let d = parse_decl("uniform highp vec4 a, b[2]").unwrap();
    assert_eq!((d.qual.as_str(), d.ty.as_str(), d.names.clone()), ("uniform", "vec4", vec!["a".to_string(), "b[2]".to_string()]));
    let d = parse_decl("flat in int out_like").unwrap();
    assert_eq!((d.qual.as_str(), d.ty.as_str()), ("in", "int"));
    assert!(parse_decl("vec4 f(vec4 x)").is_none());
    assert!(parse_decl("const float k = 1.0").is_none());
    assert!(parse_decl("uniform float k = 1.0").is_none());
    assert!(parse_decl("uniform vec4 f(int)").is_none());
    assert!(parse_decl("uniform Blk { vec4 x").is_none());
    assert!(parse_decl("layout(location = 0) out vec4 c").is_none());
    assert!(parse_decl("uniform").is_none());
    assert_eq!(base_name("x[3]"), "x");
}

fn vs_of(src: &str) -> String {
    rewrite(src, false, &mut Vec::new(), &HashMap::new(), &mut Vec::new()).0
}
fn ps_of(src: &str) -> String {
    rewrite(src, true, &mut Vec::new(), &HashMap::new(), &mut Vec::new()).0
}

#[test]
fn builtins_rewritten() {
    let v = vs_of(
        "void main() { gl_Position = gl_ModelViewMatrix * gl_ProjectionMatrix * gl_Vertex;\n\
         gl_FrontColor = gl_Color; gl_TexCoord[1] = gl_TextureMatrix[2] * gl_MultiTexCoord1;\n\
         vec3 n = gl_NormalMatrix * gl_Normal; }",
    );
    for want in [
        "in vec4 vn_Vertex;",
        "in vec4 vn_Color;",
        "in vec3 vn_Normal;",
        "in vec4 vn_MultiTexCoord1;",
        "out vec4 vn_FrontColor;",
        "out vec4 vn_TexCoord1;",
        "  mat4 vn_MV;\n",
        "  mat4 vn_P;\n",
        "  mat3 vn_NM;\n",
        "  mat4 vn_TM2;\n",
        "vn_TM2 * vn_MultiTexCoord1",
        "vn_NM * vn_Normal",
        "uniform VnU0 {",
        "void vn_main()",
        "gl_Position.y = -gl_Position.y;",
    ] {
        assert!(v.contains(want), "{want:?} not in\n{v}");
    }
    assert!(!v.contains("gl_Vertex") && !v.contains("gl_Color") && !v.contains("[1]"), "{v}");
    let p = ps_of(
        "uniform lowp sampler2D t; void main() { mediump vec4 c = texture2DProj(t, gl_TexCoord[0]) + texture2DLod(t, gl_TexCoord[3].xy, 0.0);\n\
         gl_FragColor = c * gl_Color; }",
    );
    for want in [
        "in vec4 vn_FrontColor;",
        "in vec4 vn_TexCoord0;",
        "in vec4 vn_TexCoord3;",
        "out vec4 vn_FragColor;",
        "textureProj(sampler2D(t_vnt, t_vns), vn_TexCoord0)",
        "textureLod(sampler2D(t_vnt, t_vns), vn_TexCoord3.xy, 0.0)",
        "layout(set = 1, binding = 16) uniform texture2D t_vnt;",
        "layout(set = 1, binding = 160) uniform sampler t_vns;",
        "void main()",
    ] {
        assert!(p.contains(want), "{want:?} not in\n{p}");
    }
    assert!(!p.contains("mediump") && !p.contains("lowp") && !p.contains("vn_main") && !p.contains("gl_Position"), "{p}");
    assert!(!p.contains("in vec4 vn_Color"), "{p}");
    // vertex inputs mean nothing in a pixel shader: left to naga to refuse
    let p = ps_of("void main() { gl_FragColor = vec4(gl_Normal, 1.0) + gl_Vertex; }");
    assert!(p.contains("vec4(gl_Normal, 1.0) + gl_Vertex") && !p.contains("vn_Normal") && !p.contains("vn_Vertex"), "{p}");
}

#[test]
fn preprocessor_lines_and_scopes() {
    let v = vs_of("#version 120\n#extension GL_EXT_foo : enable\n#define K 2.0\n  #ifdef X\nuniform float a;\n#endif\nfloat f() { if (true) { return K; } return a; }\nvoid main() { gl_Position = vec4(f()); }");
    assert!(!v.contains("#version 120") && !v.contains("extension"), "{v}");
    // directives are the preprocessor's: K expanded, the #ifdef X part (X undefined) gone
    assert!(!v.contains('#') || v.matches('#').count() == 1, "{v}");
    // a function body stays one piece, nested braces included, and keeps its statements
    assert!(v.contains("float f() { if (true) { return 2.0 ; } return a; }"), "{v}");
    assert!(!v.contains("float a;"), "{v}");
    let v = vs_of("#define X\n#ifdef X\nuniform float a;\n#else\nuniform float b;\n#endif\nvoid main() { gl_Position = vec4(a); }");
    assert!(v.contains("  float a;\n") && !v.contains("float b"), "{v}");
    // a '#' inside an expression line is not a directive
    let v = vs_of("void main() { gl_Position = vec4(1.0); }\nconst float k = 2.0;");
    assert!(v.contains("const float k = 2.0;\n"), "{v}");
    let v = vs_of("float k = 1.0; #extension x\nvoid main() { gl_Position = vec4(k); }");
    assert!(v.contains("#extension x"), "{v}");
    // declarations after functions and structs are still found
    let v = vs_of("struct S { float x; }; float f() { { return 1.0; } }\nuniform float b; void main() { gl_Position = vec4(f() + b); }");
    assert!(v.contains("  float b;\n") && v.contains("struct S { float x; }") && v.contains("float f() { { return 1.0; } }"), "{v}");
}

#[test]
fn locations_shared_between_stages() {
    let mut attribs = Vec::new();
    let mut vary = Vec::new();
    let mut bound = HashMap::new();
    bound.insert("pos".to_string(), 3);
    let (v, _) = rewrite(
        "attribute vec4 pos; attribute vec4 col; varying vec4 a; varying ivec2 b; void main() { a = col; b = ivec2(1); gl_Position = pos; }",
        false,
        &mut attribs,
        &bound,
        &mut vary,
    );
    assert_eq!(attribs, [("pos".to_string(), 3), ("col".to_string(), 0)]);
    assert!(v.contains("layout(location = 3) in vec4 pos;") && v.contains("layout(location = 0) in vec4 col;"), "{v}");
    assert!(v.contains("layout(location = 0) out vec4 a;") && v.contains("layout(location = 1) flat out ivec2 b;"), "{v}");
    let (p, _) = rewrite("varying ivec2 b; varying vec4 a; void main() { gl_FragColor = a; }", true, &mut attribs, &bound, &mut vary);
    assert!(p.contains("layout(location = 1) flat in ivec2 b;") && p.contains("layout(location = 0) in vec4 a;"), "{p}");
    // GLSL 1.30 fragment outputs
    let p = ps_of("#version 130\nin vec4 a; out vec4 color; void main() { color = a; }");
    assert!(p.contains("layout(location = 0) out vec4 color;") && p.contains("layout(location = 0) in vec4 a;"), "{p}");
    let p = ps_of("flat in uint u; void main() { gl_FragColor = vec4(float(u)); }");
    assert!(p.contains("flat in uint u;"), "{p}");
}

/// EntisGLS (zombie2_trial.zip) sizes uniform arrays with a macro and declares things inside
/// #ifdefs; `rewrite` moves declarations into the header and the block, so the preprocessor
/// runs first (before: UnknownVariable("MAX_BONE_PALETTE"), and the game crashed).
#[test]
fn macros_size_arrays_and_ifdefs_choose_declarations() {
    let vs = "#version 130\n#define MAX_BONE_PALETTE 24\n#define _DISABLE_MORPHING 1\n#ifdef GL_ES\n#define LOWP lowp\n#else\n#define LOWP\n#endif\n\
              #ifdef _GLSL_VERSION_130_LATER_\n#define attribute in\n#endif\n\
              attribute vec3 a_vPos;\n#ifndef _DISABLE_MORPHING\nattribute vec3 a_vMorph;\n#endif\n\
              uniform mat3 u_mat3BoneRotation[MAX_BONE_PALETTE];\nuniform vec4 u_vBoneTranslate[MAX_BONE_PALETTE];\n\
              #define BONE(k) (u_mat3BoneRotation[k] * a_vPos + u_vBoneTranslate[k].xyz)\n\
              void main() { LOWP vec3 p = BONE(MAX_BONE_PALETTE - 1); gl_Position = vec4(p, 1.0); }";
    let ps = "void main() { gl_FragColor = vec4(1.0); }";
    let prog = link(vs, ps, &HashMap::new()).unwrap_or_else(|e| panic!("{e}"));
    let b = prog.vs.block.as_ref().unwrap();
    let m = b.members.iter().find(|m| m.name == "u_mat3BoneRotation").unwrap();
    assert_eq!(m.count, Some(24));
    assert!(prog.attribs.iter().any(|a| a.0 == "a_vPos") && !prog.attribs.iter().any(|a| a.0 == "a_vMorph"), "{:?}", prog.attribs);
}

/// Random shaders whose uniform array sizes, attribute choice and constants come from
/// macros and #if/#ifdef chains translate the same as the same shader written out by hand.
#[test]
fn preprocessed_shaders_match_written_out_ones() {
    let mut seed = 0x1113_2026u64;
    let mut next = |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for case in 0..200 {
        let (n, k, on) = (1 + next(64), next(5), next(2) == 1);
        let macro_src = format!(
            "#define N {n}\n#define K {k}.0\n{}#if defined(USE_B) && N > 0\nuniform vec4 b[N];\n#define PICK b[N - 1]\n#else\nuniform vec4 c[N];\n#define PICK c[0]\n#endif\nvoid main() {{ gl_FragColor = PICK * K; }}",
            if on { "#define USE_B\n" } else { "" }
        );
        let plain = if on { format!("uniform vec4 b[{n}];\nvoid main() {{ gl_FragColor = b[{n} - 1] * {k}.0; }}") } else { format!("uniform vec4 c[{n}];\nvoid main() {{ gl_FragColor = c[0] * {k}.0; }}") };
        let vs = "attribute vec4 p; void main() { gl_Position = p; }";
        let a = link(vs, &macro_src, &HashMap::new()).unwrap_or_else(|e| panic!("case {case}: {e}"));
        let b = link(vs, &plain, &HashMap::new()).unwrap_or_else(|e| panic!("case {case} plain: {e}"));
        let shape = |p: &Program| p.ps.block.as_ref().map(|b| (b.size, b.members.iter().map(|m| (m.name.clone(), m.count, m.offset)).collect::<Vec<_>>()));
        assert_eq!(shape(&a), shape(&b), "case {case}");
    }
}

/// Boolean uniforms (EntisGLS: `uniform bool u_bEnableFog;` failed naga's validation,
/// NonHostShareable) are uint words in the block, read back as bool / bvecN, under their
/// own names for glGetUniformLocation; every type and use combination validates.
#[test]
fn boolean_uniforms_are_words() {
    for (ty, use_) in [("bool", "b ? 1.0 : 0.0"), ("bvec2", "float(b.y)"), ("bvec3", "any(b) ? 1.0 : 0.0"), ("bvec4", "all(b) ? 1.0 : 0.0")] {
        let ps = format!("uniform {ty} b; uniform float f; void main() {{ gl_FragColor = vec4({use_}, f, 0.0, 1.0); }}");
        let p = link("attribute vec4 p; void main() { gl_Position = p; }", &ps, &HashMap::new()).unwrap_or_else(|e| panic!("{ty}: {e}"));
        let b = p.ps.block.as_ref().unwrap();
        let m = b.members.iter().find(|m| m.name == "b").unwrap_or_else(|| panic!("{ty}: no member b"));
        assert_eq!((m.cols, m.count), (1, None), "{ty}");
        assert_eq!(p.sizes["b"], m.rows, "{ty}");
        let mut out = vec![0u8; b.size as usize];
        m.put(&mut out, &vec![1u32; m.rows as usize]);
        assert_eq!(&out[m.offset as usize..m.offset as usize + 4], &1u32.to_le_bytes(), "{ty}");
    }
}

#[test]
fn narrow_and_two_row_uniforms() {
    let v = vs_of("uniform mat2 r; uniform float w[4]; uniform vec3 c[2]; uniform mat2 ra[2]; uniform float s;\n\
                   void main() { gl_Position = vec4(r * vec2(w[1], w[w[0] > 0.0 ? 2 : 3]), c[1].z, s); }");
    for want in ["  mat2x4 r_vn2r;\n", "  vec4 w_vna1[4];\n", "  vec4 c_vna3[2];\n", "  mat2 ra[2];\n", "  float s;\n", "mat2(r_vn2r)", "w_vna1[1].x", "w_vna1[w_vna1[0].x > 0.0 ? 2 : 3].x", "c_vna3[1].xyz.z"] {
        assert!(v.contains(want), "{want:?} not in\n{v}");
    }
    assert_eq!(index_narrow("w [1] + w2[0] + xw[0]", "w", "W", "x"), "W[1].x + w2[0] + xw[0]");
    assert_eq!(index_narrow("w[a[1]] w", "w", "W", "xy"), "W[a[1]].xy w");
    assert_eq!(index_narrow("w[w[1]]", "w", "W", "x"), "W[W[1].x].x");
    assert_eq!(index_narrow("w[", "w", "W", "x"), "W[].x");
    assert_eq!(index_narrow("_w[0]", "_w", "W", "x"), "W[0].x");
}

#[test]
fn block_members_of_every_shape() {
    let p = link(
        "uniform mat2 r; uniform vec3 v3; uniform float fa[3]; uniform mat3x2 m32; attribute vec4 v;\n\
         void main() { gl_Position = v + vec4(r * vec2(fa[0], fa[2]), v3.x, (m32 * vec3(1.0)).x); }",
        "void main() { gl_FragColor = vec4(1.0); }",
        &HashMap::new(),
    )
    .unwrap();
    assert!(p.ps.block.is_none());
    let b = p.vs.block.as_ref().unwrap();
    let m = |n: &str| b.members.iter().find(|m| m.name == n).unwrap_or_else(|| panic!("{n}"));
    assert_eq!((m("r").cols, m("r").rows, m("r").count), (2, 2, None));
    assert_eq!((m("v3").cols, m("v3").rows), (1, 3));
    assert_eq!((m("fa").cols, m("fa").rows, m("fa").count, m("fa").stride), (1, 1, Some(3), 16));
    assert_eq!((m("m32").cols, m("m32").rows), (3, 2));
    assert_eq!((p.sizes["r"], p.sizes["v3"], p.sizes["fa"], p.sizes["m32"]), (4, 3, 1, 6));
    assert_eq!(p.attribs, [("v".to_string(), 0)]);
}

/// Sources that made naga's GLSL front end panic (fuzz/fuzz_targets/glsl.rs): errors now.
#[test]
fn malformed_sources_are_errors() {
    // an array size that is itself indexed: naga looked its index up in the function's
    // expressions instead of the constant ones (vendor/naga/MASU-PATCH.md)
    for src in ["uniform B { float a[2[2]]; };\nvoid main() { gl_FragColor = vec4(1.0); }", "float a[3[1]];\nvoid main() { gl_FragColor = vec4(a[0]); }"] {
        assert!(check(src, true).is_err(), "{src}");
        assert!(check(&src.replace("gl_FragColor", "gl_Position"), false).is_err(), "{src}");
    }
}

#[test]
fn put_packs_std140() {
    let rd = |out: &[u8], o: u32| u32::from_le_bytes(out[o as usize..o as usize + 4].try_into().unwrap());
    // 2-row matrix: columns 16 bytes apart
    let m = Member { name: "m".into(), offset: 8, cols: 2, rows: 2, count: None, stride: 0 };
    let mut out = vec![0u8; 64];
    m.put(&mut out, &[1, 2, 3, 4]);
    assert_eq!([rd(&out, 8), rd(&out, 12), rd(&out, 24), rd(&out, 28)], [1, 2, 3, 4]);
    assert_eq!(rd(&out, 16), 0);
    // vector: packed
    let v = Member { name: "v".into(), offset: 4, cols: 1, rows: 3, count: None, stride: 0 };
    let mut out = vec![0u8; 32];
    v.put(&mut out, &[7, 8, 9]);
    assert_eq!([rd(&out, 4), rd(&out, 8), rd(&out, 12)], [7, 8, 9]);
    // array of mat3x2, short data, write past the end ignored
    let a = Member { name: "a".into(), offset: 0, cols: 3, rows: 2, count: Some(2), stride: 48 };
    let mut out = vec![0u8; 80];
    a.put(&mut out, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!([rd(&out, 0), rd(&out, 4), rd(&out, 16), rd(&out, 20), rd(&out, 32), rd(&out, 36)], [1, 2, 3, 4, 5, 6]);
    assert_eq!([rd(&out, 48), rd(&out, 52), rd(&out, 64), rd(&out, 68)], [7, 8, 9, 0]);
    assert_eq!(a.words(), 6);
}
