//! The syntax tree to shader model 4/5 instructions (sm4.rs). Every function call is inlined;
//! values are lists of components (a register's component or an immediate), so swizzles,
//! struct members, matrix layouts and constant buffer offsets cost no code.

use std::collections::HashMap;

use super::ast::*;
pub use super::sm4::Lane;
use super::sm4::{self, Dst, InterpMode, Ins, Reg, RegF, SigEl, Src, SysVal};

type R<T> = Result<T, String>;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum C {
    R(Reg, Lane),
    I(u32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Obj {
    None,
    Tex(u32),
    Samp(u32),
    /// sampler2D: texture and sampler
    Legacy(u32, u32),
    /// an array of them: element i is the first's descriptor + i (the length is the type's)
    TexArr(u32),
    SampArr(u32),
    LegacyArr(u32, u32),
    /// a typed unordered access view (RWTexture*)
    Uav(u32),
    /// a place in memory (the value's type: what is there)
    Mem(Mem),
    /// a structured or raw buffer (StructuredBuffer, ByteAddressBuffer and kin)
    Buf(Res),
    /// a geometry shader's output stream (its index in `L::streams`)
    Stream(u32),
}

/// A structured or raw buffer's register: t (read-only) or u (a UAV), by symbol.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Res {
    T(u32),
    U(u32),
}

/// A place in memory, addressed by dwords.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mem {
    /// groupshared g<n> at a dword offset
    G(u32, C),
    /// element (`.1`) of a structured buffer at a dword offset in it; matrices row_major (`.3`)
    /// or column_major
    S(Res, C, C, bool),
}

impl Mem {
    fn off(self) -> C {
        match self {
            Mem::G(_, o) | Mem::S(_, _, o, _) => o,
        }
    }
    fn at(self, off: C) -> Mem {
        match self {
            Mem::G(g, _) => Mem::G(g, off),
            Mem::S(r, i, _, rm) => Mem::S(r, i, off, rm),
        }
    }
}

/// Where an atomic operates: groupshared memory at a byte address, or a UAV at an address (a
/// raw buffer's byte, a structured buffer's element and byte, a typed UAV's coordinates).
#[derive(Clone, PartialEq, Debug)]
enum Atom {
    G(u32, C),
    U(u32, Vec<C>),
}

/// Memory a shader may write: groupshared, or an element of a UAV's structured buffer.
#[derive(Clone, Copy, PartialEq, Debug)]
enum WMem {
    G(u32, C),
    U(u32, C, C, bool),
}

impl WMem {
    fn mem(self) -> Mem {
        match self {
            WMem::G(g, o) => Mem::G(g, o),
            WMem::U(u, i, o, rm) => Mem::S(Res::U(u), i, o, rm),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Val {
    pub ty: Ty,
    pub c: Vec<C>,
    pub obj: Obj,
}

impl Val {
    pub fn num(ty: Ty, c: Vec<C>) -> Val {
        Val { ty, c, obj: Obj::None }
    }
}

pub struct Opts {
    pub stage: crate::Stage,
    pub major: u32,
    pub minor: u32,
    pub row_major: bool,
    /// D3DCOMPILE_IEEE_STRICTNESS: without it isnan() is false, as fxc folds it
    pub ieee: bool,
}

impl Opts {
    pub fn pixel(&self) -> bool {
        self.stage == crate::Stage::Pixel
    }
    /// a compute shader (cs_5_0, cs_4_x)
    pub fn compute(&self) -> bool {
        self.stage == crate::Stage::Compute
    }
    /// a geometry shader (gs_4_0, gs_4_1, gs_5_0)
    pub fn geometry(&self) -> bool {
        self.stage == crate::Stage::Geometry
    }
    pub fn hull(&self) -> bool {
        self.stage == crate::Stage::Hull
    }
    pub fn domain(&self) -> bool {
        self.stage == crate::Stage::Domain
    }
}

struct TexD {
    name: String,
    view: sm4::View,
    dim: Dim,
    el: Ty,
    reg: Option<u32>,
    used: bool,
}

struct UavD {
    name: String,
    view: sm4::View,
    coherent: bool,
    dim: Dim,
    el: Ty,
    reg: Option<u32>,
    used: bool,
}

/// A place written through a copy, stored back once the write is done.
enum Wb {
    /// picked by a run-time index from a select chain: (the elements, the index, the copy)
    Sel(Vec<Vec<C>>, C, Vec<C>, B),
    /// memory of a value of this type, through a copy: what the code from this instruction on
    /// writes of the copy is stored (only that: other threads may write the rest)
    Mem(WMem, Ty, Vec<C>, usize),
    /// a typed UAV at coordinates
    Uav(u32, Vec<C>, Vec<C>),
}

struct SampD {
    name: String,
    cmp: bool,
    reg: Option<u32>,
    used: bool,
}

struct CbD {
    name: String,
    reg: Option<u32>,
    size: u32,
    used: bool,
    dynamic: bool,
    vars: Vec<sm4::CbVar>,
}

struct FnCtx {
    ret: Vec<C>,
    ret_ty: Ty,
    flag: Option<u32>,
    loops: u32,
    /// entry point: (where each output's value is, its output registers) and the return
    /// value's output registers
    entry: Option<(Vec<(Vec<C>, Vec<C>)>, Vec<C>)>,
}

pub struct L<'a> {
    pub(crate) o: Opts,
    prog: &'a Program,
    prelude: &'a Program,
    code: Vec<Ins>,
    next: u32,
    xs: Vec<u32>,
    icb: Vec<[u32; 4]>,
    marks: Vec<(u32, usize)>,
    scopes: Scopes,
    globals: HashMap<String, Val>,
    /// global name -> (cbuffer, variable)
    cbvar: HashMap<String, (usize, usize)>,
    texs: Vec<TexD>,
    samps: Vec<SampD>,
    cbs: Vec<CbD>,
    inputs: Vec<SigEl>,
    outputs: Vec<SigEl>,
    fns: Vec<FnCtx>,
    /// places written through a copy, stored back once the write is done
    wbs: Vec<Wb>,
    uavs: Vec<UavD>,
    /// groupshared variables: bytes of each
    tgsm: Vec<u32>,
    /// compute inputs read: (register type, mask)
    sys_inputs: Vec<(sm4::OpType, u8)>,
    threads: Option<[u32; 3]>,
    /// a place is wanted for a read too (compound assignment, ++/--): UAVs load it
    place_reads: bool,
    /// a geometry shader's declarations
    gs: Option<sm4::Gs>,
    /// its output streams
    streams: Vec<GsStream>,
    /// the stream whose output signature is being made
    cur_stream: u32,
    /// a hull shader's declarations (its fork phase's start set when the control point phase
    /// ends), a domain shader's
    hs: Option<sm4::Hs>,
    ds: Option<sm4::Ds>,
    /// the patch domain of a hull or domain shader ([domain(...)]): its tess factors' names
    tess_domain: Option<sm4::TessDomain>,
    /// the patch constants (PCSG): a hull shader's fork phase outputs, a domain shader's inputs
    patch: Vec<SigEl>,
    /// sig() makes patch constants (into `patch`), or input control points' elements (a patch
    /// parameter's element type)
    sig_patch: bool,
    sig_cp: bool,
    /// a hull shader's patch constant function is being lowered (its fork phase)
    in_pcf: bool,
    /// a hull shader's control points: the input ones (element type, one point's registers),
    /// the control point phase's outputs
    icp: Option<(Ty, Vec<C>)>,
    ocp: Option<(Ty, Vec<C>)>,
    /// the statics' storage, types and initializers (a hull shader's fork phase sets them
    /// again: its temps are its own)
    static_inits: Vec<(Vec<C>, Ty, Option<Expr>)>,
}

/// A geometry shader's output stream: what Append writes and where.
struct GsStream {
    el: Ty,
    regs: Vec<C>,
}

const TRUE: u32 = 0xffff_ffff;
const ONE_F: u32 = 0x3f80_0000;

fn fbits(f: f32) -> u32 {
    f.to_bits()
}

fn flt(v: u32) -> f32 {
    f32::from_bits(v)
}

/// `T a[]` (any dimension) without an initializer to size it: fxc's X3074.
fn no_size(t: &Ty) -> bool {
    matches!(t, Ty::Array(e, n) if *n == 0 || no_size(e))
}

/// The most instructions a shader may lower to (D3D11 has no limit, fxc gives up near a million
/// and a real shader is a few thousand)
pub const MAX_INS: usize = 1 << 20;

/// The local variable scopes: never empty (a declaration always has a scope to go to, so no
/// unwrap of the innermost one), innermost `top`.
#[derive(Default)]
pub struct Scopes {
    top: HashMap<String, Val>,
    outer: Vec<HashMap<String, Val>>,
}

impl Scopes {
    fn with(top: HashMap<String, Val>) -> Scopes {
        Scopes { top, outer: Vec::new() }
    }

    fn open(&mut self) {
        self.outer.push(std::mem::take(&mut self.top));
    }

    /// Closes the innermost scope (the outermost one stays, empty).
    fn close(&mut self) {
        self.top = self.outer.pop().unwrap_or_default();
    }

    fn declare(&mut self, name: String, v: Val) -> Option<Val> {
        self.top.insert(name, v)
    }

    fn innermost_first(&self) -> impl Iterator<Item = &HashMap<String, Val>> {
        std::iter::once(&self.top).chain(self.outer.iter().rev())
    }
}

pub fn ncomp(prog: &Program, t: &Ty) -> usize {
    match t {
        Ty::Array(e, n) => ncomp(prog, e) * kome::num::us(*n),
        Ty::Struct(i) => prog.structs[*i].fields.iter().map(|f| ncomp(prog, &f.ty)).sum(),
        t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Tex(..) | Ty::Sampler(_)
             | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => t.comps(),
    }
}

/// The base type of each component.
fn leaf_bases(prog: &Program, t: &Ty, out: &mut Vec<B>) {
    match t {
        Ty::Array(e, n) => {
            for _ in 0..*n {
                leaf_bases(prog, e, out);
            }
        }
        Ty::Struct(i) => {
            for f in &prog.structs[*i].fields {
                leaf_bases(prog, &f.ty, out);
            }
        }
        t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Tex(..) | Ty::Sampler(_)
             | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => {
            for _ in 0..t.comps() {
                out.push(t.base());
            }
        }
    }
}

/// Register-sized units (each a vector or a matrix row) of a type, in component order.
fn units(prog: &Program, t: &Ty, out: &mut Vec<usize>) {
    match t {
        Ty::Array(e, n) => {
            for _ in 0..*n {
                units(prog, e, out);
            }
        }
        Ty::Struct(i) => {
            for f in &prog.structs[*i].fields {
                units(prog, &f.ty, out);
            }
        }
        Ty::Mat(_, r, c) => {
            for _ in 0..r.get() {
                out.push(usize::from(*c));
            }
        }
        t if t.comps() > 0 => out.push(t.comps()),
        Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..)
             | Ty::Legacy(_) | Ty::Str | Ty::Stream(..) => {}
    }
}

fn split_sem(s: &str) -> (String, u32) {
    let d = s.trim_end_matches(|c: char| c.is_ascii_digit());
    let idx = s[d.len()..].parse().unwrap_or(0);
    (d.to_string(), idx)
}

fn pixel_out(pixel: bool, output: bool) -> bool {
    pixel && output
}

/// Argument `k` of a call.
fn arg(args: &[Expr], k: usize) -> R<&Expr> {
    args.get(k).ok_or_else(|| format!("no argument {k}"))
}

/// The first component of a value (an error for one of none).
fn c0(c: &[C]) -> R<C> {
    c.first().copied().ok_or_else(|| "a value without components".to_owned())
}

fn temp(t: u32) -> Reg {
    Reg { f: RegF::Temp(t), rel: None }
}

fn ctype(b: B) -> sm4::CompType {
    match b {
        B::Float => sm4::CompType::Float,
        B::Int => sm4::CompType::Sint,
        B::Bool | B::Uint => sm4::CompType::Uint,
    }
}

fn swizzle_of(name: &str, n: usize) -> Option<Vec<usize>> {
    if name.is_empty() || name.len() > 4 {
        return None;
    }
    let set = |s: &str| name.chars().map(|c| s.find(c)).collect::<Option<Vec<usize>>>();
    let v = set("xyzw").or_else(|| set("rgba"))?;
    if v.iter().any(|&i| i >= n.max(1)) {
        return None;
    }
    Some(v)
}

/// _11_22 / _m00_m11: (row, column) pairs
fn mat_swizzle(name: &str, r: usize, c: usize) -> Option<Vec<(usize, usize)>> {
    let mut out = Vec::new();
    let mut s = name;
    while !s.is_empty() {
        let rest = s.strip_prefix('_')?;
        let (a, b, n) = if let Some(m) = rest.strip_prefix('m') {
            let b = m.as_bytes();
            if b.len() < 2 {
                return None;
            }
            (kome::num::us(char::from(b[0]).to_digit(10)?), kome::num::us(char::from(b[1]).to_digit(10)?), 3)
        } else {
            let b = rest.as_bytes();
            if b.len() < 2 {
                return None;
            }
            (kome::num::us(char::from(b[0]).to_digit(10)?).checked_sub(1)?, kome::num::us(char::from(b[1]).to_digit(10)?).checked_sub(1)?, 2)
        };
        if a >= r || b >= c {
            return None;
        }
        out.push((a, b));
        s = &rest[n..];
    }
    Some(out)
}

fn has_return(s: &Stmt) -> bool {
    match s {
        Stmt::Return(..) => true,
        Stmt::Block(v) => v.iter().any(has_return),
        Stmt::If(_, a, b) => has_return(a) || b.as_deref().is_some_and(has_return),
        Stmt::For(_, _, _, b) | Stmt::While(_, b) | Stmt::Do(b, _) => has_return(b),
        Stmt::Switch(_, cs) => cs.iter().any(|c| c.1.iter().any(has_return)),
        Stmt::Expr(_) | Stmt::Decl(_) | Stmt::Break | Stmt::Continue | Stmt::Discard => false,
    }
}

fn has_continue(s: &Stmt) -> bool {
    match s {
        Stmt::Continue => true,
        Stmt::Block(v) => v.iter().any(has_continue),
        Stmt::If(_, a, b) => has_continue(a) || b.as_deref().is_some_and(has_continue),
        Stmt::Switch(_, cs) => cs.iter().any(|c| c.1.iter().any(has_continue)),
        Stmt::Expr(_) | Stmt::Decl(_) | Stmt::For(..) | Stmt::While(..) | Stmt::Do(..) | Stmt::Break
        | Stmt::Return(..) | Stmt::Discard => false,
    }
}

fn expr_writes(e: &Expr, name: &str, outs: &dyn Fn(&str) -> bool) -> bool {
    let root = |mut t: &Expr| loop {
        match t {
            Expr::Var(n, _) => return n == name,
            Expr::Member(a, _) | Expr::Index(a, _) => t = a,
            Expr::Int(..) | Expr::Float(_) | Expr::Bool(_) | Expr::Bin(..) | Expr::Un(..) | Expr::Assign(..)
            | Expr::Cond(..) | Expr::Call(..) | Expr::Construct(..) | Expr::Cast(..) | Expr::Method(..) | Expr::List(_)
            | Expr::Comma(..) | Expr::Str(_) => return false,
        }
    };
    match e {
        Expr::Assign(_, a, b) => root(a) || expr_writes(a, name, outs) || expr_writes(b, name, outs),
        Expr::Un(Un::PreInc | Un::PreDec | Un::PostInc | Un::PostDec, a) => root(a),
        Expr::Bin(_, a, b) | Expr::Comma(a, b) | Expr::Index(a, b) => expr_writes(a, name, outs) || expr_writes(b, name, outs),
        Expr::Un(_, a) | Expr::Cast(_, a) | Expr::Member(a, _) => expr_writes(a, name, outs),
        Expr::Cond(a, b, c) => expr_writes(a, name, outs) || expr_writes(b, name, outs) || expr_writes(c, name, outs),
        // (out arguments: user functions and the intrinsics with out parameters)
        Expr::Call(f, args, _) => {
            let o = !super::intrinsics::NO_OUT.contains(&f.as_str()) && outs(f);
            args.iter().any(|a| expr_writes(a, name, outs) || (o && root(a)))
        }
        Expr::Construct(_, args) | Expr::List(args) => args.iter().any(|a| expr_writes(a, name, outs)),
        Expr::Method(o, m, args) => expr_writes(o, name, outs) || args.iter().any(|a| expr_writes(a, name, outs) || (m == "GetDimensions" && root(a))),
        Expr::Int(..) | Expr::Float(_) | Expr::Bool(_) | Expr::Var(..) | Expr::Str(_) => false,
    }
}

fn writes(s: &Stmt, name: &str, outs: &dyn Fn(&str) -> bool) -> bool {
    match s {
        Stmt::Expr(e) => expr_writes(e, name, outs),
        Stmt::Decl(vs) => vs.iter().any(|v| v.init.as_ref().is_some_and(|e| expr_writes(e, name, outs))),
        Stmt::Block(v) => v.iter().any(|s| writes(s, name, outs)),
        Stmt::If(c, a, b) => expr_writes(c, name, outs) || writes(a, name, outs) || b.as_deref().is_some_and(|b| writes(b, name, outs)),
        Stmt::For(i, c, st, b) => {
            i.as_deref().is_some_and(|i| writes(i, name, outs))
                || c.as_ref().is_some_and(|c| expr_writes(c, name, outs))
                || st.as_ref().is_some_and(|c| expr_writes(c, name, outs))
                || writes(b, name, outs)
        }
        Stmt::While(c, b) | Stmt::Do(b, c) => expr_writes(c, name, outs) || writes(b, name, outs),
        Stmt::Switch(c, cs) => expr_writes(c, name, outs) || cs.iter().any(|c| c.1.iter().any(|s| writes(s, name, outs))),
        Stmt::Return(Some(e), _) => expr_writes(e, name, outs),
        Stmt::Break | Stmt::Continue | Stmt::Return(..) | Stmt::Discard => false,
    }
}

/// Constant folding of an elementwise operation on immediates.
fn fold(op: sm4::Opcode, a: &[u32], tru: u32) -> Option<u32> {
    use sm4::*;
    let f = |i: usize| flt(a[i]);
    let i = |k: usize| a[k].cast_signed();
    let m = |b: bool| if b { tru } else { 0 };
    Some(match op {
        MOV => a[0],
        ADD => fbits(f(0) + f(1)),
        MUL => fbits(f(0) * f(1)),
        DIV => fbits(f(0) / f(1)),
        MAD => fbits(f(0) * f(1) + f(2)),
        MIN => fbits(f(0).min(f(1))),
        MAX => fbits(f(0).max(f(1))),
        IADD => a[0].wrapping_add(a[1]),
        IMAX => i(0).max(i(1)).cast_unsigned(),
        IMIN => i(0).min(i(1)).cast_unsigned(),
        UMAX => a[0].max(a[1]),
        UMIN => a[0].min(a[1]),
        AND => a[0] & a[1],
        OR => a[0] | a[1],
        XOR => a[0] ^ a[1],
        NOT => !a[0],
        INEG => a[0].wrapping_neg(),
        ISHL => a[0].wrapping_shl(a[1] & 31),
        ISHR => (i(0) >> (a[1] & 31)).cast_unsigned(),
        USHR => a[0] >> (a[1] & 31),
        ITOF => fbits(kome::num::to_f32(i(0))),
        UTOF => fbits(kome::num::to_f32(a[0])),
        FTOI => (kome::num::trunc_i32(f(0))).cast_unsigned(),
        FTOU => kome::num::trunc_u32(f(0)),
        EQ => m(f(0) == f(1)),
        NE => m(f(0) != f(1)),
        LT => m(f(0) < f(1)),
        GE => m(f(0) >= f(1)),
        IEQ => m(a[0] == a[1]),
        INE => m(a[0] != a[1]),
        ILT => m(i(0) < i(1)),
        IGE => m(i(0) >= i(1)),
        ULT => m(a[0] < a[1]),
        UGE => m(a[0] >= a[1]),
        MOVC => {
            if a[0] != 0 {
                a[1]
            } else {
                a[2]
            }
        }
        FRC => fbits(f(0) - f(0).floor()),
        ROUND_NI => fbits(f(0).floor()),
        ROUND_PI => fbits(f(0).ceil()),
        ROUND_Z => fbits(f(0).trunc()),
        ROUND_NE => fbits(f(0).round_ties_even()),
        SQRT => fbits(f(0).sqrt()),
        RSQ => fbits(1.0 / f(0).sqrt()),
        EXP => fbits(f(0).exp2()),
        LOG => fbits(f(0).log2()),
        Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
        | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
        | Opcode::DerivRty | Opcode::Discard | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
        | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
        | Opcode::If | Opcode::Imad
        | Opcode::Imul | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Loop | Opcode::CustomData
        | Opcode::Nop | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
        | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Switch | Opcode::SinCos
        | Opcode::Udiv | Opcode::Umul | Opcode::Umad | Opcode::DclResource | Opcode::DclConstantBuffer
        | Opcode::DclSampler | Opcode::DclIndexRange | Opcode::DclGsOutputPrimitiveTopology | Opcode::DclGsInputPrimitive | Opcode::DclMaxOutputVertexCount | Opcode::DclInput
        | Opcode::DclInputSgv | Opcode::DclInputSiv | Opcode::DclInputPs | Opcode::DclInputPsSgv | Opcode::DclInputPsSiv | Opcode::DclOutput
        | Opcode::DclOutputSgv | Opcode::DclOutputSiv | Opcode::DclTemps | Opcode::DclIndexableTemp | Opcode::DclGlobalFlags | Opcode::Reserved10
        | Opcode::Lod | Opcode::Gather4 | Opcode::SamplePos | Opcode::SampleInfo | Opcode::Reserved101 | Opcode::HsDecls
        | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase | Opcode::EmitStream | Opcode::CutStream | Opcode::EmitThenCutStream
        | Opcode::InterfaceCall | Opcode::BufInfo | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine
        | Opcode::Gather4C | Opcode::Gather4Po | Opcode::Gather4PoC | Opcode::Rcp | Opcode::F32ToF16 | Opcode::F16ToF32
        | Opcode::Uaddc | Opcode::Usubb | Opcode::CountBits | Opcode::FirstBitHi | Opcode::FirstBitLo | Opcode::FirstBitShi
        | Opcode::Ubfe | Opcode::Ibfe | Opcode::Bfi | Opcode::Bfrev | Opcode::Swapc | Opcode::DclStream
        | Opcode::DclFunctionBody | Opcode::DclFunctionTable | Opcode::DclInterface | Opcode::DclInputControlPointCount | Opcode::DclOutputControlPointCount | Opcode::DclTessDomain
        | Opcode::DclTessPartitioning | Opcode::DclTessOutputPrimitive | Opcode::DclHsMaxTessFactor | Opcode::DclHsForkPhaseInstanceCount | Opcode::DclHsJoinPhaseInstanceCount | Opcode::DclThreadGroup
        | Opcode::DclUavTyped | Opcode::DclUavRaw | Opcode::DclUavStructured | Opcode::DclTgsmRaw | Opcode::DclTgsmStructured | Opcode::DclResourceRaw
        | Opcode::DclResourceStructured | Opcode::LdUavTyped | Opcode::StoreUavTyped | Opcode::LdRaw | Opcode::StoreRaw | Opcode::LdStructured
        | Opcode::StoreStructured | Opcode::AtomicAnd | Opcode::AtomicOr | Opcode::AtomicXor | Opcode::AtomicCmpStore | Opcode::AtomicIadd
        | Opcode::AtomicImax | Opcode::AtomicImin | Opcode::AtomicUmax | Opcode::AtomicUmin | Opcode::ImmAtomicAlloc | Opcode::ImmAtomicConsume
        | Opcode::ImmAtomicIadd | Opcode::ImmAtomicAnd | Opcode::ImmAtomicOr | Opcode::ImmAtomicXor | Opcode::ImmAtomicExch | Opcode::ImmAtomicCmpExch
        | Opcode::ImmAtomicImax | Opcode::ImmAtomicImin | Opcode::ImmAtomicUmax | Opcode::ImmAtomicUmin | Opcode::Sync | Opcode::Dadd
        | Opcode::Dmax | Opcode::Dmin | Opcode::Dmul | Opcode::Deq | Opcode::Dge | Opcode::Dlt
        | Opcode::Dne | Opcode::Dmov | Opcode::Dmovc | Opcode::Dtof | Opcode::Ftod | Opcode::EvalSnapped
        | Opcode::EvalSampleIndex | Opcode::EvalCentroid | Opcode::DclGsInstanceCount | Opcode::Abort | Opcode::DebugBreak | Opcode::Reserved11
        | Opcode::Ddiv | Opcode::Dfma | Opcode::Drcp | Opcode::Msad | Opcode::Dtoi | Opcode::Dtou
        | Opcode::Itod | Opcode::Utod | Opcode::Unknown => return None,
    })
}

fn neg_imm(v: u32, b: B) -> u32 {
    if b == B::Float { v ^ 0x8000_0000 } else { v.wrapping_neg() }
}

impl<'a> L<'a> {
    pub fn new(o: Opts, prog: &'a Program, prelude: &'a Program) -> Self {
        L {
            o,
            prog,
            prelude,
            code: Vec::new(),
            next: 0,
            xs: Vec::new(),
            icb: Vec::new(),
            marks: Vec::new(),
            wbs: Vec::new(),
            scopes: Scopes::default(),
            globals: HashMap::new(),
            cbvar: HashMap::new(),
            texs: Vec::new(),
            samps: Vec::new(),
            cbs: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            fns: Vec::new(),
            uavs: Vec::new(),
            tgsm: Vec::new(),
            sys_inputs: Vec::new(),
            threads: None,
            place_reads: false,
            gs: None,
            streams: Vec::new(),
            cur_stream: 0,
            hs: None,
            ds: None,
            tess_domain: None,
            patch: Vec::new(),
            sig_patch: false,
            sig_cp: false,
            in_pcf: false,
            icp: None,
            ocp: None,
            static_inits: Vec::new(),
        }
    }

