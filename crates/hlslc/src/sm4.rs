//! Shader model 4/5 code: instructions on virtual temporaries, their register allocation, the
//! token encoding and the DXBC container (RDEF, ISGN, OSGN, SHDR/SHEX).

pub use dxwgsl::tpf::{OpType, Opcode};
use dxwgsl::tpf;
use super::ast::BufKind;

/// A register file and index. Temps are virtual until `finish`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum RegF {
    Temp(u32),
    Input(u32),
    Output(u32),
    /// constant buffer (symbol, register)
    Cb(u32, u32),
    Icb(u32),
    /// indexable temporary (array, register)
    X(u32, u32),
    Depth,
    Null,
    /// compute shader inputs without an index: vThreadID (32), vThreadGroupID (33),
    /// vThreadIDInGroup (34), vThreadIDInGroupFlattened (35)
    /// a compute shader's system input (vThreadID ..)
    Sys(OpType),
    /// a typed unordered access view (symbol): store_uav_typed's destination
    Uav(u32),
    /// thread group shared memory (symbol): store_raw's destination
    G(u32),
    /// a geometry shader's input: v[vertex][register]
    GsInput(u32, u32),
    /// a hull or domain shader's input control point: vicp[point][register]
    Icp(u32, u32),
    /// a hull shader's output control point (read by its patch constant phase):
    /// vocp[point][register]
    Ocp(u32, u32),
    /// a domain shader's patch constant input: vpc[register]
    Pc(u32),
}

/// One component of a four-component register: no value outside x..w can be named.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Lane {
    X,
    Y,
    Z,
    W,
}

impl Lane {
    pub const ALL: [Lane; 4] = [Lane::X, Lane::Y, Lane::Z, Lane::W];

    /// the lane of a component counted across registers (4 per register)
    pub fn of(k: usize) -> Lane {
        match k % 4 {
            0 => Lane::X,
            1 => Lane::Y,
            2 => Lane::Z,
            _ => Lane::W,
        }
    }

    /// x, y, .. for the first `n` (at most 4) components
    pub fn first(n: usize) -> impl Iterator<Item = Lane> {
        Lane::ALL.into_iter().take(n)
    }

    /// the write mask of the first `n` (at most 4) components
    pub fn mask(n: usize) -> u8 {
        Lane::first(n).fold(0, |m, l| m | 1 << u8::from(l))
    }
}

impl From<Lane> for u8 {
    fn from(l: Lane) -> u8 {
        match l {
            Lane::X => 0,
            Lane::Y => 1,
            Lane::Z => 2,
            Lane::W => 3,
        }
    }
}

impl From<Lane> for u32 {
    fn from(l: Lane) -> u32 {
        u32::from(u8::from(l))
    }
}

impl From<Lane> for usize {
    fn from(l: Lane) -> usize {
        usize::from(u8::from(l))
    }
}

/// A register operand with an optional relative index (a temp's component) added to the last
/// index.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Reg {
    pub f: RegF,
    pub rel: Option<(u32, Lane)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    R { r: Reg, swz: [u8; 4], neg: bool, abs: bool, one: bool },
    Imm([u32; 4], bool),
    /// texture t (symbol), with a swizzle of its result
    Tex(u32, [u8; 4]),
    Samp(u32),
    /// a sampler with a component select (gather4: the channel gathered)
    SampC(u32, u8),
    /// a typed UAV (symbol) with a swizzle of its result (ld_uav_typed)
    Uav(u32, [u8; 4]),
    /// thread group shared memory (symbol) with a swizzle: the dwords ld_raw reads
    G(u32, [u8; 4]),
    /// a geometry shader's output stream m<n> (emit_stream, cut_stream)
    Stream(u32),
}

#[derive(Clone, Debug)]
pub struct Dst {
    pub r: Reg,
    pub mask: u8,
}

#[derive(Clone, Debug)]
pub struct Ins {
    pub op: Opcode,
    pub sat: bool,
    pub nz: bool,
    /// opcode-specific control bits (11..23)
    pub ctl: u32,
    pub dst: Vec<Dst>,
    pub src: Vec<Src>,
    pub offset: [i32; 3],
}

// opcodes
pub const ADD: Opcode = Opcode::Add;
pub const EMIT: Opcode = Opcode::Emit;
pub const CUT: Opcode = Opcode::Cut;
pub const EMIT_STREAM: Opcode = Opcode::EmitStream;
pub const CUT_STREAM: Opcode = Opcode::CutStream;
pub const AND: Opcode = Opcode::And;
pub const BREAK: Opcode = Opcode::Break;
pub const BREAKC: Opcode = Opcode::Breakc;
pub const CASE: Opcode = Opcode::Case;
pub const CONTINUE: Opcode = Opcode::Continue;
pub const DEFAULT: Opcode = Opcode::Default;
pub const DERIV_RTX: Opcode = Opcode::DerivRtx;
pub const DERIV_RTY: Opcode = Opcode::DerivRty;
pub const DISCARD: Opcode = Opcode::Discard;
pub const DIV: Opcode = Opcode::Div;
pub const DP2: Opcode = Opcode::Dp2;
pub const DP3: Opcode = Opcode::Dp3;
pub const DP4: Opcode = Opcode::Dp4;
pub const ELSE: Opcode = Opcode::Else;
pub const ENDIF: Opcode = Opcode::EndIf;
pub const ENDLOOP: Opcode = Opcode::EndLoop;
pub const ENDSWITCH: Opcode = Opcode::EndSwitch;
pub const EQ: Opcode = Opcode::Eq;
pub const EXP: Opcode = Opcode::Exp;
pub const FRC: Opcode = Opcode::Frc;
pub const FTOI: Opcode = Opcode::Ftoi;
pub const FTOU: Opcode = Opcode::Ftou;
pub const GE: Opcode = Opcode::Ge;
pub const IADD: Opcode = Opcode::Iadd;
pub const IF: Opcode = Opcode::If;
pub const IEQ: Opcode = Opcode::Ieq;
pub const IGE: Opcode = Opcode::Ige;
pub const ILT: Opcode = Opcode::Ilt;
pub const IMAD: Opcode = Opcode::Imad;
pub const IMAX: Opcode = Opcode::Imax;
pub const IMIN: Opcode = Opcode::Imin;
pub const IMUL: Opcode = Opcode::Imul;
pub const INE: Opcode = Opcode::Ine;
pub const INEG: Opcode = Opcode::Ineg;
pub const ISHL: Opcode = Opcode::Ishl;
pub const ISHR: Opcode = Opcode::Ishr;
pub const ITOF: Opcode = Opcode::Itof;
pub const LD: Opcode = Opcode::Ld;
pub const LD_MS: Opcode = Opcode::LdMs;
pub const LOG: Opcode = Opcode::Log;
pub const LOOP: Opcode = Opcode::Loop;
pub const LT: Opcode = Opcode::Lt;
pub const MAD: Opcode = Opcode::Mad;
pub const MIN: Opcode = Opcode::Min;
pub const MAX: Opcode = Opcode::Max;
pub const MOV: Opcode = Opcode::Mov;
pub const MOVC: Opcode = Opcode::Movc;
pub const MUL: Opcode = Opcode::Mul;
pub const NE: Opcode = Opcode::Ne;
pub const NOT: Opcode = Opcode::Not;
pub const OR: Opcode = Opcode::Or;
pub const RESINFO: Opcode = Opcode::ResInfo;
pub const SAMPLE_POS: Opcode = Opcode::SamplePos;
pub const SAMPLE_INFO: Opcode = Opcode::SampleInfo;
pub const RET: Opcode = Opcode::Ret;
pub const ROUND_NE: Opcode = Opcode::RoundNe;
pub const ROUND_NI: Opcode = Opcode::RoundNi;
pub const ROUND_PI: Opcode = Opcode::RoundPi;
pub const ROUND_Z: Opcode = Opcode::RoundZ;
pub const RSQ: Opcode = Opcode::Rsq;
pub const SAMPLE: Opcode = Opcode::Sample;
pub const SAMPLE_C: Opcode = Opcode::SampleC;
pub const SAMPLE_C_LZ: Opcode = Opcode::SampleCLz;
pub const SAMPLE_L: Opcode = Opcode::SampleL;
pub const SAMPLE_D: Opcode = Opcode::SampleD;
pub const SAMPLE_B: Opcode = Opcode::SampleB;
pub const SQRT: Opcode = Opcode::Sqrt;
pub const SWITCH: Opcode = Opcode::Switch;
pub const SINCOS: Opcode = Opcode::SinCos;
pub const UDIV: Opcode = Opcode::Udiv;
pub const ULT: Opcode = Opcode::Ult;
pub const UGE: Opcode = Opcode::Uge;
pub const UMUL: Opcode = Opcode::Umul;
pub const UMAX: Opcode = Opcode::Umax;
pub const UMIN: Opcode = Opcode::Umin;
pub const USHR: Opcode = Opcode::Ushr;
pub const UTOF: Opcode = Opcode::Utof;
pub const XOR: Opcode = Opcode::Xor;
pub const GATHER4: Opcode = Opcode::Gather4;
pub const RCP: Opcode = Opcode::Rcp;
pub const F32TOF16: Opcode = Opcode::F32ToF16;
pub const F16TOF32: Opcode = Opcode::F16ToF32;
pub const COUNTBITS: Opcode = Opcode::CountBits;
pub const FIRSTBIT_HI: Opcode = Opcode::FirstBitHi;
pub const FIRSTBIT_LO: Opcode = Opcode::FirstBitLo;
pub const FIRSTBIT_SHI: Opcode = Opcode::FirstBitShi;
pub const BFREV: Opcode = Opcode::Bfrev;
pub const LD_UAV_TYPED: Opcode = Opcode::LdUavTyped;
pub const STORE_UAV_TYPED: Opcode = Opcode::StoreUavTyped;
pub const LD_RAW: Opcode = Opcode::LdRaw;
pub const STORE_RAW: Opcode = Opcode::StoreRaw;
pub const LD_STRUCTURED: Opcode = Opcode::LdStructured;
pub const STORE_STRUCTURED: Opcode = Opcode::StoreStructured;
pub const BUFINFO: Opcode = Opcode::BufInfo;
pub const IMM_ATOMIC_ALLOC: Opcode = Opcode::ImmAtomicAlloc;
pub const IMM_ATOMIC_CONSUME: Opcode = Opcode::ImmAtomicConsume;
/// sync: ctl 1 threads in the group, 2 shared memory, 4 UAVs (group), 8 UAVs (global)
pub const SYNC: Opcode = Opcode::Sync;

