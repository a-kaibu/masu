//! A small SSA of x86 integer instructions (koji redesign, stage 3), on the current ABI.
//!
//! A run of a block's register-only integer instructions (MOV, LEA, ADD, SUB, AND, OR, XOR,
//! CMP, TEST, INC, DEC, NOT, NEG, SHL/SHR/SAR by an immediate; 32-bit registers and
//! immediates, ESP not written) becomes values: each register a node, the status flags the
//! last flag writer's lazy record (its operands nodes too). Constants fold, `x ^ x` and
//! `x - x` are 0. What it lowers to is what the instructions one by one leave (Gen::emit): the
//! changed registers in their locals, the lazy record in L_OP/L_FA.. where the flags are live
//! after the run (an INC/DEC's carry its CF from the record before it, in the run), and
//! nothing else. The run has no exit inside (no memory, no ESP write, no instruction the
//! interpreter runs), so the exit contract holds at its two ends as for the instructions one
//! by one; its end state is the same (jit32test with MASU_JIT_SSA on and off).
//!
//! Its gain is the code's size: a node is computed once, where the generator moved every
//! operand through a temporary and wrote each lazy record.

use super::{Flags, Gen, Kind, Op, Size, L_FA, L_FB, L_FC, L_FR, L_OP, L_REG, N_T, STATUS_BITS};
use iced_x86::{FlowControl, Instruction, Mnemonic, OpKind, Register, RflagsBits};
use kome::num::*;
use alloc::vec::Vec;

/// A node's index in [`Run::nodes`]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Id(usize);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Bin {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Sar,
    /// 1 if a < b (unsigned): a carry out of ADD (r < a), a borrow of SUB (a < b)
    LtU,
}

