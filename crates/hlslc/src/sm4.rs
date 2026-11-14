//! Shader model 4/5 code: instructions on virtual temporaries, their register allocation, the
//! token encoding and the DXBC container (RDEF, ISGN, OSGN, SHDR/SHEX).

pub use dxwgsl::tpf::Opcode;

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
    Sys(u32),
    /// a typed unordered access view (symbol): store_uav_typed's destination
    Uav(u32),
    /// thread group shared memory (symbol): store_raw's destination
    G(u32),
}

/// A register operand with an optional relative index (a temp's component) added to the last
/// index.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Reg {
    pub f: RegF,
    pub rel: Option<(u32, u8)>,
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
/// sync: ctl 1 threads in the group, 2 shared memory, 4 UAVs (group), 8 UAVs (global)
pub const SYNC: Opcode = Opcode::Sync;

/// A signature element of the shader's interface.
#[derive(Clone, Debug)]
pub struct SigEl {
    pub name: String,
    pub index: u32,
    /// D3D_NAME (0 for SV_Target and generic ones)
    pub sysval: u32,
    /// 1 uint, 2 sint, 3 float
    pub ctype: u32,
    pub reg: u32,
    pub mask: u8,
    /// pixel inputs: interpolation mode (1 constant, 2 linear, 3 centroid, 4 noperspective..)
    pub interp: u32,
    /// the register is oDepth
    pub depth: bool,
}

pub struct Tex {
    pub name: String,
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
    /// in registers
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
    pub sys_inputs: Vec<(u32, u8)>,
    pub uavs: Vec<Uav>,
    pub uav_map: Vec<Option<usize>>,
    /// thread group shared memory: bytes of each g register
    pub tgsm: Vec<u32>,
    /// the blob D3D10CompileShader (d3d10.dll's own compiler) writes: elements 1 for what is
    /// not an array, no texture component flags nor sample count, SV_Target / SV_Depth without
    /// a system value in the signature (d3d10refl.c)
    pub d3d10: bool,
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
        } else if i.op == ENDLOOP {
            if let Some(s) = stack.pop() {
                loops.push((s, n));
            }
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

/// operand types
const T_TEMP: u32 = 0;
const T_INPUT: u32 = 1;
const T_OUTPUT: u32 = 2;
const T_X: u32 = 3;
const T_IMM32: u32 = 4;
const T_SAMPLER: u32 = 6;
const T_RESOURCE: u32 = 7;
const T_CB: u32 = 8;
const T_ICB: u32 = 9;
const T_DEPTH: u32 = 12;
const T_NULL: u32 = 13;
const T_UAV: u32 = 30;
const T_TGSM: u32 = 31;

struct Enc<'a> {
    s: &'a Shader,
}

