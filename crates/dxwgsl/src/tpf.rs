//! Shader model 4/5 bytecode (the "tokenized program format" of DXBC's SHDR/SHEX chunk),
//! decoded into instructions with their operands. wgsl.rs writes WGSL of them.

/// A program type (the version token's bits 16-31): its own number, not a register or opcode
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ProgramType(pub u32);

impl std::fmt::Display for ProgramType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An operand type (D3D10_SB_OPERAND_TYPE, bits 12-19 of the operand token): its own number
/// space, so a register number or opcode in its place does not compile
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct OpType(pub u32);

impl std::fmt::Display for OpType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl OpType {
    /// vThreadID, vThreadGroupID, vThreadIDInGroup, vThreadIDInGroupFlattened
    pub fn is_compute_input(self) -> bool {
        matches!(self, THREAD_ID | THREAD_GROUP_ID | THREAD_ID_IN_GROUP | THREAD_ID_IN_GROUP_FLATTENED)
    }
}

// program types
pub const PIXEL: ProgramType = ProgramType(0);
pub const VERTEX: ProgramType = ProgramType(1);
pub const COMPUTE: ProgramType = ProgramType(5);

// operand types
pub const TEMP: OpType = OpType(0);
pub const INPUT: OpType = OpType(1);
pub const OUTPUT: OpType = OpType(2);
pub const INDEXABLE_TEMP: OpType = OpType(3);
pub const IMM32: OpType = OpType(4);
pub const IMM64: OpType = OpType(5);
pub const SAMPLER: OpType = OpType(6);
pub const RESOURCE: OpType = OpType(7);
pub const CONSTANT_BUFFER: OpType = OpType(8);
pub const IMM_CONSTANT_BUFFER: OpType = OpType(9);
pub const OUTPUT_DEPTH: OpType = OpType(12);
pub const NULL: OpType = OpType(13);
pub const OUTPUT_COVERAGE_MASK: OpType = OpType(15);
pub const OUTPUT_DEPTH_GE: OpType = OpType(38);
pub const UAV: OpType = OpType(30);
pub const TGSM: OpType = OpType(31);
pub const THREAD_ID: OpType = OpType(32);
pub const THREAD_GROUP_ID: OpType = OpType(33);
pub const THREAD_ID_IN_GROUP: OpType = OpType(34);
pub const THREAD_ID_IN_GROUP_FLATTENED: OpType = OpType(35);
pub const OUTPUT_DEPTH_LE: OpType = OpType(39);

#[derive(Clone, Debug, PartialEq)]
pub enum Sel {
    /// write mask (destinations, declarations)
    Mask(u8),
    Swizzle([u8; 4]),
    /// one component
    Scalar(u8),
    /// a scalar or component-less operand
    None,
}

#[derive(Clone, Debug)]
pub enum Index {
    Imm(u32),
    /// a register's value plus an offset
    Rel(Box<Operand>, u32),
}

#[derive(Clone, Debug)]
pub struct Operand {
    pub ty: OpType,
    /// 0, 1 or 4
    pub comps: u8,
    pub sel: Sel,
    pub idx: Vec<Index>,
    /// immediate values (IMM32)
    pub imm: [u32; 4],
    /// 0 none, 1 negate, 2 absolute value, 3 both
    pub modifier: u8,
}