impl Bin {
    fn fold(self, a: u32, b: u32) -> u32 {
        match self {
            Bin::Add => a.wrapping_add(b),
            Bin::Sub => a.wrapping_sub(b),
            Bin::And => a & b,
            Bin::Or => a | b,
            Bin::Xor => a ^ b,
            Bin::Shl => a << (b & 31),
            Bin::Shr => a >> (b & 31),
            Bin::Sar => (a.cast_signed() >> (b & 31)).cast_unsigned(),
            Bin::LtU => u32::from(a < b),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Node {
    /// a register's value at the run's start
    Entry(u32),
    Const(u32),
    Bin(Bin, Id, Id),
}

/// The lazy record of the last flag writer (Gen::set_lazy's)
#[derive(Clone, Copy, Debug)]
struct Rec {
    kind: Kind,
    a: Id,
    /// (INC/DEC: L_FB is left as it was)
    b: Option<Id>,
    r: Id,
    /// INC/DEC: CF before it
    c: Option<Id>,
}

/// What a run's instruction does ([`fits`] says, [`Run::add`] lowers: one match each, no other
/// instruction gets in)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Mov,
    Lea,
    /// ADD SUB AND OR XOR, CMP TEST (`writes` false)
    Alu { op: Bin, kind: Kind, writes: bool },
    /// INC (Add) / DEC (Sub)
    IncDec { op: Bin, kind: Kind },
    Not,
    Neg,
    /// by an immediate (non-zero mod 32)
    Shift { op: Bin, kind: Kind },
}

struct Run {
    nodes: Vec<Node>,
    regs: [Id; 8],
    rec: Option<Rec>,
}

impl Run {
    fn new() -> Run {
        let nodes: Vec<Node> = (0..8u32).map(Node::Entry).collect();
        Run { nodes, regs: core::array::from_fn(Id), rec: None }
    }

    fn node(&mut self, n: Node) -> Id {
        let n = match n {
            Node::Bin(op, a, b) => match (self.nodes[a.0], self.nodes[b.0]) {
                (Node::Const(x), Node::Const(y)) => Node::Const(op.fold(x, y)),
                _ if a == b && matches!(op, Bin::Xor | Bin::Sub) => Node::Const(0),
                _ => n,
            },
            Node::Entry(_) | Node::Const(_) => n,
        };
        if let Some(i) = self.nodes.iter().position(|&m| m == n) {
            return Id(i);
        }
        self.nodes.push(n);
        Id(self.nodes.len() - 1)
    }

    fn bin(&mut self, op: Bin, a: Id, b: Id) -> Id {
        self.node(Node::Bin(op, a, b))
    }

    /// Operand `i` of `ins` (a 32-bit register or an immediate)
    fn operand(&mut self, ins: &Instruction, i: u32) -> Id {
        match ins.op_kind(i) {
            OpKind::Register => self.regs[ins.op_register(i).number()],
            _ => self.node(Node::Const(lo32(ins.immediate(i)))),
        }
    }

    /// CF of the record so far (None: before the run, or not a value here)
    fn carry(&mut self) -> Option<Id> {
        let r = self.rec?;
        Some(match r.kind {
            Kind::Add => self.bin(Bin::LtU, r.r, r.a),
            Kind::Sub => self.bin(Bin::LtU, r.a, r.b?),
            Kind::Logic => self.node(Node::Const(0)),
            Kind::Inc | Kind::Dec => {
                let one = self.node(Node::Const(1));
                self.bin(Bin::And, r.c?, one)
            }
            Kind::Shl | Kind::Shr | Kind::Sar | Kind::Adc | Kind::Sbb | Kind::Mul => return None,
        })
    }

    /// Adds `ins` (one [`fits`] took): false where it cannot be added (an INC/DEC whose CF is
    /// read after it, the record before it not a value here)
    fn add(&mut self, ins: &Instruction, step: Step, cf_read: bool) -> bool {
        let d = ins.op0_register().number();
        let a = self.regs[d];
        match step {
            Step::Mov => self.regs[d] = self.operand(ins, 1),
            Step::Lea => {
                let mut v = self.node(Node::Const(ins.memory_displacement32()));
                if ins.memory_base() != Register::None {
                    let b = self.regs[ins.memory_base().number()];
                    v = self.bin(Bin::Add, b, v);
                }
                if ins.memory_index() != Register::None {
                    let x = self.regs[ins.memory_index().number()];
                    let s = self.node(Node::Const(ins.memory_index_scale().trailing_zeros()));
                    let x = self.bin(Bin::Shl, x, s);
                    v = self.bin(Bin::Add, v, x);
                }
                self.regs[d] = v;
            }
            Step::Alu { op, kind, writes } => {
                let b = self.operand(ins, 1);
                let r = self.bin(op, a, b);
                if writes {
                    self.regs[d] = r;
                }
                self.rec = Some(Rec { kind, a, b: Some(b), r, c: None });
            }
            Step::IncDec { op, kind } => {
                let c = if cf_read {
                    match self.carry() {
                        Some(c) => Some(c),
                        None => return false,
                    }
                } else {
                    None
                };
                let one = self.node(Node::Const(1));
                let r = self.bin(op, a, one);
                self.regs[d] = r;
                self.rec = Some(Rec { kind, a, b: None, r, c });
            }
            Step::Not => {
                let m = self.node(Node::Const(u32::MAX));
                self.regs[d] = self.bin(Bin::Xor, a, m);
            }
            Step::Neg => {
                let z = self.node(Node::Const(0));
                let r = self.bin(Bin::Sub, z, a);
                self.regs[d] = r;
                self.rec = Some(Rec { kind: Kind::Sub, a: z, b: Some(a), r, c: None });
            }
            Step::Shift { op, kind } => {
                let n = self.node(Node::Const(lo32(ins.immediate(1)) & 31));
                let r = self.bin(op, a, n);
                self.regs[d] = r;
                self.rec = Some(Rec { kind, a, b: Some(n), r, c: None });
            }
        }
        true
    }

    /// The nodes the lowering computes into temporaries (in order) and the roots: the changed
    /// registers, the record where it is kept
    fn plan(&self, keep_rec: bool) -> (Vec<usize>, Vec<(u32, Id)>) {
        let mut uses = alloc::vec![0u32; self.nodes.len()];
        let mut roots: Vec<Id> = Vec::new();
        let changed: Vec<(u32, Id)> = (0..8u32).filter(|&r| self.regs[us(r)] != Id(us(r))).map(|r| (r, self.regs[us(r)])).collect();
        roots.extend(changed.iter().map(|c| c.1));
        if keep_rec && let Some(r) = self.rec {
            roots.extend([Some(r.a), r.b, Some(r.r), r.c].into_iter().flatten());
        }
        let mut stack = roots.clone();
        let mut seen = alloc::vec![false; self.nodes.len()];
        while let Some(Id(i)) = stack.pop() {
            uses[i] += 1;
            if core::mem::replace(&mut seen[i], true) {
                continue;
            }
            if let Node::Bin(_, a, b) = self.nodes[i] {
                stack.extend([a, b]);
            }
        }
        for r in &roots {
            // (a root is kept: the registers are set after every root is computed)
            uses[r.0] += 1;
        }
        let temps = (0..self.nodes.len()).filter(|&i| uses[i] > 1 && matches!(self.nodes[i], Node::Bin(..))).collect();
        (temps, changed)
    }
}

/// What `ins` does in a run, where it may be in one (its operands, its effects; [`Run::add`]
/// decides the rest)
fn fits(ins: &Instruction) -> Option<Step> {
    use Mnemonic::*;
    let gpr = |r: Register| r.is_gpr32();
    if ins.flow_control() != FlowControl::Next || ins.has_lock_prefix() || super::code16(ins) || ins.op_count() == 0 || ins.op0_kind() != OpKind::Register {
        return None;
    }
    let d = ins.op0_register();
    if !gpr(d) || d == Register::ESP {
        return None;
    }
    let src = ins.op_count() == 2
        && match ins.op_kind(1) {
            OpKind::Register => gpr(ins.op_register(1)),
            OpKind::Immediate8to32 | OpKind::Immediate32 => true,
            _ => false,
        };
    let one = ins.op_count() == 1;
    let alu = |op, kind, writes| src.then_some(Step::Alu { op, kind, writes });
    let shift = |op, kind| (ins.op_count() == 2 && ins.op_kind(1) == OpKind::Immediate8 && ins.immediate(1) & 31 != 0).then_some(Step::Shift { op, kind });
    match ins.mnemonic() {
        Mov => src.then_some(Step::Mov),
        Add => alu(Bin::Add, Kind::Add, true),
        Sub => alu(Bin::Sub, Kind::Sub, true),
        And => alu(Bin::And, Kind::Logic, true),
        Or => alu(Bin::Or, Kind::Logic, true),
        Xor => alu(Bin::Xor, Kind::Logic, true),
        Cmp => alu(Bin::Sub, Kind::Sub, false),
        Test => alu(Bin::And, Kind::Logic, false),
        Inc => one.then_some(Step::IncDec { op: Bin::Add, kind: Kind::Inc }),
        Dec => one.then_some(Step::IncDec { op: Bin::Sub, kind: Kind::Dec }),
        Not => one.then_some(Step::Not),
        Neg => one.then_some(Step::Neg),
        Shl | Sal => shift(Bin::Shl, Kind::Shl),
        Shr => shift(Bin::Shr, Kind::Shr),
        Sar => shift(Bin::Sar, Kind::Sar),
        Lea => {
            let ok = |r: Register| r == Register::None || gpr(r);
            (ins.op_count() == 2 && ins.op_kind(1) == OpKind::Memory && ok(ins.memory_base()) && ok(ins.memory_index())).then_some(Step::Lea)
        }
        _ => None,
    }
}

/// Runs longer than this are cut (the plan's search is quadratic)
const MAX_RUN: usize = 32;

/// The run at `list[i..]`: its length (0: none here, fewer than two instructions), its SSA
/// and whether its record is kept (`live`: the status flags live after each instruction)
fn find(list: &[Instruction], i: usize, live: &[u32], probed: impl Fn(u32) -> bool) -> Option<(usize, Run, bool)> {
    let last = list.len() - 1;
    let mut run = Run::new();
    let mut best: Option<(usize, bool)> = None;
    let mut j = i;
    // (not the first of a pair Gen fuses into a MOVZX: set_movzx)
    let mut steps = Vec::new();
    while j < last && j - i < MAX_RUN && !probed(list[j].ip32()) && !super::set_movzx(&list[j], &list[j + 1]) {
        let Some(step) = fits(&list[j]) else { break };
        let cf_read = live[j] & RflagsBits::CF != 0;
        let mut next = Run { nodes: run.nodes.clone(), regs: run.regs, rec: run.rec };
        if !next.add(&list[j], step, cf_read) {
            break;
        }
        let keep = live[j] & STATUS_BITS != 0;
        let (temps, changed) = next.plan(keep);
        let rec_temps = if keep { next.rec.map_or(0, |r| 2 + usize::from(r.b.is_some()) + usize::from(r.c.is_some())) } else { 0 };
        if temps.len() + changed.len() + rec_temps > us(N_T) {
            break;
        }
        run = next;
        steps.push(step);
        j += 1;
        if j - i >= 2 {
            best = Some((j - i, keep));
        }
    }
    let (n, keep) = best?;
    // (the run as found up to its best end)
    let mut cut = Run::new();
    for (k, &step) in steps.iter().enumerate().take(n) {
        // (each took before: it takes again)
        cut.add(&list[i + k], step, live[i + k] & RflagsBits::CF != 0);
    }
    Some((n, cut, keep))
}

/// A run found at an instruction (see [`Found::at`])
pub(super) struct Found {
    /// how many instructions it takes
    pub(super) n: usize,
    run: Run,
    keep: bool,
}

impl Found {
    /// The register-only run at `list[i]` (two instructions at least; see the module's doc).
    /// `live`: the status flags live after each instruction.
    pub(super) fn at(g: &Gen<'_>, list: &[Instruction], i: usize, live: &[u32]) -> Option<Found> {
        let measure = g.tc.measure;
        let (n, run, keep) = find(list, i, live, |ip| measure.probed(ip))?;
        Some(Found { n, run, keep })
    }
}

/// Emits run `f` (its instructions' code: see the module's doc)
pub(super) fn emit(g: &mut Gen<'_>, f: Found) {
    let Found { n: _, run, keep } = f;
    g.temps = 0;
    let (temps, changed) = run.plan(keep);
    let mut at: Vec<Option<u32>> = alloc::vec![None; run.nodes.len()];
    for &k in &temps {
        push(g, &run, &at, Id(k));
        let t = g.temp();
        g.c.set(t);
        at[k] = Some(t);
    }
    // the roots, all before any register is set (they read the registers at the run's start)
    let root = |g: &mut Gen<'_>, at: &mut Vec<Option<u32>>, id: Id| -> Option<u32> {
        match run.nodes[id.0] {
            Node::Const(_) => None,
            Node::Entry(_) | Node::Bin(..) if at[id.0].is_some() => at[id.0],
            Node::Entry(_) | Node::Bin(..) => {
                push(g, &run, at, id);
                let t = g.temp();
                g.c.set(t);
                at[id.0] = Some(t);
                Some(t)
            }
        }
    };
    let rec = if keep { run.rec } else { None };
    if let Some(r) = rec {
        for id in [Some(r.a), r.b, Some(r.r), r.c].into_iter().flatten() {
            root(g, &mut at, id);
        }
    }
    for &(_, id) in &changed {
        root(g, &mut at, id);
    }
    for &(reg, id) in &changed {
        push(g, &run, &at, id);
        g.c.set(L_REG + reg);
    }
    if let Some(r) = rec {
        let op = Op::new(r.kind, Size::of_bits32(32));
        g.c.u32c(op.raw()).set(L_OP);
        for (l, id) in [(L_FA, Some(r.a)), (L_FB, r.b), (L_FR, Some(r.r)), (L_FC, r.c)] {
            if let Some(id) = id {
                push(g, &run, &at, id);
                g.c.set(l);
            }
        }
        g.flags = Flags::Known(op);
    }
    // what Gen::emit keeps of the registers written
    let written: u32 = changed.iter().fold(0, |m, &(r, _)| m | 1 << r);
    for r in 0..8 {
        if written & (1 << r) != 0 {
            g.safe[r] = None;
        }
    }
    g.safe2.retain(|&(b, x, _)| written & (1 << b | 1 << x) == 0);
    g.temps = 0;
}

/// Pushes node `id`: its temporary, or its expression
fn push(g: &mut Gen<'_>, run: &Run, at: &[Option<u32>], id: Id) {
    if let Some(t) = at[id.0] {
        g.c.get(t);
        return;
    }
    match run.nodes[id.0] {
        Node::Entry(r) => {
            g.c.get(L_REG + r);
        }
        Node::Const(v) => {
            g.c.u32c(v);
        }
        Node::Bin(op, a, b) => {
            push(g, run, at, a);
            push(g, run, at, b);
            match op {
                Bin::Add => g.c.add(),
                Bin::Sub => g.c.sub(),
                Bin::And => g.c.and(),
                Bin::Or => g.c.or(),
                Bin::Xor => g.c.xor(),
                Bin::Shl => g.c.shl(),
                Bin::Shr => g.c.shr_u(),
                Bin::Sar => g.c.shr_s(),
                Bin::LtU => g.c.lt_u(),
            };
        }
    }
}