impl Enc<'_> {
    /// operand token pieces: (type, indices)
    fn reg_parts(&self, r: &Reg) -> (u32, Vec<u32>) {
        match r.f {
            RegF::Temp(i) => (T_TEMP, vec![i]),
            RegF::Input(i) => (T_INPUT, vec![i]),
            RegF::Output(i) => (T_OUTPUT, vec![i]),
            RegF::Cb(s, i) => (T_CB, vec![self.s.cbs[self.s.cb_map[crate::us(s)].unwrap()].slot, i]),
            RegF::Icb(i) => (T_ICB, vec![i]),
            RegF::X(a, i) => (T_X, vec![a, i]),
            RegF::Depth => (T_DEPTH, vec![]),
            RegF::Null => (T_NULL, vec![]),
            RegF::Sys(t) => (t, vec![]),
            RegF::Uav(u) => (T_UAV, vec![self.s.uavs[self.s.uav_map[crate::us(u)].unwrap()].slot]),
            RegF::G(g) => (T_TGSM, vec![g]),
        }
    }

    /// `sel`: 0 mask, 1 swizzle, 2 select-1; `num` component count code (0 = none, 1 = one, 2 = four)
    fn reg(&self, out: &mut Vec<u32>, r: &Reg, num: u32, sel: u32, bits: u32, modifier: u32) {
        let (ty, idx) = self.reg_parts(r);
        let mut t = num | (ty << 12) | (crate::u32_of(idx.len()) << 20);
        if num == 2 {
            t |= (sel << 2) | (bits << 4);
        }
        let rel_at = idx.len().wrapping_sub(1);
        if r.rel.is_some() {
            // last index: immediate + relative
            t |= 3 << (22 + 3 * crate::u32_of(rel_at));
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
            if k == rel_at {
                if let Some((tr, c)) = r.rel {
                    // the relative part: a temp's component (select-1)
                    out.push(2 | (2 << 2) | (u32::from(c) << 4) | (T_TEMP << 12) | (1 << 20));
                    out.push(tr);
                }
            }
        }
    }

    fn src(&self, out: &mut Vec<u32>, s: &Src) {
        match s {
            Src::R { r, swz, neg, abs, one } => {
                let m = u32::from(*neg) | (u32::from(*abs) << 1);
                if matches!(r.f, RegF::Depth) {
                    self.reg(out, r, 1, 0, 0, m);
                } else if *one {
                    self.reg(out, r, 2, 2, u32::from(swz[0]), m);
                } else {
                    let b = u32::from(swz[0]) | u32::from(swz[1]) << 2 | u32::from(swz[2]) << 4 | u32::from(swz[3]) << 6;
                    self.reg(out, r, 2, 1, b, m);
                }
            }
            Src::Imm(v, one) => {
                if *one {
                    out.push(1 | (T_IMM32 << 12));
                    out.push(v[0]);
                } else {
                    out.push(2 | (T_IMM32 << 12));
                    out.extend_from_slice(v);
                }
            }
            Src::Tex(t, swz) => {
                let slot = self.s.texs[self.s.tex_map[crate::us(*t)].unwrap()].slot;
                let b = u32::from(swz[0]) | u32::from(swz[1]) << 2 | u32::from(swz[2]) << 4 | u32::from(swz[3]) << 6;
                out.push(2 | (1 << 2) | (b << 4) | (T_RESOURCE << 12) | (1 << 20));
                out.push(slot);
            }
            Src::Samp(s) => {
                let slot = self.s.samps[self.s.samp_map[crate::us(*s)].unwrap()].slot;
                out.push((T_SAMPLER << 12) | (1 << 20));
                out.push(slot);
            }
            Src::SampC(s, c) => {
                let slot = self.s.samps[self.s.samp_map[crate::us(*s)].unwrap()].slot;
                out.push(2 | (2 << 2) | (u32::from(*c) << 4) | (T_SAMPLER << 12) | (1 << 20));
                out.push(slot);
            }
            Src::Uav(u, swz) | Src::G(u, swz) => {
                let (ty, slot) = match s {
                    Src::Uav(..) => (T_UAV, self.s.uavs[self.s.uav_map[crate::us(*u)].unwrap()].slot),
                    Src::R { .. } | Src::Imm(..) | Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::G(..) => (T_TGSM, *u),
                };
                let b = u32::from(swz[0]) | u32::from(swz[1]) << 2 | u32::from(swz[2]) << 4 | u32::from(swz[3]) << 6;
                out.push(2 | (1 << 2) | (b << 4) | (ty << 12) | (1 << 20));
                out.push(slot);
            }
        }
    }

    fn dst(&self, out: &mut Vec<u32>, d: &Dst) {
        match d.r.f {
            RegF::Null => out.push(T_NULL << 12),
            RegF::Depth => self.reg(out, &d.r, 1, 0, 0, 0),
            RegF::Temp(_) | RegF::Input(_) | RegF::Output(_) | RegF::Cb(..) | RegF::Icb(_) | RegF::X(..)
            | RegF::Sys(_) | RegF::Uav(_) | RegF::G(_) => self.reg(out, &d.r, 2, 0, u32::from(d.mask), 0),
        }
    }

    fn ins(&self, out: &mut Vec<u32>, i: &Ins) {
        let at = out.len();
        let ext = i.offset != [0; 3];
        out.push(0);
        if ext {
            let o = |v: i32| (v as u32) & 0xf;
            out.push(1 | (o(i.offset[0]) << 9) | (o(i.offset[1]) << 13) | (o(i.offset[2]) << 17));
        }
        for d in &i.dst {
            self.dst(out, d);
        }
        for s in &i.src {
            self.src(out, s);
        }
        let len = crate::u32_of(out.len() - at);
        out[at] = i.op.code() | (i.ctl << 11) | (u32::from(i.sat) << 13) | (u32::from(i.nz) << 18) | (len << 24) | (u32::from(ext) << 31);
    }
}

fn decl(out: &mut Vec<u32>, op: Opcode, ctl: u32, body: &[u32]) {
    out.push(op.code() | (ctl << 11) | (crate::u32_of(body.len() + 1) << 24));
    out.extend_from_slice(body);
}