    /// shader model 2/3 output: ints and bools are floats (true 1.0), no bit operations
    fn sm3(&self) -> bool {
        self.o.major < 4
    }
    fn tru(&self) -> u32 {
        if self.sm3() {
            ONE_F
        } else {
            TRUE
        }
    }
    /// an int constant
    fn ival(&self, v: i64) -> u32 {
        if self.sm3() {
            fbits(kome::num::to_f32(v))
        } else {
            kome::num::lo32(v)
        }
    }

    /// a user function of this name takes an out parameter
    fn has_out(&self, f: &str) -> bool {
        self.prog.funcs.iter().any(|x| x.name == f && x.params.iter().any(|p| p.is_out)) || matches!(f, "sincos" | "modf")
    }

    fn st(&self, i: usize) -> &'a Struct {
        &self.prog.structs[i]
    }

    // ---- emission ----

    fn tmp(&mut self) -> u32 {
        self.next += 1;
        self.next - 1
    }

    fn emit(&mut self, op: sm4::Opcode, dst: Vec<Dst>, src: Vec<Src>) -> &mut Ins {
        self.code.push_mut(Ins { op, sat: false, nz: false, ctl: 0, dst, src, offset: [0; 3] })
    }

    fn flow(&mut self, op: sm4::Opcode) {
        self.emit(op, vec![], vec![]);
    }

    /// if_nz / if_z (and breakc, discard...) on one component
    fn cond_op(&mut self, op: sm4::Opcode, c: C, nz: bool) {
        let s = self.scalar_src(c);
        self.emit(op, vec![], vec![s]).nz = nz;
    }

    fn scalar_src(&mut self, c: C) -> Src {
        match c {
            C::I(v) => Src::Imm([v; 4], true),
            C::R(r, k) => Src::R { r, swz: [k.into(); 4], neg: false, abs: false, one: true },
        }
    }

    /// A source operand of up to 4 components (one register or immediates; others are gathered
    /// into a temp first).
    fn opnd(&mut self, cs: &[C]) -> Src {
        let n = cs.len();
        if cs.iter().all(|c| matches!(c, C::I(_))) {
            let mut v = [0u32; 4];
            for k in 0..4 {
                if let C::I(x) = cs[k.min(n - 1)] {
                    v[k] = x;
                }
            }
            return Src::Imm(v, false);
        }
        if let C::R(r0, _) = cs[0] && cs.iter().all(|c| matches!(c, C::R(r, _) if *r == r0)) {
            let mut swz = [0u8; 4];
            for (k, s) in swz.iter_mut().enumerate() {
                if let C::R(_, c) = cs[k.min(n - 1)] {
                    *s = c.into();
                }
            }
            return Src::R { r: r0, swz, neg: false, abs: false, one: false };
        }
        let t = self.tmp();
        let dst: Vec<C> = Lane::first(n).map(|k| C::R(temp(t), k)).collect();
        self.assign_raw(&dst, cs);
        let last = Lane::first(n).last().map_or(3, u8::from);
        Src::R { r: temp(t), swz: [0, 1.min(last), 2.min(last), 3.min(last)], neg: false, abs: false, one: false }
    }

    /// dst comps <- src comps, movs grouped by register (no overlap check)
    fn assign_raw(&mut self, dst: &[C], src: &[C]) {
        self.assign_op(sm4::MOV, dst, src, false);
    }

    fn assign_op(&mut self, op: sm4::Opcode, dst: &[C], src: &[C], sat: bool) {
        // group destination comps by register
        let mut groups: Vec<(Reg, Vec<(u8, C)>)> = Vec::new();
        for (d, s) in dst.iter().zip(src) {
            let C::R(r, c) = *d else { continue };
            match groups.iter_mut().find(|g| g.0 == r) {
                Some(g) => g.1.push((c.into(), *s)),
                None => groups.push((r, vec![(c.into(), *s)])),
            }
        }
        for (r, comps) in groups {
            // then by source register
            let mut sub: Vec<(Option<Reg>, Vec<(u8, C)>)> = Vec::new();
            for (dc, s) in comps {
                let key = match s {
                    C::R(r, _) => Some(r),
                    C::I(_) => None,
                };
                match sub.iter_mut().find(|g| g.0 == key) {
                    Some(g) => g.1.push((dc, s)),
                    None => sub.push((key, vec![(dc, s)])),
                }
            }
            for (key, items) in sub {
                let mask = items.iter().fold(0u8, |m, x| m | 1 << x.0);
                let s = match key {
                    None => {
                        let mut v = [0u32; 4];
                        for (dc, s) in &items {
                            if let C::I(x) = s {
                                v[usize::from(*dc)] = *x;
                            }
                        }
                        Src::Imm(v, false)
                    }
                    Some(sr) => {
                        let mut swz = [0u8; 4];
                        for (dc, s) in &items {
                            if let C::R(_, c) = s {
                                swz[usize::from(*dc)] = (*c).into();
                            }
                        }
                        Src::R { r: sr, swz, neg: false, abs: false, one: false }
                    }
                };
                self.emit(op, vec![Dst { r, mask }], vec![s]).sat = sat;
            }
        }
    }

    /// dst <- src, safe when they share registers
    fn assign(&mut self, dst: &[C], src: &[C]) {
        let dregs: Vec<Reg> = dst.iter().filter_map(|c| if let C::R(r, _) = c { Some(*r) } else { None }).collect();
        let clash = src.iter().any(|c| matches!(c, C::R(r, _) if dregs.contains(r)));
        if clash {
            let copy = self.copy(src);
            self.assign_raw(dst, &copy);
        } else {
            self.assign_raw(dst, src);
        }
    }

    /// A fresh copy of comps in temps.
    fn copy(&mut self, src: &[C]) -> Vec<C> {
        let mut out = Vec::new();
        for ch in src.chunks(4) {
            let t = self.tmp();
            let d: Vec<C> = Lane::first(ch.len()).map(|k| C::R(temp(t), k)).collect();
            self.assign_raw(&d, ch);
            out.extend(d);
        }
        out
    }

    /// An elementwise operation over comps (args the same length); modifiers per argument
    /// (1 negate, 2 absolute value).
    fn alu_m(&mut self, op: sm4::Opcode, b: B, args: &[(&[C], u8)]) -> Vec<C> {
        let mut owned: Vec<(&[C], u8)> = args.to_vec();
        let (mut op, mut b) = (op, b);
        if self.sm3() {
            // (integer operations on float values)
            use sm4::*;
            op = match op {
                IADD => ADD,
                IMAD => MAD,
                IMUL | UMUL => MUL,
                IEQ => EQ,
                INE => NE,
                ILT | ULT => LT,
                IGE | UGE => GE,
                IMAX | UMAX => MAX,
                IMIN | UMIN => MIN,
                ITOF | UTOF => MOV,
                FTOI | FTOU => ROUND_Z,
                // (of bools)
                AND => MUL,
                OR => MAX,
                INEG => {
                    owned[0].1 ^= 1;
                    MOV
                }
                x @ (Opcode::Add | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                     | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                     | Opcode::DerivRty | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
                     | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                     | Opcode::Eq | Opcode::Exp | Opcode::Frc | Opcode::Ge
                     | Opcode::If | Opcode::Ishl
                     | Opcode::Ishr | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                     | Opcode::Loop | Opcode::Lt | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                     | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Not
                     | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                     | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
                     | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
                     | Opcode::Udiv | Opcode::Umad | Opcode::Ushr | Opcode::Xor | Opcode::DclResource | Opcode::DclConstantBuffer
                     | Opcode::DclSampler | Opcode::DclIndexRange | Opcode::DclGsOutputPrimitiveTopology | Opcode::DclGsInputPrimitive | Opcode::DclMaxOutputVertexCount | Opcode::DclInput
                     | Opcode::DclInputSgv | Opcode::DclInputSiv | Opcode::DclInputPs | Opcode::DclInputPsSgv | Opcode::DclInputPsSiv | Opcode::DclOutput
                     | Opcode::DclOutputSgv | Opcode::DclOutputSiv | Opcode::DclTemps | Opcode::DclIndexableTemp | Opcode::DclGlobalFlags | Opcode::Reserved10
                     | Opcode::Lod | Opcode::Gather4 | Opcode::SamplePos | Opcode::SampleInfo | Opcode::Reserved101 | Opcode::HsDecls
                     | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase | Opcode::EmitStream | Opcode::CutStream | Opcode::EmitThenCutStream
                     | Opcode::InterfaceCall | Opcode::BufInfo | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine
                     | Opcode::Gather4C | Opcode::Gather4Po | Opcode::Gather4PoC | Opcode::Rcp | Opcode::F32ToF16 | Opcode::F16ToF32
                     | Opcode::Uaddc | Opcode::Usubb | Opcode::CountBits | Opcode::FirstBitHi | Opcode::FirstBitLo | Opcode::FirstBitShi
                     | Opcode::Ubfe | Opcode::Ibfe | Opcode::Bfi | Opcode::Bfrev | Opcode::Swapc | Opcode::DclStream
                     | Opcode::DclFunctionBody | Opcode::DclFunctionTable | Opcode::DclInterface | Opcode::DclInputControlPointCount | Opcode::DclOutputControlPointCount | Opcode::DclTessDomain
                     | Opcode::DclTessPartitioning | Opcode::DclTessOutputPrimitive | Opcode::DclHsMaxTessFactor | Opcode::DclHsForkPhaseInstanceCount | Opcode::DclHsJoinPhaseInstanceCount | Opcode::DclThreadGroup
                     | Opcode::DclUavTyped | Opcode::DclUavRaw | Opcode::DclUavStructured | Opcode::DclTgsmRaw | Opcode::DclTgsmStructured | Opcode::DclResourceRaw
                     | Opcode::DclResourceStructured | Opcode::LdUavTyped | Opcode::StoreUavTyped | Opcode::LdRaw | Opcode::StoreRaw | Opcode::LdStructured
                     | Opcode::StoreStructured | Opcode::AtomicAnd | Opcode::AtomicOr | Opcode::AtomicXor | Opcode::AtomicCmpStore | Opcode::AtomicIadd
                     | Opcode::AtomicImax | Opcode::AtomicImin | Opcode::AtomicUmax | Opcode::AtomicUmin | Opcode::ImmAtomicAlloc | Opcode::ImmAtomicConsume
                     | Opcode::ImmAtomicIadd | Opcode::ImmAtomicAnd | Opcode::ImmAtomicOr | Opcode::ImmAtomicXor | Opcode::ImmAtomicExch | Opcode::ImmAtomicCmpExch
                     | Opcode::ImmAtomicImax | Opcode::ImmAtomicImin | Opcode::ImmAtomicUmax | Opcode::ImmAtomicUmin | Opcode::Sync | Opcode::Dadd
                     | Opcode::Dmax | Opcode::Dmin | Opcode::Dmul | Opcode::Deq | Opcode::Dge | Opcode::Dlt
                     | Opcode::Dne | Opcode::Dmov | Opcode::Dmovc | Opcode::Dtof | Opcode::Ftod | Opcode::EvalSnapped
                     | Opcode::EvalSampleIndex | Opcode::EvalCentroid | Opcode::DclGsInstanceCount | Opcode::Abort | Opcode::DebugBreak | Opcode::Reserved11
                     | Opcode::Ddiv | Opcode::Dfma | Opcode::Drcp | Opcode::Msad | Opcode::Dtoi | Opcode::Dtou
                     | Opcode::Itod | Opcode::Utod | Opcode::Unknown) => x,
            };
            b = B::Float;
        }
        let args = &owned[..];
        let n = args[0].0.len();
        let foldable = !self.sm3() || super::sm3::float_op(op);
        if foldable && args.iter().all(|a| a.0.iter().all(|c| matches!(c, C::I(_)))) {
            let mut out = Vec::new();
            let mut ok = true;
            for k in 0..n {
                let vals: Vec<u32> = args
                    .iter()
                    .map(|a| {
                        let C::I(v) = a.0[k] else { unreachable!() };
                        let v = if a.1 & 2 != 0 { if b == B::Float { v & 0x7fff_ffff } else { v.cast_signed().unsigned_abs() } } else { v };
                        if a.1 & 1 != 0 { neg_imm(v, b) } else { v }
                    })
                    .collect();
                match fold(op, &vals, self.tru()) {
                    Some(v) => out.push(C::I(v)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                return out;
            }
        }
        let mut out = Vec::new();
        for st in (0..n).step_by(4) {
            let m = (n - st).min(4);
            let t = self.tmp();
            let mut srcs = Vec::new();
            for (a, md) in args {
                let mut s = self.opnd(&a[st..st + m]);
                if *md != 0 {
                    match &mut s {
                        Src::R { neg, abs, .. } => {
                            *neg = md & 1 != 0;
                            *abs = md & 2 != 0;
                        }
                        Src::Imm(v, _) => {
                            for x in v.iter_mut() {
                                if md & 2 != 0 {
                                    *x = if b == B::Float { *x & 0x7fff_ffff } else { x.cast_signed().unsigned_abs() };
                                }
                                if md & 1 != 0 {
                                    *x = neg_imm(*x, b);
                                }
                            }
                        }
                        Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_) => {}
                    }
                }
                srcs.push(s);
            }
            self.emit(op, vec![Dst { r: temp(t), mask: Lane::mask(m) }], srcs);
            out.extend(Lane::first(m).map(|k| C::R(temp(t), k)));
        }
        out
    }

    fn alu(&mut self, op: sm4::Opcode, b: B, args: &[&[C]]) -> Vec<C> {
        let a: Vec<(&[C], u8)> = args.iter().map(|x| (*x, 0u8)).collect();
        self.alu_m(op, b, &a)
    }

    /// An operation with two results (imul/udiv: hi/quotient first): which one to keep.
    fn alu2(&mut self, op: sm4::Opcode, args: &[&[C]], second: bool) -> Vec<C> {
        use sm4::*;
        if self.sm3() {
            match op {
                IMUL | UMUL => return self.alu(MUL, B::Float, args),
                UDIV => {
                    let q = self.alu(DIV, B::Float, args);
                    let tq = self.alu(ROUND_Z, B::Float, &[&q]);
                    if !second {
                        return tq;
                    }
                    return self.alu_m(MAD, B::Float, &[(&tq, 1), (args[1], 0), (args[0], 0)]);
                }
                Opcode::Add | Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                | Opcode::DerivRty | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
                | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                | Opcode::Eq | Opcode::Exp | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
                | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                | Opcode::Imax | Opcode::Imin | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                | Opcode::Loop | Opcode::Lt | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Not
                | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
                | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
                | Opcode::Ult | Opcode::Uge | Opcode::Umad | Opcode::Umax
                | Opcode::Umin | Opcode::Ushr | Opcode::Utof | Opcode::Xor | Opcode::DclResource | Opcode::DclConstantBuffer
                | Opcode::DclSampler | Opcode::DclIndexRange | Opcode::DclGsOutputPrimitiveTopology | Opcode::DclGsInputPrimitive | Opcode::DclMaxOutputVertexCount | Opcode::DclInput
                | Opcode::DclInputSgv | Opcode::DclInputSiv | Opcode::DclInputPs | Opcode::DclInputPsSgv | Opcode::DclInputPsSiv | Opcode::DclOutput
                | Opcode::DclOutputSgv | Opcode::DclOutputSiv | Opcode::DclTemps | Opcode::DclIndexableTemp | Opcode::DclGlobalFlags | Opcode::Reserved10
                | Opcode::Lod | Opcode::Gather4 | Opcode::SamplePos | Opcode::SampleInfo | Opcode::Reserved101 | Opcode::HsDecls
                | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase | Opcode::EmitStream | Opcode::CutStream | Opcode::EmitThenCutStream
                | Opcode::InterfaceCall | Opcode::BufInfo | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine
                | Opcode::Gather4C | Opcode::Gather4Po | Opcode::Gather4PoC | Opcode::Rcp | Opcode::F32ToF16 | Opcode::F16ToF32
                | Opcode::Uaddc | Opcode::Usubb | Opcode::CountBits | Opcode::FirstBitHi | Opcode::FirstBitLo | Opcode::FirstBitShi
                | Opcode::Ubfe | Opcode::Ibfe | Opcode::Bfi | Opcode::Bfrev | Opcode::Swapc | Opcode::DclStream
                | Opcode::DclFunctionBody | Opcode::DclFunctionTable | Opcode::DclInterface | Opcode::DclInputControlPointCount | Opcode::DclOutputControlPointCount | Opcode::DclTessDomain
                | Opcode::DclTessPartitioning | Opcode::DclTessOutputPrimitive | Opcode::DclHsMaxTessFactor | Opcode::DclHsForkPhaseInstanceCount | Opcode::DclHsJoinPhaseInstanceCount | Opcode::DclThreadGroup
                | Opcode::DclUavTyped | Opcode::DclUavRaw | Opcode::DclUavStructured | Opcode::DclTgsmRaw | Opcode::DclTgsmStructured | Opcode::DclResourceRaw
                | Opcode::DclResourceStructured | Opcode::LdUavTyped | Opcode::StoreUavTyped | Opcode::LdRaw | Opcode::StoreRaw | Opcode::LdStructured
                | Opcode::StoreStructured | Opcode::AtomicAnd | Opcode::AtomicOr | Opcode::AtomicXor | Opcode::AtomicCmpStore | Opcode::AtomicIadd
                | Opcode::AtomicImax | Opcode::AtomicImin | Opcode::AtomicUmax | Opcode::AtomicUmin | Opcode::ImmAtomicAlloc | Opcode::ImmAtomicConsume
                | Opcode::ImmAtomicIadd | Opcode::ImmAtomicAnd | Opcode::ImmAtomicOr | Opcode::ImmAtomicXor | Opcode::ImmAtomicExch | Opcode::ImmAtomicCmpExch
                | Opcode::ImmAtomicImax | Opcode::ImmAtomicImin | Opcode::ImmAtomicUmax | Opcode::ImmAtomicUmin | Opcode::Sync | Opcode::Dadd
                | Opcode::Dmax | Opcode::Dmin | Opcode::Dmul | Opcode::Deq | Opcode::Dge | Opcode::Dlt
                | Opcode::Dne | Opcode::Dmov | Opcode::Dmovc | Opcode::Dtof | Opcode::Ftod | Opcode::EvalSnapped
                | Opcode::EvalSampleIndex | Opcode::EvalCentroid | Opcode::DclGsInstanceCount | Opcode::Abort | Opcode::DebugBreak | Opcode::Reserved11
                | Opcode::Ddiv | Opcode::Dfma | Opcode::Drcp | Opcode::Msad | Opcode::Dtoi | Opcode::Dtou
                | Opcode::Itod | Opcode::Utod | Opcode::Unknown => {}
            }
        }
        let n = args[0].len();
        let mut out = Vec::new();
        for st in (0..n).step_by(4) {
            let m = (n - st).min(4);
            let t = self.tmp();
            let srcs: Vec<Src> = args.iter().map(|a| self.opnd(&a[st..st + m])).collect();
            let d = Dst { r: temp(t), mask: Lane::mask(m) };
            let null = Dst { r: Reg { f: RegF::Null, rel: None }, mask: 0 };
            let dst = if second { vec![null, d] } else { vec![d, null] };
            self.emit(op, dst, srcs);
            out.extend(Lane::first(m).map(|k| C::R(temp(t), k)));
        }
        out
    }

    fn imm(v: u32, n: usize) -> Vec<C> {
        vec![C::I(v); n]
    }

    // ---- types and conversions ----

    fn conv_base(&mut self, cs: &[C], from: B, to: B) -> Vec<C> {
        use sm4::*;
        if from == to || (matches!(from, B::Int | B::Uint) && matches!(to, B::Int | B::Uint)) {
            return cs.to_vec();
        }
        let n = cs.len();
        if self.sm3() {
            return match (from, to) {
                (B::Float, B::Int | B::Uint) => self.alu(ROUND_Z, to, &[cs]),
                (_, B::Bool) => self.alu(NE, to, &[cs, &Self::imm(0, n)]),
                _ => cs.to_vec(),
            };
        }
        match (from, to) {
            (B::Float, B::Int) => self.alu(FTOI, to, &[cs]),
            (B::Float, B::Uint) => self.alu(FTOU, to, &[cs]),
            (B::Int, B::Float) => self.alu(ITOF, to, &[cs]),
            (B::Uint, B::Float) => self.alu(UTOF, to, &[cs]),
            (B::Bool, B::Float) => self.alu(AND, to, &[cs, &Self::imm(ONE_F, n)]),
            (B::Bool, _) => self.alu(AND, to, &[cs, &Self::imm(1, n)]),
            (B::Float, B::Bool) => self.alu(NE, to, &[cs, &Self::imm(0, n)]),
            (_, B::Bool) => self.alu(INE, to, &[cs, &Self::imm(0, n)]),
            _ => cs.to_vec(),
        }
    }

    fn cast(&mut self, v: Val, to: &Ty) -> R<Val> {
        if &v.ty == to {
            return Ok(v);
        }
        if v.ty.is_object() || to.is_object() {
            return Err(format!("cannot convert {:?} to {:?}", v.ty, to));
        }
        let n = v.c.len();
        let m = ncomp(self.prog, to);
        let mut tb = Vec::new();
        leaf_bases(self.prog, to, &mut tb);
        // shape
        let comps: Vec<C> = if n == m {
            v.c.clone()
        } else if n == 1 {
            vec![v.c[0]; m]
        } else if let (Ty::Mat(_, r1, c1), Ty::Mat(_, r2, c2)) = (&v.ty, to) {
            if r2 > r1 || c2 > c1 {
                return Err(format!("cannot convert {:?} to {:?}", v.ty, to));
            }
            let mut o = Vec::new();
            for r in 0..r2.get() {
                for c in 0..c2.get() {
                    o.push(v.c[r * usize::from(*c1) + c]);
                }
            }
            o
        } else if m < n && v.ty.numeric() && to.numeric() {
            v.c[..m].to_vec()
        } else {
            return Err(format!("cannot convert {:?} to {:?}", v.ty, to));
        };
        let src = Val::num(if n == 1 && m != 1 { v.ty.clone() } else { v.ty.clone() }, comps);
        // per-comp bases of the source
        let mut fb = Vec::new();
        leaf_bases(self.prog, &v.ty, &mut fb);
        let mut out = Vec::new();
        for (k, c) in src.c.iter().enumerate() {
            let f = if n == 1 { fb[0] } else if n == m { fb[k] } else { fb.get(k).copied().unwrap_or(B::Float) };
            out.extend(self.conv_base(&[*c], f, tb[k]));
        }
        Ok(Val::num(to.clone(), out))
    }

    /// The type two operands of an arithmetic operator meet in.
    fn common(&self, a: &Ty, b: &Ty) -> R<Ty> {
        let base = if a.base() == B::Float || b.base() == B::Float {
            B::Float
        } else if a.base() == B::Uint || b.base() == B::Uint {
            B::Uint
        } else {
            B::Int
        };
        Ok(match (a, b) {
            // (an array, struct or object with a scalar: no common type)
            (Ty::Scalar(_), t) | (t, Ty::Scalar(_)) if t.numeric() => t.with_base(base),
            (Ty::Vec(_, n), Ty::Vec(_, m)) => Ty::vec(base, (*n).min(*m)),
            (Ty::Mat(_, r, c), Ty::Mat(_, r2, c2)) => Ty::Mat(base, (*r).min(*r2), (*c).min(*c2)),
            (Ty::Vec(_, n), Ty::Mat(_, r, c)) | (Ty::Mat(_, r, c), Ty::Vec(_, n)) if n.get() == r.get() * c.get() => {
                Ty::vec(base, *n)
            }
            _ => return Err(format!("operands of types {a:?} and {b:?}")),
        })
    }

    // ---- storage ----

    /// New storage for a variable of type `t`: temps, arrays in an indexable temp.
    fn storage(&mut self, t: &Ty) -> Vec<C> {
        match t {
            Ty::Array(e, n) if self.sm3() => {
                // (no indexable temps: elements in temps)
                let mut out = Vec::new();
                for _ in 0..*n {
                    out.extend(self.storage(e));
                }
                out
            }
            Ty::Array(..) => {
                let mut u = Vec::new();
                units(self.prog, t, &mut u);
                let id = kome::num::u32_of(self.xs.len());
                self.xs.push(kome::num::u32_of(u.len().max(1)));
                let mut out = Vec::new();
                for (k, n) in u.iter().enumerate() {
                    for c in Lane::first(*n) {
                        out.push(C::R(Reg { f: RegF::X(id, kome::num::u32_of(k)), rel: None }, c));
                    }
                }
                out
            }
            Ty::Struct(i) => {
                let fields: Vec<Ty> = self.st(*i).fields.iter().map(|f| f.ty.clone()).collect();
                let mut out = Vec::new();
                for f in fields {
                    out.extend(self.storage(&f));
                }
                out
            }
            Ty::Mat(_, r, c) => {
                let mut out = Vec::new();
                for _ in 0..r.get() {
                    let t = self.tmp();
                    self.marks.push((t, self.code.len()));
                    out.extend(Lane::first(usize::from(*c)).map(|k| C::R(temp(t), k)));
                }
                out
            }
            t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..)
                 | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => {
                let n = t.comps();
                if n == 0 {
                    return Vec::new();
                }
                let t = self.tmp();
                self.marks.push((t, self.code.len()));
                Lane::first(n).map(|k| C::R(temp(t), k)).collect()
            }
        }
    }

