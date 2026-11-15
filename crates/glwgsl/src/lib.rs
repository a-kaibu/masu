//! GLSL (OpenGL 2.x, GLSL 1.10-1.30 with the compatibility built-ins) to WGSL: the sources are
//! rewritten into Vulkan-style GLSL 450 (locations, sets and bindings, uniforms gathered into
//! one block per stage, samplers split into texture + sampler, built-ins as our own `vn_*`
//! inputs, outputs and uniforms), naga makes WGSL of that.
//!
//! [`link`] takes a program's vertex and fragment source and gives each stage's WGSL, its
//! uniform block layout ([`Member::put`] writes glUniform data into it) and samplers.

use kome::num::us;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// One linked stage
pub struct Stage {
    pub wgsl: String,
    /// the stage's uniform block (group = stage, binding 0)
    pub block: Option<Block>,
    /// sampler uniforms in binding order (texture 1 + 2k, sampler 2 + 2k)
    pub samplers: Vec<String>,
}

pub struct Block {
    pub members: Vec<Member>,
    pub size: u32,
}

/// A uniform in a block: `count` elements (None: not an array) of `cols` x `rows` 32-bit values
/// (a vector is 1 x n, a scalar 1 x 1).
pub struct Member {
    pub name: String,
    pub offset: u32,
    pub cols: u32,
    pub rows: u32,
    pub count: Option<u32>,
    /// array stride
    pub stride: u32,
}

pub struct Program {
    pub vs: Stage,
    pub ps: Stage,
    /// vertex inputs: (name, location)
    pub attribs: Vec<(String, u32)>,
    /// uniform name -> 32-bit words per element (samplers: 1)
    pub sizes: HashMap<String, u32>,
}

/// glLinkProgram: vertex and fragment source, glBindAttribLocation's locations.
pub fn link(vs: &str, ps: &str, bound: &HashMap<String, u32>) -> Result<Program, String> {
    let mut attribs = Vec::new();
    let mut varyings = Vec::new();
    let (vsrc, vsamp) = rewrite(vs, false, &mut attribs, bound, &mut varyings);
    let (psrc, psamp) = rewrite(ps, true, &mut attribs, bound, &mut varyings);
    let (vw, vm) = compile(&vsrc, false).map_err(|e| format!("vertex shader: {e}\n{vsrc}"))?;
    let (pw, pm) = compile(&psrc, true).map_err(|e| format!("fragment shader: {e}\n{psrc}"))?;
    let (vb, pb) = (block_of(&vm), block_of(&pm));
    let mut sizes = HashMap::new();
    for b in [&vb, &pb].into_iter().flatten() {
        for m in &b.members {
            sizes.insert(m.name.clone(), m.words());
        }
    }
    for s in vsamp.iter().chain(&psamp) {
        sizes.insert(s.clone(), 1);
    }
    Ok(Program {
        vs: Stage { wgsl: vw, block: vb, samplers: vsamp },
        ps: Stage { wgsl: pw, block: pb, samplers: psamp },
        attribs,
        sizes,
    })
}

