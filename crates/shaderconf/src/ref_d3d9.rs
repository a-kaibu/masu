//! A reference interpreter of Direct3D 9 pixel shaders (ps_2_0 .. ps_3_0 tokens), written from
//! the D3D9 instruction reference and the shader token format (d3d9types.h), independent of
//! dxwgsl. ALU, flow control (if/ifc/else, rep, loop with aL, break/breakc/breakp,
//! call/callnz/ret/label), predication and def/defi/defb; textures, derivatives and texkill are
//! outside it (`Err`). Like ref_dxbc: checked against the cases' expected pixels, then an
//! oracle for bytecode nobody wrote expectations for.

/// The instructions this interpreter runs: name = D3DSIO opcode (its own table, from
/// d3d9types.h; not dxwgsl's)
macro_rules! codes {
    ($($name:ident = $code:literal;)*) => {
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
        }
    };
}

codes! {
    Nop = 0;
    Mov = 1;
    Add = 2;
    Sub = 3;
    Mad = 4;
    Mul = 5;
    Rcp = 6;
    Rsq = 7;
    Dp3 = 8;
    Dp4 = 9;
    Min = 10;
    Max = 11;
    Slt = 12;
    Sge = 13;
    Exp = 14;
    Log = 15;
    Lrp = 18;
    Frc = 19;
    M4x4 = 20;
    M4x3 = 21;
    M3x4 = 22;
    M3x3 = 23;
    M3x2 = 24;
    Call = 25;
    Callnz = 26;
    Loop = 27;
    Ret = 28;
    EndLoop = 29;
    Label = 30;
    Pow = 32;
    Crs = 33;
    Abs = 35;
    Nrm = 36;
    SinCos = 37;
    Rep = 38;
    EndRep = 39;
    If = 40;
    Ifc = 41;
    Else = 42;
    EndIf = 43;
    Break = 44;
    Breakc = 45;
    Defb = 47;
    Defi = 48;
    Expp = 78;
    Logp = 79;
    Cnd = 80;
    Def = 81;
    Cmp = 88;
    Dp2Add = 90;
    Setp = 94;
    Breakp = 96;
}

#[derive(Clone, Debug)]
struct Src {
    ty: u32,
    reg: u32,
    swz: [u8; 4],
    md: u32,
    /// relative addressing: (register type, register, component)
    rel: Option<(u32, u32, u8)>,
}

#[derive(Clone, Debug)]
struct Dst {
    ty: u32,
    reg: u32,
    mask: u8,
    sat: bool,
}

#[derive(Clone, Debug)]
struct Inst {
    op: Code,
    /// comparison (ifc, breakc, setp): 1 gt, 2 eq, 3 ge, 4 lt, 5 ne, 6 le
    cmp: u32,
    dst: Option<Dst>,
    pred: Option<Src>,
    src: Vec<Src>,
    /// def/defi/defb values
    raw: [u32; 4],
}

pub struct Program {
    insts: Vec<Inst>,
    /// matching flow control instruction (if -> else/endif, else -> endif, rep/loop -> end)
    jump: Vec<usize>,
    labels: Vec<(u32, usize)>,
}

impl Program {
    pub fn show(&self) -> String {
        self.insts.iter().enumerate().map(|(n, i)| format!("{n}: {i:?}\n")).collect()
    }
}

fn rtype(w: u32) -> u32 {
    ((w >> 28) & 7) | ((w >> 8) & 0x18)
}

fn src_token(w: u32) -> Src {
    Src { ty: rtype(w), reg: w & 0x7ff, swz: std::array::from_fn(|i| kome::num::lo8((w >> (16 + 2 * i)) & 3)), md: (w >> 24) & 0xf, rel: None }
}

const NO_DST: &[Code] = &[Code::Nop, Code::Call, Code::Callnz, Code::Loop, Code::Ret, Code::EndLoop, Code::Label, Code::Rep, Code::EndRep, Code::If, Code::Ifc, Code::Else, Code::EndIf, Code::Break, Code::Breakc, Code::Breakp];