/// The SHDR/SHEX chunk.
fn program(s: &mut Shader) -> Vec<u32> {
    let temps = alloc(&mut s.code, &s.marks.clone());
    let s = &*s;
    let e = Enc { s };
    let kind = if s.threads.is_some() { 5 } else { u32::from(!s.pixel) };
    let mut w = vec![kind << 16 | (s.major << 4) | s.minor, 0];
    // global flags: refactoring allowed (+ early depth stencil)
    decl(&mut w, Opcode::DclGlobalFlags, 1 | (u32::from(s.early_depth) << 2), &[]);
    if !s.icb.is_empty() {
        w.push(0x35 | (3 << 11));
        w.push(2 + 4 * crate::u32_of(s.icb.len()));
        for v in &s.icb {
            w.extend_from_slice(v);
        }
    }
    for cb in &s.cbs {
        let t = 2 | (1 << 2) | (0xe4 << 4) | (T_CB << 12) | (2 << 20);
        decl(&mut w, Opcode::DclConstantBuffer, u32::from(cb.dynamic), &[t, cb.slot, cb.size.max(1)]);
    }
    for sm in &s.samps {
        decl(&mut w, Opcode::DclSampler, u32::from(sm.cmp), &[(T_SAMPLER << 12) | (1 << 20), sm.slot]);
    }
    for t in &s.texs {
        let r = t.ret;
        let ret = r | (r << 4) | (r << 8) | (r << 12);
        decl(&mut w, Opcode::DclResource, t.dim, &[(T_RESOURCE << 12) | (1 << 20), t.slot, ret]);
    }
    for u in &s.uavs {
        let r = u.ret;
        let ret = r | (r << 4) | (r << 8) | (r << 12);
        decl(&mut w, Opcode::DclUavTyped, u.dim, &[(T_UAV << 12) | (1 << 20), u.slot, ret]);
    }
    for (g, &bytes) in s.tgsm.iter().enumerate() {
        decl(&mut w, Opcode::DclTgsmRaw, 0, &[(T_TGSM << 12) | (1 << 20), crate::u32_of(g), bytes]);
    }
    for &(ty, mask) in &s.sys_inputs {
        // (vThreadIDInGroupFlattened is one component)
        let t = if ty == 35 { 1 | (ty << 12) } else { 2 | (u32::from(mask) << 4) | (ty << 12) };
        decl(&mut w, Opcode::DclInput, 0, &[t]);
    }
    let reg_tok = |ty: u32, mask: u8| 2 | (u32::from(mask) << 4) | (ty << 12) | (1 << 20);
    for i in &s.inputs {
        let t = reg_tok(T_INPUT, i.mask);
        if s.pixel {
            match i.sysval {
                0 => decl(&mut w, Opcode::DclInputPs, i.interp, &[t, i.reg]),
                1 => decl(&mut w, Opcode::DclInputPsSiv, i.interp, &[t, i.reg, 1]),
                v => decl(&mut w, Opcode::DclInputPsSgv, 1, &[t, i.reg, v]),
            }
        } else if i.sysval != 0 {
            decl(&mut w, Opcode::DclInputSgv, 0, &[t, i.reg, i.sysval]);
        } else {
            decl(&mut w, Opcode::DclInput, 0, &[t, i.reg]);
        }
    }
    for o in &s.outputs {
        if o.depth {
            decl(&mut w, Opcode::DclOutput, 0, &[1 | (T_DEPTH << 12)]);
        } else if o.sysval != 0 && o.sysval < 64 {
            decl(&mut w, Opcode::DclOutputSiv, 0, &[reg_tok(T_OUTPUT, o.mask), o.reg, o.sysval]);
        } else {
            decl(&mut w, Opcode::DclOutput, 0, &[reg_tok(T_OUTPUT, o.mask), o.reg]);
        }
    }
    if temps > 0 {
        decl(&mut w, Opcode::DclTemps, 0, &[temps]);
    }
    for (n, &size) in s.xs.iter().enumerate() {
        decl(&mut w, Opcode::DclIndexableTemp, 0, &[crate::u32_of(n), size, 4]);
    }
    if let Some([x, y, z]) = s.threads {
        decl(&mut w, Opcode::DclThreadGroup, 0, &[x, y, z]);
    }
    for i in &s.code {
        e.ins(&mut w, i);
    }
    w[1] = crate::u32_of(w.len());
    w
}