/// A signature element's D3D_NAME: the system values hlslc declares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SysVal {
    /// a generic (user) semantic
    Undefined,
    Position,
    ClipDistance,
    VertexId,
    PrimitiveId,
    InstanceId,
    IsFrontFace,
    SampleIndex,
    /// (in the signature only; declared as a plain output)
    Target,
    Depth,
    /// SV_TessFactor / SV_InsideTessFactor of a patch: which one
    Tess(TessFactor),
}

/// A tess factor of a patch constant signature: D3D_NAME FINAL_*_TESSFACTOR in the signature,
/// D3D10_SB_NAME FINAL_*_TESSFACTOR (each edge its own) in the declarations.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TessFactor {
    /// SV_TessFactor[i] of a quad: the U == 0, V == 0, U == 1, V == 1 edges
    QuadEdge(QuadEdge),
    /// SV_InsideTessFactor[0] (U), [1] (V) of a quad
    QuadInsideU,
    QuadInsideV,
    /// SV_TessFactor[i] of a tri: the U == 0, V == 0, W == 0 edges
    TriEdge(TriEdge),
    TriInside,
    /// SV_TessFactor[0] of an isoline: the line detail
    LineDetail,
    /// SV_TessFactor[1] of an isoline: the line density
    LineDensity,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuadEdge {
    U0,
    V0,
    U1,
    V1,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriEdge {
    U0,
    V0,
    W0,
}

/// D3D11_SB_TESSELLATOR_DOMAIN (dcl_tessellator_domain): 1 isoline, 2 tri, 3 quad
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TessDomain {
    Isoline,
    Tri,
    Quad,
}

impl TessDomain {
    pub fn of_name(s: &str) -> Option<TessDomain> {
        Some(match s {
            "isoline" => TessDomain::Isoline,
            "tri" => TessDomain::Tri,
            "quad" => TessDomain::Quad,
            _ => return None,
        })
    }
    pub fn code(self) -> u32 {
        match self {
            TessDomain::Isoline => 1,
            TessDomain::Tri => 2,
            TessDomain::Quad => 3,
        }
    }
    /// SV_TessFactor's elements, SV_InsideTessFactor's
    pub fn factors(self) -> (u32, u32) {
        match self {
            TessDomain::Isoline => (2, 0),
            TessDomain::Tri => (3, 1),
            TessDomain::Quad => (4, 2),
        }
    }
    /// The tess factor SV_TessFactor[index] (`inside`: SV_InsideTessFactor[index]) is; None past
    /// the domain's.
    pub fn factor(self, inside: bool, index: u32) -> Option<TessFactor> {
        Some(match (self, inside, index) {
            (TessDomain::Quad, false, 0) => TessFactor::QuadEdge(QuadEdge::U0),
            (TessDomain::Quad, false, 1) => TessFactor::QuadEdge(QuadEdge::V0),
            (TessDomain::Quad, false, 2) => TessFactor::QuadEdge(QuadEdge::U1),
            (TessDomain::Quad, false, 3) => TessFactor::QuadEdge(QuadEdge::V1),
            (TessDomain::Quad, true, 0) => TessFactor::QuadInsideU,
            (TessDomain::Quad, true, 1) => TessFactor::QuadInsideV,
            (TessDomain::Tri, false, 0) => TessFactor::TriEdge(TriEdge::U0),
            (TessDomain::Tri, false, 1) => TessFactor::TriEdge(TriEdge::V0),
            (TessDomain::Tri, false, 2) => TessFactor::TriEdge(TriEdge::W0),
            (TessDomain::Tri, true, 0) => TessFactor::TriInside,
            // (density, the count of lines, first: fxc's LINEDEN / LINEDET, what Vulkan
            // translators map to TessLevelOuter[0] / [1])
            (TessDomain::Isoline, false, 0) => TessFactor::LineDensity,
            (TessDomain::Isoline, false, 1) => TessFactor::LineDetail,
            _ => return None,
        })
    }
}

impl TessFactor {
    /// D3D_NAME (signatures)
    pub fn code(self) -> u32 {
        match self {
            TessFactor::QuadEdge(_) => 11,
            TessFactor::QuadInsideU | TessFactor::QuadInsideV => 12,
            TessFactor::TriEdge(_) => 13,
            TessFactor::TriInside => 14,
            TessFactor::LineDetail => 15,
            TessFactor::LineDensity => 16,
        }
    }
    /// D3D10_SB_NAME (dcl_output_siv, dcl_input_siv)
    pub fn sb_code(self) -> u32 {
        match self {
            TessFactor::QuadEdge(QuadEdge::U0) => 11,
            TessFactor::QuadEdge(QuadEdge::V0) => 12,
            TessFactor::QuadEdge(QuadEdge::U1) => 13,
            TessFactor::QuadEdge(QuadEdge::V1) => 14,
            TessFactor::QuadInsideU => 15,
            TessFactor::QuadInsideV => 16,
            TessFactor::TriEdge(TriEdge::U0) => 17,
            TessFactor::TriEdge(TriEdge::V0) => 18,
            TessFactor::TriEdge(TriEdge::W0) => 19,
            TessFactor::TriInside => 20,
            TessFactor::LineDetail => 21,
            TessFactor::LineDensity => 22,
        }
    }
}

impl SysVal {
    /// the D3D_NAME value
    pub fn code(self) -> u32 {
        match self {
            SysVal::Undefined => 0,
            SysVal::Position => 1,
            SysVal::ClipDistance => 2,
            SysVal::VertexId => 6,
            SysVal::PrimitiveId => 7,
            SysVal::InstanceId => 8,
            SysVal::IsFrontFace => 9,
            SysVal::SampleIndex => 10,
            SysVal::Target => 64,
            SysVal::Depth => 65,
            SysVal::Tess(t) => t.code(),
        }
    }
    /// the D3D10_SB_NAME a declaration gives (the D3D_NAME but for tess factors: each edge its
    /// own)
    pub fn sb_code(self) -> u32 {
        match self {
            SysVal::Tess(t) => t.sb_code(),
            SysVal::Undefined | SysVal::Position | SysVal::ClipDistance | SysVal::VertexId | SysVal::PrimitiveId | SysVal::InstanceId
            | SysVal::IsFrontFace | SysVal::SampleIndex | SysVal::Target | SysVal::Depth => self.code(),
        }
    }
    /// declared with a dcl_*_siv/sgv (not Target and Depth, which are plain outputs)
    pub fn declared(self) -> bool {
        match self {
            SysVal::Undefined | SysVal::Target | SysVal::Depth => false,
            SysVal::Position | SysVal::ClipDistance | SysVal::VertexId | SysVal::PrimitiveId
            | SysVal::InstanceId | SysVal::IsFrontFace | SysVal::SampleIndex | SysVal::Tess(_) => true,
        }
    }
}

/// A signature element's D3D_REGISTER_COMPONENT_TYPE.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompType {
    Uint,
    Sint,
    Float,
}

