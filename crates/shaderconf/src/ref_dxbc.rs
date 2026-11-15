//! A reference interpreter of shader model 4/5 pixel shaders (DXBC), written from the Direct3D
//! 11.3 functional spec and the tokenized program format (d3d11TokenizedProgramFormat.hpp),
//! independent of dxwgsl: its own decoder, and every instruction as the spec states it. It runs
//! the ALU, flow control, indexable temps and constant buffers; resources, derivatives and
//! discard are outside it (`Err`).
//!
//! What it is for: the cases' expected pixels check it (tests below); then it is the oracle
//! for bytecode nobody wrote expectations for (fuzzed programs, real games' shaders).

/// The instructions this interpreter runs: name = opcode, operand count (from the D3D10/11
/// instruction reference; its own table, not dxwgsl's)
macro_rules! codes {
    ($($name:ident = $code:literal, $n:literal;)*) => {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum Code {
            $($name,)*
        }
        impl Code {
            fn from_code(c: u32) -> Option<Code> {
                match c {
                    $($code => Some(Code::$name),)*
                    _ => None,
                }
            }
            fn code(self) -> u32 {
                match self {
                    $(Code::$name => $code,)*
                }
            }
            fn operands(self) -> usize {
                match self {
                    $(Code::$name => $n,)*
                }
            }
        }
    };
}

codes! {
    Add = 0x00, 3;
    And = 0x01, 3;
    Break = 0x02, 0;
    Breakc = 0x03, 1;
    Case = 0x06, 1;
    Continue = 0x07, 0;
    Continuec = 0x08, 1;
    Default = 0x0a, 0;
    Div = 0x0e, 3;
    Dp2 = 0x0f, 3;
    Dp3 = 0x10, 3;
    Dp4 = 0x11, 3;
    Else = 0x12, 0;
    EndIf = 0x15, 0;
    EndLoop = 0x16, 0;
    EndSwitch = 0x17, 0;
    Eq = 0x18, 3;
    Exp = 0x19, 2;
    Frc = 0x1a, 2;
    Ftoi = 0x1b, 2;
    Ftou = 0x1c, 2;
    Ge = 0x1d, 3;
    Iadd = 0x1e, 3;
    If = 0x1f, 1;
    Ieq = 0x20, 3;
    Ige = 0x21, 3;
    Ilt = 0x22, 3;
    Imad = 0x23, 4;
    Imax = 0x24, 3;
    Imin = 0x25, 3;
    Imul = 0x26, 4;
    Ine = 0x27, 3;
    Ineg = 0x28, 2;
    Ishl = 0x29, 3;
    Ishr = 0x2a, 3;
    Itof = 0x2b, 2;
    Log = 0x2f, 2;
    Loop = 0x30, 0;
    Lt = 0x31, 3;
    Mad = 0x32, 4;
    Min = 0x33, 3;
    Max = 0x34, 3;
    Mov = 0x36, 2;
    Movc = 0x37, 4;
    Mul = 0x38, 3;
    Ne = 0x39, 3;
    Nop = 0x3a, 0;
    Not = 0x3b, 2;
    Or = 0x3c, 3;
    Ret = 0x3e, 0;
    Retc = 0x3f, 1;
    RoundNe = 0x40, 2;
    RoundNi = 0x41, 2;
    RoundPi = 0x42, 2;
    RoundZ = 0x43, 2;
    Rsq = 0x44, 2;
    Sqrt = 0x4b, 2;
    Switch = 0x4c, 1;
    SinCos = 0x4d, 3;
    Udiv = 0x4e, 4;
    Ult = 0x4f, 3;
    Uge = 0x50, 3;
    Umul = 0x51, 4;
    Umad = 0x52, 4;
    Umax = 0x53, 3;
    Umin = 0x54, 3;
    Ushr = 0x55, 3;
    Utof = 0x56, 2;
    Xor = 0x57, 3;
    Rcp = 0x81, 2;
    F32ToF16 = 0x82, 2;
    F16ToF32 = 0x83, 2;
    CountBits = 0x86, 2;
    FirstBitHi = 0x87, 2;
    FirstBitLo = 0x88, 2;
    FirstBitShi = 0x89, 2;
    Ubfe = 0x8a, 4;
    Ibfe = 0x8b, 4;
    Bfi = 0x8c, 5;
    Bfrev = 0x8d, 2;
    Swapc = 0x8e, 5;
}

impl Code {
    /// a float result (the float checks apply)
    fn float_out(self) -> bool {
        matches!(self, Code::Add | Code::Div | Code::Exp | Code::Frc | Code::Log | Code::Mad | Code::Min | Code::Max | Code::Mul | Code::RoundNe | Code::RoundNi | Code::RoundPi | Code::RoundZ | Code::Rsq | Code::Sqrt | Code::Rcp | Code::F16ToF32)
    }
}

/// A register value: four 32-bit components.
pub type V = [u32; 4];