fn put_str(b: &mut Vec<u8>, s: &str) -> u32 {
    let at = crate::u32_of(b.len());
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
    w32(&mut b, crate::u32_of(els.len()));
    w32(&mut b, 8);
    let names_at = 8 + 24 * els.len();
    let mut names = Vec::new();
    for e in els {
        let off = crate::u32_of(names_at) + crate::u32_of(names.len());
        put_str(&mut names, &e.name);
        w32(&mut b, off);
        w32(&mut b, e.index);
        w32(&mut b, if d3d10 && e.sysval >= 64 { 0 } else { e.sysval });
        w32(&mut b, e.ctype);
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

/// The resource definitions (what D3DReflect reads).
/// A variable's type record at the end of `b`, then its members' (name, type, offset) and
/// their types: the record's offset. 16 bytes (class, type, rows, columns, elements, members,
/// the members' offset), 36 in shader model 5 (then four words and the type name's offset, 0).
fn rdef_type(b: &mut Vec<u8>, v: &CbVar, sm5: bool, d3d10: bool, fix_names: &mut Vec<(usize, String)>) -> u32 {
    let ta = b.len();
    let elems = if d3d10 { v.elems.max(1) } else { v.elems };
    for x in [v.class, v.ty, v.rows, v.cols, elems, v.members.len() as u16] {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b.resize(ta + if sm5 { 36 } else { 16 }, 0);
    if !v.members.is_empty() {
        let ma = b.len();
        set32(b, ta + 12, crate::u32_of(ma));
        b.resize(ma + 12 * v.members.len(), 0);
        for (i, m) in v.members.iter().enumerate() {
            let at = ma + 12 * i;
            fix_names.push((at, m.name.clone()));
            let t = rdef_type(b, m, sm5, d3d10, fix_names);
            set32(b, at + 4, t);
            set32(b, at + 8, m.offset);
        }
    }
    crate::u32_of(ta)
}

fn rdef(s: &Shader) -> Vec<u8> {
    let sm5 = s.major >= 5;
    let header = if sm5 { 60 } else { 28 };
    let mut b = vec![0u8; header];
    let ncb = crate::u32_of(s.cbs.len());
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
        binds.push((t.name.clone(), 2, t.ret, dim, t.slot, comps | u32::from(t.explicit)));
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
        binds.push((u.name.clone(), 4, u.ret, dim, u.slot, (u.comps - 1) << 2 | u32::from(u.explicit)));
    }
    for cb in &s.cbs {
        binds.push((cb.name.clone(), 0, 0, 0, cb.slot, u32::from(cb.explicit)));
    }
    let cb_at = b.len();
    // constant buffer descriptions: name, vars, var offset, size, flags, type
    b.resize(cb_at + 24 * s.cbs.len(), 0);
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
        set32(&mut b, at + 20, bd.4);
        set32(&mut b, at + 24, 1);
        set32(&mut b, at + 28, bd.5);
    }
    let var_size = if sm5 { 40 } else { 24 };
    for (n, cb) in s.cbs.iter().enumerate() {
        let at = cb_at + 24 * n;
        fix_names.push((at, cb.name.clone()));
        set32(&mut b, at + 4, crate::u32_of(cb.vars.len()));
        let vars_at = b.len();
        set32(&mut b, at + 8, crate::u32_of(vars_at));
        set32(&mut b, at + 12, cb.size * 16);
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
    set32(&mut b, 4, crate::u32_of(cb_at));
    set32(&mut b, 8, crate::u32_of(binds.len()));
    set32(&mut b, 12, crate::u32_of(bind_at));
    let kind = if s.threads.is_some() { 0x4353 } else if s.pixel { 0xffff } else { 0xfffe };
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
pub fn finish(mut s: Shader) -> Vec<u8> {
    let prog = program(&mut s);
    let shdr: Vec<u8> = prog.iter().flat_map(|w| w.to_le_bytes()).collect();
    let chunks: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"RDEF", rdef(&s)),
        (*b"ISGN", signature(&s.inputs, false, s.d3d10)),
        (*b"OSGN", signature(&s.outputs, true, s.d3d10)),
        (if s.major >= 5 { *b"SHEX" } else { *b"SHDR" }, shdr),
    ];
    container(&chunks)
}

pub fn container(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"DXBC");
    b.extend_from_slice(&[0; 16]);
    w32(&mut b, 1);
    w32(&mut b, 0);
    w32(&mut b, crate::u32_of(chunks.len()));
    let mut off = 32 + 4 * chunks.len();
    for (_, c) in chunks {
        w32(&mut b, crate::u32_of(off));
        off += 8 + c.len();
    }
    for (t, c) in chunks {
        b.extend_from_slice(t);
        w32(&mut b, crate::u32_of(c.len()));
        b.extend_from_slice(c);
    }
    let n = crate::u32_of(b.len());
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
    let bits = crate::u32_of(len).wrapping_mul(8);
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
