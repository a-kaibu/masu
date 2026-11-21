//! Shader model 2/3 output: lower.rs's instructions (in its sm3 mode, where ints and bools are
//! floats and true is 1.0) as Direct3D 9 shader assembly, assembled by d3d9asm.rs, with a CTAB
//! comment (the constant table D3DXGetShaderConstantTable and ID3DXConstantTable read).
//! Instruction count limits are not checked: the bytecode only feeds a WGSL translation (dxwgsl).

use std::collections::HashMap;

use super::sm4::*;
use crate::d3d9asm::Op as D9;

/// the operations this output has (anything else is an error here)
pub fn float_op(op: Opcode) -> bool {
    matches!(
        op,
        ADD | MUL
            | MAD
            | DIV
            | MIN
            | MAX
            | MOV
            | MOVC
            | EQ
            | NE
            | LT
            | GE
            | FRC
            | ROUND_NI
            | ROUND_PI
            | ROUND_Z
            | ROUND_NE
            | SQRT
            | RSQ
            | EXP
            | LOG
            | RCP
            | DP2
            | DP3
            | DP4
    )
}

fn op_name(op: Opcode) -> String {
    match op {
        CONTINUE => "continue".into(),
        SWITCH | CASE | DEFAULT | ENDSWITCH => "switch".into(),
        AND | OR | XOR | NOT | ISHL | ISHR | USHR => "bit operations".into(),
        COUNTBITS | FIRSTBIT_HI | FIRSTBIT_LO | FIRSTBIT_SHI | BFREV => "bit counting".into(),
        LD | RESINFO | GATHER4 | SAMPLE_C | SAMPLE_C_LZ => "this texture operation".into(),
        F32TOF16 | F16TOF32 => "half conversion".into(),
        Opcode::Add | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
        | Opcode::Continuec | Opcode::Cut | Opcode::DerivRtx
        | Opcode::DerivRty | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
        | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::Eq | Opcode::Exp | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
        | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
        | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Itof | Opcode::Label | Opcode::LdMs | Opcode::Log
        | Opcode::Loop | Opcode::Lt | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
        | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
        | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::SinCos
        | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
        | Opcode::Umin | Opcode::Utof | Opcode::DclResource | Opcode::DclConstantBuffer
        | Opcode::DclSampler | Opcode::DclIndexRange | Opcode::DclGsOutputPrimitiveTopology | Opcode::DclGsInputPrimitive | Opcode::DclMaxOutputVertexCount | Opcode::DclInput
        | Opcode::DclInputSgv | Opcode::DclInputSiv | Opcode::DclInputPs | Opcode::DclInputPsSgv | Opcode::DclInputPsSiv | Opcode::DclOutput
        | Opcode::DclOutputSgv | Opcode::DclOutputSiv | Opcode::DclTemps | Opcode::DclIndexableTemp | Opcode::DclGlobalFlags | Opcode::Reserved10
        | Opcode::Lod | Opcode::SamplePos | Opcode::SampleInfo | Opcode::Reserved101 | Opcode::HsDecls
        | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase | Opcode::EmitStream | Opcode::CutStream | Opcode::EmitThenCutStream
        | Opcode::InterfaceCall | Opcode::BufInfo | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine
        | Opcode::Gather4C | Opcode::Gather4Po | Opcode::Gather4PoC | Opcode::Rcp | Opcode::Uaddc | Opcode::Usubb | Opcode::Ubfe | Opcode::Ibfe | Opcode::Bfi | Opcode::Swapc | Opcode::DclStream
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
        | Opcode::Itod | Opcode::Utod | Opcode::Unknown => format!("operation {:#x}", op.code()),
    }
}

const SW: [char; 4] = ['x', 'y', 'z', 'w'];

fn mask_str(m: u8) -> String {
    if m == 0xf || m == 0 {
        return String::new();
    }
    let mut s = String::from(".");
    for (k, c) in SW.iter().enumerate() {
        if m & 1 << k != 0 {
            s.push(*c);
        }
    }
    s
}

fn swz_str(s: [u8; 4]) -> String {
    if s == [0, 1, 2, 3] {
        return String::new();
    }
    let mut o = String::from(".");
    for k in s {
        o.push(SW[usize::from(k)]);
    }
    o
}

fn num(v: u32) -> String {
    let f = f32::from_bits(v);
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "1e39".into()
        } else {
            "-1e39".into()
        }
    } else {
        format!("{f:?}")
    }
}

/// usage of a semantic (dcl_*)
fn usage(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_uppercase().as_str() {
        "SV_POSITION" | "POSITION" => "position",
        "POSITIONT" => "positiont",
        "NORMAL" => "normal",
        "TEXCOORD" => "texcoord",
        "COLOR" => "color",
        "TANGENT" => "tangent",
        "BINORMAL" => "binormal",
        "BLENDWEIGHT" => "blendweight",
        "BLENDINDICES" => "blendindices",
        "PSIZE" => "psize",
        "FOG" => "fog",
        "DEPTH" => "depth",
        "SV_TARGET" => "color",
        _ => return None,
    })
}

/// A component of a source operand, replicated.
fn comp(s: &Src, k: usize) -> Src {
    match s {
        Src::R { r, swz, neg, abs, one } => {
            let c = if *one { swz[0] } else { swz[k] };
            Src::R { r: *r, swz: [c; 4], neg: *neg, abs: *abs, one: true }
        }
        Src::Imm(v, scalar) => Src::Imm([if *scalar { v[0] } else { v[k] }; 4], true),
        s @ (Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_)) => s.clone(),
    }
}