#[derive(Clone, Debug)]
enum Idx {
    Imm(u32),
    /// a register's component plus an offset
    Rel(Box<Op>, u32),
}

#[derive(Clone, Debug)]
pub struct Op {
    ty: u32,
    /// for sources: which component each of x, y, z, w reads; for destinations: unused
    swz: [u8; 4],
    /// destinations: write mask
    mask: u8,
    idx: Vec<Idx>,
    imm: V,
    /// 1 negate, 2 absolute, 3 both
    md: u8,
}

#[derive(Clone, Debug)]
struct Inst {
    op: Code,
    sat: bool,
    /// (if, breakc, ...: true when testing for nonzero)
    nz: bool,
    ops: Vec<Op>,
}

pub struct Program {
    insts: Vec<Inst>,
    temps: usize,
    /// indexable temps: register -> size
    xs: Vec<(u32, u32)>,
    icb: Vec<V>,
    /// flow control: the matching instruction of each if/else/loop/switch/case
    jump: Vec<usize>,
}

impl Program {
    pub fn show(&self) -> String {
        self.insts.iter().enumerate().map(|(n, i)| format!("{n}: {i:?}\n")).collect()
    }
}

fn rd(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "short".to_string())
}

/// The SHDR/SHEX chunk's tokens.
fn tokens(code: &[u8]) -> Result<Vec<u32>, String> {
    if code.get(..4) != Some(b"DXBC") {
        return Err("not DXBC".into());
    }
    let n = kome::num::us(rd(code, 28)?);
    for i in 0..n {
        let at = kome::num::us(rd(code, 32 + 4 * i)?);
        let tag = code.get(at..at + 4).ok_or("short")?;
        if tag == b"SHDR" || tag == b"SHEX" {
            let size = kome::num::us(rd(code, at + 4)?);
            let body = code.get(at + 8..at + 8 + size).ok_or("short chunk")?;
            return Ok(body.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect());
        }
    }
    Err("no SHDR/SHEX".into())
}

fn operand(t: &[u32], at: &mut usize) -> Result<Op, String> {
    let w = *t.get(*at).ok_or("operand past the end")?;
    *at += 1;
    let comps = w & 3;
    let ty = (w >> 12) & 0xff;
    let dims = (w >> 20) & 3;
    let mut op = Op { ty, swz: [0, 1, 2, 3], mask: 0xf, idx: Vec::new(), imm: [0; 4], md: 0 };
    match comps {
        0 => op.mask = 0,
        1 => {
            op.swz = [0; 4];
            op.mask = 1;
        }
        2 => match (w >> 2) & 3 {
            0 => op.mask = kome::num::lo8((w >> 4) & 0xf),
            1 => {
                op.swz = std::array::from_fn(|i| kome::num::lo8((w >> (4 + 2 * i)) & 3));
            }
            2 => {
                let c = kome::num::lo8((w >> 4) & 3);
                op.swz = [c; 4];
                op.mask = 1 << c;
            }
            _ => return Err("selection mode 3".into()),
        },
        _ => return Err("N-component operand".into()),
    }
    let mut ext = w >> 31 != 0;
    while ext {
        let e = *t.get(*at).ok_or("short")?;
        *at += 1;
        if e & 0x3f == 1 {
            op.md = kome::num::lo8((e >> 6) & 0xff);
        }
        ext = e >> 31 != 0;
    }
    if ty == 4 {
        let n = if comps == 1 { 1 } else { 4 };
        for i in 0..n {
            op.imm[i] = *t.get(*at).ok_or("short")?;
            *at += 1;
        }
        if n == 1 {
            op.imm = [op.imm[0]; 4];
        }
        return Ok(op);
    }
    if ty == 5 {
        return Err("64-bit immediates".into());
    }
    for d in 0..dims {
        let rep = (w >> (22 + 3 * d)) & 7;
        let imm = match rep {
            0 | 3 => {
                let v = *t.get(*at).ok_or("short")?;
                *at += 1;
                v
            }
            2 => 0,
            _ => return Err("64-bit index".into()),
        };
        op.idx.push(if rep == 0 { Idx::Imm(imm) } else { Idx::Rel(Box::new(operand(t, at)?), imm) });
    }
    Ok(op)
}

