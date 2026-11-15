//! Effects: .fx source (what D3DXCreateEffect gets as text) to the compiled fx_2_0 format that
//! d3dx9fx.rs reads (the layout of Wine's d3dx9 effect parser): parameters with their initial
//! values and annotations, techniques, passes and their states, shaders compiled from the
//! passes' compile statements (or assembled from asm blocks) as resources.

use super::ast::*;
use super::Target;

/// D3DXPARAMETER_CLASS
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PClass {
    Scalar,
    Vector,
    MatrixRows,
    MatrixColumns,
    Object,
    Struct,
}

impl PClass {
    fn code(self) -> u32 {
        match self {
            PClass::Scalar => 0,
            PClass::Vector => 1,
            PClass::MatrixRows => 2,
            PClass::MatrixColumns => 3,
            PClass::Object => 4,
            PClass::Struct => 5,
        }
    }
}

/// D3DXPARAMETER_TYPE
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PType {
    Void,
    Bool,
    Int,
    Float,
    String,
    Texture,
    Texture1D,
    Texture3D,
    TextureCube,
    Sampler,
    Sampler1D,
    Sampler2D,
    Sampler3D,
    SamplerCube,
    PixelShader,
    VertexShader,
}

impl PType {
    fn code(self) -> u32 {
        match self {
            PType::Void => 0,
            PType::Bool => 1,
            PType::Int => 2,
            PType::Float => 3,
            PType::String => 4,
            PType::Texture => 5,
            PType::Texture1D => 6,
            PType::Texture3D => 8,
            PType::TextureCube => 9,
            PType::Sampler => 10,
            PType::Sampler1D => 11,
            PType::Sampler2D => 12,
            PType::Sampler3D => 13,
            PType::SamplerCube => 14,
            PType::PixelShader => 15,
            PType::VertexShader => 16,
        }
    }
    /// a sampler (its value: sampler states)
    fn is_sampler(self) -> bool {
        match self {
            PType::Sampler | PType::Sampler1D | PType::Sampler2D | PType::Sampler3D | PType::SamplerCube => true,
            PType::Void | PType::Bool | PType::Int | PType::Float | PType::String | PType::Texture
            | PType::Texture1D | PType::Texture3D | PType::TextureCube | PType::PixelShader | PType::VertexShader => false,
        }
    }
}

/// A type description (D3DX typedef).
#[derive(Clone)]
struct Td {
    ty: PType,
    class: PClass,
    name: String,
    sem: String,
    elements: u32,
    rows: u32,
    cols: u32,
    members: Vec<Td>,
}

impl Td {
    fn scalar(ty: PType) -> Td {
        Td { ty, class: PClass::Scalar, name: String::new(), sem: String::new(), elements: 0, rows: 1, cols: 1, members: Vec::new() }
    }
    fn object(ty: PType) -> Td {
        Td { ty, class: PClass::Object, name: String::new(), sem: String::new(), elements: 0, rows: 0, cols: 0, members: Vec::new() }
    }
    fn named(mut self, n: &str) -> Td {
        self.name = n.into();
        self
    }
    /// base types of each component of one element
    fn bases(&self, out: &mut Vec<PType>) {
        match self.class {
            PClass::Struct => {
                for m in &self.members {
                    for _ in 0..m.elements.max(1) {
                        m.bases(out);
                    }
                }
            }
            PClass::Object => {}
            PClass::Scalar | PClass::Vector | PClass::MatrixRows | PClass::MatrixColumns => {
                out.extend(std::iter::repeat_n(self.ty, kome::num::us(self.rows * self.cols)));
            }
        }
    }
}