    fn lookup(&mut self, name: &str, line: u32) -> R<Val> {
        for s in self.scopes.innermost_first() {
            if let Some(v) = s.get(name) {
                return Ok(v.clone());
            }
        }
        if let Some(&(cb, var)) = self.cbvar.get(name) {
            self.cbs[cb].used = true;
            self.cbs[cb].vars[var].used = true;
        }
        match self.globals.get(name).cloned() {
            Some(v) => {
                match v.obj {
                    Obj::Tex(t) => self.texs[kome::num::us(t)].used = true,
                    Obj::Samp(s) => self.samps[kome::num::us(s)].used = true,
                    Obj::Legacy(t, s) => {
                        self.texs[kome::num::us(t)].used = true;
                        self.samps[kome::num::us(s)].used = true;
                    }
                    Obj::Uav(u) | Obj::Buf(Res::U(u)) => self.uavs.get_mut(kome::num::us(u)).into_iter().for_each(|x| x.used = true),
                    Obj::Buf(Res::T(t)) => self.texs.get_mut(kome::num::us(t)).into_iter().for_each(|x| x.used = true),
                    // (arrays: the elements indexed are marked)
                    Obj::None | Obj::TexArr(_) | Obj::SampArr(_) | Obj::LegacyArr(..) | Obj::Mem(_) | Obj::Stream(_) => {}
                }
                // (bools of constant buffers are 0/1: canonical masks)
                if self.cbvar.contains_key(name) && v.ty.base() == B::Bool && v.ty.numeric() {
                    let c = self.alu(sm4::INE, B::Bool, &[&v.c, &Self::imm(0, v.c.len())]);
                    return Ok(Val::num(v.ty, c));
                }
                Ok(v)
            }
            None => Err(format!("line {line}: undeclared identifier '{name}'")),
        }
    }

    // ---- globals ----