fn neg(s: &Src) -> Src {
    match s {
        Src::R { r, swz, neg, abs, one } => Src::R { r: *r, swz: *swz, neg: !neg, abs: *abs, one: *one },
        Src::Imm(v, sc) => Src::Imm(v.map(|x| if x == 0 { 0 } else { x ^ 0x8000_0000 }), *sc),
        s @ (Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_)) => s.clone(),
    }
}

fn abs(s: &Src) -> Src {
    match s {
        Src::R { r, swz, one, .. } => Src::R { r: *r, swz: *swz, neg: false, abs: true, one: *one },
        Src::Imm(v, sc) => Src::Imm(v.map(|x| x & 0x7fff_ffff), *sc),
        s @ (Src::Tex(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_)) => s.clone(),
    }
}

/// a scratch temp (after the allocated ones), all components
fn scr(k: u32) -> Src {
    Src::R { r: Reg { f: RegF::Temp(1000 + k), rel: None }, swz: [0, 1, 2, 3], neg: false, abs: false, one: false }
}

/// A constant table type (D3DXSHADER_TYPEINFO)
fn ctype(v: &CbVar) -> (u16, u16, u16, u16, u16) {
    let ty = match v.ty {
        1 => 1,
        2 | 19 => 2,
        x => x,
    };
    (v.class, ty, v.rows, v.cols, v.elems.max(1))
}

struct Gen<'a> {
    s: &'a Shader,
    temps: u32,
    lines: Vec<String>,
    /// def registers: their components
    defs: Vec<[Option<u32>; 4]>,
    def_base: u32,
    /// (cbuffer index, old register) -> c register
    cmap: HashMap<(usize, u32), u32>,
    ins: HashMap<u32, String>,
    outs: HashMap<u32, String>,
    /// sampler slot -> texture dimension (sm4 numbering)
    samplers: Vec<(u32, u32)>,
    loops: bool,
}