impl CompType {
    pub fn code(self) -> u32 {
        match self {
            CompType::Uint => 1,
            CompType::Sint => 2,
            CompType::Float => 3,
        }
    }
}

/// A pixel input's D3D10_SB_INTERPOLATION_MODE.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InterpMode {
    /// (not a pixel input)
    Undefined,
    Constant,
    Linear,
    LinearCentroid,
    LinearNoPerspective,
    LinearNoPerspectiveCentroid,
    LinearSample,
}

impl InterpMode {
    pub fn code(self) -> u32 {
        match self {
            InterpMode::Undefined => 0,
            InterpMode::Constant => 1,
            InterpMode::Linear => 2,
            InterpMode::LinearCentroid => 3,
            InterpMode::LinearNoPerspective => 4,
            InterpMode::LinearNoPerspectiveCentroid => 5,
            InterpMode::LinearSample => 6,
        }
    }
}

/// A signature element of the shader's interface.
#[derive(Clone, Debug)]
pub struct SigEl {
    pub name: String,
    pub index: u32,
    pub sysval: SysVal,
    pub ctype: CompType,
    pub reg: u32,
    pub mask: u8,
    pub interp: InterpMode,
    /// the register is oDepth
    pub depth: bool,
    /// a geometry shader output's stream (0 elsewhere)
    pub stream: u32,
}

/// A geometry shader's declarations.
#[derive(Clone, Debug)]
pub struct Gs {
    /// D3D10_SB_PRIMITIVE of its input
    pub input: u32,
    /// [maxvertexcount(n)]
    pub maxout: u32,
    /// [instance(n)] (1 without)
    pub instances: u32,
    /// each stream's D3D10_SB_PRIMITIVE_TOPOLOGY (1 point list, 3 line strip, 5 triangle strip)
    pub streams: Vec<u32>,
}

/// D3D11_SB_TESSELLATOR_PARTITIONING: 1 integer, 2 pow2, 3 fractional_odd, 4 fractional_even
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Partitioning {
    Integer,
    Pow2,
    FractionalOdd,
    FractionalEven,
}

impl Partitioning {
    pub fn of_name(s: &str) -> Option<Partitioning> {
        Some(match s {
            "integer" => Partitioning::Integer,
            "pow2" => Partitioning::Pow2,
            "fractional_odd" => Partitioning::FractionalOdd,
            "fractional_even" => Partitioning::FractionalEven,
            _ => return None,
        })
    }
    pub fn code(self) -> u32 {
        match self {
            Partitioning::Integer => 1,
            Partitioning::Pow2 => 2,
            Partitioning::FractionalOdd => 3,
            Partitioning::FractionalEven => 4,
        }
    }
}

/// D3D11_SB_TESSELLATOR_OUTPUT_PRIMITIVE: 1 point, 2 line, 3 triangle_cw, 4 triangle_ccw
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TessOutput {
    Point,
    Line,
    TriangleCw,
    TriangleCcw,
}

impl TessOutput {
    pub fn of_name(s: &str) -> Option<TessOutput> {
        Some(match s {
            "point" => TessOutput::Point,
            "line" => TessOutput::Line,
            "triangle_cw" => TessOutput::TriangleCw,
            "triangle_ccw" => TessOutput::TriangleCcw,
            _ => return None,
        })
    }
    pub fn code(self) -> u32 {
        match self {
            TessOutput::Point => 1,
            TessOutput::Line => 2,
            TessOutput::TriangleCw => 3,
            TessOutput::TriangleCcw => 4,
        }
    }
}

/// A hull shader: its declarations, the patch constants it writes, and its two phases (the
/// control point phase, then the patch constant function as one fork phase of one instance).
#[derive(Clone, Debug)]
pub struct Hs {
    pub domain: TessDomain,
    pub partitioning: Partitioning,
    pub output: TessOutput,
    /// InputPatch<T, n>'s n, [outputcontrolpoints(n)]'s
    pub in_cps: u32,
    pub out_cps: u32,
    /// [maxtessfactor(f)]
    pub max_tess: Option<f32>,
    /// the instruction the fork phase starts at
    pub fork_at: usize,
    /// each phase's system inputs (register type, mask): the control point phase's, the fork
    /// phase's
    pub cp_sys: Vec<(OpType, u8)>,
    pub fork_sys: Vec<(OpType, u8)>,
    /// the fork phase reads the input control points (vicp), the output ones (vocp)
    pub fork_icp: bool,
    pub fork_ocp: bool,
    /// the patch constants (PCSG): o registers of the fork phase
    pub patch: Vec<SigEl>,
}

/// A domain shader's declarations and its patch constant inputs (PCSG: vpc registers).
#[derive(Clone, Debug)]
pub struct Ds {
    pub domain: TessDomain,
    /// OutputPatch<T, n>'s n (0 without one)
    pub cps: u32,
    pub patch: Vec<SigEl>,
}

/// What a t or u register is a view of.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    /// a typed view: a texture, Buffer<T>, RWTexture*
    Typed,
    /// a structured or raw buffer: its kind, the bytes of an element (0 for raw buffers), and
    /// whether the shader uses its hidden counter (Append, Consume, Increment/DecrementCounter)
    Buf { kind: super::ast::BufKind, stride: u32, counter: bool },
}

pub struct Tex {
    pub name: String,
    pub view: View,
    /// resource dimension (1 buffer, 2 1d, 3 2d, 4 2dms, 5 3d, 6 cube, 7 1darray, 8 2darray, 9
    /// 2dmsarray, 10 cubearray)
    pub dim: u32,
    /// 3 sint, 4 uint, 5 float
    pub ret: u32,
    pub comps: u32,
    pub slot: u32,
    /// register(tN) given: D3D_SIF_USERPACKED
    pub explicit: bool,
}

/// A typed unordered access view (RWTexture*)
pub struct Uav {
    pub name: String,
    pub view: View,
    /// globallycoherent
    pub coherent: bool,
    /// resource dimension as `Tex::dim`
    pub dim: u32,
    /// 3 sint, 4 uint, 5 float (1 unorm, 2 snorm)
    pub ret: u32,
    pub comps: u32,
    pub slot: u32,    pub explicit: bool,
}

pub struct Samp {
    pub name: String,
    pub cmp: bool,
    pub slot: u32,
    pub explicit: bool,
}

/// A constant buffer variable (RDEF)
#[derive(Clone)]
pub struct CbVar {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    /// D3D_SHADER_VARIABLE_CLASS: 0 scalar, 1 vector, 2 matrix rows, 3 matrix columns, 5 struct
    pub class: u16,
    /// D3D_SHADER_VARIABLE_TYPE: 1 bool, 2 int, 3 float, 19 uint
    pub ty: u16,
    pub rows: u16,
    pub cols: u16,
    pub elems: u16,
    pub used: bool,
    /// register(cN) given (shader model 2/3)
    pub explicit: bool,
    /// members of a struct (offsets from the struct's start)
    pub members: Vec<CbVar>,
}

pub struct Cb {
    pub name: String,
    pub slot: u32,
    /// in registers (16 bytes); in bytes for the element buffers (`Shader::elements`)
    pub size: u32,
    pub dynamic: bool,
    pub vars: Vec<CbVar>,
    pub explicit: bool,
}