pub fn parse(code: &[u8]) -> Result<Program, String> {
    let t = tokens(code)?;
    if t.len() < 2 {
        return Err("short program".into());
    }
    if (t[0] >> 16) & 0xffff != 0 {
        return Err("not a pixel shader".into());
    }
    let end = kome::num::us(t[1]).min(t.len());
    let mut p = Program { insts: Vec::new(), temps: 0, xs: Vec::new(), icb: Vec::new(), jump: Vec::new() };
    let mut at = 2;
    while at < end {
        let w = t[at];
        let op = w & 0x7ff;
        let len = if op == 0x35 { kome::num::us(*t.get(at + 1).ok_or("short")?) } else { kome::num::us((w >> 24) & 0x7f) };
        if len == 0 || at + len > end {
            return Err(format!("instruction {op:#x}: length {len}"));
        }
        let body = &t[at..at + len];
        match op {
            // customdata: an immediate constant buffer
            0x35 => {
                if w >> 11 == 3 {
                    p.icb = body[2..].chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
                }
            }
            // dcl_temps, dcl_indexableTemp
            0x68 => p.temps = kome::num::us(*body.get(1).ok_or("short")?),
            0x69 => p.xs.push((*body.get(1).ok_or("short")?, *body.get(2).ok_or("short")?)),
            0x58..=0x6a | 0x8f..=0xa2 => {}
            _ => {
                let mut k = 1;
                let mut ext = w >> 31 != 0;
                while ext {
                    let e = *body.get(k).ok_or("short")?;
                    k += 1;
                    ext = e >> 31 != 0;
                }
                let mut ops = Vec::new();
                while k < len {
                    ops.push(operand(body, &mut k)?);
                }
                let Some(code) = Code::from_code(op) else {
                    return Err(format!("opcode {op:#x}"));
                };
                let want = code.operands();
                if ops.len() != want {
                    return Err(format!("opcode {op:#x} with {} operands, {want} wanted", ops.len()));
                }
                p.insts.push(Inst { op: code, sat: (w >> 13) & 1 != 0, nz: (w >> 18) & 1 != 0, ops });
            }
        }
        at += len;
    }
    // matching flow control
    p.jump = vec![usize::MAX; p.insts.len()];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..p.insts.len() {
        match p.insts[i].op {
            Code::If | Code::Loop | Code::Switch => stack.push(i),
            Code::Else => {
                let s = *stack.last().ok_or("else outside an if")?;
                if p.insts[s].op != Code::If {
                    return Err("else outside an if".into());
                }
                p.jump[s] = i;
                if let Some(t) = stack.last_mut() {
                    *t = i;
                }
            }
            Code::EndIf | Code::EndLoop | Code::EndSwitch => {
                let s = stack.pop().ok_or("unbalanced flow control")?;
                let want: &[Code] = match p.insts[i].op {
                    Code::EndIf => &[Code::If, Code::Else],
                    Code::EndLoop => &[Code::Loop],
                    Code::Add | Code::And | Code::Break | Code::Breakc | Code::Case | Code::Continue
                    | Code::Continuec | Code::Default | Code::Div | Code::Dp2 | Code::Dp3 | Code::Dp4
                    | Code::Else | Code::EndSwitch | Code::Eq | Code::Exp | Code::Frc | Code::Ftoi
                    | Code::Ftou | Code::Ge | Code::Iadd | Code::If | Code::Ieq | Code::Ige
                    | Code::Ilt | Code::Imad | Code::Imax | Code::Imin | Code::Imul | Code::Ine
                    | Code::Ineg | Code::Ishl | Code::Ishr | Code::Itof | Code::Log | Code::Loop
                    | Code::Lt | Code::Mad | Code::Min | Code::Max | Code::Mov | Code::Movc
                    | Code::Mul | Code::Ne | Code::Nop | Code::Not | Code::Or | Code::Ret
                    | Code::Retc | Code::RoundNe | Code::RoundNi | Code::RoundPi | Code::RoundZ | Code::Rsq
                    | Code::Sqrt | Code::Switch | Code::SinCos | Code::Udiv | Code::Ult | Code::Uge
                    | Code::Umul | Code::Umad | Code::Umax | Code::Umin | Code::Ushr | Code::Utof
                    | Code::Xor | Code::Rcp | Code::F32ToF16 | Code::F16ToF32 | Code::CountBits | Code::FirstBitHi
                    | Code::FirstBitLo | Code::FirstBitShi | Code::Ubfe | Code::Ibfe | Code::Bfi | Code::Bfrev
                    | Code::Swapc => &[Code::Switch],
                };
                if !want.contains(&p.insts[s].op) {
                    return Err("unbalanced flow control".into());
                }
                p.jump[s] = i;
                p.jump[i] = s;
            }
            Code::Add | Code::And | Code::Break | Code::Breakc | Code::Case | Code::Continue
            | Code::Continuec | Code::Default | Code::Div | Code::Dp2 | Code::Dp3 | Code::Dp4
            | Code::Eq | Code::Exp | Code::Frc | Code::Ftoi | Code::Ftou | Code::Ge
            | Code::Iadd | Code::Ieq | Code::Ige | Code::Ilt | Code::Imad | Code::Imax
            | Code::Imin | Code::Imul | Code::Ine | Code::Ineg | Code::Ishl | Code::Ishr
            | Code::Itof | Code::Log | Code::Lt | Code::Mad | Code::Min | Code::Max
            | Code::Mov | Code::Movc | Code::Mul | Code::Ne | Code::Nop | Code::Not
            | Code::Or | Code::Ret | Code::Retc | Code::RoundNe | Code::RoundNi | Code::RoundPi
            | Code::RoundZ | Code::Rsq | Code::Sqrt | Code::SinCos | Code::Udiv | Code::Ult
            | Code::Uge | Code::Umul | Code::Umad | Code::Umax | Code::Umin | Code::Ushr
            | Code::Utof | Code::Xor | Code::Rcp | Code::F32ToF16 | Code::F16ToF32 | Code::CountBits
            | Code::FirstBitHi | Code::FirstBitLo | Code::FirstBitShi | Code::Ubfe | Code::Ibfe | Code::Bfi
            | Code::Bfrev | Code::Swapc => {}
        }
    }
    if !stack.is_empty() {
        return Err("unclosed flow control".into());
    }
    Ok(p)
}