fn td_of(prog: &Program, t: &Ty, row_major: bool) -> Td {
    let base = |b: B| match b {
        B::Bool => PType::Bool,
        B::Int | B::Uint => PType::Int,
        B::Float => PType::Float,
    };
    let mut td = match t {
        Ty::Scalar(b) => Td::scalar(base(*b)),
        Ty::Vec(b, n) => Td { class: PClass::Vector, cols: u32::from(*n), ..Td::scalar(base(*b)) },
        Ty::Mat(b, r, c) => Td { class: if row_major { PClass::MatrixRows } else { PClass::MatrixColumns }, rows: u32::from(*r), cols: u32::from(*c), ..Td::scalar(base(*b)) },
        Ty::Array(e, n) => {
            let mut td = td_of(prog, e, row_major);
            td.elements = td.elements.max(1) * n;
            return td;
        }
        Ty::Struct(i) => {
            let st = &prog.structs[*i];
            let members = st.fields.iter().map(|f| td_of(prog, &f.ty, f.row_major.unwrap_or(row_major)).named(&f.name)).collect();
            Td { class: PClass::Struct, members, rows: 0, cols: 0, ..Td::scalar(PType::Void) }
        }
        Ty::Tex(d, _) | Ty::RwTex(d, _) => Td::object(match d {
            Dim::T1D => PType::Texture1D,
            Dim::T3D => PType::Texture3D,
            Dim::Cube => PType::TextureCube,
            Dim::Buffer | Dim::T1DArray | Dim::T2D | Dim::T2DArray | Dim::T2DMS | Dim::T2DMSArray
            | Dim::CubeArray => PType::Texture,
        }),
        Ty::Legacy(d) => Td::object(match d {
            Dim::T1D => PType::Sampler1D,
            Dim::T3D => PType::Sampler3D,
            Dim::Cube => PType::SamplerCube,
            Dim::Buffer | Dim::T1DArray | Dim::T2D | Dim::T2DArray | Dim::T2DMS | Dim::T2DMSArray
            | Dim::CubeArray => PType::Sampler2D,
        }),
        Ty::Sampler(_) => Td::object(PType::Sampler),
        Ty::Str => Td::object(PType::String),
        Ty::Void => Td::scalar(PType::Void),
    };
    td.elements = 0;
    td
}

/// The value of a constant expression, component by component.
fn eval(prog: &Program, e: &Expr) -> Option<Vec<f64>> {
    eval_at(prog, e, 0)
}

/// `vars`: how many static initializers deep (`static float a = b; static float b = a;` would
/// go round for ever)
fn eval_at(prog: &Program, e: &Expr, vars: u32) -> Option<Vec<f64>> {
    let eval = |prog: &Program, e: &Expr| eval_at(prog, e, vars);
    Some(match e {
        Expr::Int(v, _) => vec![kome::num::to_f64(*v)],
        Expr::Float(f) => vec![*f],
        Expr::Bool(b) => vec![f64::from(u32::from(*b))],
        Expr::List(v) | Expr::Construct(_, v) => {
            let mut out = Vec::new();
            for x in v {
                out.extend(eval(prog, x)?);
            }
            if let (Expr::Construct(t, _), 1) = (e, out.len()) {
                let n = super::lower::ncomp(prog, t).max(1);
                out = vec![out[0]; n];
            }
            out
        }
        Expr::Cast(t, a) => {
            let v = eval(prog, a)?;
            let n = super::lower::ncomp(prog, t).max(1);
            if v.len() == 1 {
                vec![v[0]; n]
            } else {
                v.into_iter().take(n).collect()
            }
        }
        Expr::Un(Un::Neg, a) => eval(prog, a)?.into_iter().map(|x| -x).collect(),
        Expr::Bin(op, a, b) => {
            let (x, y) = (eval(prog, a)?, eval(prog, b)?);
            let n = x.len().max(y.len());
            let at = |v: &Vec<f64>, i: usize| if v.len() == 1 { v[0] } else { v.get(i).copied().unwrap_or(0.0) };
            (0..n)
                .map(|i| {
                    let (p, q) = (at(&x, i), at(&y, i));
                    Some(match op {
                        Bin::Add => p + q,
                        Bin::Sub => p - q,
                        Bin::Mul => p * q,
                        Bin::Div => p / q,
                        Bin::Or => kome::num::to_f64(kome::num::trunc_i64(p) | kome::num::trunc_i64(q)),
                        Bin::And => kome::num::to_f64(kome::num::trunc_i64(p) & kome::num::trunc_i64(q)),
                        Bin::Shl => kome::num::to_f64(kome::num::trunc_i64(p).wrapping_shl(kome::num::lo32(kome::num::trunc_i64(q)))),
                        Bin::Mod | Bin::Shr | Bin::Xor | Bin::LAnd | Bin::LOr | Bin::Lt
                        | Bin::Gt | Bin::Le | Bin::Ge | Bin::Eq | Bin::Ne => return None,
                    })
                })
                .collect::<Option<Vec<f64>>>()?
        }
        Expr::Var(n, _) => {
            if n == "TRUE" {
                return Some(vec![1.0]);
            }
            if n == "FALSE" {
                return Some(vec![0.0]);
            }
            let g = prog.globals.iter().find(|g| &g.name == n && g.is_static)?;
            if vars >= 64 {
                return None;
            }
            eval_at(prog, g.init.as_ref()?, vars + 1)?
        }
        Expr::Un(..) | Expr::Assign(..) | Expr::Cond(..) | Expr::Call(..) | Expr::Member(..) | Expr::Method(..)
        | Expr::Index(..) | Expr::Comma(..) | Expr::Str(_) => return None,
    })
}