pub fn parse(words: &[u32]) -> Result<Program, String> {
    let v = *words.first().ok_or("empty")?;
    if v >> 16 != 0xffff {
        return Err("not a pixel shader".into());
    }
    if (v >> 8) & 0xff < 2 {
        return Err("ps_1_x".into());
    }
    let mut p = Program { insts: Vec::new(), jump: Vec::new(), labels: Vec::new() };
    let mut at = 1;
    loop {
        let w = *words.get(at).ok_or("no end token")?;
        if w == 0xffff {
            break;
        }
        let op = w & 0xffff;
        if op == 0xfffe {
            at += 1 + kome::num::us((w >> 16) & 0x7fff);
            continue;
        }
        let len = kome::num::us((w >> 24) & 0xf);
        let body = words.get(at + 1..at + 1 + len).ok_or("instruction past the end")?;
        at += 1 + len;
        if op == 31 {
            // (dcl)
            continue;
        }
        // (what exec runs: anything else is outside the interpreter, run or not)
        let Some(code) = Code::from_code(op) else {
            return Err(format!("opcode {op}"));
        };
        if !matches!(code, Code::Defb | Code::Defi | Code::Def) && body.iter().any(|t| t >> 31 == 0) {
            return Err("a parameter token without bit 31".into());
        }
        let mut i = Inst { op: code, cmp: (w >> 16) & 7, dst: None, pred: None, src: Vec::new(), raw: [0; 4] };
        match code {
            Code::Defb | Code::Defi | Code::Def => {
                let d = *body.first().ok_or("short def")?;
                // (a def writes a constant register of its kind, through a parameter token)
                let want = match code {
                    Code::Def => 2,
                    Code::Defi => 7,
                    Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
                    | Code::Rcp | Code::Rsq | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max
                    | Code::Slt | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
                    | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
                    | Code::Callnz | Code::Loop | Code::Ret | Code::EndLoop | Code::Label | Code::Pow
                    | Code::Crs | Code::Abs | Code::Nrm | Code::SinCos | Code::Rep | Code::EndRep
                    | Code::If | Code::Ifc | Code::Else | Code::EndIf | Code::Break | Code::Breakc
                    | Code::Defb | Code::Expp | Code::Logp | Code::Cnd | Code::Cmp | Code::Dp2Add
                    | Code::Setp | Code::Breakp => 14,
                };
                if d >> 31 == 0 || rtype(d) != want {
                    return Err("a def of something that is not a constant register".into());
                }
                i.dst = Some(Dst { ty: rtype(d), reg: d & 0x7ff, mask: 0xf, sat: false });
                for (k, x) in body[1..].iter().take(4).enumerate() {
                    i.raw[k] = *x;
                }
                p.insts.push(i);
                continue;
            }
            Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
            | Code::Rcp | Code::Rsq | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max
            | Code::Slt | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
            | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
            | Code::Callnz | Code::Loop | Code::Ret | Code::EndLoop | Code::Label | Code::Pow
            | Code::Crs | Code::Abs | Code::Nrm | Code::SinCos | Code::Rep | Code::EndRep
            | Code::If | Code::Ifc | Code::Else | Code::EndIf | Code::Break | Code::Breakc
            | Code::Expp | Code::Logp | Code::Cnd | Code::Cmp | Code::Dp2Add | Code::Setp
            | Code::Breakp => {}
        }
        let mut k = 0;
        if !NO_DST.contains(&code) {
            let d = *body.first().ok_or("no destination")?;
            if d & (1 << 13) != 0 {
                return Err("relative destination".into());
            }
            i.dst = Some(Dst { ty: rtype(d), reg: d & 0x7ff, mask: kome::num::lo8((d >> 16) & 0xf), sat: (d >> 20) & 1 != 0 });
            if (d >> 24) & 0xf != 0 {
                return Err("shift scale".into());
            }
            k = 1;
        }
        if w & (1 << 28) != 0 {
            i.pred = Some(src_token(*body.get(k).ok_or("no predicate")?));
            k += 1;
        }
        while k < body.len() {
            let mut s = src_token(body[k]);
            k += 1;
            if body[k - 1] & (1 << 13) != 0 {
                let r = *body.get(k).ok_or("no relative register")?;
                k += 1;
                s.rel = Some((rtype(r), r & 0x7ff, kome::num::lo8((r >> 16) & 3)));
            }
            i.src.push(s);
        }
        p.insts.push(i);
    }
    p.jump = vec![usize::MAX; p.insts.len()];
    let mut stack: Vec<usize> = Vec::new();
    for n in 0..p.insts.len() {
        match p.insts[n].op {
            Code::If | Code::Ifc | Code::Rep | Code::Loop => stack.push(n),
            Code::Else => {
                let s = stack.pop().ok_or("else outside an if")?;
                if !matches!(p.insts[s].op, Code::If | Code::Ifc) {
                    return Err("else outside an if".into());
                }
                p.jump[s] = n;
                stack.push(n);
            }
            Code::EndIf | Code::EndRep | Code::EndLoop => {
                let s = stack.pop().ok_or("unbalanced flow control")?;
                let ok = match p.insts[n].op {
                    Code::EndIf => matches!(p.insts[s].op, Code::If | Code::Ifc | Code::Else),
                    Code::EndRep => p.insts[s].op == Code::Rep,
                    Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
                    | Code::Rcp | Code::Rsq | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max
                    | Code::Slt | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
                    | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
                    | Code::Callnz | Code::Loop | Code::Ret | Code::EndLoop | Code::Label | Code::Pow
                    | Code::Crs | Code::Abs | Code::Nrm | Code::SinCos | Code::Rep | Code::If
                    | Code::Ifc | Code::Else | Code::Break | Code::Breakc | Code::Defb | Code::Defi
                    | Code::Expp | Code::Logp | Code::Cnd | Code::Def | Code::Cmp | Code::Dp2Add
                    | Code::Setp | Code::Breakp => p.insts[s].op == Code::Loop,
                };
                if !ok {
                    return Err("unbalanced flow control".into());
                }
                p.jump[s] = n;
                p.jump[n] = s;
            }
            Code::Label => p.labels.push((p.insts[n].src.first().ok_or("label without a register")?.reg, n)),
            Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
            | Code::Rcp | Code::Rsq | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max
            | Code::Slt | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
            | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
            | Code::Callnz | Code::Ret | Code::Pow | Code::Crs | Code::Abs | Code::Nrm
            | Code::SinCos | Code::Break | Code::Breakc | Code::Defb | Code::Defi | Code::Expp
            | Code::Logp | Code::Cnd | Code::Def | Code::Cmp | Code::Dp2Add | Code::Setp
            | Code::Breakp => {}
        }
    }
    if !stack.is_empty() {
        return Err("unclosed flow control".into());
    }
    Ok(p)
}