pub struct Shader {
    pub pixel: bool,
    pub major: u32,
    pub minor: u32,
    pub code: Vec<Ins>,
    pub inputs: Vec<SigEl>,
    pub outputs: Vec<SigEl>,
    pub cbs: Vec<Cb>,
    pub texs: Vec<Tex>,
    pub samps: Vec<Samp>,
    /// indexable temps: registers of each
    pub xs: Vec<u32>,
    pub icb: Vec<[u32; 4]>,
    /// (virtual temp, instruction index): a variable's declaration point (live range start)
    pub marks: Vec<(u32, usize)>,
    /// cb symbol -> index into `cbs` (only the used ones are in cbs)
    pub cb_map: Vec<Option<usize>>,
    pub tex_map: Vec<Option<usize>>,
    pub samp_map: Vec<Option<usize>>,
    pub early_depth: bool,
    /// compute shaders: [numthreads(x, y, z)]
    pub threads: Option<[u32; 3]>,
    /// compute inputs read: (register type, mask)
    pub sys_inputs: Vec<(OpType, u8)>,
    pub uavs: Vec<Uav>,
    pub uav_map: Vec<Option<usize>>,
    /// the element types of the structured buffers bound (RDEF's D3D_CT_RESOURCE_BIND_INFO
    /// buffers: one variable, $Element, of the element's type), in binding order
    pub elements: Vec<Cb>,
    /// thread group shared memory: bytes of each g register
    pub tgsm: Vec<u32>,
    /// the blob D3D10CompileShader (d3d10.dll's own compiler) writes: elements 1 for what is
    /// not an array, no texture component flags nor sample count, SV_Target / SV_Depth without
    /// a system value in the signature (d3d10refl.c)
    pub d3d10: bool,
    /// a geometry shader's declarations
    pub gs: Option<Gs>,
    pub hs: Option<Hs>,
    pub ds: Option<Ds>,
}

// ---- register allocation ----

fn temps_of(i: &Ins, f: &mut dyn FnMut(u32)) {
    let reg = |r: &Reg, f: &mut dyn FnMut(u32)| {
        if let RegF::Temp(t) = r.f {
            f(t);
        }
        if let Some((t, _)) = r.rel {
            f(t);
        }
    };
    for d in &i.dst {
        reg(&d.r, f);
    }
    for s in &i.src {
        if let Src::R { r, .. } = s {
            reg(r, f);
        }
    }
}