impl Gen<'_> {
    fn line(&mut self, l: String) {
        self.lines.push(l);
    }

    /// a def constant holding `v`: register and swizzle
    fn konst(&mut self, v: [u32; 4]) -> String {
        let mut distinct: Vec<u32> = Vec::new();
        for x in v {
            if !distinct.contains(&x) {
                distinct.push(x);
            }
        }
        let fits = |d: &[Option<u32>; 4]| {
            let missing = distinct.iter().filter(|x| !d.contains(&Some(**x))).count();
            missing <= d.iter().filter(|c| c.is_none()).count()
        };
        let i = match self.defs.iter().position(fits) {
            Some(i) => i,
            None => {
                self.defs.push([None; 4]);
                self.defs.len() - 1
            }
        };
        let d = &mut self.defs[i];
        for x in &distinct {
            if !d.contains(&Some(*x)) {
                // (a def with room: found above)
                if let Some(c) = d.iter_mut().find(|c| c.is_none()) {
                    *c = Some(*x);
                }
            }
        }
        let swz: [u8; 4] = v.map(|x| d.iter().position(|c| *c == Some(x)).and_then(|k| u8::try_from(k).ok()).unwrap_or(0));
        format!("c{}{}", self.def_base + kome::num::u32_of(i), swz_str(swz))
    }

    fn reg(&self, r: &Reg) -> Result<String, String> {
        Ok(match r.f {
            RegF::Temp(t) if t >= 1000 => {
                let n = self.temps + t - 1000;
                Self::check_temp(n)?;
                format!("r{n}")
            }
            RegF::Temp(t) => format!("r{t}"),
            RegF::Input(i) => self.ins.get(&i).cloned().ok_or("an input without a register")?,
            RegF::Output(i) => self.outs.get(&i).cloned().ok_or("an output without a register")?,
            RegF::Depth => "oDepth".into(),
            RegF::Cb(sym, k) => {
                let cb = self.s.cb_map[kome::num::us(sym)].ok_or("an unused constant")?;
                let n = self.cmap.get(&(cb, k)).ok_or("constant without a register")?;
                if r.rel.is_some() {
                    return Ok(format!("c[a0.x + {n}]"));
                }
                format!("c{n}")
            }
            RegF::Icb(_) | RegF::X(..) | RegF::Null | RegF::Sys(_) | RegF::Uav(_) | RegF::G(_) | RegF::GsInput(..) => return Err("indexable temps and immediate constant buffers are not available in shader model 2/3".into()),
        })
    }

    fn src(&mut self, s: &Src) -> Result<String, String> {
        Ok(match s {
            Src::R { r, swz, neg, abs, one } => {
                let name = self.reg(r)?;
                let sw = if *one { [swz[0]; 4] } else { *swz };
                let sw = swz_str(sw);
                let sw = if sw == ".xxxx" {
                    ".x".into()
                } else if sw == ".yyyy" {
                    ".y".into()
                } else if sw == ".zzzz" {
                    ".z".into()
                } else if sw == ".wwww" {
                    ".w".into()
                } else {
                    sw
                };
                format!("{}{}{}{}", if *neg { "-" } else { "" }, name, if *abs { "_abs" } else { "" }, sw)
            }
            Src::Imm(v, scalar) => {
                let v = if *scalar { [v[0]; 4] } else { *v };
                self.konst(v)
            }
            Src::Samp(_) | Src::Tex(..) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_) => return Err("texture operand".into()),
        })
    }

    fn dst(&self, d: &Dst) -> Result<String, String> {
        let n = self.reg(&d.r)?;
        if d.r.f == RegF::Depth {
            return Ok(n);
        }
        Ok(format!("{n}{}", mask_str(d.mask)))
    }

    /// `op dst, srcs` (dst a scratch register with `mask` when `to` is None)
    fn op(&mut self, op: D9, sat: bool, d: &str, srcs: &[&Src]) -> Result<(), String> {
        let mut l = format!("{}{} {d}", op.name(), if sat { "_sat" } else { "" });
        for s in srcs {
            let t = self.src(s)?;
            l.push_str(", ");
            l.push_str(&t);
        }
        self.line(l);
        Ok(())
    }

    /// r31 for every target, ps_2_0 / vs_2_0 too (whose D3D9 limit is 12): the bound is what the
    /// bytecode and sake's translator (dxwgsl) can address. This allocator needs more temporaries
    /// than fxc for the same shader (tests::sm3's vs_2_0 uses r12), so the runtime's limit would
    /// refuse shaders fxc compiles; sake's D3D9 runs them, so staying lenient loses nothing.
    fn check_temp(n: u32) -> Result<(), String> {
        if n >= 32 {
            Err(format!("shader model 2/3 temporary register r{n} exceeds r31"))
        } else {
            Ok(())
        }
    }

    fn scr_d(&self, k: u32, mask: u8) -> Result<String, String> {
        Self::check_temp(self.temps + k)?;
        Ok(format!("r{}{}", self.temps + k, mask_str(mask)))
    }

    fn sampler(&self, s: &Src, t: &Src) -> Result<(String, u32), String> {
        let Src::Samp(sym) = s else {
            return Err("a sampler is needed".into());
        };
        let n = self.s.samp_map[kome::num::us(*sym)].ok_or("an unused sampler")?;
        let slot = self.s.samps[n].slot;
        let dim = match t {
            Src::Tex(t, _) => self.s.texs[self.s.tex_map[kome::num::us(*t)].ok_or("an unused texture")?].dim,
            Src::R { .. } | Src::Imm(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_) => 3,
        };
        Ok((format!("s{slot}"), dim))
    }

    fn ins(&mut self, i: &Ins) -> Result<(), String> {
        let pixel = self.s.pixel;
        // (relative constants: the address register first)
        let mut rel = None;
        for s in &i.src {
            if let Src::R { r: Reg { rel: Some(x), .. }, .. } = s {
                if rel.is_some_and(|y| y != *x) {
                    return Err("two indexed constant arrays in one operation".into());
                }
                rel = Some(*x);
            }
        }
        if let Some((t, c)) = rel {
            if pixel {
                return Err("indexed constants in a pixel shader".into());
            }
            let r = self.reg(&Reg { f: RegF::Temp(t), rel: None })?;
            self.line(format!("mova a0.x, {r}.{}", SW[usize::from(c)]));
        }
        let a = |k: usize| i.src.get(k).cloned().ok_or_else(|| "missing operand".to_string());
        let sat = i.sat;
        let one = Src::Imm([0x3f80_0000; 4], true);
        let zero = Src::Imm([0; 4], true);
        match i.op {
            RET => {}
            IF | BREAKC => {
                let c = self.src(&comp(&a(0)?, 0))?;
                let z = self.src(&zero)?;
                let name = if i.op == IF { "ifc" } else { "breakc" };
                self.line(format!("{name}_{} {c}, {z}", if i.nz { "ne" } else { "eq" }));
            }
            ELSE => self.line("else".into()),
            ENDIF => self.line("endif".into()),
            LOOP => {
                self.loops = true;
                self.line("rep i0".into());
            }
            ENDLOOP => self.line("endrep".into()),
            BREAK => self.line("break".into()),
            DISCARD => {
                if !pixel {
                    return Err("discard in a vertex shader".into());
                }
                let s = a(0)?;
                let n = kome::num::us(i.ctl);
                if n > 0 {
                    // clip(): any component below 0
                    let d = self.scr_d(0, 0xf)?;
                    self.op(D9::Mov, false, &d, &[&s])?;
                    let m = self.scr_d(0, Lane::mask(n))?;
                    self.line(format!("texkill {m}"));
                    return Ok(());
                }
                let c = comp(&s, 0);
                if let Src::Imm(v, _) = c {
                    if (v[0] != 0) != i.nz {
                        return Ok(());
                    }
                    let d = self.scr_d(0, 1)?;
                    self.op(D9::Mov, false, &d, &[&Src::Imm([0xbf80_0000; 4], true)])?;
                } else if i.nz {
                    let d = self.scr_d(0, 1)?;
                    self.op(D9::Mov, false, &d, &[&neg(&abs(&c))])?;
                } else {
                    let d = self.scr_d(0, 1)?;
                    self.op(D9::Cmp, false, &d, &[&neg(&abs(&c)), &Src::Imm([0xbf80_0000; 4], true), &zero])?;
                }
                let m = self.scr_d(0, 1)?;
                self.line(format!("texkill {m}"));
            }
            Opcode::Add | Opcode::And | Opcode::Call | Opcode::Callc
            | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
            | Opcode::DerivRty | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
            | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndSwitch
            | Opcode::Eq | Opcode::Exp | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
            | Opcode::Iadd | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
            | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
            | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
            | Opcode::Lt | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
            | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Not
            | Opcode::Or | Opcode::ResInfo | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
            | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
            | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
            | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
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
            | Opcode::Itod | Opcode::Utod | Opcode::Unknown => {
                let Some(d0) = i.dst.first() else {
                    return Err(op_name(i.op));
                };
                let m = d0.mask;
                let d = if d0.r.f == RegF::Null { String::new() } else { self.dst(d0)? };
                let simple = match i.op {
                    MOV => Some(D9::Mov),
                    ADD => Some(D9::Add),
                    MUL => Some(D9::Mul),
                    MAD => Some(D9::Mad),
                    MIN => Some(D9::Min),
                    MAX => Some(D9::Max),
                    FRC => Some(D9::Frc),
                    DP3 => Some(D9::Dp3),
                    DP4 => Some(D9::Dp4),
                    DERIV_RTX => Some(D9::Dsx),
                    DERIV_RTY => Some(D9::Dsy),
                    Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                    | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                    | Opcode::Eq | Opcode::Exp | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
                    | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                    | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                    | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                    | Opcode::Loop | Opcode::Lt | Opcode::CustomData
                    | Opcode::Movc | Opcode::Ne | Opcode::Nop | Opcode::Not
                    | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                    | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
                    | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
                    | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
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
                    | Opcode::Itod | Opcode::Utod | Opcode::Unknown => None,
                };
                if let Some(name) = simple {
                    if matches!(i.op, DERIV_RTX | DERIV_RTY) && !pixel {
                        return Err("derivatives in a vertex shader".into());
                    }
                    let srcs: Vec<Src> = i.src.clone();
                    let refs: Vec<&Src> = srcs.iter().collect();
                    return self.op(name, sat, &d, &refs);
                }
                match i.op {
                    DIV | SQRT | RSQ | RCP | EXP | LOG => {
                        let x = if i.op == DIV { a(1)? } else { a(0)? };
                        for k in 0..4 {
                            if m & 1 << k == 0 {
                                continue;
                            }
                            let dk = self.scr_d(0, 1 << k)?;
                            let ck = comp(&x, k);
                            match i.op {
                                DIV => self.op(D9::Rcp, false, &dk, &[&ck])?,
                                SQRT => {
                                    self.op(D9::Rsq, false, &dk, &[&ck])?;
                                    let sk = comp(&scr(0), k);
                                    self.op(D9::Rcp, false, &dk, &[&sk])?;
                                }
                                RSQ => self.op(D9::Rsq, false, &dk, &[&ck])?,
                                RCP => self.op(D9::Rcp, false, &dk, &[&ck])?,
                                EXP => self.op(D9::Exp, false, &dk, &[&ck])?,
                                Opcode::Add | Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                                | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                                | Opcode::DerivRty | Opcode::Discard | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
                                | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                                | Opcode::Eq | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
                                | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                                | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                                | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                                | Opcode::Loop | Opcode::Lt | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                                | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Not
                                | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                                | Opcode::RoundPi | Opcode::RoundZ | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
                                | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Switch | Opcode::SinCos
                                | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
                                | Opcode::Umin | Opcode::Ushr | Opcode::Utof | Opcode::Xor | Opcode::DclResource | Opcode::DclConstantBuffer
                                | Opcode::DclSampler | Opcode::DclIndexRange | Opcode::DclGsOutputPrimitiveTopology | Opcode::DclGsInputPrimitive | Opcode::DclMaxOutputVertexCount | Opcode::DclInput
                                | Opcode::DclInputSgv | Opcode::DclInputSiv | Opcode::DclInputPs | Opcode::DclInputPsSgv | Opcode::DclInputPsSiv | Opcode::DclOutput
                                | Opcode::DclOutputSgv | Opcode::DclOutputSiv | Opcode::DclTemps | Opcode::DclIndexableTemp | Opcode::DclGlobalFlags | Opcode::Reserved10
                                | Opcode::Lod | Opcode::Gather4 | Opcode::SamplePos | Opcode::SampleInfo | Opcode::Reserved101 | Opcode::HsDecls
                                | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase | Opcode::EmitStream | Opcode::CutStream | Opcode::EmitThenCutStream
                                | Opcode::InterfaceCall | Opcode::BufInfo | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine
                                | Opcode::Gather4C | Opcode::Gather4Po | Opcode::Gather4PoC | Opcode::F32ToF16 | Opcode::F16ToF32
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
                                | Opcode::Itod | Opcode::Utod | Opcode::Unknown => self.op(D9::Log, false, &dk, &[&ck])?,
                            }
                        }
                        if i.op == DIV {
                            self.op(D9::Mul, sat, &d, &[&a(0)?, &scr(0)])?;
                        } else {
                            self.op(D9::Mov, sat, &d, &[&scr(0)])?;
                        }
                    }
                    ROUND_NI => {
                        let s0 = self.scr_d(0, m)?;
                        self.op(D9::Frc, false, &s0, &[&a(0)?])?;
                        self.op(D9::Add, sat, &d, &[&a(0)?, &neg(&scr(0))])?;
                    }
                    ROUND_PI => {
                        let s0 = self.scr_d(0, m)?;
                        self.op(D9::Frc, false, &s0, &[&neg(&a(0)?)])?;
                        self.op(D9::Add, sat, &d, &[&a(0)?, &scr(0)])?;
                    }
                    ROUND_NE => {
                        let s0 = self.scr_d(0, m)?;
                        let s1 = self.scr_d(1, m)?;
                        self.op(D9::Add, false, &s0, &[&a(0)?, &Src::Imm([0x3f00_0000; 4], true)])?;
                        self.op(D9::Frc, false, &s1, &[&scr(0)])?;
                        self.op(D9::Add, sat, &d, &[&scr(0), &neg(&scr(1))])?;
                    }
                    ROUND_Z => {
                        let s0 = self.scr_d(0, m)?;
                        let x = a(0)?;
                        self.op(D9::Frc, false, &s0, &[&abs(&x)])?;
                        self.op(D9::Add, false, &s0, &[&abs(&x), &neg(&scr(0))])?;
                        if pixel {
                            self.op(D9::Cmp, sat, &d, &[&x, &scr(0), &neg(&scr(0))])?;
                        } else {
                            let s1 = self.scr_d(1, m)?;
                            self.op(D9::Sge, false, &s1, &[&x, &zero])?;
                            self.op(D9::Mad, false, &s1, &[&scr(1), &Src::Imm([0x4000_0000; 4], true), &Src::Imm([0xbf80_0000; 4], true)])?;
                            self.op(D9::Mul, sat, &d, &[&scr(0), &scr(1)])?;
                        }
                    }
                    EQ | NE | LT | GE => {
                        let (x, y) = (a(0)?, a(1)?);
                        if pixel {
                            let s0 = self.scr_d(0, m)?;
                            self.op(D9::Add, false, &s0, &[&x, &neg(&y)])?;
                            let na = neg(&abs(&scr(0)));
                            match i.op {
                                EQ => self.op(D9::Cmp, sat, &d, &[&na, &one, &zero])?,
                                NE => self.op(D9::Cmp, sat, &d, &[&na, &zero, &one])?,
                                LT => self.op(D9::Cmp, sat, &d, &[&scr(0), &zero, &one])?,
                                Opcode::Add | Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                                | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                                | Opcode::DerivRty | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
                                | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                                | Opcode::Exp | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
                                | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                                | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                                | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                                | Opcode::Loop | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                                | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Nop | Opcode::Not
                                | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                                | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
                                | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
                                | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
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
                                | Opcode::Itod | Opcode::Utod | Opcode::Unknown => self.op(D9::Cmp, sat, &d, &[&scr(0), &one, &zero])?,
                            }
                        } else {
                            match i.op {
                                LT => self.op(D9::Slt, sat, &d, &[&x, &y])?,
                                GE => self.op(D9::Sge, sat, &d, &[&x, &y])?,
                                Opcode::Add | Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                                | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                                | Opcode::DerivRty | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
                                | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                                | Opcode::Eq | Opcode::Exp | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                                | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                                | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                                | Opcode::Loop | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                                | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Not
                                | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                                | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::Sample | Opcode::SampleC | Opcode::SampleCLz
                                | Opcode::SampleL | Opcode::SampleD | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
                                | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
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
                                | Opcode::Itod | Opcode::Utod | Opcode::Unknown => {
                                    let s0 = self.scr_d(0, m)?;
                                    let s1 = self.scr_d(1, m)?;
                                    self.op(D9::Sge, false, &s0, &[&x, &y])?;
                                    self.op(D9::Sge, false, &s1, &[&y, &x])?;
                                    if i.op == EQ {
                                        self.op(D9::Mul, sat, &d, &[&scr(0), &scr(1)])?;
                                    } else {
                                        self.op(D9::Mul, false, &s0, &[&scr(0), &scr(1)])?;
                                        self.op(D9::Add, sat, &d, &[&neg(&scr(0)), &one])?;
                                    }
                                }
                            }
                        }
                    }
                    MOVC => {
                        let (c, x, y) = (a(0)?, a(1)?, a(2)?);
                        if pixel {
                            self.op(D9::Cmp, sat, &d, &[&neg(&abs(&c)), &y, &x])?;
                        } else {
                            let s0 = self.scr_d(0, m)?;
                            self.op(D9::Add, false, &s0, &[&x, &neg(&y)])?;
                            self.op(D9::Mad, sat, &d, &[&c, &scr(0), &y])?;
                        }
                    }
                    DP2 => {
                        if pixel {
                            self.op(D9::Dp2add, sat, &d, &[&a(0)?, &a(1)?, &zero])?;
                        } else {
                            let s = self.scr_d(0, 3)?;
                            self.op(D9::Mul, false, &s, &[&a(0)?, &a(1)?])?;
                            self.op(D9::Add, sat, &d, &[&comp(&scr(0), 0), &comp(&scr(0), 1)])?;
                        }
                    }
                    SINCOS => {
                        let x = a(0)?;
                        let dsin = i.dst.first().filter(|d| d.r.f != RegF::Null).cloned();
                        let dcos = i.dst.get(1).filter(|d| d.r.f != RegF::Null).cloned();
                        let mm = dsin.as_ref().map_or(0, |d| d.mask) | dcos.as_ref().map_or(0, |d| d.mask);
                        for k in 0..4 {
                            if mm & 1 << k == 0 {
                                continue;
                            }
                            let sc = self.scr_d(0, 3)?;
                            let ck = comp(&x, k);
                            if self.s.major < 3 {
                                // (D3DSINCOSCONST1/2)
                                let c1 = Src::Imm(
                                    [(-1.5500992e-6f32).to_bits(), (-2.1701389e-5f32).to_bits(), 0.0026041667f32.to_bits(), 0.00026041668f32.to_bits()],
                                    false,
                                );
                                let c2 = Src::Imm([(-0.020833334f32).to_bits(), (-0.125f32).to_bits(), 1f32.to_bits(), 0.5f32.to_bits()], false);
                                self.op(D9::Sincos, false, &sc, &[&ck, &c1, &c2])?;
                            } else {
                                self.op(D9::Sincos, false, &sc, &[&ck])?;
                            }
                            let s1 = self.scr_d(1, 1 << k)?;
                            let s2 = self.scr_d(2, 1 << k)?;
                            self.op(D9::Mov, false, &s1, &[&comp(&scr(0), 1)])?;
                            self.op(D9::Mov, false, &s2, &[&comp(&scr(0), 0)])?;
                        }
                        if let Some(ds) = dsin {
                            let dd = self.dst(&ds)?;
                            self.op(D9::Mov, sat, &dd, &[&scr(1)])?;
                        }
                        if let Some(dc) = dcos {
                            let dd = self.dst(&dc)?;
                            self.op(D9::Mov, sat, &dd, &[&scr(2)])?;
                        }
                    }
                    SAMPLE | SAMPLE_L | SAMPLE_B | SAMPLE_D => {
                        if i.offset != [0; 3] {
                            return Err("texel offsets need shader model 4".into());
                        }
                        let (coord, tex, smp) = (a(0)?, a(1)?, a(2)?);
                        let (sname, dim) = self.sampler(&smp, &tex)?;
                        let s: u32 = sname[1..].parse().map_err(|e| format!("sampler {sname}: {e}"))?;
                        if !self.samplers.iter().any(|x| x.0 == s) {
                            self.samplers.push((s, dim));
                        }
                        let tswz = match tex {
                            Src::Tex(_, s) => s,
                            Src::R { .. } | Src::Imm(..) | Src::Samp(_) | Src::SampC(..) | Src::Uav(..) | Src::G(..) | Src::Stream(_) => [0, 1, 2, 3],
                        };
                        let direct = tswz == [0, 1, 2, 3];
                        let td = if direct { d.clone() } else { self.scr_d(1, 0xf)? };
                        let op = if !pixel && i.op == SAMPLE { SAMPLE_L } else { i.op };
                        match op {
                            SAMPLE => {
                                let c = self.src(&coord)?;
                                self.line(format!("texld {td}, {c}, {sname}"));
                            }
                            SAMPLE_D => {
                                let c = self.src(&coord)?;
                                let dx = self.src(&a(3)?)?;
                                let dy = self.src(&a(4)?)?;
                                self.line(format!("texldd {td}, {c}, {sname}, {dx}, {dy}"));
                            }
                            Opcode::Add | Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                            | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                            | Opcode::DerivRty | Opcode::Discard | Opcode::Div | Opcode::Dp2 | Opcode::Dp3 | Opcode::Dp4
                            | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                            | Opcode::Eq | Opcode::Exp | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Ge
                            | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                            | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                            | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Log
                            | Opcode::Loop | Opcode::Lt | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                            | Opcode::Mov | Opcode::Movc | Opcode::Mul | Opcode::Ne | Opcode::Nop | Opcode::Not
                            | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::RoundNe | Opcode::RoundNi
                            | Opcode::RoundPi | Opcode::RoundZ | Opcode::Rsq | Opcode::SampleC | Opcode::SampleCLz
                            | Opcode::SampleL | Opcode::SampleB | Opcode::Sqrt | Opcode::Switch | Opcode::SinCos
                            | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
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
                            | Opcode::Itod | Opcode::Utod | Opcode::Unknown => {
                                let s = self.scr_d(0, 0xf)?;
                                self.op(D9::Mov, false, &s, &[&coord])?;
                                let w = self.scr_d(0, 8)?;
                                let lod = if i.op == SAMPLE { zero.clone() } else { comp(&a(3)?, 0) };
                                self.op(D9::Mov, false, &w, &[&lod])?;
                                let name = if op == SAMPLE_L { "texldl" } else { "texldb" };
                                self.line(format!("{name} {td}, r{}, {sname}", self.temps));
                            }
                        }
                        if sat && direct {
                            self.op(D9::Mov, true, &d, &[&Src::R { r: d0.r, swz: [0, 1, 2, 3], neg: false, abs: false, one: false }])?;
                        }
                        if !direct {
                            let mut sw = [0u8; 4];
                            for (k, x) in sw.iter_mut().enumerate() {
                                *x = tswz[k];
                            }
                            self.op(D9::Mov, sat, &d, &[&Src::R { r: Reg { f: RegF::Temp(1001), rel: None }, swz: sw, neg: false, abs: false, one: false }])?;
                        }
                    }
                    op @ (Opcode::Add | Opcode::And | Opcode::Break | Opcode::Breakc | Opcode::Call | Opcode::Callc
                          | Opcode::Case | Opcode::Continue | Opcode::Continuec | Opcode::Cut | Opcode::Default | Opcode::DerivRtx
                          | Opcode::DerivRty | Opcode::Discard | Opcode::Dp3 | Opcode::Dp4
                          | Opcode::Else | Opcode::Emit | Opcode::EmitThenCut | Opcode::EndIf | Opcode::EndLoop | Opcode::EndSwitch
                          | Opcode::Frc | Opcode::Ftoi | Opcode::Ftou | Opcode::Iadd | Opcode::If | Opcode::Ieq | Opcode::Ige | Opcode::Ilt | Opcode::Imad
                          | Opcode::Imax | Opcode::Imin | Opcode::Imul | Opcode::Ine | Opcode::Ineg | Opcode::Ishl
                          | Opcode::Ishr | Opcode::Itof | Opcode::Label | Opcode::Ld | Opcode::LdMs | Opcode::Loop | Opcode::Mad | Opcode::Min | Opcode::Max | Opcode::CustomData
                          | Opcode::Mov | Opcode::Mul | Opcode::Nop | Opcode::Not
                          | Opcode::Or | Opcode::ResInfo | Opcode::Ret | Opcode::Retc | Opcode::SampleC | Opcode::SampleCLz
                          | Opcode::Switch | Opcode::Udiv | Opcode::Ult | Opcode::Uge | Opcode::Umul | Opcode::Umad | Opcode::Umax
                          | Opcode::Umin | Opcode::Ushr | Opcode::Utof | Opcode::Xor | Opcode::DclResource | Opcode::DclConstantBuffer
                          | Opcode::DclSampler | Opcode::DclIndexRange | Opcode::DclGsOutputPrimitiveTopology | Opcode::DclGsInputPrimitive | Opcode::DclMaxOutputVertexCount | Opcode::DclInput
                          | Opcode::DclInputSgv | Opcode::DclInputSiv | Opcode::DclInputPs | Opcode::DclInputPsSgv | Opcode::DclInputPsSiv | Opcode::DclOutput
                          | Opcode::DclOutputSgv | Opcode::DclOutputSiv | Opcode::DclTemps | Opcode::DclIndexableTemp | Opcode::DclGlobalFlags | Opcode::Reserved10
                          | Opcode::Lod | Opcode::Gather4 | Opcode::SamplePos | Opcode::SampleInfo | Opcode::Reserved101 | Opcode::HsDecls
                          | Opcode::HsControlPointPhase | Opcode::HsForkPhase | Opcode::HsJoinPhase | Opcode::EmitStream | Opcode::CutStream | Opcode::EmitThenCutStream
                          | Opcode::InterfaceCall | Opcode::BufInfo | Opcode::DerivRtxCoarse | Opcode::DerivRtxFine | Opcode::DerivRtyCoarse | Opcode::DerivRtyFine
                          | Opcode::Gather4C | Opcode::Gather4Po | Opcode::Gather4PoC | Opcode::F32ToF16 | Opcode::F16ToF32
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
                          | Opcode::Itod | Opcode::Utod | Opcode::Unknown) => return Err(format!("{} is not available in shader model 2/3", op_name(op))),
                }
            }
        }
        Ok(())
    }
}