type V = [f32; 4];

struct Frame {
    /// the rep/loop instruction
    start: usize,
    left: u32,
    /// loop: aL before it, and the step
    al: Option<(i32, i32)>,
}

struct Run<'a> {
    p: &'a Program,
    r: Vec<V>,
    c: Vec<V>,
    i: [[i32; 4]; 16],
    b: [bool; 16],
    pr: [bool; 4],
    al: i32,
    o: [V; 4],
    /// (as ref_dxbc's: out of range, or an infinity, NaN or denormal, whose result D3D9 leaves open)
    taint: std::cell::RefCell<Option<String>>,
}

fn cmp(c: u32, a: f32, b: f32) -> bool {
    match c {
        1 => a > b,
        2 => a == b,
        3 => a >= b,
        4 => a < b,
        5 => a != b,
        6 => a <= b,
        _ => false,
    }
}

impl Run<'_> {
    fn read(&self, s: &Src) -> Result<V, String> {
        let off = match s.rel {
            None => 0,
            Some((15, _, _)) => self.al,
            Some((t, _, _)) => return Err(format!("relative addressing through register type {t}")),
        };
        let v: V = match s.ty {
            0 => *self.r.get(kome::num::us(s.reg)).ok_or("temp out of range")?,
            1 | 17 => [0.0; 4],
            2 => match usize::try_from(s.reg.cast_signed() + off).ok().and_then(|k| self.c.get(k)) {
                Some(c) => *c,
                None => {
                    return Err("constant out of range".into());
                }
            },
            7 => self.i[kome::num::us(s.reg) & 15].map(|x| kome::num::to_f32(x)),
            14 => [if self.b[kome::num::us(s.reg) & 15] { 1.0 } else { 0.0 }; 4],
            15 => [kome::num::to_f32(self.al); 4],
            19 => self.pr.map(|x| if x { 1.0 } else { 0.0 }),
            t => return Err(format!("source register type {t}")),
        };
        let v: V = std::array::from_fn(|k| v[usize::from(s.swz[k])]);
        if v.iter().any(|x| x.is_subnormal()) {
            self.taint.borrow_mut().get_or_insert_with(|| "a denormal (implementations may flush it)".into());
        }
        Ok(match s.md {
            0 => v,
            1 => v.map(|x| -x),
            2 => v.map(|x| x - 0.5),
            3 => v.map(|x| -(x - 0.5)),
            4 => v.map(|x| 2.0 * x - 1.0),
            5 => v.map(|x| -(2.0 * x - 1.0)),
            6 => v.map(|x| 1.0 - x),
            7 => v.map(|x| 2.0 * x),
            8 => v.map(|x| -2.0 * x),
            11 => v.map(f32::abs),
            12 => v.map(|x| -x.abs()),
            m => return Err(format!("source modifier {m}")),
        })
    }
    fn boolean(&self, s: &Src) -> Result<bool, String> {
        let v = match s.ty {
            14 => self.b[kome::num::us(s.reg) & 15],
            19 => self.pr[usize::from(s.swz[0])],
            t => return Err(format!("condition register type {t}")),
        };
        Ok(if s.md == 13 { !v } else { v })
    }
    fn write(&mut self, i: &Inst, v: V) -> Result<(), String> {
        let d = i.dst.as_ref().ok_or("no destination")?;
        if (0..4).any(|k| d.mask & (1 << k) != 0 && (!v[k].is_finite() || v[k].is_subnormal())) {
            self.taint.borrow_mut().get_or_insert_with(|| "an infinity, NaN or denormal".into());
        }
        let v = if d.sat { v.map(|x| if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }) } else { v };
        let pred: [bool; 4] = match &i.pred {
            None => [true; 4],
            Some(p) => std::array::from_fn(|k| self.pr[usize::from(p.swz[k])] != (p.md == 13)),
        };
        if d.ty == 19 {
            for k in 0..4 {
                if d.mask & (1 << k) != 0 && pred[k] {
                    self.pr[k] = v[k] != 0.0;
                }
            }
            return Ok(());
        }
        let slot = match d.ty {
            0 => self.r.get_mut(kome::num::us(d.reg)).ok_or("temp out of range")?,
            8 => self.o.get_mut(kome::num::us(d.reg)).ok_or("oC out of range")?,
            t => return Err(format!("destination register type {t}")),
        };
        for k in 0..4 {
            if d.mask & (1 << k) != 0 && pred[k] {
                slot[k] = v[k];
            }
        }
        Ok(())
    }
    fn s(&self, i: &Inst, n: usize) -> Result<V, String> {
        self.read(i.src.get(n).ok_or("missing source")?)
    }

    fn exec(&mut self) -> Result<(), String> {
        let p = self.p;
        let mut pc = 0;
        let mut loops: Vec<Frame> = Vec::new();
        let mut calls: Vec<usize> = Vec::new();
        let mut steps = 0u32;
        while pc < p.insts.len() {
            steps += 1;
            if steps > 1_000_000 {
                return Err("runs too long".into());
            }
            let i = &p.insts[pc];
            pc += 1;
            let dp = |a: V, b: V, n: usize| -> f32 {
                let mut d = a[0] * b[0];
                for k in 1..n {
                    d += a[k] * b[k];
                }
                d
            };
            match i.op {
                Code::Nop => {}
                Code::Mov => {
                    let a = self.s(i, 0)?;
                    self.write(i, a)?
                }
                Code::Add | Code::Sub | Code::Mul | Code::Min | Code::Max | Code::Slt | Code::Sge => {
                    let (a, b) = (self.s(i, 0)?, self.s(i, 1)?);
                    let v = std::array::from_fn(|k| match i.op {
                        Code::Add => a[k] + b[k],
                        Code::Sub => a[k] - b[k],
                        Code::Mul => a[k] * b[k],
                        Code::Min => a[k].min(b[k]),
                        Code::Max => a[k].max(b[k]),
                        Code::Slt => kome::num::to_f32(u32::from(a[k] < b[k])),
                        Code::Nop | Code::Mov | Code::Mad | Code::Rcp | Code::Rsq | Code::Dp3
                        | Code::Dp4 | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
                        | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
                        | Code::Callnz | Code::Loop | Code::Ret | Code::EndLoop | Code::Label | Code::Pow
                        | Code::Crs | Code::Abs | Code::Nrm | Code::SinCos | Code::Rep | Code::EndRep
                        | Code::If | Code::Ifc | Code::Else | Code::EndIf | Code::Break | Code::Breakc
                        | Code::Defb | Code::Defi | Code::Expp | Code::Logp | Code::Cnd | Code::Def
                        | Code::Cmp | Code::Dp2Add | Code::Setp | Code::Breakp => kome::num::to_f32(u32::from(a[k] >= b[k])),
                    });
                    self.write(i, v)?
                }
                Code::Mad => {
                    let (a, b, c) = (self.s(i, 0)?, self.s(i, 1)?, self.s(i, 2)?);
                    self.write(i, std::array::from_fn(|k| a[k] * b[k] + c[k]))?
                }
                // (scalar: the replicated component of the source)
                Code::Rcp | Code::Rsq | Code::Exp | Code::Log | Code::Expp | Code::Logp => {
                    let x = self.s(i, 0)?[0];
                    let v = match i.op {
                        Code::Rcp => 1.0 / x,
                        Code::Rsq => 1.0 / x.abs().sqrt(),
                        Code::Exp | Code::Expp => x.exp2(),
                        Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
                        | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max | Code::Slt | Code::Sge
                        | Code::Log | Code::Lrp | Code::Frc | Code::M4x4 | Code::M4x3 | Code::M3x4
                        | Code::M3x3 | Code::M3x2 | Code::Call | Code::Callnz | Code::Loop | Code::Ret
                        | Code::EndLoop | Code::Label | Code::Pow | Code::Crs | Code::Abs | Code::Nrm
                        | Code::SinCos | Code::Rep | Code::EndRep | Code::If | Code::Ifc | Code::Else
                        | Code::EndIf | Code::Break | Code::Breakc | Code::Defb | Code::Defi | Code::Logp
                        | Code::Cnd | Code::Def | Code::Cmp | Code::Dp2Add | Code::Setp | Code::Breakp => x.abs().log2(),
                    };
                    self.write(i, [v; 4])?
                }
                Code::Dp3 | Code::Dp4 => {
                    let d = dp(self.s(i, 0)?, self.s(i, 1)?, if i.op == Code::Dp3 { 3 } else { 4 });
                    self.write(i, [d; 4])?
                }
                Code::Lrp => {
                    let (a, b, c) = (self.s(i, 0)?, self.s(i, 1)?, self.s(i, 2)?);
                    self.write(i, std::array::from_fn(|k| a[k] * (b[k] - c[k]) + c[k]))?
                }
                Code::Frc => {
                    let a = self.s(i, 0)?;
                    self.write(i, a.map(|x| x - x.floor()))?
                }
                Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 => {
                    // m4x4, m4x3, m3x4, m3x3, m3x2: rows from consecutive registers
                    let (n, rows) = [(4, 4), (4, 3), (3, 4), (3, 3), (3, 2)][kome::num::us(i.op.code() - 20)];
                    let a = self.s(i, 0)?;
                    let mut v = [0.0; 4];
                    for (row, out) in v.iter_mut().enumerate().take(rows) {
                        let mut s = i.src[1].clone();
                        s.reg += kome::num::u32_of(row);
                        *out = dp(a, self.read(&s)?, n);
                    }
                    self.write(i, v)?
                }
                Code::Pow => {
                    let (a, b) = (self.s(i, 0)?[0], self.s(i, 1)?[0]);
                    self.write(i, [a.abs().powf(b); 4])?
                }
                Code::Crs => {
                    let (a, b) = (self.s(i, 0)?, self.s(i, 1)?);
                    self.write(i, [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0], 0.0])?
                }
                Code::Abs => {
                    let a = self.s(i, 0)?;
                    self.write(i, a.map(f32::abs))?
                }
                Code::Nrm => {
                    let a = self.s(i, 0)?;
                    let f = 1.0 / dp(a, a, 3).sqrt();
                    self.write(i, a.map(|x| x * f))?
                }
                Code::SinCos => {
                    let x = self.s(i, 0)?[0];
                    if x.abs() > 1e4 {
                        self.taint.borrow_mut().get_or_insert_with(|| "sincos far from 0".into());
                    }
                    self.write(i, [x.cos(), x.sin(), 0.0, 0.0])?
                }
                Code::Cnd => {
                    let (a, b, c) = (self.s(i, 0)?, self.s(i, 1)?, self.s(i, 2)?);
                    self.write(i, std::array::from_fn(|k| if a[k] > 0.5 { b[k] } else { c[k] }))?
                }
                Code::Cmp => {
                    let (a, b, c) = (self.s(i, 0)?, self.s(i, 1)?, self.s(i, 2)?);
                    self.write(i, std::array::from_fn(|k| if a[k] >= 0.0 { b[k] } else { c[k] }))?
                }
                Code::Dp2Add => {
                    let (a, b, c) = (self.s(i, 0)?, self.s(i, 1)?, self.s(i, 2)?);
                    self.write(i, [a[0] * b[0] + a[1] * b[1] + c[0]; 4])?
                }
                Code::Setp => {
                    let (a, b) = (self.s(i, 0)?, self.s(i, 1)?);
                    let v = std::array::from_fn(|k| kome::num::to_f32(u32::from(cmp(i.cmp, a[k], b[k]))));
                    self.write(i, v)?
                }
                // (applied before the program runs)
                Code::Defb | Code::Defi | Code::Def => {}
                Code::If | Code::Ifc => {
                    let t = if i.op == Code::If { self.boolean(&i.src[0])? } else { cmp(i.cmp, self.s(i, 0)?[0], self.s(i, 1)?[0]) };
                    if !t {
                        pc = p.jump[pc - 1] + 1;
                    }
                }
                Code::Else => pc = p.jump[pc - 1] + 1,
                Code::EndIf => {}
                Code::Rep | Code::Loop => {
                    let n = i.src.last().ok_or("rep/loop without a count")?;
                    if n.ty != 7 {
                        return Err("rep/loop count not an i#".into());
                    }
                    let iv = self.i[kome::num::us(n.reg) & 15];
                    let count = iv[0].clamp(0, 255).cast_unsigned();
                    if count == 0 {
                        pc = p.jump[pc - 1] + 1;
                    } else {
                        let al = (i.op == Code::Loop).then(|| {
                            let old = self.al;
                            self.al = iv[1];
                            (old, iv[2])
                        });
                        loops.push(Frame { start: pc - 1, left: count, al });
                    }
                }
                Code::EndRep | Code::EndLoop => {
                    let f = loops.last_mut().ok_or("endrep outside a loop")?;
                    f.left -= 1;
                    if f.left > 0 {
                        if let Some((_, step)) = f.al {
                            self.al += step;
                        }
                        pc = f.start + 1;
                    } else {
                        let f = loops.pop().ok_or("endrep outside a loop")?;
                        if let Some((old, _)) = f.al {
                            self.al = old;
                        }
                    }
                }
                Code::Break | Code::Breakc | Code::Breakp => {
                    let t = match i.op {
                        Code::Break => true,
                        Code::Breakc => cmp(i.cmp, self.s(i, 0)?[0], self.s(i, 1)?[0]),
                        Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
                        | Code::Rcp | Code::Rsq | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max
                        | Code::Slt | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
                        | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
                        | Code::Callnz | Code::Loop | Code::Ret | Code::EndLoop | Code::Label | Code::Pow
                        | Code::Crs | Code::Abs | Code::Nrm | Code::SinCos | Code::Rep | Code::EndRep
                        | Code::If | Code::Ifc | Code::Else | Code::EndIf | Code::Defb | Code::Defi
                        | Code::Expp | Code::Logp | Code::Cnd | Code::Def | Code::Cmp | Code::Dp2Add
                        | Code::Setp | Code::Breakp => self.boolean(&i.src[0])?,
                    };
                    if t {
                        let f = loops.pop().ok_or("break outside a loop")?;
                        if let Some((old, _)) = f.al {
                            self.al = old;
                        }
                        pc = p.jump[f.start] + 1;
                    }
                }
                Code::Call | Code::Callnz => {
                    let l = i.src.first().ok_or("call without a label")?.reg;
                    if i.op == Code::Callnz && !self.boolean(&i.src[1])? {
                        continue;
                    }
                    let at = p.labels.iter().find(|x| x.0 == l).ok_or("call to a missing label")?.1;
                    if calls.len() >= 16 {
                        return Err("calls nested too deep".into());
                    }
                    calls.push(pc);
                    pc = at + 1;
                }
                Code::Ret => match calls.pop() {
                    Some(r) => pc = r,
                    None => return Ok(()),
                },
                // (subroutines follow the main program's ret: reached only through call)
                Code::Label => return Ok(()),
            }
        }
        Ok(())
    }
}