// ---- states ----

#[derive(Clone, Copy, PartialEq)]
enum K {
    Int,
    Float,
    Bool,
    /// an enum of a kind (ENUMS)
    E(&'static str),
    Shader(bool),
    Texture,
    Sampler,
    /// shader constants: float, bool, int
    ConstF,
    ConstB,
    ConstI,
}

const RS: [(&str, K); 103] = [
    ("ZEnable", K::E("z")),
    ("FillMode", K::E("fill")),
    ("ShadeMode", K::E("shade")),
    ("ZWriteEnable", K::Bool),
    ("AlphaTestEnable", K::Bool),
    ("LastPixel", K::Bool),
    ("SrcBlend", K::E("blend")),
    ("DestBlend", K::E("blend")),
    ("CullMode", K::E("cull")),
    ("ZFunc", K::E("cmp")),
    ("AlphaRef", K::Int),
    ("AlphaFunc", K::E("cmp")),
    ("DitherEnable", K::Bool),
    ("AlphaBlendEnable", K::Bool),
    ("FogEnable", K::Bool),
    ("SpecularEnable", K::Bool),
    ("FogColor", K::Int),
    ("FogTableMode", K::E("fog")),
    ("FogStart", K::Float),
    ("FogEnd", K::Float),
    ("FogDensity", K::Float),
    ("RangeFogEnable", K::Bool),
    ("StencilEnable", K::Bool),
    ("StencilFail", K::E("stencil")),
    ("StencilZFail", K::E("stencil")),
    ("StencilPass", K::E("stencil")),
    ("StencilFunc", K::E("cmp")),
    ("StencilRef", K::Int),
    ("StencilMask", K::Int),
    ("StencilWriteMask", K::Int),
    ("TextureFactor", K::Int),
    ("Wrap0", K::Int),
    ("Wrap1", K::Int),
    ("Wrap2", K::Int),
    ("Wrap3", K::Int),
    ("Wrap4", K::Int),
    ("Wrap5", K::Int),
    ("Wrap6", K::Int),
    ("Wrap7", K::Int),
    ("Wrap8", K::Int),
    ("Wrap9", K::Int),
    ("Wrap10", K::Int),
    ("Wrap11", K::Int),
    ("Wrap12", K::Int),
    ("Wrap13", K::Int),
    ("Wrap14", K::Int),
    ("Wrap15", K::Int),
    ("Clipping", K::Bool),
    ("Lighting", K::Bool),
    ("Ambient", K::Int),
    ("FogVertexMode", K::E("fog")),
    ("ColorVertex", K::Bool),
    ("LocalViewer", K::Bool),
    ("NormalizeNormals", K::Bool),
    ("DiffuseMaterialSource", K::E("material")),
    ("SpecularMaterialSource", K::E("material")),
    ("AmbientMaterialSource", K::E("material")),
    ("EmissiveMaterialSource", K::E("material")),
    ("VertexBlend", K::Int),
    ("ClipPlaneEnable", K::Int),
    ("PointSize", K::Float),
    ("PointSize_Min", K::Float),
    ("PointSize_Max", K::Float),
    ("PointSpriteEnable", K::Bool),
    ("PointScaleEnable", K::Bool),
    ("PointScale_A", K::Float),
    ("PointScale_B", K::Float),
    ("PointScale_C", K::Float),
    ("MultiSampleAntialias", K::Bool),
    ("MultiSampleMask", K::Int),
    ("PatchEdgeStyle", K::Int),
    ("DebugMonitorToken", K::Int),
    ("IndexedVertexBlendEnable", K::Bool),
    ("ColorWriteEnable", K::Int),
    ("TweenFactor", K::Float),
    ("BlendOp", K::E("blendop")),
    ("PositionDegree", K::Int),
    ("NormalDegree", K::Int),
    ("ScissorTestEnable", K::Bool),
    ("SlopeScaleDepthBias", K::Float),
    ("AntialiasedLineEnable", K::Bool),
    ("MinTessellationLevel", K::Float),
    ("MaxTessellationLevel", K::Float),
    ("AdaptiveTess_X", K::Float),
    ("AdaptiveTess_Y", K::Float),
    ("AdaptiveTess_Z", K::Float),
    ("AdaptiveTess_W", K::Float),
    ("EnableAdaptiveTessellation", K::Bool),
    ("TwoSidedStencilMode", K::Bool),
    ("Ccw_StencilFail", K::E("stencil")),
    ("Ccw_StencilZFail", K::E("stencil")),
    ("Ccw_StencilPass", K::E("stencil")),
    ("Ccw_StencilFunc", K::E("cmp")),
    ("ColorWriteEnable1", K::Int),
    ("ColorWriteEnable2", K::Int),
    ("ColorWriteEnable3", K::Int),
    ("BlendFactor", K::Int),
    ("SRGBWriteEnable", K::Bool),
    ("DepthBias", K::Float),
    ("SeparateAlphaBlendEnable", K::Bool),
    ("SrcBlendAlpha", K::E("blend")),
    ("DestBlendAlpha", K::E("blend")),
    ("BlendOpAlpha", K::E("blendop")),
];

const TSS: [(&str, K); 18] = [
    ("ColorOp", K::E("top")),
    ("ColorArg0", K::E("ta")),
    ("ColorArg1", K::E("ta")),
    ("ColorArg2", K::E("ta")),
    ("AlphaOp", K::E("top")),
    ("AlphaArg0", K::E("ta")),
    ("AlphaArg1", K::E("ta")),
    ("AlphaArg2", K::E("ta")),
    ("ResultArg", K::E("ta")),
    ("BumpEnvMat00", K::Float),
    ("BumpEnvMat01", K::Float),
    ("BumpEnvMat10", K::Float),
    ("BumpEnvMat11", K::Float),
    ("TexCoordIndex", K::Int),
    ("BumpEnvLScale", K::Float),
    ("BumpEnvLOffset", K::Float),
    ("TextureTransformFlags", K::Int),
    ("Constant", K::Int),
];

const SAMP: [(&str, K); 13] = [
    ("AddressU", K::E("address")),
    ("AddressV", K::E("address")),
    ("AddressW", K::E("address")),
    ("BorderColor", K::Int),
    ("MagFilter", K::E("filter")),
    ("MinFilter", K::E("filter")),
    ("MipFilter", K::E("filter")),
    ("MipMapLodBias", K::Float),
    ("MaxMipLevel", K::Int),
    ("MaxAnisotropy", K::Int),
    ("SRGBTexture", K::Bool),
    ("ElementIndex", K::Int),
    ("DMapOffset", K::Int),
];

const ENUMS: &[(&str, &[(&str, u32)])] = &[
    ("z", &[("UseW", 2)]),
    ("fill", &[("Point", 1), ("Wireframe", 2), ("Solid", 3)]),
    ("shade", &[("Flat", 1), ("Gouraud", 2), ("Phong", 3)]),
    (
        "blend",
        &[
            ("Zero", 1),
            ("One", 2),
            ("SrcColor", 3),
            ("InvSrcColor", 4),
            ("SrcAlpha", 5),
            ("InvSrcAlpha", 6),
            ("DestAlpha", 7),
            ("InvDestAlpha", 8),
            ("DestColor", 9),
            ("InvDestColor", 10),
            ("SrcAlphaSat", 11),
            ("BothSrcAlpha", 12),
            ("BothInvSrcAlpha", 13),
            ("BlendFactor", 14),
            ("InvBlendFactor", 15),
        ],
    ),
    ("blendop", &[("Add", 1), ("Subtract", 2), ("RevSubtract", 3), ("Min", 4), ("Max", 5)]),
    ("cull", &[("None", 1), ("CW", 2), ("CCW", 3)]),
    ("cmp", &[("Never", 1), ("Less", 2), ("Equal", 3), ("LessEqual", 4), ("Greater", 5), ("NotEqual", 6), ("GreaterEqual", 7), ("Always", 8)]),
    ("stencil", &[("Keep", 1), ("Zero", 2), ("Replace", 3), ("IncrSat", 4), ("DecrSat", 5), ("Invert", 6), ("Incr", 7), ("Decr", 8)]),
    ("fog", &[("None", 0), ("Exp", 1), ("Exp2", 2), ("Linear", 3)]),
    ("material", &[("Material", 0), ("Color1", 1), ("Color2", 2)]),
    ("filter", &[("None", 0), ("Point", 1), ("Linear", 2), ("Anisotropic", 3), ("PyramidalQuad", 6), ("GaussianQuad", 7)]),
    ("address", &[("Wrap", 1), ("Mirror", 2), ("Clamp", 3), ("Border", 4), ("MirrorOnce", 5)]),
    (
        "top",
        &[
            ("Disable", 1),
            ("SelectArg1", 2),
            ("SelectArg2", 3),
            ("Modulate", 4),
            ("Modulate2x", 5),
            ("Modulate4x", 6),
            ("Add", 7),
            ("AddSigned", 8),
            ("AddSigned2x", 9),
            ("Subtract", 10),
            ("AddSmooth", 11),
            ("BlendDiffuseAlpha", 12),
            ("BlendTextureAlpha", 13),
            ("BlendFactorAlpha", 14),
            ("BlendTextureAlphaPM", 15),
            ("BlendCurrentAlpha", 16),
            ("PreModulate", 17),
            ("ModulateAlpha_AddColor", 18),
            ("ModulateColor_AddAlpha", 19),
            ("ModulateInvAlpha_AddColor", 20),
            ("ModulateInvColor_AddAlpha", 21),
            ("BumpEnvMap", 22),
            ("BumpEnvMapLuminance", 23),
            ("DotProduct3", 24),
            ("MultiplyAdd", 25),
            ("Lerp", 26),
        ],
    ),
    ("ta", &[("Diffuse", 0), ("Current", 1), ("Texture", 2), ("TFactor", 3), ("Specular", 4), ("Temp", 5), ("Constant", 6)]),
];

/// A state's operation number (d3dx9fx's table) and value kind.
fn state_op(name: &str) -> Option<(u32, K)> {
    let eq = |a: &str| a.eq_ignore_ascii_case(name);
    if let Some(i) = RS.iter().position(|x| eq(x.0)) {
        return Some((kome::num::u32_of(i), RS[i].1));
    }
    if let Some(i) = TSS.iter().position(|x| eq(x.0)) {
        return Some((103 + kome::num::u32_of(i), TSS[i].1));
    }
    if let Some(i) = SAMP.iter().position(|x| eq(x.0)) {
        return Some((165 + kome::num::u32_of(i), SAMP[i].1));
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "fvf" => (122, K::Int),
        "vertexshader" => (146, K::Shader(true)),
        "pixelshader" => (147, K::Shader(false)),
        "vertexshaderconstantf"
        | "vertexshaderconstant"
        | "vertexshaderconstant1"
        | "vertexshaderconstant2"
        | "vertexshaderconstant3"
        | "vertexshaderconstant4" => (148, K::ConstF),
        "vertexshaderconstantb" => (149, K::ConstB),
        "vertexshaderconstanti" => (150, K::ConstI),
        "pixelshaderconstantf" | "pixelshaderconstant" | "pixelshaderconstant1" | "pixelshaderconstant2" | "pixelshaderconstant3" | "pixelshaderconstant4" => {
            (156, K::ConstF)
        }
        "pixelshaderconstantb" => (157, K::ConstB),
        "pixelshaderconstanti" => (158, K::ConstI),
        "texture" => (164, K::Texture),
        "sampler" => (178, K::Sampler),
        _ => return None,
    })
}