    /// Constant buffer layout of a variable of `t` at byte offset `at`: (comps, end offset).
    fn cb_layout(&self, t: &Ty, row_major: bool, mut at: u32, sym: u32, out: &mut Vec<C>) -> u32 {
        let reg = |r: u32| Reg { f: RegF::Cb(sym, r), rel: None };
        match t {
            Ty::Scalar(_) | Ty::Vec(..) => {
                let n = kome::num::u32_of(t.comps());
                // (shader model 2/3: each in registers of its own)
                if at % 16 + 4 * n > 16 || self.sm3() {
                    at = at.div_ceil(16) * 16;
                }
                for k in 0..n {
                    let o = at + 4 * k;
                    out.push(C::R(reg(o / 16), Lane::of(kome::num::us(o / 4))));
                }
                if self.sm3() {
                    at + 16
                } else {
                    at + 4 * n
                }
            }
            Ty::Mat(_, r, c) => {
                at = at.div_ceil(16) * 16;
                let (r, c) = (u32::from(*r), u32::from(*c));
                for i in 0..r {
                    for j in 0..c {
                        let (rr, cc) = if row_major { (i, j) } else { (j, i) };
                        out.push(C::R(reg(at / 16 + rr), Lane::of(kome::num::us(cc))));
                    }
                }
                let end = if row_major { at + (r - 1) * 16 + c * 4 } else { at + (c - 1) * 16 + r * 4 };
                if self.sm3() { end.div_ceil(16) * 16 } else { end }
            }
            Ty::Array(e, n) => {
                at = at.div_ceil(16) * 16;
                let mut end = at;
                // (an element's size once, not a layout of it per element)
                let regs = if *n > 1 { self.cb_regs(e, row_major) } else { 0 };
                for k in 0..*n {
                    let st = at + k * 16 * regs;
                    end = self.cb_layout(e, row_major, st, sym, out);
                }
                end
            }
            Ty::Struct(i) => {
                at = at.div_ceil(16) * 16;
                for f in &self.prog.structs[*i].fields {
                    at = self.cb_layout(&f.ty, f.row_major.unwrap_or(row_major), at, sym, out);
                }
                at
            }
            Ty::Void | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..) => at,
        }
    }

    /// registers one element of `t` takes in a constant buffer array
    fn cb_regs(&self, t: &Ty, row_major: bool) -> u32 {
        let mut v = Vec::new();
        let end = self.cb_layout(t, row_major, 0, 0, &mut v);
        end.div_ceil(16)
    }

    fn rdef_var(&self, name: &str, t: &Ty, row_major: bool, off: u32, end: u32) -> sm4::CbVar {
        let el = if let Ty::Array(e, _) = t { &**e } else { t };
        let mut members = Vec::new();
        if let Ty::Struct(i) = el {
            let mut at = 0u32;
            for f in &self.prog.structs[*i].fields {
                let rm = f.row_major.unwrap_or(row_major);
                let mut v = Vec::new();
                let st = if self.sm3() { at.div_ceil(16) * 16 } else { at };
                let e = self.cb_layout(&f.ty, rm, st, 0, &mut v);
                let first = match v.first() {
                    Some(C::R(Reg { f: RegF::Cb(_, r), .. }, c)) => r * 16 + u32::from(*c) * 4,
                    _ => st,
                };
                members.push(self.rdef_var(&f.name, &f.ty, rm, first, e));
                at = e;
            }
        }
        let mut v = self.rdef_var0(name, t, row_major, off, end);
        v.members = members;
        v
    }

    /// The bytes of `t` packed tight (4 a component), as structured buffers hold it.
    fn tight_bytes(&self, t: &Ty) -> u32 {
        kome::num::u32_of(ncomp(self.prog, t)).saturating_mul(4)
    }

    /// A structured buffer element's RDEF variable: members packed tight (4 bytes a component),
    /// at byte `off`.
    fn rdef_tight(&self, name: &str, t: &Ty, row_major: bool, off: u32) -> sm4::CbVar {
        let el = if let Ty::Array(e, _) = t { &**e } else { t };
        let mut members = Vec::new();
        if let Ty::Struct(i) = el {
            let mut at = 0u32;
            for f in self.prog.structs.get(*i).map_or(&[][..], |s| &s.fields[..]) {
                members.push(self.rdef_tight(&f.name, &f.ty, f.row_major.unwrap_or(row_major), at));
                at = at.saturating_add(self.tight_bytes(&f.ty));
            }
        }
        let mut v = self.rdef_var0(name, t, row_major, off, off.saturating_add(self.tight_bytes(t)));
        v.members = members;
        v.used = true;
        v
    }

    /// The components of `t`, arrays and struct members counted in.
    fn scalars(&self, t: &Ty) -> u32 {
        match t {
            Ty::Array(e, n) => self.scalars(e) * u32::from(*n),
            Ty::Struct(i) => self.prog.structs[*i].fields.iter().map(|f| self.scalars(&f.ty)).sum(),
            Ty::Scalar(_) => 1,
            Ty::Vec(_, n) => u32::from(*n),
            Ty::Mat(_, r, c) => u32::from(*r) * u32::from(*c),
            Ty::Void | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..) => 0,
        }
    }

    fn rdef_var0(&self, name: &str, t: &Ty, row_major: bool, off: u32, end: u32) -> sm4::CbVar {
        let (el, elems) = match t {
            Ty::Array(e, n) => (&**e, crate::sat16(*n)),
            t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Struct(_) | Ty::Tex(..)
                 | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => (t, 0),
        };
        let ty = match el.base() {
            B::Bool => 1,
            B::Int => 2,
            B::Uint => 19,
            B::Float => 3,
        };
        let (class, rows, cols) = match el {
            Ty::Scalar(_) => (0, 1, 1),
            Ty::Vec(_, n) => (1, 1, u16::from(*n)),
            Ty::Mat(_, r, c) => (if row_major { 2 } else { 3 }, u16::from(*r), u16::from(*c)),
            // (a struct: one row of all its members' components)
            Ty::Void | Ty::Array(..) | Ty::Struct(_) | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..)
            | Ty::Legacy(_) | Ty::Str | Ty::Stream(..) => (5, 1, crate::sat16(self.scalars(el))),
        };
        sm4::CbVar {
            name: name.into(),
            offset: off,
            size: end - off,
            class,
            ty: if class == 5 { 0 } else { ty },
            rows,
            cols,
            elems,
            used: false,
            explicit: false,
            members: Vec::new(),
        }
    }

    fn add_cbuffer(&mut self, name: &str, reg: Option<u32>, vars: &[Var]) -> R<()> {
        let sym = kome::num::u32_of(self.cbs.len());
        let mut at = 0;
        let mut rvars = Vec::new();
        for v in vars {
            if v.ty().is_object() {
                self.add_object(v)?;
                continue;
            }
            let rm = v.row_major.unwrap_or(self.o.row_major);
            let start = match (v.packoffset, v.reg) {
                (Some(p), _) => p,
                (None, Some(('c', r))) => r * 16,
                _ => at,
            };
            let mut comps = Vec::new();
            let end = self.cb_layout(v.ty(), rm, start, sym, &mut comps);
            let first = match comps.first() {
                Some(C::R(Reg { f: RegF::Cb(_, r), .. }, c)) => r * 16 + u32::from(*c) * 4,
                _ => start,
            };
            let mut rv = self.rdef_var(&v.name, v.ty(), rm, first, end);
            rv.explicit = matches!(v.reg, Some(('c', _)));
            rvars.push(rv);
            self.cbvar.insert(v.name.clone(), (kome::num::us(sym), rvars.len() - 1));
            self.globals.insert(v.name.clone(), Val::num(v.ty().clone(), comps));
            at = at.max(end);
        }
        self.cbs.push(CbD { name: name.into(), reg, size: at.div_ceil(16), used: false, dynamic: false, vars: rvars });
        Ok(())
    }

    fn add_object(&mut self, v: &Var) -> R<()> {
        let reg = v.reg.map(|r| r.1);
        let obj = match v.ty() {
            Ty::RwTex(d, el) => {
                if v.reg.is_some_and(|r| r.0 != 'u') {
                    return Err(format!("line {}: a RWTexture's register is a u register", v.line));
                }
                self.uavs.push(UavD { name: v.name.clone(), view: sm4::View::Typed, coherent: v.coherent, dim: *d, el: (**el).clone(), reg, used: false });
                Obj::Uav(kome::num::u32_of(self.uavs.len()) - 1)
            }
            Ty::Buf(k, el) => {
                let stride = if k.raw() { 0 } else { self.tight_bytes(el) };
                if stride > 2048 {
                    return Err(format!("line {}: a structured buffer's element is more than 2048 bytes", v.line));
                }
                if !k.raw() && stride == 0 {
                    return Err(format!("line {}: a structured buffer of an element without components", v.line));
                }
                let view = sm4::View::Buf { kind: *k, stride, counter: matches!(k, BufKind::Append | BufKind::Consume) };
                if k.uav() {
                    if v.reg.is_some_and(|r| r.0 != 'u') {
                        return Err(format!("line {}: a read-write buffer's register is a u register", v.line));
                    }
                    self.uavs.push(UavD { name: v.name.clone(), view, coherent: v.coherent, dim: Dim::Buffer, el: (**el).clone(), reg, used: false });
                    Obj::Buf(Res::U(kome::num::u32_of(self.uavs.len()).saturating_sub(1)))
                } else {
                    if v.reg.is_some_and(|r| r.0 != 't') {
                        return Err(format!("line {}: a buffer's register is a t register", v.line));
                    }
                    self.texs.push(TexD { name: v.name.clone(), view, dim: Dim::Buffer, el: (**el).clone(), reg, used: false });
                    Obj::Buf(Res::T(kome::num::u32_of(self.texs.len()).saturating_sub(1)))
                }
            }
            Ty::Tex(d, el) => {
                self.texs.push(TexD { name: v.name.clone(), view: sm4::View::Typed, dim: *d, el: (**el).clone(), reg, used: false });
                Obj::Tex(kome::num::u32_of(self.texs.len()) - 1)
            }
            Ty::Sampler(cmp) => {
                self.samps.push(SampD { name: v.name.clone(), cmp: *cmp, reg, used: false });
                Obj::Samp(kome::num::u32_of(self.samps.len()) - 1)
            }
            Ty::Legacy(d) => {
                self.texs.push(TexD { name: v.name.clone(), view: sm4::View::Typed, dim: *d, el: Ty::Vec(B::Float, Width::W4), reg, used: false });
                self.samps.push(SampD { name: v.name.clone(), cmp: false, reg, used: false });
                Obj::Legacy(kome::num::u32_of(self.texs.len()) - 1, kome::num::u32_of(self.samps.len()) - 1)
            }
            Ty::Array(e, n) => {
                // one descriptor per element, on consecutive registers
                let (t0, s0) = (kome::num::u32_of(self.texs.len()), kome::num::u32_of(self.samps.len()));
                for i in 0..*n {
                    let name = format!("{}[{i}]", v.name);
                    let reg = reg.map(|r| r + i);
                    match &**e {
                        Ty::Tex(d, el) => self.texs.push(TexD { name, view: sm4::View::Typed, dim: *d, el: (**el).clone(), reg, used: false }),
                        Ty::Sampler(cmp) => self.samps.push(SampD { name, cmp: *cmp, reg, used: false }),
                        Ty::Legacy(d) => {
                            self.texs.push(TexD { name: name.clone(), view: sm4::View::Typed, dim: *d, el: Ty::Vec(B::Float, Width::W4), reg, used: false });
                            self.samps.push(SampD { name, cmp: false, reg, used: false });
                        }
                        Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Array(..) | Ty::Struct(_)
                        | Ty::RwTex(..) | Ty::Buf(..) | Ty::Str | Ty::Stream(..) => return Err(format!("line {}: arrays of arrays of textures or samplers are not supported", v.line)),
                    }
                }
                match &**e {
                    Ty::Tex(..) => Obj::TexArr(t0),
                    Ty::Sampler(_) => Obj::SampArr(s0),
                    Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Array(..) | Ty::Struct(_)
                    | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..) => Obj::LegacyArr(t0, s0),
                }
            }
            Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Struct(_) | Ty::Str | Ty::Stream(..) => return Ok(()),
        };
        self.globals.insert(v.name.clone(), Val { ty: v.ty().clone(), c: Vec::new(), obj });
        Ok(())
    }

    /// Initial value of a declared variable (lists flattened).
    fn init_value(&mut self, t: &Ty, e: &Expr) -> R<Val> {
        if let Expr::List(items) = e {
            let mut flat = Vec::new();
            self.flatten_list(items, &mut flat)?;
            // (`T a[] = { .. }`: as many elements as the list fills)
            let sized;
            let t = match t {
                Ty::Array(el, 0) => {
                    let mut eb = Vec::new();
                    leaf_bases(self.prog, el, &mut eb);
                    let total: usize = flat.iter().map(|v| v.c.len()).sum();
                    if eb.is_empty() || total == 0 {
                        return Err("an empty initializer for an array without a size".into());
                    }
                    sized = Ty::Array(el.clone(), kome::num::u32_of(total.div_ceil(eb.len())));
                    &sized
                }
                t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Array(..) | Ty::Struct(_)
                     | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => t,
            };
            let mut tb = Vec::new();
            leaf_bases(self.prog, t, &mut tb);
            let mut comps = Vec::new();
            for v in &flat {
                let mut fb = Vec::new();
                leaf_bases(self.prog, &v.ty, &mut fb);
                for (k, c) in v.c.iter().enumerate() {
                    let at = comps.len();
                    let to = tb.get(at).copied().unwrap_or(fb[k]);
                    comps.extend(self.conv_base(&[*c], fb[k], to));
                }
            }
            if comps.len() < tb.len() {
                return Err(format!("initializer has {} components, {} needed", comps.len(), tb.len()));
            }
            comps.truncate(tb.len());
            return Ok(Val::num(t.clone(), comps));
        }
        let v = self.expr(e)?;
        self.cast(v, t)
    }

    fn flatten_list(&mut self, items: &[Expr], out: &mut Vec<Val>) -> R<()> {
        for i in items {
            if let Expr::List(v) = i {
                self.flatten_list(v, out)?;
            } else {
                out.push(self.expr(i)?);
            }
        }
        Ok(())
    }

    /// Globals: constant buffers, objects, statics (initialized here, at the start of the entry).
    fn setup_globals(&mut self, extra_uniforms: &[Var]) -> R<()> {
        let prog = self.prog;
        let mut uniforms: Vec<Var> = Vec::new();
        let mut statics = Vec::new();
        for v in &prog.globals {
            if v.ty().is_object() {
                self.add_object(v)?;
            } else if v.shared {
                if !self.o.compute() {
                    return Err(format!("line {}: groupshared outside a compute shader", v.line));
                }
                if v.init.is_some() {
                    return Err(format!("line {}: groupshared variables cannot be initialized", v.line));
                }
                if no_size(v.ty()) {
                    return Err(format!("line {}: groupshared array {} without a size", v.line, v.name));
                }
                let bytes = 4 * kome::num::u32_of(ncomp(prog, v.ty()));
                // (32 KB per thread group, as Direct3D 11 allows)
                if self.tgsm.iter().sum::<u32>() + bytes > 32768 {
                    return Err(format!("line {}: more than 32 KB of groupshared memory", v.line));
                }
                self.tgsm.push(bytes);
                let g = kome::num::u32_of(self.tgsm.len()) - 1;
                self.globals.insert(v.name.clone(), Val { ty: v.ty().clone(), c: Vec::new(), obj: Obj::Mem(Mem::G(g, C::I(0))) });
            } else if matches!(v.ty(), Ty::Str) {
            } else if v.is_static {
                statics.push(v.clone());
            } else {
                uniforms.push(v.clone());
            }
        }
        uniforms.extend(extra_uniforms.iter().cloned());
        self.add_cbuffer("$Globals", None, &uniforms)?;
        for cb in &prog.cbufs {
            self.add_cbuffer(&cb.name, cb.reg, &cb.vars)?;
        }
        for v in statics {
            let t = v.ty().clone();
            let val = match &v.init {
                Some(e) => Some(self.init_value(&t, e)?),
                None => None,
            };
            let imm = val.as_ref().filter(|x| x.c.iter().all(|c| matches!(c, C::I(_))));
            if let (true, Some(val)) = (v.is_const, imm) {
                let val = val.clone();
                if let Ty::Array(..) = t {
                    // an immediate constant buffer
                    let mut u = Vec::new();
                    units(self.prog, &t, &mut u);
                    let base = kome::num::u32_of(self.icb.len());
                    let mut comps = Vec::new();
                    let mut k = 0;
                    for (n, w) in u.iter().enumerate() {
                        let mut row = [0u32; 4];
                        for (c, r) in row.iter_mut().enumerate().take(*w) {
                            if let C::I(x) = val.c[k] {
                                *r = x;
                            }
                            comps.push(C::R(Reg { f: RegF::Icb(base + kome::num::u32_of(n)), rel: None }, Lane::of(c)));
                            k += 1;
                        }
                        self.icb.push(row);
                    }
                    self.globals.insert(v.name.clone(), Val::num(t, comps));
                } else {
                    self.globals.insert(v.name.clone(), val);
                }
                continue;
            }
            let s = self.storage(&t);
            let init = match val {
                Some(x) => x.c,
                None => vec![C::I(0); s.len()],
            };
            self.assign(&s, &init);
            self.static_inits.push((s.clone(), t.clone(), v.init.clone()));
            self.globals.insert(v.name.clone(), Val::num(t, s));
        }
        Ok(())
    }

    // ---- the entry point's signature ----

    /// Signature elements of `t` with semantic `sem` (struct members their own): register comps
    /// in component order.
    fn sig(&mut self, t: &Ty, sem: Option<&str>, interp: Interp, output: bool, out: &mut Vec<C>) -> R<()> {
        if let Ty::Struct(i) = t {
            let fields = self.st(*i).fields.clone();
            for f in &fields {
                self.sig(&f.ty, f.sem.as_deref(), f.interp, output, out)?;
            }
            return Ok(());
        }
        let Some(sem) = sem else {
            return Err(format!("missing semantic on a shader {}", if output { "output" } else { "input" }));
        };
        let (name0, mut index) = split_sem(sem);
        let up = name0.to_ascii_uppercase();
        // (a geometry shader's primitive id and instance: vPrim, vGSInstanceID)
        let gs_sys = match up.as_str() {
            "SV_PRIMITIVEID" if self.o.geometry() && !output => Some(dxwgsl::tpf::INPUT_PRIMITIVEID),
            "SV_GSINSTANCEID" if self.o.geometry() && !output && self.o.major >= 5 => Some(dxwgsl::tpf::INPUT_GS_INSTANCE_ID),
            _ => None,
        };
        if let Some(ty) = gs_sys {
            if t.comps() != 1 || t.base() == B::Float || !t.numeric() {
                return Err(format!("{sem} of type {t:?}"));
            }
            if !self.sys_inputs.iter().any(|s| s.0 == ty) {
                self.sys_inputs.push((ty, 1));
            }
            out.push(C::R(Reg { f: RegF::Sys(ty), rel: None }, Lane::X));
            return Ok(());
        }
        // (a hull or domain shader's system inputs: vOutputControlPointID, vPrim, vDomain)
        let tess_sys = match up.as_str() {
            "SV_OUTPUTCONTROLPOINTID" if self.o.hull() && !output && !self.in_pcf => Some((dxwgsl::tpf::OUTPUT_CONTROL_POINT_ID, 1)),
            "SV_PRIMITIVEID" if (self.o.hull() || self.o.domain()) && !output => Some((dxwgsl::tpf::INPUT_PRIMITIVEID, 1)),
            "SV_DOMAINLOCATION" if self.o.domain() && !output => Some((dxwgsl::tpf::INPUT_DOMAIN_POINT, if self.tess_domain == Some(sm4::TessDomain::Tri) { 3 } else { 2 })),
            _ => None,
        };
        if let Some((ty, n)) = tess_sys {
            let float = ty == dxwgsl::tpf::INPUT_DOMAIN_POINT;
            if t.comps() != n || !t.numeric() || (t.base() == B::Float) != float {
                return Err(format!("{sem} of type {t:?}"));
            }
            let mask = Lane::mask(n);
            match self.sys_inputs.iter_mut().find(|s| s.0 == ty) {
                Some(s) => s.1 |= mask,
                None => self.sys_inputs.push((ty, mask)),
            }
            for c in Lane::first(n) {
                out.push(C::R(Reg { f: RegF::Sys(ty), rel: None }, c));
            }
            return Ok(());
        }
        if up == "SV_TESSFACTOR" || up == "SV_INSIDETESSFACTOR" {
            return self.tess_factor_sig(t, up == "SV_INSIDETESSFACTOR", index, output, out);
        }
        if self.o.hull() && !output && !self.sig_cp {
            return Err(format!("semantic {sem}: a hull shader's inputs are its InputPatch, OutputPatch and system values"));
        }
        if self.o.compute() {
            let ty = match up.as_str() {
                "SV_DISPATCHTHREADID" => dxwgsl::tpf::THREAD_ID,
                "SV_GROUPID" => dxwgsl::tpf::THREAD_GROUP_ID,
                "SV_GROUPTHREADID" => dxwgsl::tpf::THREAD_ID_IN_GROUP,
                "SV_GROUPINDEX" => dxwgsl::tpf::THREAD_ID_IN_GROUP_FLATTENED,
                _ => return Err(format!("semantic {sem} is not a compute shader input")),
            };
            if output {
                return Err(format!("{sem} is an input"));
            }
            let n = t.comps();
            if n == 0 || n > if ty == dxwgsl::tpf::THREAD_ID_IN_GROUP_FLATTENED { 1 } else { 3 } || t.base() == B::Float || !t.numeric() {
                return Err(format!("{sem} of type {t:?}"));
            }
            let mask = Lane::mask(n);
            match self.sys_inputs.iter_mut().find(|s| s.0 == ty) {
                Some(s) => s.1 |= mask,
                None => self.sys_inputs.push((ty, mask)),
            }
            for c in Lane::first(n) {
                out.push(C::R(Reg { f: RegF::Sys(ty), rel: None }, c));
            }
            let _ = &mut index;
            return Ok(());
        }
        // (8 render targets, 4 in shader model 2/3; 32 input/output registers)
        let max = if matches!(up.as_str(), "SV_TARGET" | "COLOR") && pixel_out(self.o.pixel(), output) {
            if self.sm3() { 4 } else { 8 }
        } else {
            32
        };
        if index >= max {
            return Err(format!("semantic {sem}: index out of range"));
        }
        let pixel = self.o.pixel();
        // shader model 2/3 names (backward compatibility)
        let (name, sysval) = match (up.as_str(), pixel, output) {
            // (shader model 2/3 pixel shaders: POSITION is not VPOS, not readable)
            ("POSITION", true, false) if self.sm3() => ("POSITION".to_string(), SysVal::Undefined),
            ("SV_POSITION", _, _) => ("SV_Position".to_string(), SysVal::Position),
            ("POSITION", false, true) | ("POSITION", true, false) | ("VPOS", true, false) | ("POSITIONT", false, true) => ("SV_Position".to_string(), SysVal::Position),
            // (in the signature as D3D_NAME_TARGET / DEPTH; declared as plain outputs)
            ("SV_TARGET", true, true) | ("COLOR", true, true) => ("SV_Target".to_string(), SysVal::Target),
            ("SV_DEPTH", true, true) | ("DEPTH", true, true) => ("SV_Depth".to_string(), SysVal::Depth),
            ("SV_VERTEXID", false, false) => ("SV_VertexID".to_string(), SysVal::VertexId),
            ("SV_INSTANCEID", false, false) => ("SV_InstanceID".to_string(), SysVal::InstanceId),
            ("SV_ISFRONTFACE", true, false) | ("VFACE", true, false) => ("SV_IsFrontFace".to_string(), SysVal::IsFrontFace),
            ("SV_PRIMITIVEID", true, false) => ("SV_PrimitiveID".to_string(), SysVal::PrimitiveId),
            ("SV_SAMPLEINDEX", true, false) => ("SV_SampleIndex".to_string(), SysVal::SampleIndex),
            ("SV_CLIPDISTANCE", _, _) => ("SV_ClipDistance".to_string(), SysVal::ClipDistance),
            _ => (name0.clone(), SysVal::Undefined),
        };
        let mut u = Vec::new();
        units(self.prog, t, &mut u);
        // (an array parameter takes a register per element: `float4 a[65536] : TEXCOORD`
        // is past the 32 registers however its first index is)
        if kome::num::us(index) + u.len() > kome::num::us(max) {
            return Err(format!("semantic {sem}: {} registers from index {index}, past {max}", u.len()));
        }
        if name == "POSITION" && pixel && !output && self.sm3() {
            // (unreadable: zeros)
            out.extend(std::iter::repeat_n(C::I(0), u.iter().sum()));
            return Ok(());
        }
        let mut bases = Vec::new();
        leaf_bases(self.prog, t, &mut bases);
        let b = bases.first().copied().unwrap_or(B::Float);
        let mut k = 0;
        // (a geometry shader's streams each number their outputs from o0)
        let stream = if output && self.o.geometry() { self.cur_stream } else { 0 };
        for n in u {
            let list = if self.sig_patch {
                &mut self.patch
            } else if output {
                &mut self.outputs
            } else {
                &mut self.inputs
            };
            let depth = name == "SV_Depth";
            // (a patch constant that is not a tess factor goes into the free components after
            // the first register's taken ones that hold it whole, as d3dcompiler packs the
            // PCSG: winref d3d11tess, `uint pid : PID` after SV_TessFactor[3] at r0.y)
            let packed = if self.sig_patch && sysval == SysVal::Undefined { pc_slot(list, n) } else { None };
            let reg = if let Some((r, _)) = packed {
                r
            } else if name == "SV_Target" {
                index
            } else {
                list.iter().filter(|e| !e.depth && e.name != "SV_Target" && e.stream == stream).map(|e| e.reg.saturating_add(1)).max().unwrap_or(0).max(
                    list.iter().filter(|e| e.name == "SV_Target").map(|e| e.reg.saturating_add(1)).max().unwrap_or(0),
                )
            };
            let interp = if !pixel || output {
                InterpMode::Undefined
            } else if sysval == SysVal::Position {
                InterpMode::LinearNoPerspective
            } else if b != B::Float || sysval != SysVal::Undefined {
                InterpMode::Constant
            } else {
                match interp {
                    Interp::Linear => InterpMode::Linear,
                    Interp::Centroid => InterpMode::LinearCentroid,
                    Interp::NoInterp => InterpMode::Constant,
                    Interp::NoPersp => InterpMode::LinearNoPerspective,
                    Interp::NoPerspCentroid => InterpMode::LinearNoPerspectiveCentroid,
                    Interp::Sample => InterpMode::LinearSample,
                }
            };
            let ct = if sysval == SysVal::IsFrontFace { sm4::CompType::Uint } else { ctype(b) };
            let at = packed.map_or(0, |(_, a)| a);
            list.push(SigEl { name: name.clone(), index, sysval, ctype: ct, reg, mask: Lane::mask(n) << at, interp, depth, stream });
            let f = if depth {
                RegF::Depth
            } else if output {
                RegF::Output(reg)
            } else if self.sig_patch {
                RegF::Pc(reg)
            } else {
                RegF::Input(reg)
            };
            for c in Lane::ALL.into_iter().skip(usize::from(at)).take(n) {
                out.push(C::R(Reg { f, rel: None }, c));
            }
            k += n;
            index += 1;
        }
        let _ = k;
        Ok(())
    }

    /// SV_TessFactor (`inside` false) or SV_InsideTessFactor elements from `index` of type `t`
    /// (a float, or an array of them: one register each, .x): patch constants, each its own
    /// tess factor of the patch's domain.
    fn tess_factor_sig(&mut self, t: &Ty, inside: bool, index: u32, output: bool, out: &mut Vec<C>) -> R<()> {
        let name = if inside { "SV_InsideTessFactor" } else { "SV_TessFactor" };
        if !self.sig_patch {
            return Err(format!("{name} outside the patch constants (a hull shader's patch constant function, a domain shader's input)"));
        }
        let Some(d) = self.tess_domain else { return Err(format!("{name} without [domain(...)]")) };
        let mut u = Vec::new();
        units(self.prog, t, &mut u);
        let mut bases = Vec::new();
        leaf_bases(self.prog, t, &mut bases);
        if u.is_empty() || u.iter().any(|&n| n != 1) || bases.iter().any(|&b| b != B::Float) {
            return Err(format!("{name} of type {t:?}: a float or an array of floats"));
        }
        let (edges, insides) = d.factors();
        for i in (index..).take(u.len()) {
            let f = d.factor(inside, i).ok_or_else(|| format!("{name}[{i}]: a {d:?} patch has {} of them", if inside { insides } else { edges }))?;
            if self.patch.iter().any(|e| e.sysval == SysVal::Tess(f)) {
                return Err(format!("{name}[{i}] twice"));
            }
            let reg = self.patch.iter().map(|e| e.reg.saturating_add(1)).max().unwrap_or(0);
            if reg >= 32 {
                return Err(format!("{name}[{i}]: past the 32 patch constant registers"));
            }
            self.patch.push(SigEl { name: name.into(), index: i, sysval: SysVal::Tess(f), ctype: sm4::CompType::Float, reg, mask: 1, interp: InterpMode::Undefined, depth: false, stream: 0 });
            out.push(C::R(Reg { f: if output { RegF::Output(reg) } else { RegF::Pc(reg) }, rel: None }, Lane::X));
        }
        Ok(())
    }

    /// A string attribute's value ([domain("tri")]...): None without the attribute.
    fn str_attr(f: &Func, n: &str) -> R<Option<String>> {
        match f.attrs.iter().find(|a| a.0.eq_ignore_ascii_case(n)) {
            None => Ok(None),
            Some((_, a)) => match a.as_slice() {
                [Expr::Str(s)] => Ok(Some(s.clone())),
                _ => Err(format!("{n} takes one string")),
            },
        }
    }

    /// A hull shader's attributes: its declarations (the fork phase's start, the counts of the
    /// control points their parameters say, set later), and the patch constant function's name.
    fn hull_attrs(&mut self, f: &Func) -> R<String> {
        let name = &f.name;
        let need = |n: &str| -> R<String> { Self::str_attr(f, n)?.ok_or(format!("{name}: a hull shader needs [{n}(...)]")) };
        let domain = need("domain")?;
        let domain = sm4::TessDomain::of_name(&domain).ok_or(format!("domain(\"{domain}\"): not isoline, tri or quad"))?;
        let part = need("partitioning")?;
        let partitioning = sm4::Partitioning::of_name(&part).ok_or(format!("partitioning(\"{part}\"): not integer, pow2, fractional_odd or fractional_even"))?;
        let topo = need("outputtopology")?;
        let output = sm4::TessOutput::of_name(&topo).ok_or(format!("outputtopology(\"{topo}\"): not point, line, triangle_cw or triangle_ccw"))?;
        // (isolines make points or lines, tris and quads points or triangles)
        let fits = match (domain, output) {
            (sm4::TessDomain::Isoline, sm4::TessOutput::Point | sm4::TessOutput::Line) => true,
            (sm4::TessDomain::Isoline, sm4::TessOutput::TriangleCw | sm4::TessOutput::TriangleCcw) => false,
            (sm4::TessDomain::Tri | sm4::TessDomain::Quad, sm4::TessOutput::Line) => false,
            (sm4::TessDomain::Tri | sm4::TessDomain::Quad, sm4::TessOutput::Point | sm4::TessOutput::TriangleCw | sm4::TessOutput::TriangleCcw) => true,
        };
        if !fits {
            return Err(format!("outputtopology(\"{topo}\") of a {domain:?} domain"));
        }
        let pcf = need("patchconstantfunc")?;
        let cps = match f.attrs.iter().find(|a| a.0.eq_ignore_ascii_case("outputcontrolpoints")) {
            None => return Err(format!("{name}: a hull shader needs [outputcontrolpoints(n)]")),
            Some((_, a)) => match a.as_slice() {
                [e] => super::parse::const_int(e).ok_or("outputcontrolpoints: not a constant")?,
                _ => return Err("outputcontrolpoints takes one number".into()),
            },
        };
        let out_cps = u32::try_from(cps).ok().filter(|n| *n <= 32).ok_or(format!("outputcontrolpoints({cps}) is out of range"))?;
        let max_tess = match f.attrs.iter().find(|a| a.0.eq_ignore_ascii_case("maxtessfactor")) {
            None => None,
            Some((_, a)) => {
                let v = match a.as_slice() {
                    [Expr::Float(v)] => *v,
                    [e] => super::parse::const_int(e).map(kome::num::to_f64).ok_or("maxtessfactor: not a constant")?,
                    _ => return Err("maxtessfactor takes one number".into()),
                };
                if !(1.0..=64.0).contains(&v) {
                    return Err(format!("maxtessfactor({v}) is out of range"));
                }
                Some(kome::num::to_f32(v))
            }
        };
        self.tess_domain = Some(domain);
        self.hs = Some(sm4::Hs { domain, partitioning, output, in_cps: 0, out_cps, max_tess, fork_at: 0, cp_sys: Vec::new(), fork_sys: Vec::new(), fork_icp: false, fork_ocp: false, patch: Vec::new() });
        Ok(pcf)
    }

    /// A patch parameter (InputPatch<T, n> / OutputPatch<T, n>): its control points' registers
    /// (vicp[point][register], or the hull shader's vocp) copied to temporaries.
    fn patch_param(&mut self, k: PatchKind, t: &Ty, p: &Var) -> R<Val> {
        let Ty::Array(el, n) = t else { return Err(format!("{}: a patch is an array", p.name)) };
        let n = *n;
        if !(1..=32).contains(&n) {
            return Err(format!("{}: a patch of {n} control points (1 to 32)", p.name));
        }
        let el: &Ty = el;
        let regs = match (self.o.stage, k) {
            (crate::Stage::Hull, PatchKind::Input) => {
                let Some(h) = self.hs.as_mut() else { return Err("a hull shader without its declarations".into()) };
                if h.in_cps != 0 && h.in_cps != n {
                    return Err(format!("{}: InputPatch of {n} control points, another of {}", p.name, h.in_cps));
                }
                h.in_cps = n;
                if self.in_pcf {
                    h.fork_icp = true;
                }
                match &self.icp {
                    Some((ty, regs)) if ty == el => regs.clone(),
                    Some(_) => return Err(format!("{}: the patch constant function's InputPatch of another type than the control point function's", p.name)),
                    None => {
                        let mut regs = Vec::new();
                        self.sig_cp = true;
                        let r = self.sig(el, p.sem.as_deref(), p.interp, false, &mut regs);
                        self.sig_cp = false;
                        r?;
                        self.icp = Some((el.clone(), regs.clone()));
                        regs
                    }
                }
                .into_iter()
                .map(|c| (c, false))
                .collect::<Vec<_>>()
            }
            (crate::Stage::Hull, PatchKind::Output) => {
                if !self.in_pcf {
                    return Err(format!("{}: an OutputPatch is the patch constant function's", p.name));
                }
                let Some(h) = self.hs.as_mut() else { return Err("a hull shader without its declarations".into()) };
                if n != h.out_cps {
                    return Err(format!("{}: OutputPatch of {n} control points, outputcontrolpoints({})", p.name, h.out_cps));
                }
                h.fork_ocp = true;
                match &self.ocp {
                    Some((ty, regs)) if ty == el => regs.iter().map(|&c| (c, true)).collect(),
                    _ => return Err(format!("{}: an OutputPatch of another type than the control point function returns", p.name)),
                }
            }
            (crate::Stage::Domain, PatchKind::Output) => {
                let Some(d) = self.ds.as_mut() else { return Err("a domain shader without its declarations".into()) };
                if d.cps != 0 {
                    return Err(format!("{}: a second OutputPatch", p.name));
                }
                d.cps = n;
                let mut regs = Vec::new();
                self.sig_cp = true;
                let r = self.sig(el, p.sem.as_deref(), p.interp, false, &mut regs);
                self.sig_cp = false;
                r?;
                regs.into_iter().map(|c| (c, false)).collect()
            }
            (crate::Stage::Domain, PatchKind::Input) => return Err(format!("{}: a domain shader's control points are an OutputPatch", p.name)),
            (crate::Stage::Vertex | crate::Stage::Pixel | crate::Stage::Geometry | crate::Stage::Compute, PatchKind::Input | PatchKind::Output) => {
                return Err(format!("{}: a patch outside a hull or domain shader", p.name));
            }
        };
        let mut all = Vec::new();
        for v in 0..n {
            for &(c, ocp) in &regs {
                // (an input register of the patch's point v: vicp[v][r]; an output one: vocp[v][r])
                all.push(if let C::R(Reg { f, rel }, l) = c {
                    let f = if let (RegF::Input(r), false) = (f, ocp) {
                        RegF::Icp(v, r)
                    } else if let (RegF::Output(r), true) = (f, ocp) {
                        RegF::Ocp(v, r)
                    } else {
                        f
                    };
                    C::R(Reg { f, rel }, l)
                } else {
                    c
                });
            }
        }
        let st = self.storage(t);
        self.assign(&st, &all);
        Ok(Val::num(t.clone(), st))
    }

    /// A hull shader's patch constant function `name`, its fork phase: after the control point
    /// phase, its statics set again, its outputs the patch constants.
    fn hull_pcf(&mut self, name: &str) -> R<()> {
        let prog = self.prog;
        let f = prog.funcs.iter().find(|f| f.name == name && f.body.is_some()).ok_or(format!("patch constant function '{name}' not found"))?;
        let at = self.code.len();
        let cp_sys = std::mem::take(&mut self.sys_inputs);
        let Some(h) = self.hs.as_mut() else { return Err("a hull shader without its declarations".into()) };
        h.fork_at = at;
        h.cp_sys = cp_sys;
        self.in_pcf = true;
        self.scopes.open();
        for (st, t, init) in self.static_inits.clone() {
            let v = match &init {
                Some(e) => self.init_value(&t, e)?.c,
                None => vec![C::I(0); st.len()],
            };
            self.assign(&st, &v);
        }
        let Some(body) = f.body.as_ref() else { return Err(format!("{name} has no body")) };
        let mut outs = Vec::new();
        for p in &f.params {
            let t = p.ty().clone();
            if p.uniform {
                return Err(format!("{name}: uniform parameter {} of a patch constant function", p.name));
            }
            if let Some(k) = p.patch {
                let v = self.patch_param(k, &t, p)?;
                self.scopes.declare(p.name.clone(), v);
                continue;
            }
            let mut regs = Vec::new();
            if p.is_out {
                self.sig_patch = true;
                let r = self.sig(&t, p.sem.as_deref(), p.interp, true, &mut regs);
                self.sig_patch = false;
                r?;
                let s = self.storage(&t);
                outs.push((s.clone(), regs));
                self.scopes.declare(p.name.clone(), Val::num(t, s));
            } else {
                self.sig(&t, p.sem.as_deref(), p.interp, false, &mut regs)?;
                let s = self.storage(&t);
                self.assign(&s, &regs);
                self.scopes.declare(p.name.clone(), Val::num(t, s));
            }
        }
        let mut ret_regs = Vec::new();
        if f.ret != Ty::Void {
            self.sig_patch = true;
            let r = self.sig(&f.ret.clone(), f.ret_sem.as_deref(), Interp::Linear, true, &mut ret_regs);
            self.sig_patch = false;
            r?;
        }
        // (every tess factor of the domain: the tessellator reads them all)
        if let Some(d) = self.tess_domain {
            let (edges, insides) = d.factors();
            for (inside, n) in [(false, edges), (true, insides)] {
                for i in 0..n {
                    let Some(want) = d.factor(inside, i) else { continue };
                    if !self.patch.iter().any(|e| e.sysval == SysVal::Tess(want)) {
                        return Err(format!("{name}: no {}[{i}] (a {d:?} patch's)", if inside { "SV_InsideTessFactor" } else { "SV_TessFactor" }));
                    }
                }
            }
        }
        self.fns.push(FnCtx { ret: Vec::new(), ret_ty: f.ret.clone(), flag: None, loops: 0, entry: Some((outs, ret_regs)) });
        self.stmts(body)?;
        if f.ret == Ty::Void {
            self.write_outputs()?;
        }
        if self.code.last().map(|i| i.op) != Some(sm4::RET) || self.code.len() == at {
            self.flow(sm4::RET);
        }
        let fork_sys = std::mem::take(&mut self.sys_inputs);
        if let Some(h) = self.hs.as_mut() {
            h.fork_sys = fork_sys;
        }
        self.in_pcf = false;
        Ok(())
    }

    // ---- entry ----

    /// The entry point `name`; `args` (an effect's compile statement) give its first uniform
    /// parameters (or ones without a semantic) their values.
    pub fn entry(mut self, name: &str, args: &[Expr]) -> R<sm4::Shader> {
        let prog = self.prog;
        let f = prog.funcs.iter().find(|f| f.name == name && f.body.is_some()).ok_or(format!("entry point '{name}' not found"))?;
        let bindable = |p: &&Var| p.uniform || (p.sem.is_none() && !p.is_out && !matches!(p.ty(), Ty::Struct(_)));
        let bound: Vec<&Var> = f.params.iter().filter(bindable).take(args.len()).collect();
        if bound.len() < args.len() {
            return Err(format!("{name}: {} arguments for {} uniform parameters", args.len(), bound.len()));
        }
        let is_bound = |p: &Var| bound.iter().any(|b| b.name == p.name);
        let uniforms: Vec<Var> = f.params.iter().filter(|p| p.uniform && !is_bound(p)).cloned().collect();
        if self.o.compute() {
            let (_, a) = f.attrs.iter().find(|a| a.0.eq_ignore_ascii_case("numthreads")).ok_or(format!("{name}: a compute shader needs [numthreads(x, y, z)]"))?;
            let n: Vec<i64> = a.iter().map(|e| super::parse::const_int(e).ok_or("numthreads: not a constant")).collect::<Result<_, _>>()?;
            let [x, y, z] = n[..] else { return Err("numthreads takes 3 numbers".into()) };
            // (Direct3D 11: x, y up to 1024, z up to 64, 1024 threads; shader model 4: 768, z 1)
            let max = if self.o.major >= 5 { 1024 } else { 768 };
            if x < 1 || y < 1 || z < 1 || x > 1024 || y > 1024 || z > if self.o.major >= 5 { 64 } else { 1 } || x * y * z > max {
                return Err(format!("numthreads({x}, {y}, {z}) is out of range"));
            }
            let [Ok(x), Ok(y), Ok(z)] = [x, y, z].map(u32::try_from) else {
                return Err(format!("numthreads({x}, {y}, {z}) is out of range"));
            };
            self.threads = Some([x, y, z]);
        }
        if self.o.geometry() {
            let int_attr = |n: &str| -> R<Option<i64>> {
                match f.attrs.iter().find(|a| a.0.eq_ignore_ascii_case(n)) {
                    None => Ok(None),
                    Some((_, a)) => match a.as_slice() {
                        [e] => super::parse::const_int(e).map(Some).ok_or(format!("{n}: not a constant")),
                        _ => Err(format!("{n} takes one number")),
                    },
                }
            };
            let maxout = int_attr("maxvertexcount")?.ok_or(format!("{name}: a geometry shader needs [maxvertexcount(n)]"))?;
            let maxout = u32::try_from(maxout).ok().filter(|m| (1..=1024).contains(m)).ok_or(format!("maxvertexcount({maxout}) is out of range"))?;
            let instances = match int_attr("instance")? {
                None => 1,
                Some(_) if self.o.major < 5 => return Err(format!("{name}: [instance(n)] needs gs_5_0")),
                Some(n) => u32::try_from(n).ok().filter(|n| (1..=32).contains(n)).ok_or(format!("instance({n}) is out of range"))?,
            };
            if f.ret != Ty::Void {
                return Err(format!("{name}: a geometry shader returns void"));
            }
            self.gs = Some(sm4::Gs { input: 0, maxout, instances, streams: Vec::new() });
        }
        let pcf = if self.o.hull() { Some(self.hull_attrs(f)?) } else { None };
        if self.o.domain() {
            let d = Self::str_attr(f, "domain")?.ok_or(format!("{name}: a domain shader needs [domain(...)]"))?;
            let domain = sm4::TessDomain::of_name(&d).ok_or(format!("domain(\"{d}\"): not isoline, tri or quad"))?;
            self.tess_domain = Some(domain);
            self.ds = Some(sm4::Ds { domain, cps: 0, patch: Vec::new() });
        }
        self.scopes.open();
        self.setup_globals(&uniforms)?;
        for (p, a) in bound.iter().zip(args) {
            let v = self.expr(a)?;
            let v = self.cast(v, p.ty())?;
            self.scopes.declare(p.name.clone(), v);
        }
        let Some(body) = f.body.as_ref() else { return Err(format!("{} has no body", f.name)) };
        let mut outs = Vec::new();
        for p in f.params.iter().filter(|p| !p.uniform && !is_bound(p)) {
            let t = p.ty().clone();
            if let Ty::Stream(kind, el) = &t {
                let k = self.gs_stream(*kind, el, p)?;
                self.scopes.declare(p.name.clone(), Val { ty: t.clone(), c: Vec::new(), obj: Obj::Stream(k) });
                continue;
            }
            if let Some(prim) = p.prim {
                let v = self.gs_input(prim, &t, p)?;
                self.scopes.declare(p.name.clone(), v);
                continue;
            }
            if let Some(k) = p.patch {
                let v = self.patch_param(k, &t, p)?;
                self.scopes.declare(p.name.clone(), v);
                continue;
            }
            if self.o.domain() && !p.is_out {
                // (a domain shader's inputs besides its control points and system values: the
                // patch constants)
                let mut regs = Vec::new();
                self.sig_patch = true;
                let r = self.sig(&t, p.sem.as_deref(), p.interp, false, &mut regs);
                self.sig_patch = false;
                r?;
                let s = self.storage(&t);
                self.assign(&s, &regs);
                self.scopes.declare(p.name.clone(), Val::num(t, s));
                continue;
            }
            if p.is_out {
                let mut regs = Vec::new();
                self.sig(&t, p.sem.as_deref(), p.interp, true, &mut regs)?;
                let s = self.storage(&t);
                if p.is_in {
                    let mut iregs = Vec::new();
                    self.sig(&t, p.sem.as_deref(), p.interp, false, &mut iregs)?;
                    self.assign(&s, &iregs);
                }
                outs.push((s.clone(), regs));
                self.scopes.declare(p.name.clone(), Val::num(t, s));
            } else {
                let mut regs = Vec::new();
                self.sig(&t, p.sem.as_deref(), p.interp, false, &mut regs)?;
                let v = if body.iter().any(|s| writes(s, &p.name, &|f| self.has_out(f))) || matches!(t, Ty::Array(..)) {
                    let s = self.storage(&t);
                    self.assign(&s, &regs);
                    s
                } else {
                    regs
                };
                self.scopes.declare(p.name.clone(), Val::num(t, v));
            }
        }
        let mut ret_regs = Vec::new();
        if f.ret != Ty::Void {
            self.sig(&f.ret.clone(), f.ret_sem.as_deref(), Interp::Linear, true, &mut ret_regs)?;
            if self.o.hull() {
                self.ocp = Some((f.ret.clone(), ret_regs.clone()));
            }
        }
        if self.sm3() {
            // (no ret before the end: returns as in an inlined function, outputs at the end)
            let ret = if f.ret == Ty::Void { Vec::new() } else { self.storage(&f.ret) };
            let early = body.iter().enumerate().any(|(i, s)| has_return(s) && (i + 1 < body.len() || !matches!(s, Stmt::Return(..))));
            let flag = if early {
                let t = self.tmp();
                self.marks.push((t, self.code.len()));
                self.assign(&[C::R(temp(t), Lane::X)], &[C::I(0)]);
                Some(t)
            } else {
                None
            };
            self.fns.push(FnCtx { ret: ret.clone(), ret_ty: f.ret.clone(), flag, loops: 0, entry: Some((outs, ret_regs.clone())) });
            self.stmts(body)?;
            self.assign(&ret_regs, &ret);
            self.write_outputs()?;
            return self.finish();
        }
        self.fns.push(FnCtx { ret: Vec::new(), ret_ty: f.ret.clone(), flag: None, loops: 0, entry: Some((outs, ret_regs)) });
        self.stmts(body)?;
        if f.ret == Ty::Void {
            self.write_outputs()?;
        }
        if self.code.last().map(|i| i.op) != Some(sm4::RET) {
            self.flow(sm4::RET);
        }
        if let Some(pcf) = pcf {
            self.hull_pcf(&pcf)?;
            if self.hs.as_ref().is_some_and(|h| h.in_cps == 0) {
                return Err(format!("{name}: a hull shader needs an InputPatch"));
            }
        }
        self.finish()
    }

    /// A geometry shader's output stream parameter: its output signature (stream `k`, the
    /// next one) and the stream's index.
    fn gs_stream(&mut self, kind: StreamKind, el: &Ty, p: &Var) -> R<u32> {
        let k = kome::num::u32_of(self.streams.len());
        if !self.o.geometry() {
            return Err(format!("{}: an output stream outside a geometry shader", p.name));
        }
        if !p.is_out || !p.is_in {
            return Err(format!("{}: an output stream is an inout parameter", p.name));
        }
        if k >= 1 && self.o.major < 5 || k >= 4 {
            return Err(format!("{}: stream {k} (more than one stream needs gs_5_0, at most 4)", p.name));
        }
        self.cur_stream = k;
        let mut regs = Vec::new();
        self.sig(el, p.sem.as_deref(), Interp::Linear, true, &mut regs)?;
        self.cur_stream = 0;
        let topo = match kind {
            StreamKind::Point => 1,
            StreamKind::Line => 3,
            StreamKind::Triangle => 5,
        };
        if let Some(g) = self.gs.as_mut() {
            g.streams.push(topo);
            // (several streams: point lists, as the runtime requires)
            if g.streams.len() > 1 && g.streams.iter().any(|&t| t != 1) {
                return Err(format!("{}: multiple streams must all be PointStreams", p.name));
            }
            // (1024 scalars an invocation may write: D3D11_GS_MAX_OUTPUT_VERTEX_COUNT_ACROSS_INSTANCES)
            let scalars = kome::num::u32_of(regs.len()).saturating_mul(g.maxout);
            if scalars > 1024 {
                return Err(format!("maxvertexcount({}) of {} components: {scalars} scalars, over 1024", g.maxout, regs.len()));
            }
        }
        self.streams.push(GsStream { el: el.clone(), regs });
        Ok(k)
    }

    /// A geometry shader's input array (`triangle V v[3]`): its signature (one vertex's) and
    /// the value, v[vertex][register] copied to temporaries (indexed at run time like any array).
    fn gs_input(&mut self, prim: Prim, t: &Ty, p: &Var) -> R<Val> {
        let Some(g) = self.gs.as_mut() else { return Err(format!("{}: a primitive input outside a geometry shader", p.name)) };
        let Ty::Array(el, n) = t else { return Err(format!("{}: a primitive input is an array", p.name)) };
        if *n != prim.vertices() {
            return Err(format!("{}: {:?} takes {} vertices, not {n}", p.name, prim, prim.vertices()));
        }
        if g.input != 0 {
            return Err(format!("{}: a second primitive input", p.name));
        }
        g.input = prim.code();
        let mut regs = Vec::new();
        self.sig(el, p.sem.as_deref(), p.interp, false, &mut regs)?;
        let mut all = Vec::new();
        for v in 0..*n {
            for c in &regs {
                all.push(if let C::R(Reg { f: RegF::Input(r), rel }, l) = *c { C::R(Reg { f: RegF::GsInput(v, r), rel }, l) } else { *c });
            }
        }
        let st = self.storage(t);
        self.assign(&st, &all);
        Ok(Val::num(t.clone(), st))
    }

    /// Append(v) / RestartStrip() of output stream `k`.
    fn stream_method(&mut self, k: u32, name: &str, args: &[Expr]) -> R<Val> {
        let sm5 = self.o.major >= 5;
        match (name, args) {
            ("Append", [a]) => {
                let v = self.expr(a)?;
                let Some(st) = self.streams.get(kome::num::us(k)) else { return Err("an unknown stream".into()) };
                let (el, regs) = (st.el.clone(), st.regs.clone());
                let v = self.cast(v, &el)?;
                self.assign(&regs, &v.c);
                if sm5 {
                    self.emit(sm4::EMIT_STREAM, vec![], vec![Src::Stream(k)]);
                } else {
                    self.emit(sm4::EMIT, vec![], vec![]);
                }
            }
            ("RestartStrip", []) => {
                if sm5 {
                    self.emit(sm4::CUT_STREAM, vec![], vec![Src::Stream(k)]);
                } else {
                    self.emit(sm4::CUT, vec![], vec![]);
                }
            }
            _ => return Err(format!("{name} of a stream ({} arguments)", args.len())),
        }
        Ok(Val::num(Ty::Void, Vec::new()))
    }

    fn write_outputs(&mut self) -> R<()> {
        let Some((outs, _)) = self.fns.last().and_then(|f| f.entry.clone()) else { return Err("outputs written outside the entry point".into()) };
        for (s, regs) in outs {
            self.assign(&regs, &s);
        }
        // ([maxtessfactor(m)]: a patch constant function's tess factors leave it as min(f, m),
        // a NaN as m: Windows' d3dcompiler, measured in winref d3d11tess, where the domain shader
        // reads 5.5 back for NaN and 64)
        if self.in_pcf && let Some(m) = self.hs.as_ref().and_then(|h| h.max_tess) {
            let regs: Vec<u32> = self.patch.iter().filter(|e| matches!(e.sysval, SysVal::Tess(_))).map(|e| e.reg).collect();
            for r in regs {
                let o = [C::R(Reg { f: RegF::Output(r), rel: None }, Lane::X)];
                let v = self.alu_m(sm4::MIN, B::Float, &[(&o, 0), (&[C::I(m.to_bits())], 0)]);
                self.assign(&o, &v);
            }
        }
        Ok(())
    }

    fn finish(self) -> R<sm4::Shader> {
        // (each structured buffer's element type: RDEF's resource bind info buffers)
        let mut elements = Vec::new();
        let bufs = self.texs.iter().filter(|t| t.used).map(|t| (&t.name, t.view, &t.el)).chain(self.uavs.iter().filter(|u| u.used).map(|u| (&u.name, u.view, &u.el)));
        for (name, view, el) in bufs {
            if let sm4::View::Buf { kind, stride, .. } = view && !kind.raw() {
                let var = self.rdef_tight("$Element", el, self.o.row_major, 0);
                elements.push(sm4::Cb { name: name.clone(), slot: 0, size: stride, dynamic: false, vars: vec![var], explicit: false });
            }
        }
        // resource slots: explicit registers first, then the lowest free in declaration order
        let mut taken: Vec<u32> = self.texs.iter().filter(|t| t.used).filter_map(|t| t.reg).collect();
        let mut tex_map = vec![None; self.texs.len()];
        let mut texs = Vec::new();
        let mut legacy_slot: HashMap<usize, u32> = HashMap::new();
        for (n, t) in self.texs.iter().enumerate() {
            if !t.used {
                continue;
            }
            let slot = t.reg.unwrap_or_else(|| {
                let mut s = 0;
                while taken.contains(&s) {
                    s += 1;
                }
                taken.push(s);
                s
            });
            legacy_slot.insert(n, slot);
            let dim = match t.dim {
                Dim::Buffer => 1,
                Dim::T1D => 2,
                Dim::T2D => 3,
                Dim::T2DMS => 4,
                Dim::T3D => 5,
                Dim::Cube => 6,
                Dim::T1DArray => 7,
                Dim::T2DArray => 8,
                Dim::T2DMSArray => 9,
                Dim::CubeArray => 10,
            };
            let ret = match t.el.base() {
                B::Int => 3,
                B::Uint | B::Bool => 4,
                B::Float => 5,
            };
            tex_map[n] = Some(texs.len());
            texs.push(sm4::Tex { name: t.name.clone(), view: t.view, dim, ret, comps: kome::num::u32_of(t.el.comps().max(1)), slot, explicit: t.reg.is_some() });
        }
        let mut staken: Vec<u32> = self.samps.iter().filter(|t| t.used).filter_map(|t| t.reg).collect();
        let mut samp_map = vec![None; self.samps.len()];
        let mut samps = Vec::new();
        for (n, s) in self.samps.iter().enumerate() {
            if !s.used {
                continue;
            }
            // (a sampler2D: the slot of its texture)
            let legacy = self.globals.values().find_map(|v| match v.obj {
                Obj::Legacy(t, sm) if kome::num::us(sm) == n => legacy_slot.get(&kome::num::us(t)).copied(),
                Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_)
                | Obj::LegacyArr(..) | Obj::Uav(_) | Obj::Mem(_) | Obj::Buf(_) | Obj::Stream(_) => None,
            });
            let slot = s.reg.or(legacy).unwrap_or_else(|| {
                let mut k = 0;
                while staken.contains(&k) {
                    k += 1;
                }
                k
            });
            staken.push(slot);
            samp_map[n] = Some(samps.len());
            samps.push(sm4::Samp { name: s.name.clone(), cmp: s.cmp, slot, explicit: s.reg.is_some() });
        }
        let mut ctaken: Vec<u32> = self.cbs.iter().filter(|c| c.used).filter_map(|c| c.reg).collect();
        let mut cb_map = vec![None; self.cbs.len()];
        let mut cbs = Vec::new();
        for (n, c) in self.cbs.into_iter().enumerate() {
            if !c.used {
                continue;
            }
            let slot = c.reg.unwrap_or_else(|| {
                let mut k = 0;
                while ctaken.contains(&k) {
                    k += 1;
                }
                ctaken.push(k);
                k
            });
            cb_map[n] = Some(cbs.len());
            cbs.push(sm4::Cb { name: c.name, slot, size: c.size, dynamic: c.dynamic, vars: c.vars, explicit: c.reg.is_some() });
        }
        let mut utaken: Vec<u32> = self.uavs.iter().filter(|u| u.used).filter_map(|u| u.reg).collect();
        let mut uav_map = vec![None; self.uavs.len()];
        let mut uavs = Vec::new();
        for (n, u) in self.uavs.iter().enumerate() {
            if !u.used {
                continue;
            }
            let slot = u.reg.unwrap_or_else(|| {
                let mut k = 0;
                while utaken.contains(&k) {
                    k += 1;
                }
                utaken.push(k);
                k
            });
            let dim = match u.dim {
                Dim::Buffer => 1,
                Dim::T1D => 2,
                Dim::T2D => 3,
                Dim::T3D => 5,
                Dim::T1DArray => 7,
                Dim::T2DArray | Dim::T2DMS | Dim::T2DMSArray | Dim::Cube | Dim::CubeArray => 8,
            };
            let ret = match u.el.base() {
                B::Int => 3,
                B::Uint | B::Bool => 4,
                B::Float => 5,
            };
            uav_map[n] = Some(uavs.len());
            uavs.push(sm4::Uav { name: u.name.clone(), view: u.view, coherent: u.coherent, dim, ret, comps: kome::num::u32_of(u.el.comps().max(1)), slot, explicit: u.reg.is_some() });
        }
        let patch = self.patch;
        Ok(sm4::Shader {
            elements,
            threads: self.threads,
            sys_inputs: self.sys_inputs,
            uavs,
            uav_map,
            tgsm: self.tgsm,
            pixel: self.o.pixel(),
            major: self.o.major,
            minor: self.o.minor,
            code: self.code,
            inputs: self.inputs,
            outputs: self.outputs,
            cbs,
            texs,
            samps,
            xs: self.xs,
            icb: self.icb,
            marks: self.marks,
            cb_map,
            tex_map,
            samp_map,
            early_depth: false,
            d3d10: false,
            gs: self.gs,
            hs: self.hs.map(|h| sm4::Hs { patch: patch.clone(), ..h }),
            ds: self.ds.map(|d| sm4::Ds { patch: patch.clone(), ..d }),
        })
    }

    // ---- statements ----

    /// The function being lowered (statements are lowered only inside one)
    fn fctx(&mut self) -> R<&mut FnCtx> {
        self.fns.last_mut().ok_or_else(|| "a statement outside a function".to_string())
    }

    /// Into a loop or switch (`enter`) or out of one.
    fn loop_depth(&mut self, enter: bool) -> R<()> {
        let f = self.fctx()?;
        f.loops = if enter { f.loops.saturating_add(1) } else { f.loops.saturating_sub(1) };
        Ok(())
    }

    fn stmts(&mut self, ss: &[Stmt]) -> R<()> {
        for (i, s) in ss.iter().enumerate() {
            self.stmt(s)?;
            if let Some(f) = self.fctx()?.flag && has_return(s) && i + 1 < ss.len() {
                self.cond_op(sm4::IF, C::R(temp(f), Lane::X), false);
                self.stmts(&ss[i + 1..])?;
                self.flow(sm4::ENDIF);
                return Ok(());
            }
        }
        Ok(())
    }

    fn scoped(&mut self, s: &Stmt) -> R<()> {
        self.scopes.open();
        let r = self.stmt(s);
        self.scopes.close();
        r
    }

    fn truth(&mut self, v: Val) -> R<C> {
        // (a void value has no components: `if (voidfn())`, review A-6)
        let Some(&c) = v.c.first().filter(|_| v.ty.numeric()) else {
            return Err("a condition must be a scalar".into());
        };
        Ok(match v.ty.base() {
            B::Bool => c,
            B::Float => self.alu(sm4::NE, B::Bool, &[&[c], &[C::I(0)]])[0],
            B::Int | B::Uint => c,
        })
    }

    /// after a loop or switch whose body returned (inlined function with a flag): leave the
    /// enclosing loop too
    fn after_breakable(&mut self, body_returns: bool) -> R<()> {
        let f = self.fctx()?;
        if let (Some(flag), true) = (f.flag, body_returns) && f.loops > 0 {
            self.cond_op(sm4::BREAKC, C::R(temp(flag), Lane::X), true);
        }
        Ok(())
    }

    /// One statement; each kind's work in a function of its own, so that nesting costs only
    /// this frame and the kind's (MAX_NEST levels fit the guest's 1 MiB stack).
    fn stmt(&mut self, s: &Stmt) -> R<()> {
        use sm4::*;
        match s {
            Stmt::Expr(e) => {
                self.expr(e)?;
            }
            Stmt::Decl(vs) => self.stmt_decl(vs)?,
            Stmt::Block(v) => {
                self.scopes.open();
                let r = self.stmts(v);
                self.scopes.close();
                r?;
            }
            Stmt::If(c, a, b) => self.stmt_if(c, a, b)?,
            Stmt::For(init, cond, step, body) => self.stmt_for(init, cond, step, body)?,
            Stmt::While(c, body) => self.stmt_while(c, body)?,
            Stmt::Do(body, c) => self.stmt_do(body, c)?,
            Stmt::Switch(v, cases) => self.stmt_switch(v, cases)?,
            Stmt::Break => self.flow(BREAK),
            Stmt::Continue => self.flow(CONTINUE),
            Stmt::Discard => {
                self.emit(DISCARD, vec![], vec![Src::Imm([TRUE; 4], true)]).nz = true;
            }
            Stmt::Return(e, line) => self.stmt_return(e, line)?,
        }
        Ok(())
    }

    #[inline(never)]
    fn stmt_decl(&mut self, vs: &[Var]) -> R<()> {
        for v in vs {
            let t = v.ty().clone();
            if t.is_object() {
                let val = match &v.init {
                    Some(e) => self.expr(e)?,
                    None => return Err(format!("line {}: local texture or sampler without a value", v.line)),
                };
                self.scopes.declare(v.name.clone(), val);
                continue;
            }
            if v.is_static {
                return Err(format!("line {}: static locals are not supported", v.line));
            }
            let init = match &v.init {
                Some(e) => Some(self.init_value(&t, e)?),
                None => None,
            };
            let t = init.as_ref().map_or(t, |i| i.ty.clone());
            // (`T a[];` takes its size from an initializer list; without one it has none)
            if no_size(&t) {
                return Err(format!("line {}: array {} without a size or an initializer", v.line, v.name));
            }
            // (a constant: no storage)
            if v.is_const && !matches!(t, Ty::Array(..)) && let Some(i) = &init && i.c.iter().all(|c| matches!(c, C::I(_))) {
                self.scopes.declare(v.name.clone(), i.clone());
                continue;
            }
            let st = self.storage(&t);
            if let Some(i) = init {
                self.assign(&st, &i.c);
            }
            self.scopes.declare(v.name.clone(), Val::num(t, st));
        }
        Ok(())
    }

    #[inline(never)]
    fn stmt_if(&mut self, c: &Expr, a: &Stmt, b: &Option<Box<Stmt>>) -> R<()> {
        use sm4::*;
        let cv = self.expr(c)?;
        let t = self.truth(cv)?;
        if let C::I(v) = t {
            // a constant condition
            if v != 0 {
                self.scoped(a)?;
            } else if let Some(b) = b {
                self.scoped(b)?;
            }
            return Ok(());
        }
        self.cond_op(IF, t, true);
        self.scoped(a)?;
        if let Some(b) = b {
            self.flow(ELSE);
            self.scoped(b)?;
        }
        self.flow(ENDIF);
        Ok(())
    }

    #[inline(never)]
    fn stmt_for(&mut self, init: &Option<Box<Stmt>>, cond: &Option<Expr>, step: &Option<Expr>, body: &Stmt) -> R<()> {
        use sm4::*;
        self.scopes.open();
        if let Some(i) = init {
            self.stmt(i)?;
        }
        let cont = has_continue(body);
        let first = if cont && step.is_some() {
            let f = self.tmp();
            self.marks.push((f, self.code.len()));
            self.assign(&[C::R(temp(f), Lane::X)], &[C::I(self.tru())]);
            Some(f)
        } else {
            None
        };
        self.flow(LOOP);
        self.loop_depth(true)?;
        if let (Some(f), Some(st)) = (first, step) {
            self.cond_op(IF, C::R(temp(f), Lane::X), false);
            self.expr(st)?;
            self.flow(ENDIF);
            self.assign(&[C::R(temp(f), Lane::X)], &[C::I(0)]);
        }
        if let Some(c) = cond {
            let cv = self.expr(c)?;
            let t = self.truth(cv)?;
            self.cond_op(BREAKC, t, false);
        }
        self.scoped(body)?;
        if let (None, Some(st)) = (first, step) {
            self.expr(st)?;
        }
        self.loop_depth(false)?;
        self.flow(ENDLOOP);
        self.scopes.close();
        self.after_breakable(has_return(body))?;
        Ok(())
    }

    #[inline(never)]
    fn stmt_while(&mut self, c: &Expr, body: &Stmt) -> R<()> {
        use sm4::*;
        self.flow(LOOP);
        self.loop_depth(true)?;
        let cv = self.expr(c)?;
        let t = self.truth(cv)?;
        self.cond_op(BREAKC, t, false);
        self.scoped(body)?;
        self.loop_depth(false)?;
        self.flow(ENDLOOP);
        self.after_breakable(has_return(body))?;
        Ok(())
    }

    #[inline(never)]
    fn stmt_do(&mut self, body: &Stmt, c: &Expr) -> R<()> {
        use sm4::*;
        let cont = has_continue(body);
        let first = if cont {
            let f = self.tmp();
            self.marks.push((f, self.code.len()));
            self.assign(&[C::R(temp(f), Lane::X)], &[C::I(self.tru())]);
            Some(f)
        } else {
            None
        };
        self.flow(LOOP);
        self.loop_depth(true)?;
        if let Some(f) = first {
            self.cond_op(IF, C::R(temp(f), Lane::X), false);
            let cv = self.expr(c)?;
            let t = self.truth(cv)?;
            self.cond_op(BREAKC, t, false);
            self.flow(ENDIF);
            self.assign(&[C::R(temp(f), Lane::X)], &[C::I(0)]);
        }
        self.scoped(body)?;
        if first.is_none() {
            let cv = self.expr(c)?;
            let t = self.truth(cv)?;
            self.cond_op(BREAKC, t, false);
        }
        self.loop_depth(false)?;
        self.flow(ENDLOOP);
        self.after_breakable(has_return(body))?;
        Ok(())
    }

    #[inline(never)]
    fn stmt_switch(&mut self, v: &Expr, cases: &[(Vec<Option<Expr>>, Vec<Stmt>)]) -> R<()> {
        use sm4::*;
        let val = self.expr(v)?;
        let val = self.cast(val, &Ty::Scalar(B::Int))?;
        let s = self.scalar_src(c0(&val.c)?);
        let s = match s {
            Src::Imm(..) => {
                let c = self.copy(&val.c);
                self.scalar_src(c0(&c)?)
            }
            s @ (Src::R { .. } | Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_)) => s,
        };
        self.emit(SWITCH, vec![], vec![s]);
        self.loop_depth(true)?;
        let mut returns = false;
        for (labels, body) in cases {
            for l in labels {
                match l {
                    Some(e) => {
                        let k = super::parse::const_int(e).ok_or("case label is not a constant")?;
                        self.emit(CASE, vec![], vec![Src::Imm([kome::num::lo32(k); 4], true)]);
                    }
                    None => self.flow(DEFAULT),
                }
            }
            self.scopes.open();
            self.stmts(body)?;
            self.scopes.close();
            returns |= body.iter().any(has_return);
            // (a case falling through to the end of the switch)
            if !matches!(self.code.last().map(|i| i.op), Some(BREAK | RET | CONTINUE)) {
                self.flow(BREAK);
            }
        }
        self.loop_depth(false)?;
        self.flow(ENDSWITCH);
        self.after_breakable(returns)?;
        Ok(())
    }

    #[inline(never)]
    fn stmt_return(&mut self, e: &Option<Expr>, line: &u32) -> R<()> {
        use sm4::*;
        let rt = self.fctx()?.ret_ty.clone();
        let val = match e {
            Some(e) if rt != Ty::Void => {
                let v = self.expr(e)?;
                Some(self.cast(v, &rt).map_err(|m| format!("line {line}: return value: {m}"))?)
            }
            Some(e) => {
                self.expr(e)?;
                None
            }
            None => None,
        };
        if let (Some((_, ret_regs)), false) = (self.fctx()?.entry.clone(), self.sm3()) {
            if let Some(v) = val {
                self.assign(&ret_regs, &v.c);
            }
            self.write_outputs()?;
            self.flow(RET);
            return Ok(());
        }
        if let Some(v) = val {
            let r = self.fctx()?.ret.clone();
            self.assign(&r, &v.c);
        }
        if let Some(f) = self.fctx()?.flag {
            self.assign(&[C::R(temp(f), Lane::X)], &[C::I(self.tru())]);
            if self.fctx()?.loops > 0 {
                self.flow(BREAK);
            }
        }
        Ok(())
    }

    // ---- expressions ----

    fn lit_int(&self, v: i64, unsigned: bool) -> Val {
        Val::num(Ty::Scalar(if unsigned { B::Uint } else { B::Int }), vec![C::I(self.ival(v))])
    }

    pub fn expr(&mut self, e: &Expr) -> R<Val> {
        Ok(match e {
            Expr::Int(v, u) => self.lit_int(*v, *u),
            Expr::Float(f) => Val::num(Ty::Scalar(B::Float), vec![C::I(fbits(kome::num::to_f32(*f)))]),
            Expr::Bool(b) => Val::num(Ty::Scalar(B::Bool), vec![C::I(if *b { self.tru() } else { 0 })]),
            Expr::Var(n, line) => self.expr_var(n, *line)?,
            Expr::Comma(a, b) => self.expr_comma(a, b)?,
            Expr::List(_) => return Err("initializer list in an expression".into()),
            Expr::Str(_) => return Err("a string in an expression".into()),
            Expr::Construct(t, args) => self.expr_construct(t, args)?,
            Expr::Cast(t, a) => self.expr_cast(t, a)?,
            Expr::Un(op, a) => self.expr_un(op, a)?,
            Expr::Bin(op, a, b) => self.expr_bin(op, a, b)?,
            Expr::Assign(op, a, b) => self.expr_assign(op, a, b)?,
            Expr::Cond(c, a, b) => self.expr_cond(c, a, b)?,
            Expr::Member(a, name) => self.expr_member(a, name)?,
            Expr::Index(a, i) => self.expr_index(a, i)?,
            Expr::Call(name, args, line) => self.call(name, args, *line)?,
            Expr::Method(o, name, args) => self.method(o, name, args)?,
        })
    }

    // (each kind of expression in a function of its own: `expr` recurses through them, so its
    // frame is what one level of nesting costs; MAX_NEST levels fit the guest's 1 MiB stack)
    #[inline(never)]
    fn expr_var(&mut self, n: &str, line: u32) -> R<Val> {
        let v = self.lookup(n, line)?;
        Ok(self.shared_value(v))
    }

    #[inline(never)]
    fn expr_comma(&mut self, a: &Expr, b: &Expr) -> R<Val> {
        self.expr(a)?;
        self.expr(b)
    }

    #[inline(never)]
    fn expr_cast(&mut self, t: &Ty, a: &Expr) -> R<Val> {
        let v = self.expr(a)?;
        self.cast(v, t)
    }

    #[inline(never)]
    fn expr_construct(&mut self, t: &Ty, args: &[Expr]) -> R<Val> {
        Ok({
            let mut flat = Vec::new();
            for a in args {
                flat.push(self.expr(a)?);
            }
            let mut tb = Vec::new();
            leaf_bases(self.prog, t, &mut tb);
            let n: usize = flat.iter().map(|v| v.c.len()).sum();
            // (float4(x): every component x)
            if n == 1 && tb.len() > 1 && let Some(x) = flat.pop() {
                return self.cast(x, t);
            }
            if n != tb.len() {
                return Err(format!("constructor of {t:?} with {n} components"));
            }
            let mut comps = Vec::new();
            for v in &flat {
                let mut fb = Vec::new();
                leaf_bases(self.prog, &v.ty, &mut fb);
                for (k, c) in v.c.iter().enumerate() {
                    let to = tb[comps.len()];
                    comps.extend(self.conv_base(&[*c], fb[k], to));
                }
            }
            Val::num(t.clone(), comps)
        })
    }

    /// ++ and -- (before or after)
    #[inline(never)]
    fn expr_incdec(&mut self, op: &Un, a: &Expr) -> R<Val> {
        use sm4::*;
        Ok({
            let mark = self.wbs.len();
            self.place_reads = true;
            let p = self.place(a);
            self.place_reads = false;
            let p = p?;
            let old = self.copy(&p.c);
            let fl = p.ty.base() == B::Float || self.sm3();
            let one = if fl { fbits(1.0) } else { 1 };
            let dec = matches!(op, Un::PreDec | Un::PostDec);
            let c = if fl {
                self.alu_m(ADD, B::Float, &[(&old, 0), (&Self::imm(one, old.len()), u8::from(dec))])
            } else {
                let v = if dec { one.wrapping_neg() } else { one };
                self.alu(IADD, p.ty.base(), &[&old, &Self::imm(v, old.len())])
            };
            self.assign(&p.c, &c);
            self.write_back(mark);
            if matches!(op, Un::PostInc | Un::PostDec) {
                Val::num(p.ty, old)
            } else {
                Val::num(p.ty, c)
            }
        })
    }

    #[inline(never)]
    fn expr_un(&mut self, op: &Un, a: &Expr) -> R<Val> {
        use sm4::*;
        Ok(match op {
        Un::Neg => {
            let v = self.expr(a)?;
            let b = if v.ty.base() == B::Bool { B::Int } else { v.ty.base() };
            let v = self.cast(v.clone(), &v.ty.with_base(b))?;
            let c = if b == B::Float {
                self.alu_m(MOV, b, &[(&v.c, 1)])
            } else {
                self.alu(INEG, b, &[&v.c])
            };
            Val::num(v.ty, c)
        }
        Un::Not => {
            let v = self.expr(a)?;
            let v = self.cast(v.clone(), &v.ty.with_base(B::Bool))?;
            let c = self.alu(IEQ, B::Bool, &[&v.c, &Self::imm(0, v.c.len())]);
            Val::num(v.ty, c)
        }
        Un::BitNot => {
            if self.sm3() {
                return Err("bitwise operators need shader model 4".into());
            }
            let v = self.expr(a)?;
            let c = self.alu(NOT, v.ty.base(), &[&v.c]);
            Val::num(v.ty, c)
        }
        Un::PreInc | Un::PreDec | Un::PostInc | Un::PostDec => self.expr_incdec(op, a)?,
        })
    }

    #[inline(never)]
    fn expr_bin(&mut self, op: &Bin, a: &Expr, b: &Expr) -> R<Val> {
        Ok({
            let x = self.expr(a)?;
            let y = self.expr(b)?;
            self.binary(*op, x, y)?
        })
    }

    #[inline(never)]
    fn expr_assign(&mut self, op: &Option<Bin>, a: &Expr, b: &Expr) -> R<Val> {
        Ok({
            let mark = self.wbs.len();
            self.place_reads = op.is_some();
            let p = self.place(a);
            self.place_reads = false;
            let p = p?;
            let mut v = self.expr(b)?;
            if let Some(op) = op {
                let cur = Val::num(p.ty.clone(), p.c.clone());
                v = self.binary(*op, cur, v)?;
            }
            let v = self.cast(v, &p.ty)?;
            self.assign(&p.c, &v.c);
            self.write_back(mark);
            Val::num(p.ty, p.c)
        })
    }

    #[inline(never)]
    fn expr_cond(&mut self, c: &Expr, a: &Expr, b: &Expr) -> R<Val> {
        use sm4::*;
        Ok({
            let cv = self.expr(c)?;
            if !cv.ty.numeric() || cv.c.is_empty() {
                return Err("the condition of ?: must be a number".into());
            }
            let x = self.expr(a)?;
            let y = self.expr(b)?;
            let t = if x.ty.numeric() && y.ty.numeric() { self.common(&x.ty, &y.ty)? } else { x.ty.clone() };
            // (a vector condition picks per component)
            let t = if cv.ty.comps() > 1 && t.comps() == 1 { Ty::vec_of(t.base(), cv.ty.comps()).ok_or("a condition of more than 4 components")? } else { t };
            // (a narrower vector condition: the values truncated to it, as fxc does)
            let t = match t {
                Ty::Vec(b, n) if cv.ty.comps() > 1 && kome::num::u32_of(cv.ty.comps()) < u32::from(n) => Ty::vec_of(b, cv.ty.comps()).ok_or("a condition of more than 4 components")?,
                t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Mat(..) | Ty::Array(..) | Ty::Struct(_)
                     | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => t,
            };
            let x = self.cast(x, &t)?;
            let y = self.cast(y, &t)?;
            let cb = self.cast(cv.clone(), &cv.ty.with_base(B::Bool))?;
            let n = x.c.len();
            let cc: Vec<C> = if cb.c.len() == 1 { vec![cb.c[0]; n] } else { cb.c[..n.min(cb.c.len())].to_vec() };
            // (a vector condition for a matrix: as many components as the values, or the
            // result would be typed with more components than it has, review A-6)
            if cc.len() != n {
                return Err("the condition of ?: and its values differ in size".into());
            }
            if let Some(C::I(v)) = cc.first().filter(|_| cc.iter().all(|c| *c == cc[0])) {
                return Ok(if *v != 0 { x } else { y });
            }
            let r = self.alu(MOVC, t.base(), &[&cc, &x.c, &y.c]);
            Val::num(t, r)
        })
    }

    #[inline(never)]
    fn expr_member(&mut self, a: &Expr, name: &str) -> R<Val> {
        Ok({
            let v = self.expr_place_or_value(a)?;
            let v = self.member(v, name)?;
            self.shared_value(v)
        })
    }

    #[inline(never)]
    fn expr_index(&mut self, a: &Expr, i: &Expr) -> R<Val> {
        Ok({
            let v = self.expr_place_or_value(a)?;
            let iv = self.expr(i)?;
            let v = self.index(v, iv)?;
            self.shared_value(v)
        })
    }

    /// An expression that may name groupshared memory: not read yet when it does (indexing and
    /// members pick a part first).
    fn expr_place_or_value(&mut self, e: &Expr) -> R<Val> {
        match e {
            Expr::Var(n, line) => self.lookup(n, *line),
            Expr::Index(a, i) => {
                let v = self.expr_place_or_value(a)?;
                let iv = self.expr(i)?;
                self.index(v, iv)
            }
            Expr::Member(a, name) => {
                let v = self.expr_place_or_value(a)?;
                self.member(v, name)
            }
            e @ (Expr::Int(..) | Expr::Float(_) | Expr::Bool(_) | Expr::Bin(..) | Expr::Un(..) | Expr::Assign(..)
                 | Expr::Cond(..) | Expr::Call(..) | Expr::Construct(..) | Expr::Cast(..) | Expr::Method(..) | Expr::List(_)
                 | Expr::Comma(..) | Expr::Str(_)) => self.expr(e),
        }
    }

    /// An lvalue: storage comps.
    #[inline(never)]
    fn place(&mut self, e: &Expr) -> R<Val> {
        let v = self.place_in(e)?;
        let v = match v.obj {
            // (memory: through a copy, stored back)
            Obj::Mem(_) => self.mem_place(v)?,
            Obj::Buf(_) => return Err("assignment to a buffer object".into()),
            Obj::Stream(_) => v,
            Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_)
            | Obj::LegacyArr(..) | Obj::Uav(_) => v,
        };
        if !v.c.iter().all(writable) {
            return Err("assignment to a constant, a uniform or an input".into());
        }
        Ok(v)
    }

    /// `place` without the last step: groupshared memory is still the memory (arrays and
    /// structs there are indexed and picked from without reading them).
    #[inline(never)]
    fn place_in(&mut self, e: &Expr) -> R<Val> {
        Ok(match e {
            Expr::Var(n, line) => self.lookup(n, *line)?,
            Expr::Member(a, n) => {
                let v = self.place_in(a)?;
                match v.obj {
                    Obj::Mem(_) if matches!(v.ty, Ty::Struct(_)) => self.mem_member(v, n)?,
                    Obj::Mem(_) => {
                        let p = self.mem_place(v)?;
                        self.member(p, n)?
                    }
                    Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_)
                    | Obj::LegacyArr(..) | Obj::Uav(_) | Obj::Buf(_) | Obj::Stream(_) => self.member(v, n)?,
                }
            }
            Expr::Index(a, i) => {
                let v = self.place_in(a)?;
                let v = match v.obj {
                    Obj::Mem(_) if !matches!(v.ty, Ty::Array(..)) => self.mem_place(v)?,
                    Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_)
                    | Obj::LegacyArr(..) | Obj::Uav(_) | Obj::Mem(_) | Obj::Buf(_) | Obj::Stream(_) => v,
                };
                let mut iv = self.expr(i)?;
                if let (Ty::RwTex(_, el), Obj::Uav(u)) = (&v.ty, v.obj) {
                    // (written through a copy, stored when the write is done)
                    let el = (**el).clone();
                    let coord = self.uav_coord(u, iv)?;
                    let cp = if self.place_reads {
                        let cur = self.uav_load(u, &coord);
                        self.copy(&cur.c)
                    } else {
                        self.storage(&el)
                    };
                    self.wbs.push(Wb::Uav(u, coord, cp.clone()));
                    return Ok(Val::num(el, cp));
                }
                if let Obj::Mem(_) | Obj::Buf(_) = v.obj {
                    return self.index(v, iv);
                }
                if matches!(iv.c.first(), Some(C::I(_))) || !matches!(v.ty, Ty::Array(..) | Ty::Vec(..) | Ty::Mat(..)) {
                    self.index(v, iv)?
                } else {
                    iv = self.cast(iv, &Ty::Scalar(B::Int))?;
                    let r = self.index(v.clone(), iv.clone())?;
                    // (a register indexed relative to the variable's own storage is the place)
                    let own = |c: &C| match c {
                        C::R(Reg { f, rel: Some(_) }, _) => v.c.iter().any(|d| matches!(d, C::R(Reg { f: g, .. }, _) if g == f)),
                        C::R(..) | C::I(_) => false,
                    };
                    if !r.c.iter().all(own) {
                        let k = match &v.ty {
                            Ty::Array(e, _) => ncomp(self.prog, e),
                            Ty::Mat(_, _, c) => usize::from(*c),
                            Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Struct(_) | Ty::Tex(..) | Ty::Sampler(_)
                            | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..) => 1,
                        };
                        // (the elements are written back: a constant buffer's or an input's are not
                        // places)
                        if !v.c.iter().all(writable) {
                            return Err("assignment to a constant, a uniform or an input".into());
                        }
                        let elems: Vec<Vec<C>> = v.c.chunks(k.max(1)).map(|e| e.to_vec()).collect();
                        // (the select may hand back an element itself: write to a copy)
                        let cp = if r.c.iter().any(|c| v.c.contains(c)) { self.copy(&r.c) } else { r.c.clone() };
                        self.wbs.push(Wb::Sel(elems, iv.c[0], cp.clone(), r.ty.base()));
                        Val::num(r.ty, cp)
                    } else {
                        r
                    }
                }
            }
            Expr::Int(..) | Expr::Float(_) | Expr::Bool(_) | Expr::Bin(..) | Expr::Un(..) | Expr::Assign(..)
            | Expr::Cond(..) | Expr::Call(..) | Expr::Construct(..) | Expr::Cast(..) | Expr::Method(..) | Expr::List(_)
            | Expr::Comma(..) | Expr::Str(_) => return Err("assignment to something that is not a variable".into()),
        })
    }

    /// Stores the places picked by a run-time index since `mark` back (innermost first):
    /// element k = index == k ? the copy : element k.
    #[inline(never)]
    fn write_back(&mut self, mark: usize) {
        while let Some(wb) = (self.wbs.len() > mark).then(|| self.wbs.pop()).flatten() {
            match wb {
                Wb::Sel(elems, iv, cp, b) => {
                    for (k, e) in elems.iter().enumerate() {
                        let m = self.alu(sm4::IEQ, B::Bool, &[&[iv], &[C::I(self.ival(i64::from(kome::num::u32_of(k))))]])[0];
                        let n = e.len();
                        let v = self.alu(sm4::MOVC, b, &[&vec![m; n], &cp, e]);
                        self.assign(e, &v);
                    }
                }
                Wb::Mem(m, ty, cp, from) => {
                    // (the copy's components the code since `from` wrote)
                    let written: Vec<bool> = cp
                        .iter()
                        .map(|c| match c {
                            C::R(r, k) => self.code.get(from..).unwrap_or_default().iter().any(|i| i.dst.iter().any(|d| d.r == *r && d.mask & (1 << u8::from(*k)) != 0)),
                            C::I(_) => false,
                        })
                        .collect();
                    self.mem_store(m, &ty, &cp, &written);
                }
                Wb::Uav(u, coord, cp) => {
                    let a = self.opnd(&coord);
                    let mut v = cp.clone();
                    // (all four components are written: the view's format drops the rest)
                    while v.len() < 4 {
                        v.push(v[0]);
                    }
                    let s = self.opnd(&v);
                    self.emit(sm4::STORE_UAV_TYPED, vec![Dst { r: Reg { f: RegF::Uav(u), rel: None }, mask: 0xf }], vec![a, s]);
                }
            }
        }
    }

    // ---- compute shader memory ----

    /// A value in memory (not an array) through a copy that is stored back.
    #[inline(never)]
    fn mem_place(&mut self, v: Val) -> R<Val> {
        let Obj::Mem(m) = v.obj else { return Err("not memory".into()) };
        if matches!(v.ty, Ty::Array(..)) {
            return Err(match m {
                Mem::G(..) => "a whole groupshared array as a value",
                Mem::S(..) => "a whole array in a buffer as a value",
            }
            .into());
        }
        let w = match m {
            Mem::G(g, o) => WMem::G(g, o),
            Mem::S(Res::U(u), i, o, rm) => WMem::U(u, i, o, rm),
            Mem::S(Res::T(_), ..) => return Err("assignment to a read-only buffer".into()),
        };
        let cur = self.mem_load(m, &v.ty);
        let cp = self.copy(&cur);
        self.wbs.push(Wb::Mem(w, v.ty.clone(), cp.clone(), self.code.len()));
        Ok(Val::num(v.ty, cp))
    }

    /// A value in memory read now (arrays stay places).
    #[inline(never)]
    fn shared_value(&mut self, v: Val) -> Val {
        match v.obj {
            Obj::Mem(m) if !matches!(v.ty, Ty::Array(..)) => {
                let c = self.mem_load(m, &v.ty);
                Val::num(v.ty, c)
            }
            Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_)
            | Obj::LegacyArr(..) | Obj::Uav(_) | Obj::Mem(_) | Obj::Buf(_) | Obj::Stream(_) => v,
        }
    }

    /// The dword (from the value's start) of each of a value's components in memory: packed
    /// tight, struct members in order, matrices by rows (`rm`) or by columns.
    fn mem_layout(&self, t: &Ty, rm: bool, at: u32, out: &mut Vec<u32>) {
        match t {
            Ty::Mat(_, r, c) => {
                let (r, c) = (u32::from(*r), u32::from(*c));
                for i in 0..r {
                    for j in 0..c {
                        let k = if rm { i.saturating_mul(c).saturating_add(j) } else { j.saturating_mul(r).saturating_add(i) };
                        out.push(at.saturating_add(k));
                    }
                }
            }
            Ty::Array(e, n) => {
                let k = kome::num::u32_of(ncomp(self.prog, e));
                for i in 0..*n {
                    self.mem_layout(e, rm, at.saturating_add(i.saturating_mul(k)), out);
                }
            }
            Ty::Struct(i) => {
                let mut a = at;
                for f in self.prog.structs.get(*i).map_or(&[][..], |s| &s.fields[..]) {
                    self.mem_layout(&f.ty, f.row_major.unwrap_or(rm), a, out);
                    a = a.saturating_add(kome::num::u32_of(ncomp(self.prog, &f.ty)));
                }
            }
            t @ (Ty::Void | Ty::Scalar(_) | Ty::Vec(..) | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..)
                 | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => out.extend(at..at.saturating_add(kome::num::u32_of(t.comps()))),
        }
    }

    /// `off` + `k` dwords as a byte address
    fn shared_addr(&mut self, off: C, k: u32) -> C {
        let b = self.alu(sm4::ISHL, B::Int, &[&[off], &[C::I(2)]])[0];
        if k == 0 { b } else { self.alu(sm4::IADD, B::Int, &[&[b], &[C::I(4 * k)]])[0] }
    }

    /// Reads `n` (1 to 4) dwords of memory from `k` dwords past the place into a temp.
    #[inline(never)]
    fn mem_read4(&mut self, m: Mem, k: u32, n: usize) -> Vec<C> {
        let a = self.shared_addr(m.off(), k);
        let a = self.scalar_src(a);
        let t = self.tmp();
        let d = vec![Dst { r: temp(t), mask: Lane::mask(n) }];
        match m {
            Mem::G(g, _) => {
                self.emit(sm4::LD_RAW, d, vec![a, Src::G(g, [0, 1, 2, 3])]);
            }
            Mem::S(r, i, _, _) => {
                let i = self.scalar_src(i);
                let res = match r {
                    Res::T(t) => Src::Tex(t, [0, 1, 2, 3]),
                    Res::U(u) => Src::Uav(u, [0, 1, 2, 3]),
                };
                self.emit(sm4::LD_STRUCTURED, d, vec![i, a, res]);
            }
        }
        Lane::first(n).map(|c| C::R(temp(t), c)).collect()
    }

    /// A value of type `t` read from memory. Bools of buffers are canonical masks (what the
    /// CPU wrote there may be 1).
    #[inline(never)]
    fn mem_load(&mut self, m: Mem, t: &Ty) -> Vec<C> {
        let rm = match m {
            Mem::G(..) => true,
            Mem::S(_, _, _, rm) => rm,
        };
        let mut lay = Vec::new();
        self.mem_layout(t, rm, 0, &mut lay);
        let n = lay.iter().max().map_or(0, |&m| m.saturating_add(1));
        let mut words = Vec::new();
        for k in (0..n).step_by(4) {
            let w = self.mem_read4(m, k, kome::num::us(n.saturating_sub(k).min(4)));
            words.extend(w);
        }
        let mut out: Vec<C> = lay.iter().filter_map(|&d| words.get(kome::num::us(d)).copied()).collect();
        if let Mem::S(..) = m {
            let mut bases = Vec::new();
            leaf_bases(self.prog, t, &mut bases);
            for (c, b) in out.iter_mut().zip(bases) {
                if b == B::Bool && let Some(&x) = self.alu(sm4::INE, B::Bool, &[&[*c], &[C::I(0)]]).first() {
                    *c = x;
                }
            }
        }
        out
    }

    /// Stores the components of `v` (a value of type `t`) that `written` names, in runs of up
    /// to 4 dwords next to each other.
    #[inline(never)]
    fn mem_store(&mut self, m: WMem, t: &Ty, v: &[C], written: &[bool]) {
        let rm = match m {
            WMem::G(..) => true,
            WMem::U(_, _, _, rm) => rm,
        };
        let mut lay = Vec::new();
        self.mem_layout(t, rm, 0, &mut lay);
        let mut at: Vec<(u32, C)> = lay.iter().zip(v).zip(written).filter(|(_, w)| **w).map(|((&d, &c), _)| (d, c)).collect();
        at.sort_by_key(|x| x.0);
        // (runs of up to 4 dwords next to each other)
        let mut runs: Vec<(u32, Vec<C>)> = Vec::new();
        for (d, c) in at {
            match runs.last_mut() {
                Some((first, run)) if run.len() < 4 && first.checked_add(kome::num::u32_of(run.len())) == Some(d) => run.push(c),
                _ => runs.push((d, vec![c])),
            }
        }
        for (first, run) in runs {
            let a = self.shared_addr(m.mem().off(), first);
            let a = self.scalar_src(a);
            let s = self.opnd(&run);
            let mask = Lane::mask(run.len());
            match m {
                WMem::G(g, _) => {
                    self.emit(sm4::STORE_RAW, vec![Dst { r: Reg { f: RegF::G(g), rel: None }, mask }], vec![a, s]);
                }
                WMem::U(u, i, _, _) => {
                    let i = self.scalar_src(i);
                    self.emit(sm4::STORE_STRUCTURED, vec![Dst { r: Reg { f: RegF::Uav(u), rel: None }, mask }], vec![i, a, s]);
                }
            }
        }
    }

    /// Stores a whole value of type `t` to memory.
    fn mem_store_all(&mut self, m: WMem, t: &Ty, v: &[C]) {
        let all = vec![true; v.len()];
        self.mem_store(m, t, v, &all);
    }

    /// A member of a struct in memory, as a place.
    #[inline(never)]
    fn mem_member(&mut self, v: Val, name: &str) -> R<Val> {
        let (Obj::Mem(m), Ty::Struct(i)) = (v.obj, &v.ty) else { return Err(format!(".{name} of memory that is no struct")) };
        let mut at = 0usize;
        for f in self.prog.structs.get(*i).map_or(&[][..], |s| &s.fields[..]) {
            let n = ncomp(self.prog, &f.ty);
            if f.name == name {
                let off = c0(&self.alu(sm4::IADD, B::Int, &[&[m.off()], &[C::I(kome::num::u32_of(at))]]))?;
                let m = match m.at(off) {
                    Mem::S(r, i, o, rm) => Mem::S(r, i, o, f.row_major.unwrap_or(rm)),
                    g @ Mem::G(..) => g,
                };
                return Ok(Val { ty: f.ty.clone(), c: Vec::new(), obj: Obj::Mem(m) });
            }
            at = at.saturating_add(n);
        }
        Err(format!("no member '{name}'"))
    }

    /// A UAV's coordinates (uint, as many as its dimension takes).
    fn uav_coord(&mut self, u: u32, iv: Val) -> R<Vec<C>> {
        let d = self.uavs[kome::num::us(u)].dim;
        let n = d.coords() + usize::from(d.array());
        let iv = self.cast(iv.clone(), &Ty::vec_of(B::Uint, iv.ty.comps().max(1)).ok_or("a UAV coordinate of more than 4 components")?)?;
        if iv.c.len() != n {
            return Err(format!("{n} coordinates index this RWTexture"));
        }
        Ok(iv.c)
    }

    fn uav_load(&mut self, u: u32, coord: &[C]) -> Val {
        let el = self.uavs[kome::num::us(u)].el.clone();
        let a = self.opnd(coord);
        let t = self.tmp();
        self.emit(sm4::LD_UAV_TYPED, vec![Dst { r: temp(t), mask: 0xf }], vec![a, Src::Uav(u, [0, 1, 2, 3])]);
        let n = el.comps().max(1);
        Val::num(el, Lane::first(n).map(|k| C::R(temp(t), k)).collect())
    }

    fn member(&mut self, v: Val, name: &str) -> R<Val> {
        if let Obj::Mem(_) = v.obj {
            if matches!(v.ty, Ty::Struct(_)) {
                return self.mem_member(v, name);
            }
            // (an array stays a place in shared_value: .x of it is no member)
            if matches!(v.ty, Ty::Array(..)) {
                return Err(format!(".{name} of a groupshared array"));
            }
            let v = self.shared_value(v);
            return self.member(v, name);
        }
        match &v.ty {
            Ty::Scalar(b) | Ty::Vec(b, _) => {
                let n = v.c.len();
                let s = swizzle_of(name, n).ok_or(format!("bad swizzle .{name}"))?;
                let c: Vec<C> = s.iter().map(|&i| v.c[i]).collect();
                Ok(Val::num(Ty::vec_of(*b, c.len()).ok_or(format!("bad swizzle .{name}"))?, c))
            }
            Ty::Mat(b, r, c) => {
                let s = mat_swizzle(name, usize::from(*r), usize::from(*c)).ok_or(format!("bad matrix member .{name}"))?;
                let comps: Vec<C> = s.iter().map(|&(i, j)| v.c[i * usize::from(*c) + j]).collect();
                Ok(Val::num(Ty::vec_of(*b, comps.len()).ok_or(format!("bad matrix member .{name}"))?, comps))
            }
            Ty::Struct(i) => {
                let mut at = 0;
                for f in &self.prog.structs[*i].fields {
                    let n = ncomp(self.prog, &f.ty);
                    if f.name == name {
                        return Ok(Val::num(f.ty.clone(), v.c[at..at + n].to_vec()));
                    }
                    at += n;
                }
                Err(format!("no member '{name}'"))
            }
            t @ (Ty::Void | Ty::Array(..) | Ty::Tex(..) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => Err(format!("member .{name} of {t:?}")),
        }
    }

    /// An index value as an int comp in a temp register.
    fn index_reg(&mut self, iv: Val, stride: u32) -> R<(u32, Lane)> {
        let iv = self.cast(iv, &Ty::Scalar(B::Int))?;
        let mut c = iv.c[0];
        if stride > 1 && self.sm3() {
            c = self.alu(sm4::MUL, B::Float, &[&[c], &[C::I(fbits(kome::num::to_f32(stride)))]])[0];
        } else if stride > 1 {
            c = if stride.is_power_of_two() {
                self.alu(sm4::ISHL, B::Int, &[&[c], &[C::I(stride.trailing_zeros())]])[0]
            } else {
                self.alu2(sm4::IMUL, &[&[c], &[C::I(stride)]], true)[0]
            };
        }
        if let C::R(Reg { f: RegF::Temp(t), rel: None }, k) = c {
            return Ok((t, k));
        }
        let cp = self.copy(&[c]);
        let C::R(Reg { f: RegF::Temp(t), .. }, k) = cp[0] else { unreachable!() };
        Ok((t, k))
    }

    /// element `i` of elements (each a comp list): constant index, a register with a relative
    /// index, or a select chain
    fn select(&mut self, elems: Vec<Vec<C>>, iv: Val, ty: Ty) -> R<Val> {
        if let Some(C::I(k)) = iv.c.first() {
            let k = if iv.ty.base() == B::Float || self.sm3() { kome::num::trunc_i64(flt(*k)) } else { i64::from(k.cast_signed()) };
            let e = usize::try_from(k).ok().and_then(|k| elems.get(k)).ok_or(format!("index {k} out of range"))?;
            return Ok(Val::num(ty, e.clone()));
        }
        // a regular layout: element k = element 0 with register + k * stride
        let addressable = |c: &C| matches!(c, C::R(Reg { f: RegF::X(..) | RegF::Cb(..) | RegF::Icb(_), rel: None }, _));
        let regno = |c: &C| match c {
            C::R(Reg { f: RegF::X(_, r) | RegF::Cb(_, r) | RegF::Icb(r), .. }, _) => *r,
            C::R(..) | C::I(_) => 0,
        };
        let e0 = elems.first().ok_or("index into an array of no elements")?;
        // (shader model 3 pixel shaders: no address register)
        if !e0.is_empty() && e0.iter().all(addressable) && !(self.sm3() && self.o.pixel()) {
            let stride = if elems.len() > 1 { regno(&elems[1][0]).wrapping_sub(regno(&e0[0])) } else { 1 };
            let regular = elems.iter().enumerate().all(|(k, e)| {
                e.len() == e0.len()
                    && e.iter().zip(e0).all(|(a, b)| match (a, b) {
                        (C::R(ra, ca), C::R(rb, cb)) => {
                            ca == cb
                                && regno(a) == regno(b).wrapping_add(stride.wrapping_mul(kome::num::u32_of(k)))
                                && std::mem::discriminant(&ra.f) == std::mem::discriminant(&rb.f)
                        }
                        _ => false,
                    })
            });
            if regular && stride > 0 && stride < 4096 {
                let rel = self.index_reg(iv, stride)?;
                let mut out = Vec::new();
                for c in e0 {
                    if let C::R(r, k) = c {
                        if let RegF::Cb(s, _) = r.f {
                            let cb = &mut self.cbs[kome::num::us(s)];
                            cb.dynamic = true;
                        }
                        out.push(C::R(Reg { f: r.f, rel: Some(rel) }, *k));
                    }
                }
                return Ok(Val::num(ty, out));
            }
        }
        // a select chain
        let iv = self.cast(iv, &Ty::Scalar(B::Int))?;
        let mut cur = elems[0].clone();
        for (k, e) in elems.iter().enumerate().skip(1) {
            let m = self.alu(sm4::IEQ, B::Bool, &[&iv.c, &[C::I(self.ival(i64::from(kome::num::u32_of(k))))]])[0];
            let n = cur.len();
            cur = self.alu(sm4::MOVC, ty.base(), &[&vec![m; n], e, &cur]);
        }
        Ok(Val::num(ty, cur))
    }

    /// An element of a structured buffer, as a place.
    #[inline(never)]
    fn buf_index(&mut self, r: Res, t: &Ty, iv: Val) -> R<Val> {
        let Ty::Buf(k, el) = t else { return Err("[] of a buffer object of no buffer type".into()) };
        if !matches!(k, BufKind::Structured | BufKind::RwStructured) {
            return Err(format!("[] of a {k:?} buffer"));
        }
        if iv.ty.comps() != 1 {
            return Err("a structured buffer's index is one uint".into());
        }
        let iv = self.cast(iv, &Ty::Scalar(B::Uint))?;
        Ok(Val { ty: (**el).clone(), c: Vec::new(), obj: Obj::Mem(Mem::S(r, c0(&iv.c)?, C::I(0), self.o.row_major)) })
    }

    fn index(&mut self, v: Val, iv: Val) -> R<Val> {
        if let Obj::Buf(r) = v.obj {
            return self.buf_index(r, &v.ty, iv);
        }
        if let Obj::Mem(m) = v.obj {
            let off = m.off();
            let Ty::Array(e, n) = &v.ty else {
                // (a vector or matrix in groupshared memory: read, then indexed)
                let v = self.shared_value(v);
                return self.index(v, iv);
            };
            let k = kome::num::u32_of(ncomp(self.prog, e));
            let iv = self.cast(iv, &Ty::Scalar(B::Int))?;
            if let C::I(i) = c0(&iv.c)? {
                let i = i.cast_signed();
                if i64::from(i) >= i64::from(*n) || i < 0 {
                    return Err(format!("index {i} out of range"));
                }
            }
            let off = self.alu(sm4::IMAD, B::Int, &[&iv.c, &[C::I(k)], &[off]])[0];
            return Ok(Val { ty: (**e).clone(), c: Vec::new(), obj: Obj::Mem(m.at(off)) });
        }
        if let (Ty::RwTex(..), Obj::Uav(u)) = (&v.ty, v.obj) {
            let coord = self.uav_coord(u, iv)?;
            return Ok(self.uav_load(u, &coord));
        }
        if let (Ty::Array(e, n), Obj::TexArr(_) | Obj::SampArr(_) | Obj::LegacyArr(..)) = (&v.ty, v.obj) {
            // an element of an array of textures/samplers: the index must be known when compiling
            let i = match iv.c.first() {
                Some(&C::I(i)) if iv.c.len() == 1 && u32::from(i) < *n => u32::from(i),
                Some(&C::I(_)) => return Err("texture or sampler array index out of range".into()),
                _ => return Err("a texture or sampler array needs a constant index".into()),
            };
            let obj = match v.obj {
                Obj::TexArr(t) => {
                    self.texs[kome::num::us(t + i)].used = true;
                    Obj::Tex(t + i)
                }
                Obj::SampArr(s) => {
                    self.samps[kome::num::us(s + i)].used = true;
                    Obj::Samp(s + i)
                }
                Obj::LegacyArr(t, s) => {
                    self.texs[kome::num::us(t + i)].used = true;
                    self.samps[kome::num::us(s + i)].used = true;
                    Obj::Legacy(t + i, s + i)
                }
                Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::Uav(_) | Obj::Mem(_) | Obj::Buf(_) | Obj::Stream(_) => unreachable!(),
            };
            return Ok(Val { ty: (**e).clone(), c: Vec::new(), obj });
        }
        match &v.ty {
            Ty::Array(e, n) => {
                let k = ncomp(self.prog, e);
                let mut elems: Vec<Vec<C>> = (0..kome::num::us(*n)).map(|i| v.c[i * k..(i + 1) * k].to_vec()).collect();
                let not_reg = !elems.is_empty()
                    && !elems[0].iter().all(|c| matches!(c, C::R(Reg { f: RegF::X(..) | RegF::Cb(..) | RegF::Icb(_), .. }, _)));
                if not_reg && !matches!(iv.c.first(), Some(C::I(_))) && *n > 4 && !self.sm3() {
                    // (into an indexable temp first)
                    let st = self.storage(&v.ty);
                    self.assign(&st, &v.c);
                    elems = (0..kome::num::us(*n)).map(|i| st[i * k..(i + 1) * k].to_vec()).collect();
                }
                self.select(elems, iv, (**e).clone())
            }
            Ty::Vec(b, n) => {
                let elems: Vec<Vec<C>> = v.c.iter().map(|c| vec![*c]).collect();
                let _ = n;
                self.select(elems, iv, Ty::Scalar(*b))
            }
            Ty::Mat(b, r, c) => {
                let w = *c;
                let c = c.get();
                let elems: Vec<Vec<C>> = (0..r.get()).map(|i| v.c[i * c..(i + 1) * c].to_vec()).collect();
                self.select(elems, iv, Ty::vec(*b, w))
            }
            Ty::Tex(..) => {
                let t = match v.obj {
                    Obj::Tex(t) => t,
                    Obj::None | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_) | Obj::LegacyArr(..)
                    | Obj::Uav(_) | Obj::Mem(_) | Obj::Buf(_) | Obj::Stream(_) => return Err("texture[] of a non-texture".into()),
                };
                self.load(t, iv, [0; 3], false)
            }
            t @ (Ty::Void | Ty::Scalar(_) | Ty::Struct(_) | Ty::Sampler(_) | Ty::RwTex(..) | Ty::Buf(..) | Ty::Legacy(_) | Ty::Str | Ty::Stream(..)) => Err(format!("index of {t:?}")),
        }
    }

    fn binary(&mut self, op: Bin, x: Val, y: Val) -> R<Val> {
        use sm4::*;
        if matches!(op, Bin::LAnd | Bin::LOr) {
            let t = self.common(&x.ty, &y.ty)?.with_base(B::Bool);
            let a = self.cast(x, &t)?;
            let b = self.cast(y, &t)?;
            let c = self.alu(if op == Bin::LAnd { AND } else { OR }, B::Bool, &[&a.c, &b.c]);
            return Ok(Val::num(t, c));
        }
        let mut t = self.common(&x.ty, &y.ty)?;
        if t.base() == B::Bool {
            t = t.with_base(B::Int);
        }
        if matches!(op, Bin::Shl | Bin::Shr | Bin::And | Bin::Or | Bin::Xor | Bin::Mod) && t.base() == B::Float && op != Bin::Mod {
            return Err("bitwise operator on floats".into());
        }
        if matches!(op, Bin::Shl | Bin::Shr | Bin::And | Bin::Or | Bin::Xor) && self.sm3() {
            return Err("bitwise operators need shader model 4".into());
        }
        let sm3 = self.sm3();
        let a = self.cast(x, &t)?;
        let b = self.cast(y, &t)?;
        let base = t.base();
        let f = base == B::Float;
        let u = base == B::Uint;
        let cmp = |s: &mut Self, o: sm4::Opcode, swap: bool| {
            let c = if swap { s.alu(o, B::Bool, &[&b.c, &a.c]) } else { s.alu(o, B::Bool, &[&a.c, &b.c]) };
            Val::num(t.with_base(B::Bool), c)
        };
        Ok(match op {
            Bin::Add => Val::num(t.clone(), self.alu(if f { ADD } else { IADD }, base, &[&a.c, &b.c])),
            Bin::Sub => Val::num(t.clone(), self.alu_m(if f { ADD } else { IADD }, base, &[(&a.c, 0), (&b.c, 1)])),
            Bin::Mul => {
                if f || sm3 {
                    Val::num(t.clone(), self.alu(MUL, base, &[&a.c, &b.c]))
                } else if a.c.iter().chain(&b.c).all(|c| matches!(c, C::I(_))) {
                    let c = a.c.iter().zip(&b.c).map(|(x, y)| if let (C::I(x), C::I(y)) = (x, y) { C::I(x.wrapping_mul(*y)) } else { *x }).collect();
                    Val::num(t.clone(), c)
                } else {
                    Val::num(t.clone(), self.alu2(if u { UMUL } else { IMUL }, &[&a.c, &b.c], true))
                }
            }
            Bin::Div => {
                if f {
                    Val::num(t.clone(), self.alu(DIV, base, &[&a.c, &b.c]))
                } else if sm3 {
                    let q = self.alu(DIV, base, &[&a.c, &b.c]);
                    Val::num(t.clone(), self.alu(ROUND_Z, base, &[&q]))
                } else if u {
                    Val::num(t.clone(), self.alu2(UDIV, &[&a.c, &b.c], false))
                } else {
                    let c = self.idiv(&a.c, &b.c, false);
                    Val::num(t.clone(), c)
                }
            }
            Bin::Mod => {
                if f || sm3 {
                    // a - b * trunc(a / b)
                    let q = self.alu(DIV, base, &[&a.c, &b.c]);
                    let tq = self.alu(ROUND_Z, base, &[&q]);
                    let c = self.alu_m(MAD, base, &[(&tq, 1), (&b.c, 0), (&a.c, 0)]);
                    Val::num(t.clone(), c)
                } else if u {
                    Val::num(t.clone(), self.alu2(UDIV, &[&a.c, &b.c], true))
                } else {
                    let c = self.idiv(&a.c, &b.c, true);
                    Val::num(t.clone(), c)
                }
            }
            Bin::Shl => Val::num(t.clone(), self.alu(ISHL, base, &[&a.c, &b.c])),
            Bin::Shr => Val::num(t.clone(), self.alu(if u { USHR } else { ISHR }, base, &[&a.c, &b.c])),
            Bin::And => Val::num(t.clone(), self.alu(AND, base, &[&a.c, &b.c])),
            Bin::Or => Val::num(t.clone(), self.alu(OR, base, &[&a.c, &b.c])),
            Bin::Xor => Val::num(t.clone(), self.alu(XOR, base, &[&a.c, &b.c])),
            Bin::Lt => cmp(self, if f { LT } else if u { ULT } else { ILT }, false),
            Bin::Gt => cmp(self, if f { LT } else if u { ULT } else { ILT }, true),
            Bin::Le => cmp(self, if f { GE } else if u { UGE } else { IGE }, true),
            Bin::Ge => cmp(self, if f { GE } else if u { UGE } else { IGE }, false),
            Bin::Eq => cmp(self, if f { EQ } else { IEQ }, false),
            Bin::Ne => cmp(self, if f { NE } else { INE }, false),
            Bin::LAnd | Bin::LOr => unreachable!(),
        })
    }

    /// signed division (or remainder) through udiv
    fn idiv(&mut self, a: &[C], b: &[C], rem: bool) -> Vec<C> {
        use sm4::*;
        let na = self.alu(INEG, B::Int, &[a]);
        let aa = self.alu(IMAX, B::Int, &[a, &na]);
        let nb = self.alu(INEG, B::Int, &[b]);
        let ab = self.alu(IMAX, B::Int, &[b, &nb]);
        let q = self.alu2(UDIV, &[&aa, &ab], rem);
        // the sign: of a (remainder) or of a ^ b
        let s = if rem { a.to_vec() } else { self.alu(XOR, B::Int, &[a, b]) };
        let neg = self.alu(ILT, B::Bool, &[&s, &Self::imm(0, s.len())]);
        let nq = self.alu(INEG, B::Int, &[&q]);
        self.alu(MOVC, B::Int, &[&neg, &nq, &q])
    }

    // ---- calls ----

    fn find_func(&self, name: &str, args: &[Val]) -> Option<&'a Func> {
        let prog = self.prog;
        let cands: Vec<&Func> = prog.funcs.iter().filter(|f| f.name == name && f.params.len() == args.len() && f.body.is_some()).collect();
        if !cands.is_empty() {
            // the closest parameter types
            return cands.into_iter().min_by_key(|f| {
                f.params
                    .iter()
                    .zip(args)
                    .map(|(p, a)| {
                        let pt = p.ty();
                        u32::from(pt != &a.ty) * 2 + u32::from(ncomp(prog, pt) != ncomp(prog, &a.ty)) * 4 + u32::from(pt.base() != a.ty.base())
                    })
                    .sum::<u32>()
            });
        }
        let pre = self.prelude;
        let n = args.iter().map(|a| a.c.len()).max().unwrap_or(1).max(1);
        pre.funcs.iter().find(|f| f.name == name && f.params.len() == args.len() && f.params.first().is_some_and(|p| p.ty().comps() == n))
    }

    #[inline(never)]
    fn call(&mut self, name: &str, args: &[Expr], line: u32) -> R<Val> {
        // (out arguments picked by a run-time index are stored back after the call)
        let mark = self.wbs.len();
        let r = self.call_inner(name, args, line);
        self.write_back(mark);
        r
    }

    fn call_inner(&mut self, name: &str, args: &[Expr], line: u32) -> R<Val> {
        // user functions (and the prelude's) take their out arguments as places
        let user = self.prog.funcs.iter().any(|f| f.name == name);
        let pre = !user && self.prelude.funcs.iter().any(|f| f.name == name);
        if user || pre {
            let mut vals = Vec::new();
            let fs: Vec<&Func> = if user { self.prog.funcs.iter().collect() } else { self.prelude.funcs.iter().collect() };
            let outs: Vec<bool> = (0..args.len())
                .map(|i| fs.iter().filter(|f| f.name == name).any(|f| f.params.get(i).is_some_and(|p| p.is_out)))
                .collect();
            for (i, a) in args.iter().enumerate() {
                vals.push(if outs[i] { self.place(a)? } else { self.expr(a)? });
            }
            let f = self.find_func(name, &vals).ok_or(format!("line {line}: no overload of {name} for these arguments"))?;
            return self.inline(f, vals, line);
        }
        super::intrinsics::call(self, name, args, line)
    }

    fn inline(&mut self, f: &'a Func, args: Vec<Val>, line: u32) -> R<Val> {
        if self.fns.len() > 64 {
            return Err(format!("line {line}: calls nested too deep (recursion?)"));
        }
        // (every call is inlined: f1 calling f0 twice, f2 f1 twice... is 2^n instructions)
        if self.code.len() > MAX_INS {
            return Err(format!("line {line}: shader past {MAX_INS} instructions"));
        }
        let Some(body) = f.body.as_ref() else { return Err(format!("line {line}: {} has no body", f.name)) };
        let mut scope = HashMap::new();
        let mut copy_back = Vec::new();
        for (p, a) in f.params.iter().zip(args) {
            let t = p.ty().clone();
            if t.is_object() {
                scope.insert(p.name.clone(), a);
                continue;
            }
            if p.is_out {
                let st = self.storage(&t);
                if p.is_in {
                    let v = self.cast(a.clone(), &t)?;
                    self.assign(&st, &v.c);
                }
                copy_back.push((st.clone(), a, t.clone()));
                scope.insert(p.name.clone(), Val::num(t, st));
                continue;
            }
            let v = self.cast(a, &t).map_err(|m| format!("line {line}: argument {} of {}: {m}", p.name, f.name))?;
            let v = if body.iter().any(|s| writes(s, &p.name, &|f| self.has_out(f))) {
                let st = self.storage(&t);
                self.assign(&st, &v.c);
                Val::num(t, st)
            } else {
                v
            };
            scope.insert(p.name.clone(), v);
        }
        let ret = if f.ret == Ty::Void { Vec::new() } else { self.storage(&f.ret) };
        // returns before the end: a flag that skips the rest
        let early = body.iter().enumerate().any(|(i, s)| has_return(s) && (i + 1 < body.len() || !matches!(s, Stmt::Return(..))));
        let flag = if early {
            let t = self.tmp();
            self.marks.push((t, self.code.len()));
            self.assign(&[C::R(temp(t), Lane::X)], &[C::I(0)]);
            Some(t)
        } else {
            None
        };
        // (the callee sees globals and its parameters only)
        let saved = std::mem::replace(&mut self.scopes, Scopes::with(scope));
        self.fns.push(FnCtx { ret: ret.clone(), ret_ty: f.ret.clone(), flag, loops: 0, entry: None });
        let r = self.stmts(body);
        self.fns.pop();
        self.scopes = saved;
        r?;
        for (st, a, t) in copy_back {
            let v = self.cast(Val::num(t, st), &a.ty)?;
            self.assign(&a.c, &v.c);
        }
        // (the result in fresh registers: the callee's storage may be reused by a later call)
        Ok(Val::num(f.ret.clone(), ret))
    }

    #[inline(never)]
    fn method(&mut self, o: &Expr, name: &str, args: &[Expr]) -> R<Val> {
        let ov = self.expr(o)?;
        if let Obj::Stream(k) = ov.obj {
            return self.stream_method(k, name, args);
        }
        if let (Ty::Buf(k, el), Obj::Buf(r)) = (&ov.ty, ov.obj) {
            return self.buf_method(r, *k, (**el).clone(), name, args);
        }
        super::intrinsics::method(self, ov, name, args)
    }

    // ---- structured and raw buffers, atomics ----

    /// A buffer's resource operand (all four dwords).
    fn res_src(r: Res) -> Src {
        match r {
            Res::T(t) => Src::Tex(t, [0, 1, 2, 3]),
            Res::U(u) => Src::Uav(u, [0, 1, 2, 3]),
        }
    }

    /// A uint argument (one component).
    fn uint_arg(&mut self, e: Option<&Expr>, what: &str) -> R<C> {
        let e = e.ok_or_else(|| format!("{what} is missing"))?;
        let v = self.expr(e)?;
        if v.ty.comps() != 1 {
            return Err(format!("{what} is one uint"));
        }
        c0(&self.cast(v, &Ty::Scalar(B::Uint))?.c)
    }

    /// Writes `v` to the out argument `e`.
    fn out_arg(&mut self, e: &Expr, v: Val) -> R<()> {
        let mark = self.wbs.len();
        let p = self.place(e)?;
        let v = self.cast(v, &p.ty)?;
        self.assign(&p.c, &v.c);
        self.write_back(mark);
        Ok(())
    }

    /// The methods of StructuredBuffer, ByteAddressBuffer and kin.
    #[inline(never)]
    fn buf_method(&mut self, r: Res, k: BufKind, el: Ty, name: &str, args: &[Expr]) -> R<Val> {
        use sm4::*;
        let n_args = |n: usize| -> R<()> { (args.len() == n).ok_or_else(|| format!("{name} of a buffer takes {n} arguments")) };
        let uav = |what: &str| -> R<u32> {
            match r {
                Res::U(u) => Ok(u),
                Res::T(_) => Err(format!("{what} of a read-only buffer")),
            }
        };
        let rm = self.o.row_major;
        Ok(match (name, k.raw()) {
            ("GetDimensions", raw) => {
                n_args(if raw { 1 } else { 2 })?;
                let t = self.tmp();
                self.emit(BUFINFO, vec![Dst { r: temp(t), mask: 1 }], vec![Self::res_src(r)]);
                self.out_arg(arg(args, 0)?, Val::num(Ty::Scalar(B::Uint), vec![C::R(temp(t), Lane::X)]))?;
                if !raw {
                    let stride = self.tight_bytes(&el);
                    self.out_arg(arg(args, 1)?, Val::num(Ty::Scalar(B::Uint), vec![C::I(stride)]))?;
                }
                Val::num(Ty::Void, vec![])
            }
            ("Load" | "Load2" | "Load3" | "Load4", true) => {
                if args.len() == 2 {
                    return Err(format!("{name} with a status (tiled resources) is not compiled"));
                }
                n_args(1)?;
                let n = match name {
                    "Load" => 1,
                    "Load2" => 2,
                    "Load3" => 3,
                    _ => 4,
                };
                let a = self.uint_arg(args.first(), "the byte address")?;
                let a = self.scalar_src(a);
                let t = self.tmp();
                self.emit(LD_RAW, vec![Dst { r: temp(t), mask: Lane::mask(n) }], vec![a, Self::res_src(r)]);
                let ty = Ty::vec_of(B::Uint, n).ok_or("a load of more than 4 dwords")?;
                Val::num(ty, Lane::first(n).map(|c| C::R(temp(t), c)).collect())
            }
            ("Store" | "Store2" | "Store3" | "Store4", true) => {
                n_args(2)?;
                let u = uav(name)?;
                let n = match name {
                    "Store" => 1,
                    "Store2" => 2,
                    "Store3" => 3,
                    _ => 4,
                };
                let a = self.uint_arg(args.first(), "the byte address")?;
                let v = self.expr(arg(args, 1)?)?;
                let v = self.cast(v, &Ty::vec_of(B::Uint, n).ok_or("a store of more than 4 dwords")?)?;
                let a = self.scalar_src(a);
                let s = self.opnd(&v.c);
                self.emit(STORE_RAW, vec![Dst { r: Reg { f: RegF::Uav(u), rel: None }, mask: Lane::mask(n) }], vec![a, s]);
                Val::num(Ty::Void, vec![])
            }
            (n, true) if n.starts_with("Interlocked") => {
                let u = uav(name)?;
                let a = self.uint_arg(args.first(), "the byte address")?;
                // (RWByteAddressBuffer's Interlocked* have int and uint forms: the value's picks)
                self.atomic(name, Atom::U(u, vec![a]), None, args.get(1..).unwrap_or_default())?
            }
            ("Load", false) => {
                if args.len() == 2 {
                    return Err("Load with a status (tiled resources) is not compiled".into());
                }
                n_args(1)?;
                let i = self.uint_arg(args.first(), "the element index")?;
                let c = self.mem_load(Mem::S(r, i, C::I(0), rm), &el);
                Val::num(el, c)
            }
            ("IncrementCounter" | "DecrementCounter", false) if k == BufKind::RwStructured => {
                n_args(0)?;
                let u = uav(name)?;
                let c = self.counter(u, name == "IncrementCounter");
                Val::num(Ty::Scalar(B::Uint), vec![c])
            }
            ("Append", false) if k == BufKind::Append => {
                n_args(1)?;
                let u = uav(name)?;
                let v = self.expr(arg(args, 0)?)?;
                let v = self.cast(v, &el)?;
                let i = self.counter(u, true);
                self.mem_store_all(WMem::U(u, i, C::I(0), rm), &el, &v.c);
                Val::num(Ty::Void, vec![])
            }
            ("Consume", false) if k == BufKind::Consume => {
                n_args(0)?;
                let u = uav(name)?;
                let i = self.counter(u, false);
                let c = self.mem_load(Mem::S(r, i, C::I(0), rm), &el);
                Val::num(el, c)
            }
            _ => return Err(format!("unknown method {name} of a {k:?} buffer")),
        })
    }

    /// The hidden counter of UAV `u`: incremented (imm_atomic_alloc: the count before) or
    /// decremented (imm_atomic_consume: the count after).
    fn counter(&mut self, u: u32, inc: bool) -> C {
        if let Some(sm4::View::Buf { counter, .. }) = self.uavs.get_mut(kome::num::us(u)).map(|d| &mut d.view) {
            *counter = true;
        }
        let t = self.tmp();
        let op = if inc { sm4::IMM_ATOMIC_ALLOC } else { sm4::IMM_ATOMIC_CONSUME };
        self.emit(op, vec![Dst { r: temp(t), mask: 1 }, Dst { r: Reg { f: RegF::Uav(u), rel: None }, mask: 1 }], vec![]);
        C::R(temp(t), Lane::X)
    }

    /// Where an Interlocked* intrinsic's destination is: memory, or a typed UAV's texel, and
    /// the integer type there.
    #[inline(never)]
    fn atomic_dest(&mut self, e: &Expr) -> R<(Atom, B)> {
        if let Expr::Index(a, i) = e {
            let v = self.place_in(a)?;
            if let (Ty::RwTex(_, el), Obj::Uav(u)) = (&v.ty, v.obj) {
                let b = el.base();
                if el.comps() != 1 || !matches!(b, B::Int | B::Uint) {
                    return Err("an atomic on a texel that is not one int or uint".into());
                }
                let iv = self.expr(i)?;
                let coord = self.uav_coord(u, iv)?;
                return Ok((Atom::U(u, coord), b));
            }
        }
        // (`.x` of an int in memory: that int)
        let scalar_x = match e {
            Expr::Member(a, n) if matches!(n.as_str(), "x" | "r") => {
                let inner = self.place_in(a)?;
                (matches!(inner.ty, Ty::Scalar(_)) && matches!(inner.obj, Obj::Mem(_))).then_some(inner)
            }
            Expr::Int(..) | Expr::Float(_) | Expr::Bool(_) | Expr::Var(..) | Expr::Bin(..) | Expr::Un(..) | Expr::Assign(..) | Expr::Cond(..)
            | Expr::Call(..) | Expr::Construct(..) | Expr::Cast(..) | Expr::Method(..) | Expr::List(_) | Expr::Comma(..) | Expr::Str(_)
            | Expr::Index(..) | Expr::Member(..) => None,
        };
        let v = match scalar_x {
            Some(v) => v,
            None => self.place_in(e)?,
        };
        let b = v.ty.base();
        if !matches!(v.ty, Ty::Scalar(B::Int | B::Uint)) {
            return Err("an atomic's destination is one int or uint".into());
        }
        match v.obj {
            Obj::Mem(Mem::G(g, off)) => {
                let a = self.shared_addr(off, 0);
                Ok((Atom::G(g, a), b))
            }
            Obj::Mem(Mem::S(Res::U(u), i, off, _)) => {
                let a = self.shared_addr(off, 0);
                Ok((Atom::U(u, vec![i, a]), b))
            }
            Obj::Mem(Mem::S(Res::T(_), ..)) => Err("an atomic on a read-only buffer".into()),
            Obj::None | Obj::Tex(_) | Obj::Samp(_) | Obj::Legacy(..) | Obj::TexArr(_) | Obj::SampArr(_) | Obj::LegacyArr(..)
            | Obj::Uav(_) | Obj::Buf(_) | Obj::Stream(_) => Err("an atomic's destination is groupshared memory or a UAV".into()),
        }
    }

    /// InterlockedAdd .. InterlockedCompareStore on `at` (of integers `b`): `args` the value(s)
    /// then the out argument with what was there before.
    #[inline(never)]
    fn atomic(&mut self, name: &str, at: Atom, b: Option<B>, args: &[Expr]) -> R<Val> {
        use sm4::Opcode as O;
        let mut vals = Vec::new();
        for e in args.iter().take(2) {
            vals.push(self.expr(e)?);
        }
        let b = b.unwrap_or(if vals.last().is_some_and(|v| v.ty.base() == B::Int) { B::Int } else { B::Uint });
        let signed = b == B::Int;
        // (plain op, the op returning what was there, the values it takes)
        let (plain, imm, nv): (Option<sm4::Opcode>, sm4::Opcode, usize) = match name {
            "InterlockedAdd" => (Some(O::AtomicIadd), O::ImmAtomicIadd, 1),
            "InterlockedAnd" => (Some(O::AtomicAnd), O::ImmAtomicAnd, 1),
            "InterlockedOr" => (Some(O::AtomicOr), O::ImmAtomicOr, 1),
            "InterlockedXor" => (Some(O::AtomicXor), O::ImmAtomicXor, 1),
            "InterlockedMin" if signed => (Some(O::AtomicImin), O::ImmAtomicImin, 1),
            "InterlockedMin" => (Some(O::AtomicUmin), O::ImmAtomicUmin, 1),
            "InterlockedMax" if signed => (Some(O::AtomicImax), O::ImmAtomicImax, 1),
            "InterlockedMax" => (Some(O::AtomicUmax), O::ImmAtomicUmax, 1),
            "InterlockedExchange" => (None, O::ImmAtomicExch, 1),
            "InterlockedCompareExchange" => (None, O::ImmAtomicCmpExch, 2),
            "InterlockedCompareStore" => (Some(O::AtomicCmpStore), O::AtomicCmpStore, 2),
            _ => return Err(format!("unknown intrinsic {name}")),
        };
        let with_out = args.len() == nv.saturating_add(1) && name != "InterlockedCompareStore";
        if args.len() != nv.saturating_add(usize::from(with_out)) || (plain.is_none() && !with_out) {
            return Err(format!("{name}: wrong number of arguments"));
        }
        let mut src = Vec::new();
        let (res, addr) = match &at {
            Atom::G(g, a) => (Reg { f: RegF::G(*g), rel: None }, vec![*a]),
            Atom::U(u, a) => (Reg { f: RegF::Uav(*u), rel: None }, a.clone()),
        };
        src.push(self.opnd(&addr));
        for v in vals.into_iter().take(nv) {
            if v.ty.comps() != 1 {
                return Err(format!("{name}'s values are one int or uint"));
            }
            let v = self.cast(v, &Ty::Scalar(b))?;
            src.push(self.scalar_src(c0(&v.c)?));
        }
        let rd = Dst { r: res, mask: 1 };
        match (plain, with_out) {
            (Some(op), false) => {
                self.emit(op, vec![rd], src);
            }
            (None | Some(_), true) => {
                let t = self.tmp();
                self.emit(imm, vec![Dst { r: temp(t), mask: 1 }, rd], src);
                self.out_arg(arg(args, nv)?, Val::num(Ty::Scalar(b), vec![C::R(temp(t), Lane::X)]))?;
            }
            (None, false) => return Err(format!("{name} takes an out argument")),
        }
        Ok(Val::num(Ty::Void, vec![]))
    }

    pub(super) fn x_interlocked(&mut self, name: &str, args: &[Expr]) -> R<Val> {
        let dest = args.first().ok_or_else(|| format!("{name} takes a destination"))?;
        let (at, b) = self.atomic_dest(dest)?;
        self.atomic(name, at, Some(b), args.get(1..).unwrap_or_default())
    }

    // ---- for intrinsics.rs ----

    pub(super) fn x_alu(&mut self, op: sm4::Opcode, b: B, args: &[&[C]]) -> Vec<C> {
        self.alu(op, b, args)
    }
    pub(super) fn x_alu_m(&mut self, op: sm4::Opcode, b: B, args: &[(&[C], u8)]) -> Vec<C> {
        self.alu_m(op, b, args)
    }
    pub(super) fn x_alu2(&mut self, op: sm4::Opcode, args: &[&[C]], second: bool) -> Vec<C> {
        self.alu2(op, args, second)
    }
    pub(super) fn x_cast(&mut self, v: Val, t: &Ty) -> R<Val> {
        self.cast(v, t)
    }
    pub(super) fn x_common(&self, a: &Ty, b: &Ty) -> R<Ty> {
        self.common(a, b)
    }
    pub(super) fn x_place(&mut self, e: &Expr) -> R<Val> {
        self.place(e)
    }
    pub(super) fn x_assign(&mut self, d: &[C], s: &[C]) {
        self.assign(d, s)
    }
    pub(super) fn x_assign_sat(&mut self, d: &[C], s: &[C]) {
        self.assign_op(sm4::MOV, d, s, true)
    }
    pub(super) fn x_tmp(&mut self) -> u32 {
        self.tmp()
    }
    pub(super) fn x_opnd(&mut self, cs: &[C]) -> Src {
        self.opnd(cs)
    }
    pub(super) fn x_last(&mut self) -> Option<&mut Ins> {
        self.code.last_mut()
    }
    pub(super) fn x_emit(&mut self, op: sm4::Opcode, dst: Vec<Dst>, src: Vec<Src>) -> &mut Ins {
        self.emit(op, dst, src)
    }
    pub(super) fn x_cond(&mut self, op: sm4::Opcode, c: C, nz: bool) {
        self.cond_op(op, c, nz)
    }
    pub(super) fn x_tex_el(&self, t: u32) -> (Dim, Ty) {
        let d = &self.texs[kome::num::us(t)];
        (d.dim, d.el.clone())
    }
    pub(super) fn x_samp_cmp(&mut self, s: u32) {
        self.samps[kome::num::us(s)].cmp = true;
    }
    pub(super) fn x_sm3(&self) -> bool {
        self.sm3()
    }
    pub(super) fn x_major(&self) -> u32 {
        self.o.major
    }
}

/// Whether a component is storage an assignment may write: a temp, an output, an indexable temp.
fn writable(c: &C) -> bool {
    matches!(c, C::R(Reg { f: RegF::Temp(_) | RegF::Output(_) | RegF::X(..) | RegF::Depth, .. }, _))
}

/// Where d3dcompiler packs a patch constant of `n` components among the PCSG's elements so far:
/// the first register with `n` free components after its highest taken one (register, first
/// component), None for a register of its own.
fn pc_slot(list: &[SigEl], n: usize) -> Option<(u32, u8)> {
    let mut regs: Vec<u32> = list.iter().map(|e| e.reg).collect();
    regs.sort_unstable();
    regs.dedup();
    regs.into_iter().find_map(|r| {
        let taken = list.iter().filter(|e| e.reg == r).fold(0u8, |m, e| m | e.mask);
        let at = u8::try_from(8u32.saturating_sub(taken.leading_zeros())).unwrap_or(4);
        (usize::from(at).saturating_add(n) <= 4).then_some((r, at))
    })
}