/// glCompileShader: one stage on its own (linking does it again with the other stage);
/// Err((message, the rewritten GLSL 450)).
pub fn check(src: &str, pixel: bool) -> Result<(), (String, String)> {
    let (src, _) = rewrite(src, pixel, &mut Vec::new(), &HashMap::new(), &mut Vec::new());
    compile(&src, pixel).map(|_| ()).map_err(|e| (e, src))
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
        } else if b[i] == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                if b[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

/// Replaces identifiers by `f` (None: keep).
fn map_idents(s: &str, f: &mut dyn FnMut(&str, &str) -> Option<String>) -> String {
    let mut out = String::with_capacity(s.len());
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == '_') && (i == 0 || !is_ident(b[i - 1])) {
            let st = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            let id: String = b[st..i].iter().collect();
            let rest: String = b[i..b.len().min(i + 4)].iter().collect();
            match f(&id, &rest) {
                Some(r) => {
                    out.push_str(&r);
                    // gl_TexCoord[n] -> vn_TexCoordn eats "[n]"
                    if id == "gl_TexCoord" || id == "gl_TextureMatrix" || id == "gl_FragData" || id == "gl_MultiTexCoord" {
                        if let Some(close) = b[i..].iter().position(|&c| c == ']') {
                            i += close + 1;
                        }
                    }
                }
                None => out.push_str(&id),
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

struct Decl {
    qual: String,
    ty: String,
    names: Vec<String>,
}

/// A global declaration "qualifiers type a, b[2];" (None: something else).
fn parse_decl(stmt: &str) -> Option<Decl> {
    let words: Vec<&str> = stmt.split_whitespace().collect();
    let quals = ["attribute", "varying", "uniform", "in", "out"];
    let skip = ["highp", "mediump", "lowp", "centroid", "flat", "smooth", "noperspective", "invariant", "const"];
    let mut q = None;
    let mut rest = Vec::new();
    for w in &words {
        if quals.contains(w) && q.is_none() {
            q = Some(w.to_string());
        } else if !skip.contains(w) {
            rest.push(*w);
        }
    }
    let q = q?;
    if rest.first()?.starts_with("layout") {
        return None;
    }
    let ty = rest.first()?.to_string();
    let names = rest[1..].join(" ");
    if names.contains('(') || names.contains('{') || names.contains('=') {
        return None;
    }
    let names = names.split(',').map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    Some(Decl { qual: q, ty, names })
}

fn base_name(n: &str) -> &str {
    n.split('[').next().unwrap_or(n).trim()
}

/// Rewrites one stage. `attribs`: vertex input locations by name (assigned as found when
/// missing); `varyings`: locations by name, shared by both stages.
/// The GLSL preprocessor run on its own (#define, #if/#ifdef/#elif/#else/#endif, macro
/// expansion; #version, #extension and #pragma dropped): `rewrite` moves declarations into the
/// header and the uniform block, away from the #defines and #ifs around them, so it gets source
/// without either (EntisGLS: `uniform mat3 u_mat3BoneRotation[MAX_BONE_PALETTE];` met an
/// undefined macro; declarations inside an #ifdef were moved out of it). A source the
/// preprocessor rejects is passed on as it is.
fn preprocess(src: &str) -> String {
    use pp_rs::token::{Punct as P, TokenValue as T};
    let mut out = String::with_capacity(src.len());
    let mut prev: Option<usize> = None;
    for item in pp_rs::pp::Preprocessor::new(src) {
        let Ok(t) = item else { return src.to_string() };
        let text = match &t.value {
            T::Version(_) | T::Extension(_) | T::Pragma(_) => continue,
            T::Ident(s) => s.clone(),
            T::Integer(i) => format!("{}{}", i.value, if i.signed { "" } else { "u" }),
            // (the literal as written where the token has its own text, else the value)
            T::Float(f) => match src.get(us(t.location.start)..us(t.location.end)) {
                Some(s) if s.parse::<f64>().is_ok() || s.trim_end_matches(['f', 'F']).parse::<f64>().is_ok() => s.trim_end_matches(['f', 'F']).to_string(),
                _ => format!("{:?}", f.value),
            },
            T::Punct(p) => match p {
                P::AddAssign => "+=", P::SubAssign => "-=", P::MulAssign => "*=", P::DivAssign => "/=", P::ModAssign => "%=",
                P::LeftShiftAssign => "<<=", P::RightShiftAssign => ">>=", P::AndAssign => "&=", P::XorAssign => "^=", P::OrAssign => "|=",
                P::Increment => "++", P::Decrement => "--", P::LogicalAnd => "&&", P::LogicalOr => "||", P::LogicalXor => "^^",
                P::LessEqual => "<=", P::GreaterEqual => ">=", P::EqualEqual => "==", P::NotEqual => "!=", P::LeftShift => "<<", P::RightShift => ">>",
                P::LeftBrace => "{", P::RightBrace => "}", P::LeftParen => "(", P::RightParen => ")", P::LeftBracket => "[", P::RightBracket => "]",
                P::LeftAngle => "<", P::RightAngle => ">", P::Semicolon => ";", P::Comma => ",", P::Colon => ":", P::Dot => ".", P::Equal => "=",
                P::Bang => "!", P::Minus => "-", P::Tilde => "~", P::Plus => "+", P::Star => "*", P::Slash => "/", P::Percent => "%",
                P::Pipe => "|", P::Caret => "^", P::Ampersand => "&", P::Question => "?",
            }
            .to_string(),
        };
        // (the spacing of the source where the tokens follow each other there, so `w[4]` stays
        // one piece for `rewrite`; a space where a macro or a directive came between)
        let (start, end) = (us(t.location.start), us(t.location.end));
        match prev.and_then(|p| src.get(p..start)) {
            Some(gap) if gap.chars().all(char::is_whitespace) => out.push_str(if gap.is_empty() { "" } else if gap.contains('\n') { "\n" } else { " " }),
            _ if prev.is_some() => out.push(' '),
            _ => {}
        }
        out.push_str(&text);
        prev = Some(end);
    }
    out
}

fn rewrite(src: &str, pixel: bool, attribs: &mut Vec<(String, u32)>, bound: &HashMap<String, u32>, varyings: &mut Vec<String>) -> (String, Vec<String>) {
    let set = u32::from(pixel);
    let src = strip_comments(&preprocess(src));
    let mut head = String::from("#version 450\n");
    let mut body = String::new();
    let mut block = String::new();
    let mut samplers: Vec<String> = Vec::new();
    let mut two_rows: Vec<(String, &str)> = Vec::new();
    let mut bools: Vec<(String, String)> = Vec::new();
    let mut narrow_arrays: Vec<(String, &str)> = Vec::new();
    let mut extra_in: Vec<(String, String)> = Vec::new();
    let mut extra_out: Vec<(String, String)> = Vec::new();
    let mut extra_uni: Vec<(String, String)> = Vec::new();
    let mut frag_out = false;

    let att_loc = |name: &str, attribs: &mut Vec<(String, u32)>| -> u32 {
        if let Some(a) = attribs.iter().find(|a| a.0 == name) {
            return a.1;
        }
        let l = bound.get(name).copied().unwrap_or_else(|| (0..16).find(|l| !attribs.iter().any(|a| a.1 == *l) && !bound.values().any(|b| b == l)).unwrap_or(15));
        attribs.push((name.to_string(), l));
        l
    };
    let var_loc = |name: &str, varyings: &mut Vec<String>| -> usize {
        match varyings.iter().position(|v| v == name) {
            Some(i) => i,
            None => {
                varyings.push(name.to_string());
                varyings.len() - 1
            }
        }
    };

    // built-ins
    let src = map_idents(&src, &mut |id, rest| {
        let digit = rest.chars().nth(1).filter(|c| c.is_ascii_digit());
        Some(match id {
            "gl_Vertex" if !pixel => {
                extra_in.push(("vec4".into(), "vn_Vertex".into()));
                "vn_Vertex".into()
            }
            "gl_Color" if !pixel => {
                extra_in.push(("vec4".into(), "vn_Color".into()));
                "vn_Color".into()
            }
            "gl_Normal" if !pixel => {
                extra_in.push(("vec3".into(), "vn_Normal".into()));
                "vn_Normal".into()
            }
            _ if id.starts_with("gl_MultiTexCoord") && !pixel => {
                let n = format!("vn_{}", &id[3..]);
                extra_in.push(("vec4".into(), n.clone()));
                n
            }
            "gl_Color" | "gl_FrontColor" | "gl_BackColor" => {
                let n = "vn_FrontColor".to_string();
                if pixel { extra_in.push(("vec4".into(), n.clone())) } else { extra_out.push(("vec4".into(), n.clone())) }
                n
            }
            "gl_TexCoord" => {
                let n = format!("vn_TexCoord{}", digit.unwrap_or('0'));
                if pixel { extra_in.push(("vec4".into(), n.clone())) } else { extra_out.push(("vec4".into(), n.clone())) }
                n
            }
            "gl_FragColor" | "gl_FragData" => {
                frag_out = true;
                "vn_FragColor".into()
            }
            "gl_ModelViewProjectionMatrix" => {
                extra_uni.push(("mat4".into(), "vn_MVP".into()));
                "vn_MVP".into()
            }
            "gl_ModelViewMatrix" => {
                extra_uni.push(("mat4".into(), "vn_MV".into()));
                "vn_MV".into()
            }
            "gl_ProjectionMatrix" => {
                extra_uni.push(("mat4".into(), "vn_P".into()));
                "vn_P".into()
            }
            "gl_NormalMatrix" => {
                extra_uni.push(("mat3".into(), "vn_NM".into()));
                "vn_NM".into()
            }
            "gl_TextureMatrix" => {
                let n = format!("vn_TM{}", digit.unwrap_or('0'));
                extra_uni.push(("mat4".into(), n.clone()));
                n
            }
            "texture2D" | "textureCube" | "texture1D" | "shadow2D" => "texture".into(),
            "texture2DProj" => "textureProj".into(),
            "texture2DLod" => "textureLod".into(),
            "highp" | "mediump" | "lowp" => String::new(),
            _ => return None,
        })
    });

    // global statements
    let chars: Vec<char> = src.chars().collect();
    let mut depth = 0;
    let mut stmt = String::new();
    let mut i = 0;
    let mut line_start = true;
    while i < chars.len() {
        let ch = chars[i];
        if depth == 0 && line_start && ch == '#' {
            let end = chars[i..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| i + p);
            let line: String = chars[i..end].iter().collect();
            if !line.trim_start_matches('#').trim_start().starts_with("version") && !line.contains("extension") {
                body.push_str(&line);
                body.push('\n');
            }
            i = end;
            continue;
        }
        line_start = ch == '\n' || (line_start && ch.is_whitespace());
        match ch {
            '{' => {
                depth += 1;
                stmt.push(ch);
            }
            '}' => {
                depth -= 1;
                stmt.push(ch);
                if depth == 0 {
                    body.push_str(&stmt);
                    stmt.clear();
                }
            }
            ';' if depth == 0 => {
                let s = stmt.trim().to_string();
                stmt.clear();
                if s.starts_with("precision") {
                } else if let Some(d) = parse_decl(&s) {
                    for n in &d.names {
                        let b = base_name(n);
                        match d.qual.as_str() {
                            "uniform" if d.ty.starts_with("sampler") => samplers.push(b.to_string()),
                            "uniform" => {
                                // (2-row matrices have no std140 layout in WGSL: kept as 4-row ones
                                // of which the shader reads the top two rows, see `TWO_ROWS`)
                                // (arrays of scalars and short vectors: elements 16 bytes apart in
                                // std140, which WGSL cannot say of them; kept as vec4 arrays, see
                                // `NARROW`)
                                let narrow = NARROW.iter().find(|t| t.0 == d.ty).filter(|_| n.contains('['));
                                // (booleans have no host-shareable layout: uint words the shader
                                // turns back into bool, see `BOOLS`)
                                if let Some(&(_, u)) = BOOLS.iter().find(|t| t.0 == d.ty).filter(|_| !n.contains('[')) {
                                    let _ = writeln!(block, "  {u} {n}{BOOL_SUFFIX};");
                                    bools.push((n.clone(), d.ty.clone()));
                                    continue;
                                }
                                match (TWO_ROWS.iter().find(|t| t.0 == d.ty && !n.contains('[')), narrow) {
                                    (Some(&(_, wide, gl)), _) => {
                                        let _ = writeln!(block, "  {wide} {n}{TWO_ROWS_SUFFIX};");
                                        two_rows.push((n.clone(), gl));
                                    }
                                    (None, Some(&(_, wide, swz))) => {
                                        let b = base_name(n);
                                        let _ = writeln!(block, "  {wide} {b}{NARROW_SUFFIX}{}{};", swz.len(), &n[b.len()..]);
                                        narrow_arrays.push((b.to_string(), swz));
                                    }
                                    _ => {
                                        let _ = writeln!(block, "  {} {};", d.ty, n);
                                    }
                                }
                            }
                            "attribute" | "in" if !pixel => {
                                let l = att_loc(b, attribs);
                                let _ = writeln!(head, "layout(location = {l}) in {} {};", d.ty, n);
                            }
                            "varying" | "out" if !pixel => {
                                let l = var_loc(b, varyings);
                                let flat = if d.ty.starts_with('i') || d.ty.starts_with('u') { "flat " } else { "" };
                                let _ = writeln!(head, "layout(location = {l}) {flat}out {} {};", d.ty, n);
                            }
                            "varying" | "in" => {
                                let l = var_loc(b, varyings);
                                let flat = if d.ty.starts_with('i') || d.ty.starts_with('u') { "flat " } else { "" };
                                let _ = writeln!(head, "layout(location = {l}) {flat}in {} {};", d.ty, n);
                            }
                            _ => {
                                // fragment output (GLSL 1.30)
                                let _ = writeln!(head, "layout(location = 0) out {} {};", d.ty, n);
                            }
                        }
                    }
                } else {
                    body.push_str(&s);
                    body.push_str(";\n");
                }
            }
            _ => stmt.push(ch),
        }
        i += 1;
    }
    body.push_str(&stmt);

    // built-in declarations
    let mut seen = HashSet::new();
    for (ty, n) in &extra_in {
        if seen.insert(n.clone()) {
            if pixel {
                let l = var_loc(n, varyings);
                let _ = writeln!(head, "layout(location = {l}) in {ty} {n};");
            } else {
                let l = att_loc(n, attribs);
                let _ = writeln!(head, "layout(location = {l}) in {ty} {n};");
            }
        }
    }
    for (ty, n) in &extra_out {
        if seen.insert(n.clone()) {
            let l = var_loc(n, varyings);
            let _ = writeln!(head, "layout(location = {l}) out {ty} {n};");
        }
    }
    for (ty, n) in &extra_uni {
        if seen.insert(n.clone()) {
            let _ = writeln!(block, "  {ty} {n};");
        }
    }
    if frag_out {
        head.push_str("layout(location = 0) out vec4 vn_FragColor;\n");
    }
    if !block.is_empty() {
        let _ = write!(head, "layout(set = {set}, binding = 0) uniform VnU{set} {{\n{block}}};\n");
    }
    // samplers: texture + sampler, uses as sampler2D(t, s)
    for (k, s) in samplers.iter().enumerate() {
        let _ = writeln!(head, "layout(set = {set}, binding = {}) uniform texture2D {s}_vnt;", 16 + k);
        let _ = writeln!(head, "layout(set = {set}, binding = {}) uniform sampler {s}_vns;", 160 + k);
    }
    let mut body = body;
    for (n, swz) in &narrow_arrays {
        body = index_narrow(&body, n, &format!("{n}{NARROW_SUFFIX}{}", swz.len()), swz);
    }
    let body = map_idents(&body, &mut |id, _| {
        if samplers.iter().any(|s| s == id) {
            Some(format!("sampler2D({id}_vnt, {id}_vns)"))
        } else if let Some((_, ty)) = bools.iter().find(|t| t.0 == id) {
            Some(format!("{ty}({id}{BOOL_SUFFIX})"))
        } else if let Some((_, gl)) = two_rows.iter().find(|t| t.0 == id) {
            Some(format!("{gl}({id}{TWO_ROWS_SUFFIX})"))
        } else if id == "main" && !pixel {
            Some("vn_main".into())
        } else {
            None
        }
    });
    let mut out = head;
    out.push_str(&body);
    if !pixel {
        out.push_str("\nvoid main() {\n  vn_main();\n  gl_Position.y = -gl_Position.y;\n  gl_Position.z = (gl_Position.z + gl_Position.w) * 0.5;\n}\n");
    }
    (out, samplers)
}

/// GLSL 450 -> (WGSL, module)
fn compile(src: &str, pixel: bool) -> Result<(String, naga::Module), String> {
    let stage = if pixel { naga::ShaderStage::Fragment } else { naga::ShaderStage::Vertex };
    // (no catch_unwind: sake is built with panic=abort, so a panic here ends the game. naga's
    // GLSL front end panics that game shaders reach are fixed in vendor/naga, found by
    // fuzz/fuzz_targets/glsl.rs; regressions in tests::malformed_sources_are_errors)
    let m = naga::front::glsl::Frontend::default().parse(&naga::front::glsl::Options::from(stage), src).map_err(|e| format!("{e:?}"))?;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&m)
        .map_err(|e| format!("naga validation: {e:?}"))?;
    let text = naga::back::wgsl::write_string(&m, &info, naga::back::wgsl::WriterFlags::empty()).map_err(|e| format!("naga wgsl-out: {e}"))?;
    Ok((format!("diagnostic(off, derivative_uniformity);\n{text}"), m))
}

/// Uniform matrices with two rows: (GLSL type, the 4-row type the block holds, the constructor
/// that takes its top rows back).
const TWO_ROWS: [(&str, &str, &str); 4] = [("mat2", "mat2x4", "mat2"), ("mat2x2", "mat2x4", "mat2"), ("mat3x2", "mat3x4", "mat3x2"), ("mat4x2", "mat4x4", "mat4x2")];
const TWO_ROWS_SUFFIX: &str = "_vn2r";
/// Array element types narrower than 16 bytes: (GLSL type, the vec4 type the block holds, the
/// components the shader reads).
const NARROW: [(&str, &str, &str); 6] =
    [("float", "vec4", "x"), ("vec2", "vec4", "xy"), ("vec3", "vec4", "xyz"), ("int", "ivec4", "x"), ("ivec2", "ivec4", "xy"), ("ivec3", "ivec4", "xyz")];
const NARROW_SUFFIX: &str = "_vna";
/// Boolean uniforms (not host-shareable in WGSL): (GLSL type, the uint type the block holds).
/// glUniform*i / *f write their 32-bit words; any non-zero word is true, as GL reads them.
const BOOLS: [(&str, &str); 4] = [("bool", "uint"), ("bvec2", "uvec2"), ("bvec3", "uvec3"), ("bvec4", "uvec4")];
const BOOL_SUFFIX: &str = "_vnb";

/// `name[i]` -> `wide[i].swz` in `s` (indices rewritten too).
fn index_narrow(s: &str, name: &str, wide: &str, swz: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == '_') && (i == 0 || !is_ident(b[i - 1])) {
            let st = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            let id: String = b[st..i].iter().collect();
            let mut j = i;
            while j < b.len() && b[j].is_whitespace() {
                j += 1;
            }
            if id == name && j < b.len() && b[j] == '[' {
                let mut depth = 0;
                let mut k = j;
                while k < b.len() {
                    match b[k] {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    k += 1;
                }
                let inner: String = b[j + 1..k.min(b.len())].iter().collect();
                out.push_str(&format!("{wide}[{}].{swz}", index_narrow(&inner, name, wide, swz)));
                i = k + 1;
            } else {
                out.push_str(&id);
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

// (a u32 offset or count as an index: lossless on every target, wasm32 and 64-bit hosts)
const _: () = assert!(usize::BITS >= u32::BITS);

fn lanes(s: naga::VectorSize) -> u32 {
    match s {
        naga::VectorSize::Bi => 2,
        naga::VectorSize::Tri => 3,
        naga::VectorSize::Quad => 4,
    }
}

fn block_of(m: &naga::Module) -> Option<Block> {
    let shape = |t: &naga::TypeInner| match t {
        naga::TypeInner::Vector { size, .. } => (1, lanes(*size)),
        naga::TypeInner::Matrix { columns, rows, .. } => (lanes(*columns), lanes(*rows)),
        // (a GLSL uniform's element: a scalar; the rest do not occur in a uniform block)
        naga::TypeInner::Scalar(_)
        | naga::TypeInner::CooperativeMatrix { .. }
        | naga::TypeInner::Atomic(_)
        | naga::TypeInner::Pointer { .. }
        | naga::TypeInner::ValuePointer { .. }
        | naga::TypeInner::Array { .. }
        | naga::TypeInner::Struct { .. }
        | naga::TypeInner::Image { .. }
        | naga::TypeInner::Sampler { .. }
        | naga::TypeInner::AccelerationStructure { .. }
        | naga::TypeInner::RayQuery { .. }
        | naga::TypeInner::BindingArray { .. } => (1, 1),
    };
    for (_, g) in m.global_variables.iter() {
        if g.space != naga::AddressSpace::Uniform {
            continue;
        }
        if let naga::TypeInner::Struct { members, span } = &m.types[g.ty].inner {
            let members = members
                .iter()
                .map(|x| {
                    let inner = &m.types[x.ty].inner;
                    let (cols, rows, count, stride) = if let naga::TypeInner::Array { base, size, stride } = inner {
                        let n = match size {
                            naga::ArraySize::Constant(n) => n.get(),
                            naga::ArraySize::Pending(_) | naga::ArraySize::Dynamic => 1,
                        };
                        let (c, r) = shape(&m.types[*base].inner);
                        (c, r, Some(n), *stride)
                    } else {
                        let (c, r) = shape(inner);
                        (c, r, None, 0)
                    };
                    let name = x.name.clone().unwrap_or_default();
                    let narrow = name.rfind(NARROW_SUFFIX).filter(|&i| name[i + NARROW_SUFFIX.len()..].parse::<u32>().is_ok());
                    if let Some(n) = name.strip_suffix(BOOL_SUFFIX) {
                        return Member { name: n.to_string(), offset: x.offset, cols, rows, count, stride };
                    }
                    match (name.strip_suffix(TWO_ROWS_SUFFIX), narrow) {
                        (Some(n), _) => Member { name: n.to_string(), offset: x.offset, cols, rows: 2, count, stride },
                        (None, Some(i)) => {
                            let rows = name[i + NARROW_SUFFIX.len()..].parse().unwrap_or(4);
                            Member { name: name[..i].to_string(), offset: x.offset, cols: 1, rows, count, stride }
                        }
                        (None, None) => Member { name, offset: x.offset, cols, rows, count, stride },
                    }
                })
                .collect();
            return Some(Block { members, size: *span });
        }
    }
    None
}

impl Member {
    /// 32-bit words one element takes in GL (glUniform data)
    pub fn words(&self) -> u32 {
        self.cols * self.rows
    }

    /// Writes GL-packed `v` (elements one after another) into the block `out` (std140-style).
    pub fn put(&self, out: &mut [u8], v: &[u32]) {
        let mut w = |o: u32, x: u32| {
            let o = us(o);
            if let Some(d) = out.get_mut(o..o + 4) {
                d.copy_from_slice(&x.to_le_bytes());
            }
        };
        let ew = us(self.words());
        // matrix columns are 16 bytes apart (std140, 2-row ones too); vectors and scalars are packed
        let cstride = if self.cols == 1 { 0 } else { 16 };
        for k in 0..self.count.unwrap_or(1) {
            let base = self.offset + k * self.stride;
            let s = &v[(us(k) * ew).min(v.len())..((us(k) + 1) * ew).min(v.len())];
            for (i, &x) in (0u32..).zip(s) {
                let (ci, ri) = (i / self.rows, i % self.rows);
                w(base + ci * cstride + ri * 4, x);
            }
        }
    }
}

#[cfg(test)]
mod tests;