fn enum_value(kind: &str, name: &str) -> Option<u32> {
    if name.eq_ignore_ascii_case("true") {
        return Some(1);
    }
    if name.eq_ignore_ascii_case("false") {
        return Some(0);
    }
    let (_, list) = ENUMS.iter().find(|e| e.0 == kind)?;
    let n = name.trim_start_matches("D3DBLEND_").trim_start_matches("D3DCULL_").trim_start_matches("D3DCMP_").trim_start_matches("D3DTEXF_");
    let n = n.trim_start_matches("D3DTADDRESS_").trim_start_matches("D3DBLENDOP_").trim_start_matches("D3DTOP_").trim_start_matches("D3DTA_");
    let n = n.trim_start_matches("D3DSTENCILOP_");
    list.iter().find(|x| x.0.eq_ignore_ascii_case(n)).map(|x| x.1)
}

// ---- writing ----

struct W<'a> {
    prog: &'a Program,
    pre: &'a Program,
    flags: u32,
    /// the area offsets point into (starts with an empty name at 0)
    b: Vec<u8>,
    objects: u32,
    shaders: u32,
    /// (object id, data) and resources (technique or -1, pass or parameter, element, state,
    /// usage, data)
    strings: Vec<(u32, Vec<u8>)>,
    res: Vec<(u32, u32, u32, u32, u32, Vec<u8>)>,
    /// top-level parameter names (references)
    params: Vec<String>,
    warnings: Vec<String>,
}