fn f(x: u32) -> f32 {
    f32::from_bits(x)
}
fn b(x: f32) -> u32 {
    x.to_bits()
}

/// The functional spec's float min/max: a NaN operand gives the other one.
fn fmin(a: f32, c: f32) -> f32 {
    if a.is_nan() { c } else if c.is_nan() { a } else if a < c || (a == c && a.is_sign_negative()) { a } else { c }
}
fn fmax(a: f32, c: f32) -> f32 {
    if a.is_nan() { c } else if c.is_nan() { a } else if a > c || (a == c && c.is_sign_negative()) { a } else { c }
}

/// ftoi: NaN 0, out of range saturates.
fn ftoi(x: f32) -> u32 {
    if x.is_nan() { 0 } else { (kome::num::trunc_i32(x)).cast_unsigned() }
}
fn ftou(x: f32) -> u32 {
    if x.is_nan() { 0 } else { kome::num::trunc_u32(x) }
}

/// f32 -> f16 bits as WARP converts (dxwgsl's d3d_f32tof16): toward zero, finite overflow to
/// 65504, infinities stay, NaNs 0x7fff (with the sign), float denormals 0.
fn f32tof16(x: f32) -> u32 {
    let b = x.to_bits();
    let (s, a) = ((b >> 16) & 0x8000, b & 0x7fff_ffff);
    if a > 0x7f80_0000 {
        s | 0x7fff
    } else if a == 0x7f80_0000 {
        s | 0x7c00
    } else if a >= 0x477f_e000 {
        s | 0x7bff
    } else if a >= 0x3880_0000 {
        s | (a - 0x3800_0000) >> 13
    } else if a < 0x0080_0000 {
        s
    } else {
        s | kome::num::trunc_u32(f32::from_bits(a) * 16777216.0)
    }
}

struct Run<'a> {
    p: &'a Program,
    r: Vec<V>,
    x: Vec<(u32, Vec<V>)>,
    o: [V; 8],
    cb: &'a [Vec<V>],
    /// why the result is not one every implementation gives: a read or write out of range
    /// (robust access may return anything in range), or a float op that gave an infinity or
    /// NaN (WGSL leaves those indeterminate)
    taint: std::cell::RefCell<Option<String>>,
    /// the other answer where WGSL lets the sign of a zero go: float results of -0 are +0, and
    /// min/max of +0 and -0 give the other zero
    zero_flip: bool,
}

/// Float ops whose results the oracle wants finite.
#[derive(Clone, Copy, PartialEq)]
enum K {
    F,
    I,
    U,
}