impl Operand {
    /// The register number (first immediate index).
    pub fn reg(&self) -> u32 {
        match self.idx.first() {
            Some(Index::Imm(i)) => *i,
            _ => 0,
        }
    }
    /// The write mask (destinations): components 0-3.
    pub fn mask(&self) -> u8 {
        match self.sel {
            Sel::Mask(m) => m,
            Sel::Scalar(c) => 1 << c,
            Sel::Swizzle(_) | Sel::None => if self.comps == 1 { 1 } else { 0xf },
        }
    }
    /// The swizzle (sources): source component for each of x, y, z, w.
    pub fn swizzle(&self) -> [u8; 4] {
        match self.sel {
            Sel::Swizzle(s) => s,
            Sel::Scalar(c) => [c; 4],
            Sel::Mask(_) | Sel::None => [0, 1, 2, 3],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Inst {
    pub op: Opcode,
    /// the opcode's number (bits 0-10 of its token: what messages show; any number for Unknown)
    pub code: u32,
    /// the opcode token's bits 11-23 (opcode-specific controls)
    pub ctl: u32,
    pub sat: bool,
    /// for if/breakc/continuec/retc/discard: true when the test is "nonzero"
    pub nz: bool,
    /// immediate texel offsets (aoffimmi)
    pub offset: [i32; 3],
    pub ops: Vec<Operand>,
    /// declaration words after the operands (counts, return types, names...)
    pub extra: Vec<u32>,
    /// immediate constant buffer data
    pub data: Vec<u32>,
}

pub struct Program {
    pub kind: ProgramType,
    pub major: u32,
    pub insts: Vec<Inst>,
}

/// How the decoder reads an instruction's words.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    /// an instruction with this many operands (a program that gives another count is rejected)
    Ops(usize),
    /// a declaration: this many operands, then words of its own (Inst::extra)
    Decl(usize),
    /// OP_CUSTOMDATA: a length, then data (Inst::data)
    Data,
    /// not checked: operands up to the instruction's end (the hull, geometry, double and atomic
    /// instructions wgsl.rs does not translate, and unknown opcodes)
    Unchecked,
}

macro_rules! opcodes {
    ($($name:ident = $code:literal => $shape:ident $(($n:literal))?,)*) => {
        /// An instruction's opcode (D3D10_SB_OPCODE_TYPE and D3D11_SB_OPCODE_TYPE,
        /// d3d11TokenizedProgramFormat.hpp): a match on it names every one, so a new opcode is
        /// a compile error wherever instructions are told apart.
        #[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
        pub enum Opcode {
            $($name,)*
            /// none of those (Inst::code has the number)
            Unknown,
        }

        impl Opcode {
            pub fn from_code(c: u32) -> Opcode {
                match c {
                    $($code => Opcode::$name,)*
                    _ => Opcode::Unknown,
                }
            }

            /// How its words are read (the operand counts: d3d11TokenizedProgramFormat.hpp)
            pub fn shape(self) -> Shape {
                match self {
                    $(Opcode::$name => Shape::$shape $(($n))?,)*
                    Opcode::Unknown => Shape::Unchecked,
                }
            }
        }
    };
}

opcodes! {
    Add = 0x00 => Ops(3),
    And = 0x01 => Ops(3),
    Break = 0x02 => Ops(0),
    Breakc = 0x03 => Ops(1),
    Call = 0x04 => Ops(1),
    Callc = 0x05 => Ops(2),
    Case = 0x06 => Ops(1),
    Continue = 0x07 => Ops(0),
    Continuec = 0x08 => Ops(1),
    Cut = 0x09 => Ops(0),
    Default = 0x0a => Ops(0),
    DerivRtx = 0x0b => Ops(2),
    DerivRty = 0x0c => Ops(2),
    Discard = 0x0d => Ops(1),
    Div = 0x0e => Ops(3),
    Dp2 = 0x0f => Ops(3),
    Dp3 = 0x10 => Ops(3),
    Dp4 = 0x11 => Ops(3),
    Else = 0x12 => Ops(0),
    Emit = 0x13 => Ops(0),
    EmitThenCut = 0x14 => Ops(0),
    EndIf = 0x15 => Ops(0),
    EndLoop = 0x16 => Ops(0),
    EndSwitch = 0x17 => Ops(0),
    Eq = 0x18 => Ops(3),
    Exp = 0x19 => Ops(2),
    Frc = 0x1a => Ops(2),
    Ftoi = 0x1b => Ops(2),
    Ftou = 0x1c => Ops(2),
    Ge = 0x1d => Ops(3),
    Iadd = 0x1e => Ops(3),
    If = 0x1f => Ops(1),
    Ieq = 0x20 => Ops(3),
    Ige = 0x21 => Ops(3),
    Ilt = 0x22 => Ops(3),
    Imad = 0x23 => Ops(4),
    Imax = 0x24 => Ops(3),
    Imin = 0x25 => Ops(3),
    Imul = 0x26 => Ops(4),
    Ine = 0x27 => Ops(3),
    Ineg = 0x28 => Ops(2),
    Ishl = 0x29 => Ops(3),
    Ishr = 0x2a => Ops(3),
    Itof = 0x2b => Ops(2),
    Label = 0x2c => Ops(1),
    Ld = 0x2d => Ops(3),
    LdMs = 0x2e => Ops(4),
    Log = 0x2f => Ops(2),
    Loop = 0x30 => Ops(0),
    Lt = 0x31 => Ops(3),
    Mad = 0x32 => Ops(4),
    Min = 0x33 => Ops(3),
    Max = 0x34 => Ops(3),
    CustomData = 0x35 => Data,
    Mov = 0x36 => Ops(2),
    Movc = 0x37 => Ops(4),
    Mul = 0x38 => Ops(3),
    Ne = 0x39 => Ops(3),
    Nop = 0x3a => Ops(0),
    Not = 0x3b => Ops(2),
    Or = 0x3c => Ops(3),
    ResInfo = 0x3d => Ops(3),
    Ret = 0x3e => Ops(0),
    Retc = 0x3f => Ops(1),
    RoundNe = 0x40 => Ops(2),
    RoundNi = 0x41 => Ops(2),
    RoundPi = 0x42 => Ops(2),
    RoundZ = 0x43 => Ops(2),
    Rsq = 0x44 => Ops(2),
    Sample = 0x45 => Ops(4),
    SampleC = 0x46 => Ops(5),
    SampleCLz = 0x47 => Ops(5),
    SampleL = 0x48 => Ops(5),
    SampleD = 0x49 => Ops(6),
    SampleB = 0x4a => Ops(5),
    Sqrt = 0x4b => Ops(2),
    Switch = 0x4c => Ops(1),
    SinCos = 0x4d => Ops(3),
    Udiv = 0x4e => Ops(4),
    Ult = 0x4f => Ops(3),
    Uge = 0x50 => Ops(3),
    Umul = 0x51 => Ops(4),
    Umad = 0x52 => Ops(4),
    Umax = 0x53 => Ops(3),
    Umin = 0x54 => Ops(3),
    Ushr = 0x55 => Ops(3),
    Utof = 0x56 => Ops(2),
    Xor = 0x57 => Ops(3),
    DclResource = 0x58 => Decl(1),
    DclConstantBuffer = 0x59 => Decl(1),
    DclSampler = 0x5a => Decl(1),
    DclIndexRange = 0x5b => Decl(1),
    DclGsOutputPrimitiveTopology = 0x5c => Decl(0),
    DclGsInputPrimitive = 0x5d => Decl(0),
    DclMaxOutputVertexCount = 0x5e => Decl(0),
    DclInput = 0x5f => Decl(1),
    DclInputSgv = 0x60 => Decl(1),
    DclInputSiv = 0x61 => Decl(1),
    DclInputPs = 0x62 => Decl(1),
    DclInputPsSgv = 0x63 => Decl(1),
    DclInputPsSiv = 0x64 => Decl(1),
    DclOutput = 0x65 => Decl(1),
    DclOutputSgv = 0x66 => Decl(1),
    DclOutputSiv = 0x67 => Decl(1),
    DclTemps = 0x68 => Decl(0),
    DclIndexableTemp = 0x69 => Decl(0),
    DclGlobalFlags = 0x6a => Decl(0),
    Reserved10 = 0x6b => Unchecked,
    Lod = 0x6c => Ops(4),
    Gather4 = 0x6d => Ops(4),
    SamplePos = 0x6e => Ops(3),
    SampleInfo = 0x6f => Ops(2),
    Reserved101 = 0x70 => Unchecked,
    HsDecls = 0x71 => Unchecked,
    HsControlPointPhase = 0x72 => Unchecked,
    HsForkPhase = 0x73 => Unchecked,
    HsJoinPhase = 0x74 => Unchecked,
    EmitStream = 0x75 => Unchecked,
    CutStream = 0x76 => Unchecked,
    EmitThenCutStream = 0x77 => Unchecked,
    InterfaceCall = 0x78 => Unchecked,
    BufInfo = 0x79 => Ops(2),
    DerivRtxCoarse = 0x7a => Ops(2),
    DerivRtxFine = 0x7b => Ops(2),
    DerivRtyCoarse = 0x7c => Ops(2),
    DerivRtyFine = 0x7d => Ops(2),
    Gather4C = 0x7e => Ops(5),
    Gather4Po = 0x7f => Ops(5),
    Gather4PoC = 0x80 => Ops(6),
    Rcp = 0x81 => Ops(2),
    F32ToF16 = 0x82 => Ops(2),
    F16ToF32 = 0x83 => Ops(2),
    Uaddc = 0x84 => Ops(4),
    Usubb = 0x85 => Ops(4),
    CountBits = 0x86 => Ops(2),
    FirstBitHi = 0x87 => Ops(2),
    FirstBitLo = 0x88 => Ops(2),
    FirstBitShi = 0x89 => Ops(2),
    Ubfe = 0x8a => Ops(4),
    Ibfe = 0x8b => Ops(4),
    Bfi = 0x8c => Ops(5),
    Bfrev = 0x8d => Ops(2),
    Swapc = 0x8e => Ops(5),
    DclStream = 0x8f => Decl(0),
    DclFunctionBody = 0x90 => Decl(0),
    DclFunctionTable = 0x91 => Decl(0),
    DclInterface = 0x92 => Decl(0),
    DclInputControlPointCount = 0x93 => Decl(0),
    DclOutputControlPointCount = 0x94 => Decl(0),
    DclTessDomain = 0x95 => Decl(0),
    DclTessPartitioning = 0x96 => Decl(0),
    DclTessOutputPrimitive = 0x97 => Decl(0),
    DclHsMaxTessFactor = 0x98 => Decl(0),
    DclHsForkPhaseInstanceCount = 0x99 => Decl(0),
    DclHsJoinPhaseInstanceCount = 0x9a => Decl(0),
    DclThreadGroup = 0x9b => Decl(0),
    DclUavTyped = 0x9c => Decl(1),
    DclUavRaw = 0x9d => Decl(1),
    DclUavStructured = 0x9e => Decl(1),
    DclTgsmRaw = 0x9f => Decl(1),
    DclTgsmStructured = 0xa0 => Decl(1),
    DclResourceRaw = 0xa1 => Decl(1),
    DclResourceStructured = 0xa2 => Decl(1),
    LdUavTyped = 0xa3 => Ops(3),
    StoreUavTyped = 0xa4 => Ops(3),
    LdRaw = 0xa5 => Ops(3),
    StoreRaw = 0xa6 => Ops(3),
    LdStructured = 0xa7 => Ops(4),
    StoreStructured = 0xa8 => Ops(4),
    AtomicAnd = 0xa9 => Unchecked,
    AtomicOr = 0xaa => Unchecked,
    AtomicXor = 0xab => Unchecked,
    AtomicCmpStore = 0xac => Unchecked,
    AtomicIadd = 0xad => Unchecked,
    AtomicImax = 0xae => Unchecked,
    AtomicImin = 0xaf => Unchecked,
    AtomicUmax = 0xb0 => Unchecked,
    AtomicUmin = 0xb1 => Unchecked,
    ImmAtomicAlloc = 0xb2 => Unchecked,
    ImmAtomicConsume = 0xb3 => Unchecked,
    ImmAtomicIadd = 0xb4 => Unchecked,
    ImmAtomicAnd = 0xb5 => Unchecked,
    ImmAtomicOr = 0xb6 => Unchecked,
    ImmAtomicXor = 0xb7 => Unchecked,
    ImmAtomicExch = 0xb8 => Unchecked,
    ImmAtomicCmpExch = 0xb9 => Unchecked,
    ImmAtomicImax = 0xba => Unchecked,
    ImmAtomicImin = 0xbb => Unchecked,
    ImmAtomicUmax = 0xbc => Unchecked,
    ImmAtomicUmin = 0xbd => Unchecked,
    Sync = 0xbe => Ops(0),
    Dadd = 0xbf => Unchecked,
    Dmax = 0xc0 => Unchecked,
    Dmin = 0xc1 => Unchecked,
    Dmul = 0xc2 => Unchecked,
    Deq = 0xc3 => Unchecked,
    Dge = 0xc4 => Unchecked,
    Dlt = 0xc5 => Unchecked,
    Dne = 0xc6 => Unchecked,
    Dmov = 0xc7 => Unchecked,
    Dmovc = 0xc8 => Unchecked,
    Dtof = 0xc9 => Unchecked,
    Ftod = 0xca => Unchecked,
    EvalSnapped = 0xcb => Unchecked,
    EvalSampleIndex = 0xcc => Unchecked,
    EvalCentroid = 0xcd => Unchecked,
    DclGsInstanceCount = 0xce => Unchecked,
    Abort = 0xcf => Unchecked,
    DebugBreak = 0xd0 => Unchecked,
    Reserved11 = 0xd1 => Unchecked,
    Ddiv = 0xd2 => Unchecked,
    Dfma = 0xd3 => Unchecked,
    Drcp = 0xd4 => Unchecked,
    Msad = 0xd5 => Unchecked,
    Dtoi = 0xd6 => Unchecked,
    Dtou = 0xd7 => Unchecked,
    Itod = 0xd8 => Unchecked,
    Utod = 0xd9 => Unchecked,
}

/// The operand count of each instruction (declarations and the hull/geometry shader
/// instructions left out): a program that gives another count is rejected.
pub fn arity(op: Opcode) -> Option<usize> {
    match op.shape() {
        Shape::Ops(n) => Some(n),
        Shape::Decl(_) | Shape::Data | Shape::Unchecked => None,
    }
}

struct Reader<'a> {
    w: &'a [u32],
    p: usize,
    end: usize,
}

impl Reader<'_> {
    fn next(&mut self) -> Result<u32, String> {
        if self.p >= self.end {
            return Err("truncated instruction".into());
        }
        self.p += 1;
        Ok(self.w[self.p - 1])
    }