/// A uniform's place in the constant table.
pub struct Uni {
    pub name: String,
    pub reg: u32,
    pub count: u32,
}

fn put_str(b: &mut Vec<u8>, s: &str) -> u32 {
    let at = kome::num::u32_of(b.len());
    b.extend_from_slice(s.as_bytes());
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    at
}

fn set16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn set32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// a D3DXSHADER_TYPEINFO (and its members) at the end of `b`
fn put_type(b: &mut Vec<u8>, v: &CbVar) -> u32 {
    let at = b.len();
    b.resize(at + 16, 0);
    let (class, ty, rows, cols, elems) = ctype(v);
    for (k, x) in [class, ty, rows, cols, elems, crate::sat16(v.members.len())].iter().enumerate() {
        set16(b, at + 2 * k, *x);
    }
    if !v.members.is_empty() {
        let mi = b.len();
        b.resize(mi + 8 * v.members.len(), 0);
        set32(b, at + 12, kome::num::u32_of(mi));
        for (k, m) in v.members.iter().enumerate() {
            let n = put_str(b, &m.name);
            let t = put_type(b, m);
            set32(b, mi + 8 * k, n);
            set32(b, mi + 8 * k + 4, t);
        }
    }
    kome::num::u32_of(at)
}

/// The CTAB body: (name, register set, register, count, variable).
fn ctab(target: &str, entries: &[(String, u16, u32, u32, CbVar)]) -> Vec<u8> {
    let mut b = vec![0u8; 28];
    set32(&mut b, 0, 28);
    let info = b.len();
    b.resize(info + 20 * entries.len(), 0);
    let creator = put_str(&mut b, "sake HLSL compiler");
    let tgt = put_str(&mut b, target);
    set32(&mut b, 4, creator);
    let v: u32 = if target.starts_with("ps") { 0xffff_0000 } else { 0xfffe_0000 };
    let digits: Vec<u32> = target.split('_').skip(1).map(|x| x.parse().unwrap_or(0)).collect();
    set32(&mut b, 8, v | digits.first().copied().unwrap_or(0) << 8 | digits.get(1).copied().unwrap_or(0));
    set32(&mut b, 12, kome::num::u32_of(entries.len()));
    set32(&mut b, 16, kome::num::u32_of(info));
    set32(&mut b, 24, tgt);
    for (k, (name, set, reg, count, var)) in entries.iter().enumerate() {
        let e = info + 20 * k;
        let n = put_str(&mut b, name);
        let t = put_type(&mut b, var);
        set32(&mut b, e, n);
        set16(&mut b, e + 4, *set);
        set16(&mut b, e + 6, crate::sat16(*reg));
        set16(&mut b, e + 8, crate::sat16(*count));
        set32(&mut b, e + 12, t);
    }
    b
}