impl Run<'_> {
    fn index(&self, i: &Idx) -> Result<u32, String> {
        Ok(match i {
            Idx::Imm(n) => *n,
            Idx::Rel(o, n) => self.src(o, K::U)?[0].wrapping_add(*n),
        })
    }
    /// The register an operand names (all four components).
    fn reg(&self, o: &Op) -> Result<V, String> {
        let i0 = o.idx.first().map(|i| self.index(i)).transpose()?.unwrap_or(0);
        Ok(match o.ty {
            0 => *self.r.get(kome::num::us(i0)).ok_or("temp out of range")?,
            2 => self.o[kome::num::us(i0) & 7],
            3 => {
                let i1 = self.index(o.idx.get(1).ok_or("x# without index")?)?;
                let x = &self.x.iter().find(|x| x.0 == i0).ok_or("undeclared x#")?.1;
                // (out of range: 0, as reads of a register past the end)
                x.get(kome::num::us(i1)).copied().unwrap_or_else(|| self.taint("x# read out of range"))
            }
            4 => o.imm,
            8 => {
                let i1 = self.index(o.idx.get(1).ok_or("cb without index")?)?;
                self.cb.get(kome::num::us(i0)).and_then(|c| c.get(kome::num::us(i1))).copied().unwrap_or_else(|| self.taint("cb read out of range"))
            }
            9 => self.p.icb.get(kome::num::us(i0)).copied().unwrap_or_else(|| self.taint("icb read out of range")),
            t => return Err(format!("source register type {t}")),
        })
    }
    /// A source read as kind `k` (modifiers: float negate/abs; integer negate).
    fn src(&self, o: &Op, k: K) -> Result<V, String> {
        let v = self.reg(o)?;
        let v: V = std::array::from_fn(|i| v[usize::from(o.swz[i])]);
        Ok(v.map(|x| match (k, o.md) {
            (_, 0) => x,
            (K::F, 1) => x ^ 0x8000_0000,
            (K::F, 2) => x & 0x7fff_ffff,
            (K::F, 3) => x | 0x8000_0000,
            (_, 1) => x.wrapping_neg(),
            _ => x,
        }))
    }
    fn dst(&mut self, o: &Op, v: V, sat: bool) -> Result<(), String> {
        let v = if sat { v.map(|x| b(if f(x).is_nan() { 0.0 } else { f(x).clamp(0.0, 1.0) })) } else { v };
        if o.ty == 13 {
            return Ok(());
        }
        let i0 = kome::num::us(o.idx.first().map(|i| self.index(i)).transpose()?.unwrap_or(0));
        let i1 = o.idx.get(1).map(|i| self.index(i)).transpose()?;
        let slot: &mut V = match o.ty {
            0 => self.r.get_mut(i0).ok_or("temp out of range")?,
            2 => self.o.get_mut(i0).ok_or("output out of range")?,
            3 => {
                let x = &mut self.x.iter_mut().find(|x| x.0 == kome::num::u32_of(i0)).ok_or("undeclared x#")?.1;
                match x.get_mut(kome::num::us(i1.unwrap_or(0))) {
                    Some(s) => s,
                    // (writes past the end are dropped)
                    None => {
                        self.taint("x# write out of range");
                        return Ok(());
                    }
                }
            }
            t => return Err(format!("destination register type {t}")),
        };
        for c in 0..4 {
            if o.mask & (1 << c) != 0 {
                slot[c] = v[c];
            }
        }
        Ok(())
    }
    /// Component-wise: d = g(a, b, c) on sources read as `k`.
    fn cw(&mut self, i: &Inst, k: K, g: impl Fn(&[u32]) -> u32) -> Result<(), String> {
        let s: Vec<V> = i.ops[1..].iter().map(|o| self.src(o, k)).collect::<Result<_, _>>()?;
        let mut v: V = std::array::from_fn(|c| g(&s.iter().map(|x| x[c]).collect::<Vec<_>>()));
        if self.zero_flip && i.op.float_out() {
            for x in &mut v {
                if *x == 0x8000_0000 {
                    *x = 0;
                }
            }
        }
        // (float math or comparisons on a denormal: implementations may flush it)
        if (i.op.float_out() || matches!(i.op, Code::Eq | Code::Ge | Code::Lt | Code::Ne)) && s.iter().flatten().any(|&x| f(x).is_subnormal()) {
            self.taint("a denormal operand");
        }
        if i.op.float_out() {
            self.finite(&i.ops[0], &v);
        }
        self.dst(&i.ops[0], v, i.sat)
    }
    fn taint(&self, why: &str) -> V {
        self.taint.borrow_mut().get_or_insert_with(|| why.to_string());
        [0; 4]
    }
    /// (the written components)
    fn finite(&self, d: &Op, v: &V) {
        if (0..4).any(|c| d.mask & (1 << c) != 0 && (!f(v[c]).is_finite() || f(v[c]).is_subnormal())) {
            self.taint("a float op gave an infinity, NaN or denormal");
        }
    }
    fn cond(&self, i: &Inst) -> Result<bool, String> {
        let v = self.src(&i.ops[0], K::U)?[0];
        Ok((v != 0) == i.nz)
    }

    fn exec(&mut self) -> Result<(), String> {
        let p = self.p;
        let mut pc = 0;
        let mut steps = 0u32;
        // (enclosing loops and switches: their opening instruction)
        let mut blocks: Vec<usize> = Vec::new();
        while pc < p.insts.len() {
            steps += 1;
            if steps > 1_000_000 {
                return Err("runs too long".into());
            }
            let i = &p.insts[pc];
            pc += 1;
            use K::*;
            match i.op {
                Code::Add => self.cw(i, F, |a| b(f(a[0]) + f(a[1])))?,
                Code::And => self.cw(i, U, |a| a[0] & a[1])?,
                Code::Break | Code::Breakc => {
                    if i.op == Code::Break || self.cond(i)? {
                        let s = blocks.pop().ok_or("break outside a loop")?;
                        pc = p.jump[s] + 1;
                    }
                }
                Code::Continue | Code::Continuec => {
                    if i.op == Code::Continue || self.cond(i)? {
                        let s = *blocks.iter().rev().find(|&&s| p.insts[s].op == Code::Loop).ok_or("continue outside a loop")?;
                        while blocks.last().is_some_and(|&b| b != s) {
                            blocks.pop();
                        }
                        pc = s + 1;
                    }
                }
                Code::Div => self.cw(i, F, |a| b(f(a[0]) / f(a[1])))?,
                Code::Dp2 | Code::Dp3 | Code::Dp4 => {
                    let n = kome::num::us(i.op.code() - 0x0d);
                    let a = self.src(&i.ops[1], F)?;
                    let c = self.src(&i.ops[2], F)?;
                    if a[..n].iter().chain(&c[..n]).any(|&x| f(x).is_subnormal()) {
                        self.taint("a denormal operand");
                    }
                    let mut d = f(a[0]) * f(c[0]);
                    for k in 1..n {
                        d += f(a[k]) * f(c[k]);
                    }
                    self.finite(&i.ops[0], &[b(d); 4]);
                    self.dst(&i.ops[0], [b(d); 4], i.sat)?;
                }
                Code::Else => pc = p.jump[pc - 1] + 1,
                Code::EndIf => {}
                Code::EndLoop => pc = p.jump[pc - 1] + 1,
                Code::EndSwitch => {
                    blocks.pop();
                }
                Code::Case | Code::Default => {}
                Code::Eq => self.cw(i, F, |a| if f(a[0]) == f(a[1]) { !0 } else { 0 })?,
                Code::Ge => self.cw(i, F, |a| if f(a[0]) >= f(a[1]) { !0 } else { 0 })?,
                Code::Lt => self.cw(i, F, |a| if f(a[0]) < f(a[1]) { !0 } else { 0 })?,
                Code::Ne => self.cw(i, F, |a| if f(a[0]) != f(a[1]) { !0 } else { 0 })?,
                Code::Ieq => self.cw(i, I, |a| if a[0] == a[1] { !0 } else { 0 })?,
                Code::Ige => self.cw(i, I, |a| if a[0].cast_signed() >= a[1].cast_signed() { !0 } else { 0 })?,
                Code::Ilt => self.cw(i, I, |a| if (a[0].cast_signed()) < a[1].cast_signed() { !0 } else { 0 })?,
                Code::Ine => self.cw(i, I, |a| if a[0] != a[1] { !0 } else { 0 })?,
                Code::Ult => self.cw(i, U, |a| if a[0] < a[1] { !0 } else { 0 })?,
                Code::Uge => self.cw(i, U, |a| if a[0] >= a[1] { !0 } else { 0 })?,
                Code::Exp => self.cw(i, F, |a| b(f(a[0]).exp2()))?,
                // (below 1: -tiny gives 0x3f7fffff; infinities NaN)
                Code::Frc => self.cw(i, F, |a| b(if f(a[0]).is_finite() { (f(a[0]) - f(a[0]).floor()).min(f32::from_bits(0x3f7f_ffff)) } else { f32::NAN }))?,
                Code::Ftoi => self.cw(i, F, |a| ftoi(f(a[0])))?,
                Code::Ftou => self.cw(i, F, |a| ftou(f(a[0])))?,
                Code::Itof => self.cw(i, I, |a| b(kome::num::to_f32(a[0].cast_signed())))?,
                Code::Utof => self.cw(i, U, |a| b(kome::num::to_f32(a[0])))?,
                Code::Iadd => self.cw(i, I, |a| a[0].wrapping_add(a[1]))?,
                Code::If => {
                    if !self.cond(i)? {
                        let j = p.jump[pc - 1];
                        // (to the else's body, or past the endif)
                        pc = j + 1;
                    }
                }
                Code::Imad => self.cw(i, I, |a| a[0].wrapping_mul(a[1]).wrapping_add(a[2]))?,
                Code::Umad => self.cw(i, U, |a| a[0].wrapping_mul(a[1]).wrapping_add(a[2]))?,
                Code::Imax => self.cw(i, I, |a| (a[0].cast_signed()).max(a[1].cast_signed()).cast_unsigned())?,
                Code::Imin => self.cw(i, I, |a| (a[0].cast_signed()).min(a[1].cast_signed()).cast_unsigned())?,
                Code::Umax => self.cw(i, U, |a| a[0].max(a[1]))?,
                Code::Umin => self.cw(i, U, |a| a[0].min(a[1]))?,
                Code::Imul | Code::Umul => {
                    // (hi, lo) = a * b
                    let k = if i.op == Code::Imul { I } else { U };
                    let a = self.src(&i.ops[2], k)?;
                    let c = self.src(&i.ops[3], k)?;
                    let wide: [u64; 4] = std::array::from_fn(|n| {
                        if k == I { (i64::from(a[n].cast_signed()) * i64::from(c[n].cast_signed())).cast_unsigned() } else { u64::from(a[n]) * u64::from(c[n]) }
                    });
                    self.dst(&i.ops[0], wide.map(|x| kome::num::lo32(x >> 32)), false)?;
                    self.dst(&i.ops[1], wide.map(|x| kome::num::lo32(x)), false)?;
                }
                Code::Udiv => {
                    let a = self.src(&i.ops[2], U)?;
                    let c = self.src(&i.ops[3], U)?;
                    let q = std::array::from_fn(|n| a[n].checked_div(c[n]).unwrap_or(!0));
                    let r = std::array::from_fn(|n| a[n].checked_rem(c[n]).unwrap_or(!0));
                    self.dst(&i.ops[0], q, false)?;
                    self.dst(&i.ops[1], r, false)?;
                }
                Code::Ineg => self.cw(i, I, |a| a[0].wrapping_neg())?,
                Code::Ishl => self.cw(i, I, |a| a[0] << (a[1] & 31))?,
                Code::Ishr => self.cw(i, I, |a| ((a[0].cast_signed()) >> (a[1] & 31)).cast_unsigned())?,
                Code::Ushr => self.cw(i, U, |a| a[0] >> (a[1] & 31))?,
                // (denormals are 0: WARP flushes them)
                Code::Log => self.cw(i, F, |a| b(if f(a[0]).is_subnormal() { f32::NEG_INFINITY } else { f(a[0]).log2() }))?,
                Code::Loop => blocks.push(pc - 1),
                Code::Mad => self.cw(i, F, |a| b(f(a[0]) * f(a[1]) + f(a[2])))?,
                Code::Min => {
                    let flip = self.zero_flip;
                    self.cw(i, F, |a| b(if flip && f(a[0]) == 0.0 && f(a[1]) == 0.0 { fmax(f(a[0]), f(a[1])) } else { fmin(f(a[0]), f(a[1])) }))?
                }
                Code::Max => {
                    let flip = self.zero_flip;
                    self.cw(i, F, |a| b(if flip && f(a[0]) == 0.0 && f(a[1]) == 0.0 { fmin(f(a[0]), f(a[1])) } else { fmax(f(a[0]), f(a[1])) }))?
                }
                Code::Mov => self.cw(i, F, |a| a[0])?,
                Code::Movc => {
                    let c = self.src(&i.ops[1], U)?;
                    let a = self.src(&i.ops[2], F)?;
                    let e = self.src(&i.ops[3], F)?;
                    self.dst(&i.ops[0], std::array::from_fn(|n| if c[n] != 0 { a[n] } else { e[n] }), i.sat)?;
                }
                Code::Mul => self.cw(i, F, |a| b(f(a[0]) * f(a[1])))?,
                Code::Nop => {}
                Code::Not => self.cw(i, U, |a| !a[0])?,
                Code::Or => self.cw(i, U, |a| a[0] | a[1])?,
                Code::Xor => self.cw(i, U, |a| a[0] ^ a[1])?,
                Code::Ret => return Ok(()),
                Code::Retc => {
                    if self.cond(i)? {
                        return Ok(());
                    }
                }
                Code::RoundNe => self.cw(i, F, |a| b(f(a[0]).round_ties_even()))?,
                Code::RoundNi => self.cw(i, F, |a| b(f(a[0]).floor()))?,
                Code::RoundPi => self.cw(i, F, |a| b(f(a[0]).ceil()))?,
                Code::RoundZ => self.cw(i, F, |a| b(f(a[0]).trunc()))?,
                Code::Rsq => self.cw(i, F, |a| b(1.0 / f(a[0]).sqrt()))?,
                Code::Sqrt => self.cw(i, F, |a| b(f(a[0]).sqrt()))?,
                Code::Switch => {
                    let s = self.src(&i.ops[0], I)?[0];
                    let start = pc - 1;
                    let end = p.jump[start];
                    blocks.push(start);
                    // the case with that label at this level, else default, else past the end
                    let mut depth = 0;
                    let mut target = None;
                    let mut default = None;
                    for j in start + 1..end {
                        match p.insts[j].op {
                            Code::If | Code::Loop | Code::Switch => depth += 1,
                            Code::EndIf | Code::EndLoop | Code::EndSwitch => depth -= 1,
                            Code::Case if depth == 0 && p.insts[j].ops[0].imm[0] == s && target.is_none() => target = Some(j),
                            Code::Default if depth == 0 => default = Some(j),
                            Code::Add | Code::And | Code::Break | Code::Breakc | Code::Case | Code::Continue
                            | Code::Continuec | Code::Default | Code::Div | Code::Dp2 | Code::Dp3 | Code::Dp4
                            | Code::Else | Code::Eq | Code::Exp | Code::Frc | Code::Ftoi | Code::Ftou
                            | Code::Ge | Code::Iadd | Code::Ieq | Code::Ige | Code::Ilt | Code::Imad
                            | Code::Imax | Code::Imin | Code::Imul | Code::Ine | Code::Ineg | Code::Ishl
                            | Code::Ishr | Code::Itof | Code::Log | Code::Lt | Code::Mad | Code::Min
                            | Code::Max | Code::Mov | Code::Movc | Code::Mul | Code::Ne | Code::Nop
                            | Code::Not | Code::Or | Code::Ret | Code::Retc | Code::RoundNe | Code::RoundNi
                            | Code::RoundPi | Code::RoundZ | Code::Rsq | Code::Sqrt | Code::SinCos | Code::Udiv
                            | Code::Ult | Code::Uge | Code::Umul | Code::Umad | Code::Umax | Code::Umin
                            | Code::Ushr | Code::Utof | Code::Xor | Code::Rcp | Code::F32ToF16 | Code::F16ToF32
                            | Code::CountBits | Code::FirstBitHi | Code::FirstBitLo | Code::FirstBitShi | Code::Ubfe | Code::Ibfe
                            | Code::Bfi | Code::Bfrev | Code::Swapc => {}
                        }
                    }
                    match target.or(default) {
                        Some(j) => pc = j + 1,
                        None => {
                            blocks.pop();
                            pc = end + 1;
                        }
                    }
                }
                Code::SinCos => {
                    let a = self.src(&i.ops[2], F)?;
                    if a.iter().any(|&x| f(x).abs() > 1e4) {
                        self.taint("sincos far from 0");
                    }
                    self.dst(&i.ops[0], a.map(|x| b(f(x).sin())), i.sat)?;
                    self.dst(&i.ops[1], a.map(|x| b(f(x).cos())), i.sat)?;
                }
                Code::Rcp => self.cw(i, F, |a| b(1.0 / f(a[0])))?,
                Code::F32ToF16 => self.cw(i, F, |a| f32tof16(f(a[0])))?,
                Code::F16ToF32 => self.cw(i, U, |a| b(half::f16::from_bits(kome::num::lo16(a[0])).to_f32()))?,
                Code::CountBits => self.cw(i, U, |a| a[0].count_ones())?,
                Code::FirstBitHi => self.cw(i, U, |a| if a[0] == 0 { !0 } else { a[0].leading_zeros() })?,
                Code::FirstBitLo => self.cw(i, U, |a| if a[0] == 0 { !0 } else { a[0].trailing_zeros() })?,
                Code::FirstBitShi => self.cw(i, I, |a| {
                    let x = a[0].cast_signed();
                    if x == 0 || x == -1 { !0 } else if x < 0 { (!a[0]).leading_zeros() } else { a[0].leading_zeros() }
                })?,
                Code::Ubfe | Code::Ibfe => self.cw(i, U, |a| {
                    let (w, o, x) = (a[0] & 31, a[1] & 31, a[2]);
                    if w == 0 {
                        0
                    } else if w + o < 32 {
                        let sh = x << (32 - w - o);
                        if i.op == Code::Ubfe { sh >> (32 - w) } else { (sh.cast_signed() >> (32 - w)).cast_unsigned() }
                    } else if i.op == Code::Ubfe {
                        x >> o
                    } else {
                        (x.cast_signed() >> o).cast_unsigned()
                    }
                })?,
                Code::Bfi => self.cw(i, U, |a| {
                    let (w, o, x, y) = (a[0] & 31, a[1] & 31, a[2], a[3]);
                    let m = kome::num::lo32((1u64 << w) - 1);
                    let m = m.wrapping_shl(o);
                    (x.wrapping_shl(o) & m) | (y & !m)
                })?,
                Code::Bfrev => self.cw(i, U, |a| a[0].reverse_bits())?,
                Code::Swapc => {
                    let c = self.src(&i.ops[2], U)?;
                    let a = self.src(&i.ops[3], U)?;
                    let e = self.src(&i.ops[4], U)?;
                    let d0 = std::array::from_fn(|n| if c[n] != 0 { e[n] } else { a[n] });
                    let d1 = std::array::from_fn(|n| if c[n] != 0 { a[n] } else { e[n] });
                    self.dst(&i.ops[0], d0, false)?;
                    self.dst(&i.ops[1], d1, false)?;
                }
            }
        }
        Ok(())
    }
}

/// Runs a pixel shader on constant buffers `cb`; returns o0..o7, and why they may not be what
/// every implementation gives (see `Run::taint`).
pub fn run(p: &Program, cb: &[Vec<V>]) -> Result<([V; 8], Option<String>), String> {
    let go = |zero_flip| -> Result<([V; 8], Option<String>), String> {
        let mut r = Run {
            p,
            r: vec![[0; 4]; p.temps],
            x: p.xs.iter().map(|&(n, s)| (n, vec![[0; 4]; kome::num::us(s)])).collect(),
            o: [[0; 4]; 8],
            cb,
            taint: Default::default(),
            zero_flip,
        };
        r.exec()?;
        Ok((r.o, r.taint.into_inner()))
    };
    let (o, t) = go(false)?;
    // (WGSL may ignore the sign of a zero: when it reaches the outputs, through a movc, a bitwise
    // op or a division, any answer is right; oracle seed 447 #587: max(-18, -0.0) into a movc)
    if t.is_none() && go(true).map(|(o2, _)| o2 != o).unwrap_or(true) {
        return Ok((o, Some("the sign of a zero decided the result".into())));
    }
    Ok((o, t))
}