    fn operand(&mut self) -> Result<Operand, String> {
        let t = self.next()?;
        let num = t & 3;
        let comps: u8 = match num {
            0 => 0,
            1 => 1,
            _ => 4,
        };
        let sel = if num == 2 {
            match (t >> 2) & 3 {
                0 => Sel::Mask(crate::lo8((t >> 4) & 0xf)),
                1 => Sel::Swizzle([4, 6, 8, 10].map(|sh| crate::lo8((t >> sh) & 3))),
                _ => Sel::Scalar(crate::lo8((t >> 4) & 3)),
            }
        } else {
            Sel::None
        };
        if num == 3 {
            return Err("N-component operand".into());
        }
        let ty = OpType((t >> 12) & 0xff);
        let dims = (t >> 20) & 3;
        let mut modifier = 0;
        if t >> 31 != 0 {
            let mut e = self.next()?;
            loop {
                if e & 0x3f == 1 {
                    modifier = crate::lo8(e >> 6);
                }
                if e >> 31 == 0 {
                    break;
                }
                e = self.next()?;
            }
        }
        let mut imm = [0u32; 4];
        match ty {
            IMM32 => {
                for v in imm.iter_mut().take(if comps == 4 { 4 } else { 1 }) {
                    *v = self.next()?;
                }
                if comps == 1 {
                    imm = [imm[0]; 4];
                }
            }
            IMM64 => return Err("64-bit immediates".into()),
            _ => {}
        }
        let mut idx = Vec::new();
        for i in 0..dims {
            let rep = (t >> (22 + 3 * i)) & 7;
            idx.push(match rep {
                0 => Index::Imm(self.next()?),
                1 => {
                    let lo = self.next()?;
                    self.next()?;
                    Index::Imm(lo)
                }
                2 => Index::Rel(Box::new(self.operand()?), 0),
                3 => {
                    let off = self.next()?;
                    Index::Rel(Box::new(self.operand()?), off)
                }
                _ => return Err(format!("index representation {rep}")),
            });
        }
        Ok(Operand { ty, comps, sel, idx, imm, modifier })
    }
}