/// The shader's assembly text and constant table.
pub fn assembly(mut s: Shader) -> Result<(String, Vec<u8>), String> {
    let temps = alloc(&mut s.code, &s.marks);
    // alloc densely numbers all ordinary temporaries below `temps`. Validate
    // their budget once; emitter-only scratch registers are checked on use.
    Gen::check_temp(temps.saturating_sub(1))?;
    let (pixel, major, minor) = (s.pixel, s.major.max(2), if s.major < 2 { 0 } else { s.minor });
    let target = format!("{}_{major}_{minor}", if pixel { "ps" } else { "vs" });
    // uniforms: explicit registers first, then the lowest free
    let mut taken: Vec<(u32, u32)> = Vec::new();
    let mut cmap = HashMap::new();
    let mut entries: Vec<(String, u16, u32, u32, CbVar)> = Vec::new();
    let mut vars: Vec<(usize, &CbVar)> = Vec::new();
    for (ci, cb) in s.cbs.iter().enumerate() {
        for v in cb.vars.iter().filter(|v| v.used) {
            vars.push((ci, v));
        }
    }
    vars.sort_by_key(|(_, v)| !v.explicit);
    for (ci, v) in vars {
        let old = v.offset / 16;
        let count = v.size.div_ceil(16).max(1);
        let reg = if v.explicit {
            old
        } else {
            let mut r = 0;
            while taken.iter().any(|&(a, n)| r < a + n && a < r + count) {
                r += 1;
            }
            r
        };
        taken.push((reg, count));
        for k in 0..count {
            cmap.insert((ci, old + k), reg + k);
        }
        entries.push((v.name.clone(), 2, reg, count, v.clone()));
    }
    let def_base = taken.iter().map(|&(a, n)| a + n).max().unwrap_or(0);
    // inputs and outputs
    let mut ins = HashMap::new();
    let mut outs = HashMap::new();
    let mut head = vec![target.clone()];
    for e in &s.inputs {
        let up = e.name.to_ascii_uppercase();
        let mask = mask_str(e.mask);
        let name = if pixel && up == "SV_POSITION" {
            if major < 3 {
                return Err("VPOS needs ps_3_0".into());
            }
            head.push("dcl vPos.xy".into());
            "vPos".to_string()
        } else if pixel && up == "SV_ISFRONTFACE" {
            if major < 3 {
                return Err("VFACE needs ps_3_0".into());
            }
            head.push("dcl vFace".into());
            "vFace".to_string()
        } else if pixel && major < 3 {
            let r = match up.as_str() {
                "TEXCOORD" => format!("t{}", e.index),
                "COLOR" => format!("v{}", e.index),
                _ => return Err(format!("ps_2_0 input semantic {}{}", e.name, e.index)),
            };
            head.push(format!("dcl {r}{mask}"));
            r
        } else {
            let u = usage(&e.name).ok_or(format!("input semantic {}{} in shader model 2/3", e.name, e.index))?;
            let r = format!("v{}", e.reg);
            head.push(format!("dcl_{u}{} {r}{mask}", e.index));
            r
        };
        ins.insert(e.reg, name);
    }
    for e in &s.outputs {
        let up = e.name.to_ascii_uppercase();
        let name = if pixel {
            match up.as_str() {
                "SV_TARGET" => format!("oC{}", e.index),
                "SV_DEPTH" => "oDepth".into(),
                _ => return Err(format!("pixel shader output semantic {}{}", e.name, e.index)),
            }
        } else if major < 3 {
            match up.as_str() {
                "SV_POSITION" => "oPos".into(),
                "TEXCOORD" => format!("oT{}", e.index),
                "COLOR" => format!("oD{}", e.index),
                "FOG" => "oFog".into(),
                "PSIZE" => "oPts".into(),
                _ => return Err(format!("vs_2_0 output semantic {}{}", e.name, e.index)),
            }
        } else {
            let u = usage(&e.name).ok_or(format!("output semantic {}{} in shader model 2/3", e.name, e.index))?;
            let r = format!("o{}", e.reg);
            head.push(format!("dcl_{u}{} {r}{}", e.index, mask_str(e.mask)));
            r
        };
        if !e.depth {
            outs.insert(e.reg, name);
        }
    }
    let mut g = Gen { s: &s, temps, lines: Vec::new(), defs: Vec::new(), def_base, cmap, ins, outs, samplers: Vec::new(), loops: false };
    let code = s.code.clone();
    for i in &code {
        g.ins(i)?;
    }
    // samplers
    let mut samplers = g.samplers.clone();
    samplers.sort();
    for (slot, dim) in &samplers {
        let t = match dim {
            6 => "cube",
            5 => "volume",
            _ => "2d",
        };
        head.push(format!("dcl_{t} s{slot}"));
        if let Some(sm) = s.samps.iter().find(|x| x.slot == *slot) {
            let ty = match dim {
                6 => 14,
                5 => 13,
                2 => 11,
                _ => 12,
            };
            let var =
                CbVar { name: sm.name.clone(), offset: 0, size: 0, class: 4, ty, rows: 1, cols: 1, elems: 1, used: true, explicit: false, members: Vec::new() };
            entries.push((sm.name.clone(), 3, *slot, 1, var));
        }
    }
    for (k, d) in g.defs.iter().enumerate() {
        let v: Vec<String> = d.iter().map(|c| num(c.unwrap_or(0))).collect();
        head.push(format!("def c{}, {}", def_base + kome::num::u32_of(k), v.join(", ")));
    }
    if g.loops {
        head.push("defi i0, 255, 0, 1, 0".into());
    }
    let mut text = head.join("\n");
    text.push('\n');
    text.push_str(&g.lines.join("\n"));
    text.push('\n');
    // (CTAB types: D3D10's 19 uint is an int)
    for e in &mut entries {
        if e.4.ty == 19 {
            e.4.ty = 2;
        }
    }
    Ok((text, ctab(&target, &entries)))
}

/// The shader's bytecode (with its CTAB comment after the version token).
pub fn finish(s: Shader) -> Result<Vec<u8>, String> {
    let (text, tab) = assembly(s)?;
    let words = crate::d3d9asm::assemble(&text).map_err(|e| format!("internal error: {e}\n{text}"))?;
    let mut out = Vec::new();
    out.extend_from_slice(&words[0].to_le_bytes());
    let mut body = b"CTAB".to_vec();
    body.extend_from_slice(&tab);
    while body.len() % 4 != 0 {
        body.push(0);
    }
    // (a comment's length has 15 bits: a longer constant table would set bit 31 and break the
    // token stream)
    if body.len() / 4 > 0x7fff {
        return Err(format!("constant table of {} dwords (at most 32767)", body.len() / 4));
    }
    out.extend_from_slice(&(0xfffe | ((kome::num::u32_of(body.len()) / 4) << 16)).to_le_bytes());
    out.extend_from_slice(&body);
    for w in &words[1..] {
        out.extend_from_slice(&w.to_le_bytes());
    }
    Ok(out)
}