impl W<'_> {
    fn put(&mut self, v: u32) {
        self.b.extend_from_slice(&v.to_le_bytes());
    }
    fn at(&self) -> u32 {
        kome::num::u32_of(self.b.len())
    }
    fn name(&mut self, s: &str) -> u32 {
        if s.is_empty() {
            return 0;
        }
        let at = self.at();
        self.put(kome::num::u32_of(s.len()) + 1);
        self.b.extend_from_slice(s.as_bytes());
        self.b.push(0);
        while self.b.len() % 4 != 0 {
            self.b.push(0);
        }
        at
    }
    fn names(&mut self, t: &Td, out: &mut Vec<(u32, u32)>) {
        let n = self.name(&t.name);
        let s = self.name(&t.sem);
        out.push((n, s));
        for m in &t.members {
            self.names(m, out);
        }
    }
    fn td_words(t: &Td, names: &mut std::slice::Iter<(u32, u32)>, out: &mut Vec<u32>) {
        let Some(&(n, s)) = names.next() else { return };
        out.extend([t.ty.code(), t.class.code(), n, s, t.elements]);
        match t.class {
            PClass::Vector => out.extend([t.cols, t.rows]),
            PClass::Scalar | PClass::MatrixRows | PClass::MatrixColumns => out.extend([t.rows, t.cols]),
            PClass::Struct => {
                out.push(kome::num::u32_of(t.members.len()));
                for m in &t.members {
                    Self::td_words(m, names, out);
                }
            }
            PClass::Object => {}
        }
    }
    fn typedef(&mut self, t: &Td) -> u32 {
        let mut names = Vec::new();
        self.names(t, &mut names);
        let mut words = Vec::new();
        Self::td_words(t, &mut names.iter(), &mut words);
        let at = self.at();
        for w in words {
            self.put(w);
        }
        at
    }
    fn object(&mut self) -> u32 {
        self.objects += 1;
        self.objects - 1
    }
    /// numeric value bytes of `t` (all its elements) from component values
    fn numbers(t: &Td, vals: &[f64]) -> Vec<u8> {
        let mut bases = Vec::new();
        t.bases(&mut bases);
        let n = bases.len();
        let mut out = Vec::new();
        for e in 0..kome::num::us(t.elements.max(1)) {
            for (k, b) in bases.iter().enumerate() {
                let i = e * n + k;
                let v = if vals.len() == 1 { vals[0] } else { vals.get(i).copied().unwrap_or(0.0) };
                let w = match *b {
                    PType::Float => (kome::num::to_f32(v)).to_bits(),
                    PType::Bool => u32::from(v != 0.0),
                    PType::Void | PType::Int | PType::String | PType::Texture | PType::Texture1D | PType::Texture3D
                    | PType::TextureCube | PType::Sampler | PType::Sampler1D | PType::Sampler2D | PType::Sampler3D
                    | PType::SamplerCube | PType::PixelShader | PType::VertexShader => kome::num::lo32(kome::num::trunc_i64(v)),
                };
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        out
    }
    /// A value: numbers, or object ids / sampler states per element; its offset.
    fn value(&mut self, t: &Td, init: Option<&Expr>, states: &[FxState], top: u32) -> Result<u32, String> {
        let bytes = if t.class == PClass::Object {
            let mut b = Vec::new();
            for e in 0..t.elements.max(1) {
                if t.ty.is_sampler() {
                    let recs = self.states(states, u32::MAX, top, if t.elements > 0 { e } else { u32::MAX }, true)?;
                    b.extend_from_slice(&(kome::num::u32_of(recs.len()) / 4).to_le_bytes());
                    for w in recs {
                        b.extend_from_slice(&w.to_le_bytes());
                    }
                } else {
                    let id = self.object();
                    if t.ty == PType::String {
                        let s = match init {
                            Some(Expr::Str(s)) => s.clone(),
                            _ => String::new(),
                        };
                        let mut d = s.into_bytes();
                        d.push(0);
                        self.strings.push((id, d));
                    }
                    b.extend_from_slice(&id.to_le_bytes());
                }
            }
            b
        } else {
            let vals = match init {
                Some(e) => eval(self.prog, e).unwrap_or_else(|| {
                    self.warnings.push(format!("initial value of {} is not a constant", t.name));
                    Vec::new()
                }),
                None => Vec::new(),
            };
            Self::numbers(t, &vals)
        };
        let at = self.at();
        self.b.extend_from_slice(&bytes);
        Ok(at)
    }
    fn annotations(&mut self, annos: &[Anno]) -> Result<Vec<u32>, String> {
        let mut out = Vec::new();
        for a in annos {
            let t = td_of(self.prog, &a.ty, false).named(&a.name);
            let td = self.typedef(&t);
            let v = self.value(&t, Some(&a.value), &[], 0)?;
            out.extend([td, v]);
        }
        Ok(out)
    }
    /// A compiled shader (or none: an error is a warning, and the pass gets no shader).
    fn shader(&mut self, v: &FxVal, vertex: bool) -> Option<Vec<u8>> {
        match v {
            FxVal::Compile(target, entry, args) => {
                let r = super::parse_target(target).and_then(|t: Target| {
                    if t.pixel == vertex {
                        return Err(format!("{target} for a {} shader", if vertex { "vertex" } else { "pixel" }));
                    }
                    let s = super::lower_entry(self.prog, self.pre, entry, args, &t, self.flags)?;
                    super::finish(s, &t)
                });
                match r {
                    Ok(b) => Some(b),
                    Err(e) => {
                        self.warnings.push(format!("{entry} ({target}): {e}"));
                        None
                    }
                }
            }
            FxVal::Asm(text) => match crate::d3d9asm::assemble(text) {
                Ok(w) => Some(w.iter().flat_map(|x| x.to_le_bytes()).collect()),
                Err(e) => {
                    self.warnings.push(format!("asm: {e}"));
                    None
                }
            },
            FxVal::Expr(_) | FxVal::Ref(_) | FxVal::Null => None,
        }
    }
    /// State records (op, index, typedef, value: 4 words each); `ti` the technique (u32::MAX for
    /// a sampler's states, then `index` is the parameter and `elem` its element).
    fn states(&mut self, states: &[FxState], ti: u32, index: u32, elem: u32, sampler: bool) -> Result<Vec<u32>, String> {
        let mut out = Vec::new();
        for st in states {
            let Some((op, kind)) = state_op(&st.name) else {
                self.warnings.push(format!("line {}: state {} is not supported", st.line, st.name));
                continue;
            };
            let si = kome::num::u32_of(out.len() / 4);
            let idx = st.index.unwrap_or(0);
            // a reference to a parameter: <name>, (name) or name
            let reference = match &st.value {
                FxVal::Ref(n) => Some(n.clone()),
                FxVal::Expr(Expr::Var(n, _)) if self.params.contains(n) => Some(n.clone()),
                FxVal::Expr(_) | FxVal::Compile(..) | FxVal::Asm(_) | FxVal::Null => None,
            };
            let (td, val): (Td, Vec<u8>) = match (kind, &st.value) {
                (K::Shader(vs), v) => {
                    let td = Td::object(if vs { PType::VertexShader } else { PType::PixelShader });
                    let id = self.object();
                    if reference.is_none() && let Some(code) = self.shader(v, vs) {
                        self.shaders += 1;
                        self.res.push((ti, index, elem, si, 0, code));
                    }
                    (td, id.to_le_bytes().to_vec())
                }
                (K::Texture, _) => (Td::object(PType::Texture), self.object().to_le_bytes().to_vec()),
                (K::Sampler, _) => (Td::object(PType::Sampler), 0u32.to_le_bytes().to_vec()),
                _ if reference.is_some() => (Td::scalar(PType::Int), vec![0; 4]),
                (K::ConstF | K::ConstB | K::ConstI, FxVal::Expr(e)) => {
                    let v = eval(self.prog, e).ok_or(format!("line {}: {} is not a constant", st.line, st.name))?;
                    let ty = match kind {
                        K::ConstB => PType::Bool,
                        K::ConstI => PType::Int,
                        K::Int | K::Float | K::Bool | K::E(_) | K::Shader(_) | K::Texture | K::Sampler | K::ConstF => PType::Float,
                    };
                    let rows = kome::num::u32_of(v.len().div_ceil(4).max(1));
                    let td = Td { class: PClass::MatrixRows, rows, cols: 4, ..Td::scalar(ty) };
                    (td.clone(), Self::numbers(&td, &v))
                }
                (_, FxVal::Expr(e)) => {
                    let v = match (kind, e) {
                        (K::E(k), Expr::Var(n, _)) => {
                            vec![f64::from(enum_value(k, n).ok_or(format!("line {}: {n} is not a value of {}", st.line, st.name))?)]
                        }
                        (_, Expr::Var(n, _)) if n.eq_ignore_ascii_case("true") || n.eq_ignore_ascii_case("false") => {
                            vec![f64::from(u32::from(n.eq_ignore_ascii_case("true")))]
                        }
                        _ => eval(self.prog, e).ok_or(format!("line {}: the value of {} is not a constant", st.line, st.name))?,
                    };
                    let ty = match kind {
                        K::Float => PType::Float,
                        K::Bool => PType::Bool,
                        K::Int | K::E(_) | K::Shader(_) | K::Texture | K::Sampler | K::ConstF | K::ConstB | K::ConstI => PType::Int,
                    };
                    let td = Td::scalar(ty);
                    (td.clone(), Self::numbers(&td, &v[..1.min(v.len())]))
                }
                (_, v) => return Err(format!("line {}: bad value for state {} ({v:?})", st.line, st.name)),
            };
            if let Some(n) = &reference {
                let mut d = n.clone().into_bytes();
                d.push(0);
                self.res.push((ti, index, elem, si, 1, d));
            }
            let _ = sampler;
            let t = self.typedef(&td);
            let v = self.at();
            self.b.extend_from_slice(&val);
            out.extend([op, idx, t, v]);
        }
        Ok(out)
    }
}

/// Program `prog` (with its effect parts) as a compiled effect.
pub fn effect(prog: &Program, pre: &Program, flags: u32) -> Result<Vec<u8>, String> {
    let row_major = flags & super::PACK_MATRIX_ROW_MAJOR != 0;
    let mut w = W { prog, pre, flags, b: vec![0; 4], objects: 0, shaders: 0, strings: Vec::new(), res: Vec::new(), params: Vec::new(), warnings: Vec::new() };
    // parameters: the non-static globals (and constant buffers' variables), shader variables
    let mut vars: Vec<&Var> = prog.globals.iter().filter(|g| !g.is_static).collect();
    for cb in &prog.cbufs {
        vars.extend(cb.vars.iter());
    }
    w.params = vars.iter().map(|v| v.name.clone()).chain(prog.shaders.iter().map(|s| s.0.clone())).collect();
    let mut recs: Vec<u32> = Vec::new();
    let mut nparams = 0;
    for (k, v) in vars.iter().enumerate() {
        let mut t = td_of(prog, v.ty(), v.row_major.unwrap_or(row_major)).named(&v.name);
        t.sem = v.sem.clone().unwrap_or_default();
        let td = w.typedef(&t);
        let val = w.value(&t, v.init.as_ref(), &v.states, kome::num::u32_of(k))?;
        let annos = w.annotations(&v.annos)?;
        recs.extend([td, val, 0, kome::num::u32_of(annos.len() / 2)]);
        recs.extend(annos);
        nparams += 1;
    }
    for (name, vertex, val, annos) in &prog.shaders {
        let t = Td::object(if *vertex { PType::VertexShader } else { PType::PixelShader }).named(name);
        let td = w.typedef(&t);
        let id = w.object();
        if let Some(code) = w.shader(val, *vertex) {
            w.shaders += 1;
            w.strings.push((id, code));
        }
        let at = w.at();
        w.put(id);
        let annos = w.annotations(annos)?;
        recs.extend([td, at, 0, kome::num::u32_of(annos.len() / 2)]);
        recs.extend(annos);
        nparams += 1;
    }
    // techniques
    for (ti, t) in prog.techniques.iter().enumerate() {
        let n = w.name(&t.name);
        let annos = w.annotations(&t.annos)?;
        recs.extend([n, kome::num::u32_of(annos.len() / 2), kome::num::u32_of(t.passes.len())]);
        recs.extend(annos);
        for (pi, p) in t.passes.iter().enumerate() {
            let n = w.name(&p.name);
            let annos = w.annotations(&p.annos)?;
            let states = w.states(&p.states, kome::num::u32_of(ti), kome::num::u32_of(pi), u32::MAX, false)?;
            recs.extend([n, kome::num::u32_of(annos.len() / 2), kome::num::u32_of(states.len() / 4)]);
            recs.extend(annos);
            recs.extend(states);
        }
    }
    for m in &w.warnings {
        eprintln!("sake: effect: {m}");
    }
    let start = w.at();
    let (objects, shaders) = (w.objects, w.shaders);
    w.put(nparams);
    w.put(kome::num::u32_of(prog.techniques.len()));
    w.put(shaders);
    w.put(objects);
    for r in recs {
        w.put(r);
    }
    let strings = std::mem::take(&mut w.strings);
    let res = std::mem::take(&mut w.res);
    w.put(kome::num::u32_of(strings.len()));
    w.put(kome::num::u32_of(res.len()));
    let blob = |w: &mut W, d: &[u8]| {
        w.put(kome::num::u32_of(d.len()));
        w.b.extend_from_slice(d);
        while w.b.len() % 4 != 0 {
            w.b.push(0);
        }
    };
    for (id, d) in &strings {
        w.put(*id);
        blob(&mut w, d);
    }
    for (ti, index, elem, si, usage, d) in &res {
        for x in [*ti, *index, *elem, *si, *usage] {
            w.put(x);
        }
        blob(&mut w, d);
    }
    let mut out = 0xfeff0901u32.to_le_bytes().to_vec();
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&w.b);
    Ok(out)
}