/// Virtual temps to registers (live ranges over the linear code, extended over loops they cross);
/// the number of registers.
pub(super) fn alloc(code: &mut [Ins], marks: &[(u32, usize)]) -> u32 {
    use std::collections::HashMap;
    let mut iv: HashMap<u32, (usize, usize)> = HashMap::new();
    let note = |t: u32, at: usize, iv: &mut HashMap<u32, (usize, usize)>| {
        let e = iv.entry(t).or_insert((at, at));
        e.0 = e.0.min(at);
        e.1 = e.1.max(at);
    };
    for (n, i) in code.iter().enumerate() {
        temps_of(i, &mut |t| note(t, n, &mut iv));
    }
    for &(t, at) in marks {
        if iv.contains_key(&t) {
            note(t, at, &mut iv);
        }
    }
    // loops
    let mut loops = Vec::new();
    let mut stack = Vec::new();
    for (n, i) in code.iter().enumerate() {
        if i.op == LOOP {
            stack.push(n);
        } else if i.op == ENDLOOP && let Some(s) = stack.pop() {
            loops.push((s, n));
        }
    }
    loop {
        let mut changed = false;
        for v in iv.values_mut() {
            for &(s, e) in &loops {
                let overlaps = v.0 <= e && v.1 >= s;
                let inside = v.0 >= s && v.1 <= e;
                if overlaps && !inside && (v.0 > s || v.1 < e) {
                    v.0 = v.0.min(s);
                    v.1 = v.1.max(e);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut order: Vec<(u32, usize, usize)> = iv.iter().map(|(&t, &(a, b))| (t, a, b)).collect();
    order.sort_by_key(|x| (x.1, x.0));
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut active: Vec<(usize, u32)> = Vec::new();
    let mut free: Vec<u32> = Vec::new();
    let mut n = 0;
    for (t, a, b) in order {
        active.retain(|&(end, r)| {
            if end < a {
                free.push(r);
                false
            } else {
                true
            }
        });
        free.sort_unstable_by(|x, y| y.cmp(x));
        let r = free.pop().unwrap_or_else(|| {
            n += 1;
            n - 1
        });
        active.push((b, r));
        map.insert(t, r);
    }
    let fix = |r: &mut Reg| {
        if let RegF::Temp(t) = r.f {
            r.f = RegF::Temp(map[&t]);
        }
        if let Some((t, c)) = r.rel {
            r.rel = Some((map[&t], c));
        }
    };
    for i in code.iter_mut() {
        for d in &mut i.dst {
            fix(&mut d.r);
        }
        for s in &mut i.src {
            if let Src::R { r, .. } = s {
                fix(r);
            }
        }
    }
    n
}

// ---- encoding ----

/// an operand type's token bits
fn ty_bits(t: OpType) -> u32 {
    t.0 << 12
}

struct Enc<'a> {
    s: &'a Shader,
}

impl Enc<'_> {
    /// operand token pieces: (type, indices)
    fn reg_parts(&self, r: &Reg) -> Result<(OpType, Vec<u32>), String> {
        Ok(match r.f {
            RegF::Temp(i) => (tpf::TEMP, vec![i]),
            RegF::Input(i) => (tpf::INPUT, vec![i]),
            RegF::Output(i) => (tpf::OUTPUT, vec![i]),
            RegF::Cb(s, i) => (tpf::CONSTANT_BUFFER, vec![bound(&self.s.cb_map, &self.s.cbs, s, |c| c.slot, "constant buffer")?, i]),
            RegF::Icb(i) => (tpf::IMM_CONSTANT_BUFFER, vec![i]),
            RegF::X(a, i) => (tpf::INDEXABLE_TEMP, vec![a, i]),
            RegF::Depth => (tpf::OUTPUT_DEPTH, vec![]),
            RegF::Null => (tpf::NULL, vec![]),
            RegF::Sys(t) => (t, vec![]),
            RegF::Uav(u) => (tpf::UAV, vec![bound(&self.s.uav_map, &self.s.uavs, u, |u| u.slot, "UAV")?]),
            RegF::G(g) => (tpf::TGSM, vec![g]),
            RegF::GsInput(v, i) => (tpf::INPUT, vec![v, i]),
            RegF::Icp(v, i) => (tpf::INPUT_CONTROL_POINT, vec![v, i]),
            RegF::Ocp(v, i) => (tpf::OUTPUT_CONTROL_POINT, vec![v, i]),
            RegF::Pc(i) => (tpf::INPUT_PATCH_CONSTANT, vec![i]),
        })
    }

    /// `sel`: 0 mask, 1 swizzle, 2 select-1; `num` component count code (0 = none, 1 = one, 2 = four)
    fn reg(&self, out: &mut Vec<u32>, r: &Reg, num: u32, sel: u32, bits: u32, modifier: u32) -> Result<(), String> {
        let (ty, idx) = self.reg_parts(r)?;
        let mut t = num | ty_bits(ty) | (kome::num::u32_of(idx.len()) << 20);
        if num == 2 {
            t |= (sel << 2) | (bits << 4);
        }
        let rel_at = idx.len().wrapping_sub(1);
        if r.rel.is_some() {
            // last index: immediate + relative
            t |= 3 << (22 + 3 * kome::num::u32_of(rel_at));
        }
        if modifier != 0 {
            t |= 1 << 31;
        }
        out.push(t);
        if modifier != 0 {
            out.push(1 | (modifier << 6));
        }
        for (k, &v) in idx.iter().enumerate() {
            out.push(v);
            if k == rel_at && let Some((tr, c)) = r.rel {
                // the relative part: a temp's component (select-1)
                out.push(2 | (2 << 2) | (u32::from(c) << 4) | ty_bits(tpf::TEMP) | (1 << 20));
                out.push(tr);
            }
        }
        Ok(())
    }

    fn src(&self, out: &mut Vec<u32>, s: &Src) -> Result<(), String> {
        match s {
            Src::R { r, swz, neg, abs, one } => {
                let m = u32::from(*neg) | (u32::from(*abs) << 1);
                if matches!(r.f, RegF::Depth) {
                    self.reg(out, r, 1, 0, 0, m)?;
                } else if *one {
                    self.reg(out, r, 2, 2, u32::from(swz[0]), m)?;
                } else {
                    let b = u32::from(swz[0]) | u32::from(swz[1]) << 2 | u32::from(swz[2]) << 4 | u32::from(swz[3]) << 6;
                    self.reg(out, r, 2, 1, b, m)?;
                }
            }
            Src::Imm(v, one) => {
                if *one {
                    out.push(1 | ty_bits(tpf::IMM32));
                    out.push(v[0]);
                } else {
                    out.push(2 | ty_bits(tpf::IMM32));
                    out.extend_from_slice(v);
                }
            }
            Src::Tex(t, swz) => {
                let slot = bound(&self.s.tex_map, &self.s.texs, *t, |t| t.slot, "texture")?;
                let b = u32::from(swz[0]) | u32::from(swz[1]) << 2 | u32::from(swz[2]) << 4 | u32::from(swz[3]) << 6;
                out.push(2 | (1 << 2) | (b << 4) | ty_bits(tpf::RESOURCE) | (1 << 20));
                out.push(slot);
            }
            Src::Samp(s) => {
                let slot = bound(&self.s.samp_map, &self.s.samps, *s, |s| s.slot, "sampler")?;
                out.push(ty_bits(tpf::SAMPLER) | (1 << 20));
                out.push(slot);
            }
            Src::SampC(s, c) => {
                let slot = bound(&self.s.samp_map, &self.s.samps, *s, |s| s.slot, "sampler")?;
                out.push(2 | (2 << 2) | (u32::from(*c) << 4) | ty_bits(tpf::SAMPLER) | (1 << 20));
                out.push(slot);
            }
            Src::Stream(k) => {
                out.push(ty_bits(tpf::STREAM) | (1 << 20));
                out.push(*k);
            }
            Src::Uav(u, swz) | Src::G(u, swz) => {
                let (ty, slot) = match s {
                    Src::Uav(..) => (tpf::UAV, bound(&self.s.uav_map, &self.s.uavs, *u, |u| u.slot, "UAV")?),
                    Src::R { .. } | Src::Imm(..) | Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::G(..) | Src::Stream(_) => (tpf::TGSM, *u),
                };
                let b = u32::from(swz[0]) | u32::from(swz[1]) << 2 | u32::from(swz[2]) << 4 | u32::from(swz[3]) << 6;
                out.push(2 | (1 << 2) | (b << 4) | ty_bits(ty) | (1 << 20));
                out.push(slot);
            }
        }
        Ok(())
    }

    fn dst(&self, out: &mut Vec<u32>, d: &Dst) -> Result<(), String> {
        match d.r.f {
            RegF::Null => {
                out.push(ty_bits(tpf::NULL));
                Ok(())
            }
            RegF::Depth => self.reg(out, &d.r, 1, 0, 0, 0),
            RegF::Temp(_) | RegF::Input(_) | RegF::Output(_) | RegF::Cb(..) | RegF::Icb(_) | RegF::X(..)
            | RegF::Sys(_) | RegF::Uav(_) | RegF::G(_) | RegF::GsInput(..) | RegF::Icp(..) | RegF::Ocp(..) | RegF::Pc(_) => self.reg(out, &d.r, 2, 0, u32::from(d.mask), 0),
        }
    }

    fn ins(&self, out: &mut Vec<u32>, i: &Ins) -> Result<(), String> {
        let at = out.len();
        let ext = i.offset != [0; 3];
        out.push(0);
        if ext {
            let o = |v: i32| v.cast_unsigned() & 0xf;
            out.push(1 | (o(i.offset[0]) << 9) | (o(i.offset[1]) << 13) | (o(i.offset[2]) << 17));
        }
        for d in &i.dst {
            self.dst(out, d)?;
        }
        for s in &i.src {
            self.src(out, s)?;
        }
        let len = kome::num::u32_of(out.len() - at);
        out[at] = i.op.code() | (i.ctl << 11) | (u32::from(i.sat) << 13) | (u32::from(i.nz) << 18) | (len << 24) | (u32::from(ext) << 31);
        Ok(())
    }
}

/// The slot a symbol was bound to (lower.rs binds every symbol an instruction names; an
/// unbound one is an error, not a panic)
fn bound<T>(map: &[Option<usize>], v: &[T], sym: u32, slot: impl Fn(&T) -> u32, what: &str) -> Result<u32, String> {
    map.get(kome::num::us(sym)).copied().flatten().and_then(|k| v.get(k)).map(slot).ok_or_else(|| format!("an unbound {what} (symbol {sym})"))
}

fn decl(out: &mut Vec<u32>, op: Opcode, ctl: u32, body: &[u32]) {
    out.push(op.code() | (ctl << 11) | (kome::num::u32_of(body.len() + 1) << 24));
    out.extend_from_slice(body);
}

/// The vertices of a D3D10_SB_PRIMITIVE (dcl_inputprimitive): point 1, line 2, triangle 3,
/// line with adjacency 4, triangle with adjacency 6.
pub fn gs_vertices(prim: u32) -> u32 {
    match prim {
        1 => 1,
        2 => 2,
        6 => 4,
        7 => 6,
        _ => 3,
    }
}

/// The SHDR/SHEX chunk.
fn program(s: &mut Shader) -> Result<Vec<u32>, String> {
    let temps = alloc(&mut s.code, &s.marks.clone());
    let s = &*s;
    let e = Enc { s };
    let kind = if s.threads.is_some() {
        5
    } else if s.gs.is_some() {
        2
    } else if s.hs.is_some() {
        3
    } else if s.ds.is_some() {
        4
    } else {
        u32::from(!s.pixel)
    };
    let mut w = vec![kind << 16 | (s.major << 4) | s.minor, 0];
    // (a hull shader's declarations of its own, before the phases)
    if let Some(h) = &s.hs {
        decl(&mut w, Opcode::HsDecls, 0, &[]);
        decl(&mut w, Opcode::DclInputControlPointCount, h.in_cps, &[]);
        decl(&mut w, Opcode::DclOutputControlPointCount, h.out_cps, &[]);
        decl(&mut w, Opcode::DclTessDomain, h.domain.code(), &[]);
        decl(&mut w, Opcode::DclTessPartitioning, h.partitioning.code(), &[]);
        decl(&mut w, Opcode::DclTessOutputPrimitive, h.output.code(), &[]);
        if let Some(f) = h.max_tess {
            decl(&mut w, Opcode::DclHsMaxTessFactor, 0, &[f.to_bits()]);
        }
    }
    if let Some(ds) = &s.ds {
        decl(&mut w, Opcode::DclTessDomain, ds.domain.code(), &[]);
        decl(&mut w, Opcode::DclInputControlPointCount, ds.cps, &[]);
    }
    // global flags: refactoring allowed (+ early depth stencil)
    decl(&mut w, Opcode::DclGlobalFlags, 1 | (u32::from(s.early_depth) << 2), &[]);
    if !s.icb.is_empty() {
        w.push(0x35 | (3 << 11));
        w.push(2 + 4 * kome::num::u32_of(s.icb.len()));
        for v in &s.icb {
            w.extend_from_slice(v);
        }
    }
    for cb in &s.cbs {
        let t = 2 | (1 << 2) | (0xe4 << 4) | ty_bits(tpf::CONSTANT_BUFFER) | (2 << 20);
        decl(&mut w, Opcode::DclConstantBuffer, u32::from(cb.dynamic), &[t, cb.slot, cb.size.max(1)]);
    }
    for sm in &s.samps {
        decl(&mut w, Opcode::DclSampler, u32::from(sm.cmp), &[ty_bits(tpf::SAMPLER) | (1 << 20), sm.slot]);
    }
    for t in &s.texs {
        let r = t.ret;
        let ret = r | (r << 4) | (r << 8) | (r << 12);
        let reg = ty_bits(tpf::RESOURCE) | (1 << 20);
        match t.view {
            View::Typed => decl(&mut w, Opcode::DclResource, t.dim, &[reg, t.slot, ret]),
            View::Buf { kind, stride, .. } if !kind.raw() => decl(&mut w, Opcode::DclResourceStructured, 0, &[reg, t.slot, stride]),
            View::Buf { .. } => decl(&mut w, Opcode::DclResourceRaw, 0, &[reg, t.slot]),
        }
    }
    for u in &s.uavs {
        let r = u.ret;
        let ret = r | (r << 4) | (r << 8) | (r << 12);
        let reg = ty_bits(tpf::UAV) | (1 << 20);
        // (D3D11_SB_GLOBALLY_COHERENT_ACCESS: bit 16; D3D11_SB_UAV_HAS_ORDER_PRESERVING_COUNTER:
        // bit 23, of the opcode token)
        let coherent = u32::from(u.coherent) << 5;
        match u.view {
            View::Typed => decl(&mut w, Opcode::DclUavTyped, u.dim | coherent, &[reg, u.slot, ret]),
            View::Buf { kind, stride, counter } if !kind.raw() => {
                decl(&mut w, Opcode::DclUavStructured, coherent | u32::from(counter) << 12, &[reg, u.slot, stride])
            }
            View::Buf { .. } => decl(&mut w, Opcode::DclUavRaw, coherent, &[reg, u.slot]),
        }
    }
    for (g, &bytes) in s.tgsm.iter().enumerate() {
        decl(&mut w, Opcode::DclTgsmRaw, 0, &[ty_bits(tpf::TGSM) | (1 << 20), kome::num::u32_of(g), bytes]);
    }
    let reg_tok = |ty: OpType, mask: u8| 2 | (u32::from(mask) << 4) | ty_bits(ty) | (1 << 20);
    // (a register of two indices: [count][register])
    let reg_tok2 = |ty: OpType, mask: u8| (reg_tok(ty, mask) & !(3 << 20)) | (2 << 20);
    let sys_decls = |w: &mut Vec<u32>, sys: &[(OpType, u8)]| {
        for &(ty, mask) in sys {
            // (vThreadIDInGroupFlattened, vOutputControlPointID, vForkInstanceID, vCoverage are one component)
            let t = if matches!(ty, tpf::THREAD_ID_IN_GROUP_FLATTENED | tpf::OUTPUT_CONTROL_POINT_ID | tpf::INPUT_FORK_INSTANCE_ID | tpf::INPUT_COVERAGE_MASK) { 1 | ty_bits(ty) } else { 2 | (u32::from(mask) << 4) | ty_bits(ty) };
            decl(w, Opcode::DclInput, 0, &[t]);
        }
    };
    let out_decl = |w: &mut Vec<u32>, o: &SigEl| {
        if o.depth {
            decl(w, Opcode::DclOutput, 0, &[1 | ty_bits(tpf::OUTPUT_DEPTH)]);
        } else if o.sysval.declared() {
            decl(w, Opcode::DclOutputSiv, 0, &[reg_tok(tpf::OUTPUT, o.mask), o.reg, o.sysval.sb_code()]);
        } else {
            decl(w, Opcode::DclOutput, 0, &[reg_tok(tpf::OUTPUT, o.mask), o.reg]);
        }
    };
    // (input control points: vicp[n][register], or a hull shader's output ones, vocp)
    let cp_decls = |w: &mut Vec<u32>, ty: OpType, n: u32, els: &[SigEl]| {
        for i in els {
            if i.sysval == SysVal::Undefined {
                decl(w, Opcode::DclInput, 0, &[reg_tok2(ty, i.mask), n, i.reg]);
            } else {
                decl(w, Opcode::DclInputSiv, 0, &[reg_tok2(ty, i.mask), n, i.reg, i.sysval.sb_code()]);
            }
        }
    };
    let temp_decls = |w: &mut Vec<u32>| {
        if temps > 0 {
            decl(w, Opcode::DclTemps, 0, &[temps]);
        }
        for (n, &size) in s.xs.iter().enumerate() {
            decl(w, Opcode::DclIndexableTemp, 0, &[kome::num::u32_of(n), size, 4]);
        }
    };
    if let Some(h) = &s.hs {
        // the control point phase, then the patch constant function's fork phase: each with
        // its own inputs, outputs and temps
        let (cp, fork) = s.code.split_at(h.fork_at.min(s.code.len()));
        decl(&mut w, Opcode::HsControlPointPhase, 0, &[]);
        sys_decls(&mut w, &h.cp_sys);
        cp_decls(&mut w, tpf::INPUT_CONTROL_POINT, h.in_cps, &s.inputs);
        for o in &s.outputs {
            out_decl(&mut w, o);
        }
        temp_decls(&mut w);
        for i in cp {
            e.ins(&mut w, i)?;
        }
        decl(&mut w, Opcode::HsForkPhase, 0, &[]);
        decl(&mut w, Opcode::DclHsForkPhaseInstanceCount, 0, &[1]);
        sys_decls(&mut w, &h.fork_sys);
        if h.fork_icp {
            cp_decls(&mut w, tpf::INPUT_CONTROL_POINT, h.in_cps, &s.inputs);
        }
        if h.fork_ocp {
            cp_decls(&mut w, tpf::OUTPUT_CONTROL_POINT, h.out_cps, &s.outputs);
        }
        for o in &h.patch {
            out_decl(&mut w, o);
        }
        temp_decls(&mut w);
        for i in fork {
            e.ins(&mut w, i)?;
        }
        let n = kome::num::u32_of(w.len());
        if let Some(len) = w.get_mut(1) {
            *len = n;
        }
        return Ok(w);
    }
    sys_decls(&mut w, &s.sys_inputs);
    if let Some(ds) = &s.ds {
        cp_decls(&mut w, tpf::INPUT_CONTROL_POINT, ds.cps, &s.inputs);
        for p in &ds.patch {
            let t = reg_tok(tpf::INPUT_PATCH_CONSTANT, p.mask);
            if p.sysval == SysVal::Undefined {
                decl(&mut w, Opcode::DclInput, 0, &[t, p.reg]);
            } else {
                decl(&mut w, Opcode::DclInputSiv, 0, &[t, p.reg, p.sysval.sb_code()]);
            }
        }
    }
    for i in s.inputs.iter().filter(|_| s.ds.is_none()) {
        let t = reg_tok(tpf::INPUT, i.mask);
        if s.pixel {
            match i.sysval {
                SysVal::Undefined => decl(&mut w, Opcode::DclInputPs, i.interp.code(), &[t, i.reg]),
                SysVal::Position => decl(&mut w, Opcode::DclInputPsSiv, i.interp.code(), &[t, i.reg, 1]),
                v @ (SysVal::ClipDistance | SysVal::VertexId | SysVal::PrimitiveId | SysVal::InstanceId
                     | SysVal::IsFrontFace | SysVal::SampleIndex | SysVal::Target | SysVal::Depth | SysVal::Tess(_)) => {
                    decl(&mut w, Opcode::DclInputPsSgv, 1, &[t, i.reg, v.code()]);
                }
            }
        } else if let Some(g) = &s.gs {
            // (v[vertices][register])
            let t = (t & !(3 << 20)) | (2 << 20);
            let n = gs_vertices(g.input);
            if i.sysval == SysVal::Undefined {
                decl(&mut w, Opcode::DclInput, 0, &[t, n, i.reg]);
            } else {
                decl(&mut w, Opcode::DclInputSiv, 0, &[t, n, i.reg, i.sysval.code()]);
            }
        } else if i.sysval != SysVal::Undefined {
            decl(&mut w, Opcode::DclInputSgv, 0, &[t, i.reg, i.sysval.code()]);
        } else {
            decl(&mut w, Opcode::DclInput, 0, &[t, i.reg]);
        }
    }
    if let Some(g) = &s.gs {
        decl(&mut w, Opcode::DclGsInputPrimitive, g.input, &[]);
    }
    match &s.gs {
        // (each stream: dcl_stream m<k> (shader model 5), its topology, then its outputs)
        Some(g) => {
            for (k, &topo) in (0u32..).zip(&g.streams) {
                if s.major >= 5 {
                    decl(&mut w, Opcode::DclStream, 0, &[ty_bits(tpf::STREAM) | (1 << 20), k]);
                }
                decl(&mut w, Opcode::DclGsOutputPrimitiveTopology, topo, &[]);
                for o in s.outputs.iter().filter(|o| o.stream == k) {
                    out_decl(&mut w, o);
                }
            }
            decl(&mut w, Opcode::DclMaxOutputVertexCount, 0, &[g.maxout]);
            if g.instances > 1 {
                decl(&mut w, Opcode::DclGsInstanceCount, 0, &[g.instances]);
            }
        }
        None => {
            for o in &s.outputs {
                out_decl(&mut w, o);
            }
        }
    }
    temp_decls(&mut w);
    if let Some([x, y, z]) = s.threads {
        decl(&mut w, Opcode::DclThreadGroup, 0, &[x, y, z]);
    }
    for i in &s.code {
        e.ins(&mut w, i)?;
    }
    w[1] = kome::num::u32_of(w.len());
    Ok(w)
}

fn put_str(b: &mut Vec<u8>, s: &str) -> u32 {
    let at = kome::num::u32_of(b.len());
    b.extend_from_slice(s.as_bytes());
    b.push(0);
    at
}

fn w32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

fn set32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn signature(els: &[SigEl], used_out: bool, d3d10: bool) -> Vec<u8> {
    let mut b = Vec::new();
    w32(&mut b, kome::num::u32_of(els.len()));
    w32(&mut b, 8);
    let names_at = 8 + 24 * els.len();
    let mut names = Vec::new();
    for e in els {
        let off = kome::num::u32_of(names_at) + kome::num::u32_of(names.len());
        put_str(&mut names, &e.name);
        w32(&mut b, off);
        w32(&mut b, e.index);
        w32(&mut b, if d3d10 && matches!(e.sysval, SysVal::Target | SysVal::Depth) { 0 } else { e.sysval.code() });
        w32(&mut b, e.ctype.code());
        w32(&mut b, if e.depth { u32::MAX } else { e.reg });
        // mask, then the read/write mask (outputs: the complement of what is written)
        let rw = if used_out { (!e.mask) & 0xf } else { e.mask };
        w32(&mut b, u32::from(e.mask) | (u32::from(rw) << 8));
    }
    b.extend_from_slice(&names);
    while b.len() % 4 != 0 {
        b.push(0xab);
    }
    b
}

/// OSG5: the elements of `signature` (outputs) with each one's stream first.
fn signature5(els: &[SigEl]) -> Vec<u8> {
    let mut b = Vec::new();
    w32(&mut b, kome::num::u32_of(els.len()));
    w32(&mut b, 8);
    let names_at = els.len().saturating_mul(28).saturating_add(8);
    let mut names = Vec::new();
    for e in els {
        let off = kome::num::u32_of(names_at).saturating_add(kome::num::u32_of(names.len()));
        put_str(&mut names, &e.name);
        w32(&mut b, e.stream);
        w32(&mut b, off);
        w32(&mut b, e.index);
        w32(&mut b, e.sysval.code());
        w32(&mut b, e.ctype.code());
        w32(&mut b, e.reg);
        w32(&mut b, u32::from(e.mask) | (u32::from((!e.mask) & 0xf) << 8));
    }
    b.extend_from_slice(&names);
    while b.len() % 4 != 0 {
        b.push(0xab);
    }
    b
}

/// The resource definitions (what D3DReflect reads).
/// A variable's type record at the end of `b`, then its members' (name, type, offset) and
/// their types: the record's offset. 16 bytes (class, type, rows, columns, elements, members,
/// the members' offset), 36 in shader model 5 (then four words and the type name's offset, 0).
fn rdef_type(b: &mut Vec<u8>, v: &CbVar, sm5: bool, d3d10: bool, fix_names: &mut Vec<(usize, String)>) -> u32 {
    let ta = b.len();
    let elems = if d3d10 { v.elems.max(1) } else { v.elems };
    for x in [v.class, v.ty, v.rows, v.cols, elems, crate::sat16(v.members.len())] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.resize(ta + if sm5 { 36 } else { 16 }, 0);
    if !v.members.is_empty() {
        let ma = b.len();
        set32(b, ta + 12, kome::num::u32_of(ma));
        b.resize(ma + 12 * v.members.len(), 0);
        for (i, m) in v.members.iter().enumerate() {
            let at = ma + 12 * i;
            fix_names.push((at, m.name.clone()));
            let t = rdef_type(b, m, sm5, d3d10, fix_names);
            set32(b, at + 4, t);
            set32(b, at + 8, m.offset);
        }
    }
    kome::num::u32_of(ta)
}

fn rdef(s: &Shader) -> Vec<u8> {
    let sm5 = s.major >= 5;
    let header = if sm5 { 60 } else { 28 };
    let mut b = vec![0u8; header];
    let ncb = kome::num::u32_of(s.cbs.len()).saturating_add(kome::num::u32_of(s.elements.len()));
    // (bind type, slot) -> the element bytes of a structured buffer
    let mut strides = std::collections::HashMap::new();
    for (view, slot, uav) in s.texs.iter().map(|t| (t.view, t.slot, false)).chain(s.uavs.iter().map(|u| (u.view, u.slot, true))) {
        if let View::Buf { kind, stride, counter } = view && !kind.raw() {
            let ty = match kind {
                BufKind::RwStructured if counter => 11,
                BufKind::RwStructured => 6,
                BufKind::Append => 9,
                BufKind::Consume => 10,
                BufKind::Structured | BufKind::Raw | BufKind::RwRaw => if uav { 6 } else { 5 },
            };
            strides.insert((ty, slot), stride);
        }
    }
    // bound resources: samplers, textures, cbuffers
    let mut binds: Vec<(String, u32, u32, u32, u32, u32)> = Vec::new(); // name, type, ret, dim, slot, flags
    for sm in &s.samps {
        binds.push((sm.name.clone(), 3, 0, 0, sm.slot, u32::from(sm.cmp) * 2 | u32::from(sm.explicit)));
    }
    for t in &s.texs {
        let dim = match t.dim {
            1 => 1,
            2 => 2,
            3 => 4,
            4 => 6,
            5 => 8,
            6 => 9,
            7 => 3,
            8 => 5,
            9 => 7,
            _ => 10,
        };
        let comps = if s.d3d10 { 0 } else { (t.comps - 1) << 2 };
        match t.view {
            View::Typed => binds.push((t.name.clone(), 2, t.ret, dim, t.slot, comps | u32::from(t.explicit))),
            // (D3D_SIT_STRUCTURED 5, D3D_SIT_BYTEADDRESS 7; D3D_RETURN_TYPE_MIXED, a buffer)
            View::Buf { kind, .. } => binds.push((t.name.clone(), if kind.raw() { 7 } else { 5 }, 6, 1, t.slot, u32::from(t.explicit))),
        }
    }
    for u in &s.uavs {
        let dim = match u.dim {
            1 => 1,
            2 => 2,
            3 => 4,
            5 => 8,
            7 => 3,
            _ => 5,
        };
        match u.view {
            View::Typed => binds.push((u.name.clone(), 4, u.ret, dim, u.slot, u.comps.saturating_sub(1) << 2 | u32::from(u.explicit))),
            // (D3D_SIT_UAV_RWSTRUCTURED 6, RWBYTEADDRESS 8, APPEND_STRUCTURED 9,
            // CONSUME_STRUCTURED 10, RWSTRUCTURED_WITH_COUNTER 11)
            View::Buf { kind, counter, .. } => {
                let ty = match kind {
                    BufKind::RwStructured if counter => 11,
                    BufKind::RwStructured => 6,
                    BufKind::RwRaw => 8,
                    BufKind::Append => 9,
                    BufKind::Consume => 10,
                    BufKind::Structured | BufKind::Raw => 4,
                };
                binds.push((u.name.clone(), ty, 6, 1, u.slot, u32::from(u.explicit)));
            }
        }
    }
    for cb in &s.cbs {
        binds.push((cb.name.clone(), 0, 0, 0, cb.slot, u32::from(cb.explicit)));
    }
    let cb_at = b.len();
    // constant buffer descriptions: name, vars, var offset, size, flags, type
    b.resize(cb_at.saturating_add(s.cbs.len().saturating_add(s.elements.len()).saturating_mul(24)), 0);
    let bind_at = b.len();
    b.resize(bind_at + 32 * binds.len(), 0);
    let mut fix_names: Vec<(usize, String)> = Vec::new();
    for (n, bd) in binds.iter().enumerate() {
        let at = bind_at + 32 * n;
        fix_names.push((at, bd.0.clone()));
        set32(&mut b, at + 4, bd.1);
        set32(&mut b, at + 8, bd.2);
        set32(&mut b, at + 12, bd.3);
        set32(&mut b, at + 16, if (bd.1 == 2 || bd.1 == 4) && !s.d3d10 { u32::MAX } else { 0 });
        // (a structured buffer's NumSamples: the bytes of an element)
        if let Some(&stride) = strides.get(&(bd.1, bd.4)) {
            set32(&mut b, at.saturating_add(16), stride);
        }
        set32(&mut b, at + 20, bd.4);
        set32(&mut b, at + 24, 1);
        set32(&mut b, at + 28, bd.5);
    }
    let var_size = if sm5 { 40 } else { 24 };
    for (n, (cb, info)) in s.cbs.iter().map(|c| (c, false)).chain(s.elements.iter().map(|c| (c, true))).enumerate() {
        let at = cb_at + 24 * n;
        fix_names.push((at, cb.name.clone()));
        // (D3D_CT_RESOURCE_BIND_INFO: a structured buffer's element, its size in bytes)
        if info {
            set32(&mut b, at.saturating_add(20), 3);
        }
        set32(&mut b, at + 4, kome::num::u32_of(cb.vars.len()));
        let vars_at = b.len();
        set32(&mut b, at + 8, kome::num::u32_of(vars_at));
        set32(&mut b, at + 12, if info { cb.size } else { cb.size.saturating_mul(16) });
        b.resize(vars_at + var_size * cb.vars.len(), 0);
        for (k, v) in cb.vars.iter().enumerate() {
            let va = vars_at + var_size * k;
            fix_names.push((va, v.name.clone()));
            set32(&mut b, va + 4, v.offset);
            set32(&mut b, va + 8, v.size);
            set32(&mut b, va + 12, if v.used { 2 } else { 0 });
            // the type
            let ta = rdef_type(&mut b, v, sm5, s.d3d10, &mut fix_names);
            set32(&mut b, va + 16, ta);
            if sm5 {
                // texture start, size, sampler start, size
                set32(&mut b, va + 24, u32::MAX);
                set32(&mut b, va + 32, u32::MAX);
            }
        }
    }
    for (at, name) in fix_names {
        let off = put_str(&mut b, &name);
        set32(&mut b, at, off);
    }
    let creator = put_str(&mut b, "sake HLSL compiler");
    set32(&mut b, 0, ncb);
    set32(&mut b, 4, kome::num::u32_of(cb_at));
    set32(&mut b, 8, kome::num::u32_of(binds.len()));
    set32(&mut b, 12, kome::num::u32_of(bind_at));
    // (D3D11_SHVER: CS 0x4353, GS 0x4753, HS 0x4853, DS 0x4453, PS 0xffff, VS 0xfffe)
    let kind = if s.threads.is_some() {
        0x4353
    } else if s.gs.is_some() {
        0x4753
    } else if s.hs.is_some() {
        0x4853
    } else if s.ds.is_some() {
        0x4453
    } else if s.pixel {
        0xffff
    } else {
        0xfffe
    };
    set32(&mut b, 16, (kind << 16) | (s.major << 8) | s.minor);
    set32(&mut b, 20, 0x100);
    set32(&mut b, 24, creator);
    if sm5 {
        b[28..32].copy_from_slice(b"RD11");
        for (k, v) in [60u32, 24, 32, 40, 36, 12, 0].iter().enumerate() {
            set32(&mut b, 32 + 4 * k, *v);
        }
    }
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b
}

/// The whole DXBC blob.
pub fn finish(mut s: Shader) -> Result<Vec<u8>, String> {
    let prog = program(&mut s)?;
    let shdr: Vec<u8> = prog.iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"RDEF", rdef(&s)),
        (*b"ISGN", signature(&s.inputs, false, s.d3d10)),
        // (a shader model 5 geometry shader's outputs name their stream: OSG5)
        if s.gs.is_some() && s.major >= 5 { (*b"OSG5", signature5(&s.outputs)) } else { (*b"OSGN", signature(&s.outputs, true, s.d3d10)) },
    ];
    // (the patch constants: a hull shader's outputs, a domain shader's inputs)
    // (in register order, then component, as d3dcompiler lists them: winref d3d11tess, PID at
    // r0.y second)
    let by_reg = |p: &[SigEl]| {
        let mut v = p.to_vec();
        v.sort_by_key(|e| (e.reg, e.mask.trailing_zeros()));
        v
    };
    if let Some(h) = &s.hs {
        chunks.push((*b"PCSG", signature(&by_reg(&h.patch), true, false)));
    }
    if let Some(d) = &s.ds {
        chunks.insert(2, (*b"PCSG", signature(&by_reg(&d.patch), false, false)));
    }
    chunks.push((if s.major >= 5 { *b"SHEX" } else { *b"SHDR" }, shdr));
    Ok(container(&chunks))
}