/// Runs a pixel shader with float constants `c` (c0, c1, ...); returns oC0..oC3, and why they
/// may not be what every implementation gives.
pub fn run(p: &Program, c: &[[f32; 4]]) -> Result<([[f32; 4]; 4], Option<String>), String> {
    let mut cs = vec![[0.0; 4]; 256];
    cs[..c.len()].copy_from_slice(c);
    let mut r = Run { p, r: vec![[0.0; 4]; 32], c: cs, i: [[0; 4]; 16], b: [false; 16], pr: [false; 4], al: 0, o: [[0.0; 4]; 4], taint: Default::default() };
    // def, defi, defb: whatever the application set
    let mut seen = Vec::new();
    for i in &p.insts {
        let Some(d) = &i.dst else { continue };
        if matches!(i.op, Code::Defb | Code::Defi | Code::Def) {
            if seen.contains(&(i.op, d.reg)) {
                return Err("a constant defined twice".into());
            }
            seen.push((i.op, d.reg));
        }
        match i.op {
            Code::Def => {
                if let Some(c) = r.c.get_mut(kome::num::us(d.reg)) {
                    *c = i.raw.map(f32::from_bits);
                }
            }
            Code::Defi => r.i[kome::num::us(d.reg) & 15] = i.raw.map(|x| x.cast_signed()),
            Code::Defb => r.b[kome::num::us(d.reg) & 15] = i.raw[0] != 0,
            Code::Nop | Code::Mov | Code::Add | Code::Sub | Code::Mad | Code::Mul
            | Code::Rcp | Code::Rsq | Code::Dp3 | Code::Dp4 | Code::Min | Code::Max
            | Code::Slt | Code::Sge | Code::Exp | Code::Log | Code::Lrp | Code::Frc
            | Code::M4x4 | Code::M4x3 | Code::M3x4 | Code::M3x3 | Code::M3x2 | Code::Call
            | Code::Callnz | Code::Loop | Code::Ret | Code::EndLoop | Code::Label | Code::Pow
            | Code::Crs | Code::Abs | Code::Nrm | Code::SinCos | Code::Rep | Code::EndRep
            | Code::If | Code::Ifc | Code::Else | Code::EndIf | Code::Break | Code::Breakc
            | Code::Expp | Code::Logp | Code::Cnd | Code::Cmp | Code::Dp2Add | Code::Setp
            | Code::Breakp => {}
        }
    }
    r.exec()?;
    Ok((r.o, r.taint.into_inner()))
}