/// Decode SHDR/SHEX.
pub fn decode(chunk: &[u8]) -> Result<Program, String> {
    let w: Vec<u32> = chunk.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if w.len() < 2 {
        return Err("empty shader".into());
    }
    let kind = ProgramType(w[0] >> 16);
    let major = (w[0] >> 4) & 0xf;
    let len = crate::us(w[1]).min(w.len());
    let mut insts = Vec::new();
    let mut p = 2;
    while p < len {
        let t = w[p];
        let code = t & 0x7ff;
        let op = Opcode::from_code(code);
        if op.shape() == Shape::Data {
            let n = crate::us(*w.get(p + 1).ok_or("truncated customdata")?);
            // (checked: on wasm32 `p + n` with n near 2^32 wrapped below `len` and the slice
            // below panicked, the VM gone, review A-7)
            if n < 2 || p.checked_add(n).is_none_or(|e| e > len) {
                return Err("bad customdata".into());
            }
            insts.push(Inst {
                op,
                code,
                ctl: t >> 11,
                sat: false,
                nz: false,
                offset: [0; 3],
                ops: Vec::new(),
                extra: Vec::new(),
                data: w[p + 2..p + n].to_vec(),
            });
            p += n;
            continue;
        }
        // (bits 24..30: the length in dwords)
        let n = crate::us((t >> 24) & 0x7f);
        if n == 0 || p + n > len {
            return Err(format!("bad instruction length at {p}"));
        }
        let mut r = Reader { w: &w, p: p + 1, end: p + n };
        let mut inst = Inst {
            op,
            code,
            ctl: (t >> 11) & 0x1fff,
            sat: (t >> 13) & 1 != 0,
            nz: (t >> 18) & 1 != 0,
            offset: [0; 3],
            ops: Vec::new(),
            extra: Vec::new(),
            data: Vec::new(),
        };
        if t >> 31 != 0 {
            loop {
                let e = r.next()?;
                if e & 0x3f == 1 {
                    let s4 = |v: u32| (v.cast_signed() << 28) >> 28;
                    inst.offset = [s4(e >> 9), s4(e >> 13), s4(e >> 17)];
                }
                if e >> 31 == 0 {
                    break;
                }
            }
        }
        // declarations that are not just operands
        let (operands, words) = match op.shape() {
            Shape::Decl(n) => (n, true),
            Shape::Ops(_) | Shape::Data | Shape::Unchecked => (usize::MAX, false),
        };
        let mut k = 0;
        let mut last = r.p;
        while r.p < r.end && k < operands {
            last = r.p;
            inst.ops.push(r.operand()?);
            k += 1;
        }
        // fxc ends samplepos with a zero word past its three operands (its length counts it)
        if op == Opcode::SamplePos && inst.ops.len() == 4 && r.p == r.end && r.end - last == 1 && r.w[last] == 0 {
            inst.ops.pop();
        }
        if words {
            while r.p < r.end {
                inst.extra.push(r.next()?);
            }
        }
        if operands == 1 && inst.ops.len() != 1 {
            return Err(format!("declaration {code:#x} at {p} without its operand"));
        }
        if let Some(k) = arity(op) {
            if inst.ops.len() != k || r.p != r.end {
                return Err(format!("instruction {code:#x} at {p}: {} operands, {k} wanted", inst.ops.len()));
            }
        }
        p += n;
        insts.push(inst);
    }
    Ok(Program { kind, major, insts })
}

#[cfg(test)]
mod tests;