pub fn container(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"DXBC");
    b.extend_from_slice(&[0; 16]);
    w32(&mut b, 1);
    w32(&mut b, 0);
    w32(&mut b, kome::num::u32_of(chunks.len()));
    let mut off = 32 + 4 * chunks.len();
    for (_, c) in chunks {
        w32(&mut b, kome::num::u32_of(off));
        off += 8 + c.len();
    }
    for (t, c) in chunks {
        b.extend_from_slice(t);
        w32(&mut b, kome::num::u32_of(c.len()));
        b.extend_from_slice(c);
    }
    let n = kome::num::u32_of(b.len());
    set32(&mut b, 24, n);
    let h = checksum(&b[20..]);
    b[4..20].copy_from_slice(&h);
    b
}

// ---- the DXBC checksum: MD5 with its own padding ----

pub(crate) const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501, 0x698098d8, 0x8b44f7af,
    0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa,
    0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8,
    0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244, 0x432aff97,
    0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1, 0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1,
    0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

fn md5_block(st: &mut [u32; 4], blk: &[u8]) {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // (RFC 1321's table, floor(|sin(i + 1)| * 2^32): a constant, not 64 sines per block;
    // md5_table_is_the_sines checks it)
    let k = &MD5_K;
    let m: [u32; 16] = std::array::from_fn(|i| u32::from_le_bytes([blk[4 * i], blk[4 * i + 1], blk[4 * i + 2], blk[4 * i + 3]]));
    let [mut a, mut b, mut c, mut d] = *st;
    for i in 0..64 {
        let (f, g) = match i / 16 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let t = d;
        d = c;
        c = b;
        b = b.wrapping_add(a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(m[g]).rotate_left(S[i]));
        a = t;
    }
    st[0] = st[0].wrapping_add(a);
    st[1] = st[1].wrapping_add(b);
    st[2] = st[2].wrapping_add(c);
    st[3] = st[3].wrapping_add(d);
}

/// The checksum of a DXBC blob (from offset 20 on).
pub fn checksum(data: &[u8]) -> [u8; 16] {
    let mut st = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let len = data.len();
    let full = len / 64 * 64;
    for blk in data[..full].chunks_exact(64) {
        md5_block(&mut st, blk);
    }
    let rest = &data[full..];
    let bits = kome::num::u32_of(len).wrapping_mul(8);
    let mut last = [0u8; 64];
    if rest.len() >= 56 {
        last[..rest.len()].copy_from_slice(rest);
        last[rest.len()] = 0x80;
        md5_block(&mut st, &last);
        last = [0; 64];
        last[0..4].copy_from_slice(&bits.to_le_bytes());
        last[60..64].copy_from_slice(&((bits >> 2) | 1).to_le_bytes());
    } else {
        last[0..4].copy_from_slice(&bits.to_le_bytes());
        last[4..4 + rest.len()].copy_from_slice(rest);
        last[4 + rest.len()] = 0x80;
        last[60..64].copy_from_slice(&((bits >> 2) | 1).to_le_bytes());
    }
    md5_block(&mut st, &last);
    let mut out = [0u8; 16];
    for (k, v) in st.iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}
