//! The generated wasm run on terms: every path of its function through to its return, each
//! with the condition it takes and the memory it leaves. Memory is bytes at known addresses
//! (a store or load at an address that is not known ends the path as opaque, and the case
//! undecided); a call to an import (the interpreter's `step`, the flags' `eflags`, ...) ends it
//! as opaque too: what runs there is not this wasm.

use crate::expr::{self as x, E};
use std::collections::BTreeMap;
use wasmparser::{BlockType, Operator, ValType};

#[derive(Clone)]
struct Label {
    looping: bool,
    /// the op index of its Block/Loop/If and of its End
    start: usize,
    end: usize,
    /// values a branch to it carries
    arity: usize,
    height: usize,
}

#[derive(Clone)]
struct State {
    pc: usize,
    stack: Vec<E>,
    locals: Vec<E>,
    labels: Vec<Label>,
    mem: BTreeMap<u32, E>,
    cond: Vec<E>,
}

/// How one path ended.
pub enum End {
    /// returned: the memory it wrote (bytes, by address) and its results
    Return { cond: Vec<E>, mem: BTreeMap<u32, E>, results: Vec<E> },
    /// `unreachable` or a load or store past memory: a trap
    Trap { cond: Vec<E>, why: String },
    /// something this executor does not follow (an import's call, an address not known)
    Opaque { cond: Vec<E>, why: String },
}

pub struct Func<'a> {
    pub nparams: usize,
    ops: Vec<Operator<'a>>,
    /// for each Block/Loop/If: its Else (if any) and End
    ends: Vec<(Option<usize>, usize)>,
    nlocals: usize,
    imports: Vec<String>,
    results: usize,
    types: Vec<(usize, usize)>,
}

fn arity(bt: &BlockType, types: &[(usize, usize)]) -> (usize, usize) {
    match bt {
        BlockType::Empty => (0, 0),
        BlockType::Type(_) => (0, 1),
        BlockType::FuncType(i) => types[*i as usize],
    }
}

/// The module's first defined function (a region's or a function's entry), its imports' names.
pub fn parse(bytes: &[u8]) -> Result<Func<'_>, String> {
    let mut types = Vec::new();
    let mut imports = Vec::new();
    let mut func_types = Vec::new();
    let mut body = None;
    let mut nparams = 0;
    let mut results = 0;
    for p in wasmparser::Parser::new(0).parse_all(bytes) {
        match p.map_err(|e| e.to_string())? {
            wasmparser::Payload::TypeSection(s) => {
                for rg in s {
                    for t in rg.map_err(|e| e.to_string())?.into_types() {
                        let f = t.unwrap_func();
                        types.push((f.params().len(), f.results().len()));
                    }
                }
            }
            wasmparser::Payload::ImportSection(s) => {
                for i in s.into_imports() {
                    let i = i.map_err(|e| e.to_string())?;
                    if let wasmparser::TypeRef::Func(_) = i.ty {
                        imports.push(i.name.to_string());
                    }
                }
            }
            wasmparser::Payload::FunctionSection(s) => {
                for t in s {
                    func_types.push(t.map_err(|e| e.to_string())? as usize);
                }
            }
            wasmparser::Payload::CodeSectionEntry(b) if body.is_none() => {
                let (np, nr) = types[func_types[0]];
                nparams = np;
                results = nr;
                body = Some(b);
            }
            _ => {}
        }
    }
    let body = body.ok_or("no function")?;
    let mut nlocals = nparams;
    for l in body.get_locals_reader().map_err(|e| e.to_string())? {
        let (n, t) = l.map_err(|e| e.to_string())?;
        if !matches!(t, ValType::I32 | ValType::I64) {
            return Err(format!("a local of type {t:?}"));
        }
        nlocals += n as usize;
    }
    let mut ops = Vec::new();
    let mut r = body.get_operators_reader().map_err(|e| e.to_string())?;
    while !r.eof() {
        ops.push(r.read().map_err(|e| e.to_string())?);
    }
    let mut ends = vec![(None, 0); ops.len()];
    let mut open = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        match op {
            Operator::Block { .. } | Operator::Loop { .. } | Operator::If { .. } => open.push(i),
            Operator::Else => {
                let &o = open.last().ok_or("else outside if")?;
                ends[o].0 = Some(i);
            }
            Operator::End => {
                if let Some(o) = open.pop() {
                    ends[o].1 = i;
                }
            }
            _ => {}
        }
    }
    Ok(Func { nparams, ops, ends, nlocals, imports, results, types })
}

impl Func<'_> {
    /// Every path from `params` (the region's arguments) and `mem` (bytes known before),
    /// at most `max_paths` of them.
    pub fn run(&self, params: Vec<E>, mem: BTreeMap<u32, E>, max_paths: usize) -> Vec<End> {
        let mut locals = params;
        while locals.len() < self.nlocals {
            locals.push(x::c(32, 0));
        }
        let top = Label { looping: false, start: 0, end: self.ops.len() - 1, arity: self.results, height: 0 };
        let mut work = vec![State { pc: 0, stack: Vec::new(), locals, labels: vec![top], mem, cond: Vec::new() }];
        let mut out = Vec::new();
        while let Some(s) = work.pop() {
            if out.len() + work.len() > max_paths {
                out.push(End::Opaque { cond: s.cond, why: format!("more than {max_paths} paths") });
                continue;
            }
            self.step_all(s, &mut work, &mut out);
        }
        out
    }

    fn branch(&self, s: &mut State, depth: u32) -> bool {
        let at = s.labels.len() - 1 - depth as usize;
        let l = s.labels[at].clone();
        let carried: Vec<E> = s.stack.split_off(s.stack.len() - l.arity);
        s.stack.truncate(l.height);
        s.stack.extend(carried);
        if l.looping {
            s.labels.truncate(at + 1);
            s.pc = l.start + 1;
        } else {
            s.labels.truncate(at);
            s.pc = l.end + 1;
        }
        // (a branch out of the function's own block returns)
        s.labels.is_empty()
    }

    fn load(s: &State, addr: u32, n: u32) -> E {
        let byte = |a: u32| s.mem.get(&a).cloned().unwrap_or_else(|| x::c(8, 0));
        let mut v = byte(addr);
        for i in 1..n {
            v = x::concat(&byte(addr + i), &v);
        }
        v
    }

    fn store(s: &mut State, addr: u32, v: &E, n: u32) {
        for i in 0..n {
            s.mem.insert(addr + i, x::extract(v, 8 * i + 7, 8 * i));
        }
    }

    fn step_all(&self, mut s: State, work: &mut Vec<State>, out: &mut Vec<End>) {
        for _ in 0..200_000 {
            if s.pc >= self.ops.len() {
                let n = self.results;
                let results = s.stack.split_off(s.stack.len() - n);
                out.push(End::Return { cond: s.cond, mem: s.mem, results });
                return;
            }
            let pc = s.pc;
            s.pc += 1;
            macro_rules! pop { () => { match s.stack.pop() { Some(v) => v, None => { out.push(End::Opaque { cond: s.cond, why: "an operand stack the validator would not pass".into() }); return; } } }; }
            macro_rules! un { ($f:expr) => {{ let a = pop!(); let r = $f(&a); s.stack.push(r); }}; }
            macro_rules! bi { ($f:expr) => {{ let b = pop!(); let a = pop!(); let r = $f(&a, &b); s.stack.push(r); }}; }
            macro_rules! cmp { ($f:expr) => {{ let b = pop!(); let a = pop!(); let r: E = $f(&a, &b); s.stack.push(x::zext(&r, 32)); }}; }
            macro_rules! sh { ($op:expr) => {{ let b = pop!(); let a = pop!(); let k = x::and(&b, &x::c(b.w, u64::from(b.w) - 1)); s.stack.push(x::bin($op, &a, &k)); }}; }
            macro_rules! addr { ($m:expr, $n:expr) => {{
                let base = pop!();
                match x::konst(&base) {
                    Some(b) => {
                        let a = b + $m.offset;
                        if a + $n > 1u64 << 32 { out.push(End::Trap { cond: s.cond, why: format!("access at {a:#x}") }); return; }
                        a as u32
                    }
                    None => { out.push(End::Opaque { cond: s.cond, why: "an address not known".into() }); return; }
                }
            }}; }
            macro_rules! ld { ($m:expr, $n:expr, $w:expr, $signed:expr) => {{
                let a = addr!($m, $n);
                let v = Self::load(&s, a, $n);
                s.stack.push(if $signed { x::sext(&v, $w) } else { x::zext(&v, $w) });
            }}; }
            macro_rules! st { ($m:expr, $n:expr) => {{
                let v = pop!();
                let a = addr!($m, $n);
                let v = if v.w > 8 * $n { x::extract(&v, 8 * $n - 1, 0) } else { v };
                Self::store(&mut s, a, &v, $n);
            }}; }
            let rot = |a: &E, b: &E, left: bool| {
                let w = a.w;
                let k = x::and(b, &x::c(w, u64::from(w) - 1));
                let back = x::and(&x::sub(&x::c(w, u64::from(w)), &k), &x::c(w, u64::from(w) - 1));
                if left { x::or(&x::shl(a, &k), &x::lshr(a, &back)) } else { x::or(&x::lshr(a, &k), &x::shl(a, &back)) }
            };
            match &self.ops[pc] {
                Operator::Nop => {}
                Operator::Unreachable => {
                    out.push(End::Trap { cond: s.cond, why: "unreachable".into() });
                    return;
                }
                Operator::Block { blockty } | Operator::Loop { blockty } => {
                    let (p, r) = arity(blockty, &self.types);
                    let looping = matches!(self.ops[pc], Operator::Loop { .. });
                    s.labels.push(Label { looping, start: pc, end: self.ends[pc].1, arity: if looping { p } else { r }, height: s.stack.len() - p });
                }
                Operator::If { blockty } => {
                    let (p, r) = arity(blockty, &self.types);
                    let cnd = pop!();
                    let (els, end) = self.ends[pc];
                    let label = Label { looping: false, start: pc, end, arity: r, height: s.stack.len() - p };
                    let taken = x::nonzero(&cnd);
                    let mut other = None;
                    match x::konst(&taken) {
                        Some(1) => {}
                        Some(_) => {
                            match els { Some(e) => s.pc = e + 1, None => { s.pc = end + 1; continue; } }
                        }
                        None => {
                            let mut f = s.clone();
                            f.cond.push(x::bnot(&taken));
                            match els {
                                Some(e) => { f.pc = e + 1; f.labels.push(label.clone()); other = Some(f); }
                                None => { f.pc = end + 1; work.push(f); }
                            }
                            s.cond.push(taken);
                        }
                    }
                    if let Some(f) = other { work.push(f); }
                    s.labels.push(label);
                }
                Operator::Else => {
                    // (the then-arm's end: on past the else-arm)
                    let Some(l) = s.labels.pop() else {
                        out.push(End::Opaque { cond: s.cond, why: "an else without its if".into() });
                        return;
                    };
                    s.pc = l.end + 1;
                }
                Operator::End => {
                    if s.labels.pop().is_some() && s.labels.is_empty() {
                        s.pc = self.ops.len();
                    }
                }
                Operator::Br { relative_depth } => {
                    if self.branch(&mut s, *relative_depth) { s.pc = self.ops.len(); }
                }
                Operator::BrIf { relative_depth } => {
                    let cnd = x::nonzero(&pop!());
                    match x::konst(&cnd) {
                        Some(1) => { if self.branch(&mut s, *relative_depth) { s.pc = self.ops.len(); } }
                        Some(_) => {}
                        None => {
                            let mut t = s.clone();
                            t.cond.push(cnd.clone());
                            if self.branch(&mut t, *relative_depth) { t.pc = self.ops.len(); }
                            work.push(t);
                            s.cond.push(x::bnot(&cnd));
                        }
                    }
                }
                Operator::BrTable { targets } => {
                    let i = pop!();
                    let Some(i) = x::konst(&i) else {
                        out.push(End::Opaque { cond: s.cond, why: "br_table on an index not known".into() });
                        return;
                    };
                    let Ok(all) = targets.targets().collect::<Result<Vec<u32>, _>>() else {
                        out.push(End::Opaque { cond: s.cond, why: "a br_table the reader could not read".into() });
                        return;
                    };
                    let d = all.get(i as usize).copied().unwrap_or(targets.default());
                    if self.branch(&mut s, d) { s.pc = self.ops.len(); }
                }
                Operator::Return => { s.pc = self.ops.len(); }
                Operator::Call { function_index } => {
                    let why = match self.imports.get(*function_index as usize) {
                        Some(n) => format!("calls the import `{n}`"),
                        None => format!("calls function {function_index}"),
                    };
                    out.push(End::Opaque { cond: s.cond, why });
                    return;
                }
                Operator::ReturnCallIndirect { .. } | Operator::CallIndirect { .. } | Operator::ReturnCall { .. } => {
                    out.push(End::Opaque { cond: s.cond, why: "chains to another translation".into() });
                    return;
                }
                Operator::Drop => { pop!(); }
                Operator::Select => {
                    let cnd = x::nonzero(&pop!());
                    let b = pop!();
                    let a = pop!();
                    s.stack.push(x::ite(&cnd, &a, &b));
                }
                Operator::LocalGet { local_index } => s.stack.push(s.locals[*local_index as usize].clone()),
                Operator::LocalSet { local_index } => { let v = pop!(); s.locals[*local_index as usize] = v; }
                Operator::LocalTee { local_index } => { let v = pop!(); s.locals[*local_index as usize] = v.clone(); s.stack.push(v); }
                Operator::I32Const { value } => s.stack.push(x::c(32, u64::from(*value as u32))),
                Operator::I64Const { value } => s.stack.push(x::c(64, *value as u64)),
                Operator::I32Add | Operator::I64Add => bi!(x::add),
                Operator::I32Sub | Operator::I64Sub => bi!(x::sub),
                Operator::I32Mul | Operator::I64Mul => bi!(|a: &E, b: &E| x::bin(x::Op::Mul, a, b)),
                Operator::I32And | Operator::I64And => bi!(x::and),
                Operator::I32Or | Operator::I64Or => bi!(x::or),
                Operator::I32Xor | Operator::I64Xor => bi!(x::xor),
                Operator::I32Shl | Operator::I64Shl => sh!(x::Op::Shl),
                Operator::I32ShrU | Operator::I64ShrU => sh!(x::Op::Lshr),
                Operator::I32ShrS | Operator::I64ShrS => sh!(x::Op::Ashr),
                Operator::I32Rotl | Operator::I64Rotl => bi!(|a: &E, b: &E| rot(a, b, true)),
                Operator::I32Rotr | Operator::I64Rotr => bi!(|a: &E, b: &E| rot(a, b, false)),
                Operator::I32Eqz | Operator::I64Eqz => un!(|a: &E| x::zext(&x::eq(a, &x::c(a.w, 0)), 32)),
                Operator::I32Eq | Operator::I64Eq => cmp!(x::eq),
                Operator::I32Ne | Operator::I64Ne => cmp!(|a: &E, b: &E| x::bnot(&x::eq(a, b))),
                Operator::I32LtU | Operator::I64LtU => cmp!(x::ult),
                Operator::I32LtS | Operator::I64LtS => cmp!(x::slt),
                Operator::I32GtU | Operator::I64GtU => cmp!(|a: &E, b: &E| x::ult(b, a)),
                Operator::I32GtS | Operator::I64GtS => cmp!(|a: &E, b: &E| x::slt(b, a)),
                Operator::I32LeU | Operator::I64LeU => cmp!(|a: &E, b: &E| x::bnot(&x::ult(b, a))),
                Operator::I32LeS | Operator::I64LeS => cmp!(|a: &E, b: &E| x::bnot(&x::slt(b, a))),
                Operator::I32GeU | Operator::I64GeU => cmp!(|a: &E, b: &E| x::bnot(&x::ult(a, b))),
                Operator::I32GeS | Operator::I64GeS => cmp!(|a: &E, b: &E| x::bnot(&x::slt(a, b))),
                Operator::I32Clz | Operator::I32Ctz | Operator::I32Popcnt | Operator::I64Clz | Operator::I64Ctz | Operator::I64Popcnt => {
                    let a = pop!();
                    let Some(v) = x::konst(&a) else {
                        out.push(End::Opaque { cond: s.cond, why: "clz/ctz/popcnt of a value not known".into() });
                        return;
                    };
                    let w = a.w;
                    let n = match &self.ops[pc] {
                        Operator::I32Clz | Operator::I64Clz => w - v.bit_width().min(w),
                        Operator::I32Ctz | Operator::I64Ctz => v.trailing_zeros().min(w),
                        _ => v.count_ones(),
                    };
                    s.stack.push(x::c(w, u64::from(n)));
                }
                Operator::I64ExtendI32U => un!(|a: &E| x::zext(a, 64)),
                Operator::I64ExtendI32S => un!(|a: &E| x::sext(a, 64)),
                Operator::I32WrapI64 => un!(|a: &E| x::extract(a, 31, 0)),
                Operator::I32Extend8S => un!(|a: &E| x::sext(&x::extract(a, 7, 0), 32)),
                Operator::I32Extend16S => un!(|a: &E| x::sext(&x::extract(a, 15, 0), 32)),
                Operator::I64Extend8S => un!(|a: &E| x::sext(&x::extract(a, 7, 0), 64)),
                Operator::I64Extend16S => un!(|a: &E| x::sext(&x::extract(a, 15, 0), 64)),
                Operator::I64Extend32S => un!(|a: &E| x::sext(&x::extract(a, 31, 0), 64)),
                Operator::I32Load { memarg } => ld!(memarg, 4, 32, false),
                Operator::I64Load { memarg } => ld!(memarg, 8, 64, false),
                Operator::I32Load8U { memarg } => ld!(memarg, 1, 32, false),
                Operator::I32Load8S { memarg } => ld!(memarg, 1, 32, true),
                Operator::I32Load16U { memarg } => ld!(memarg, 2, 32, false),
                Operator::I32Load16S { memarg } => ld!(memarg, 2, 32, true),
                Operator::I64Load8U { memarg } => ld!(memarg, 1, 64, false),
                Operator::I64Load16U { memarg } => ld!(memarg, 2, 64, false),
                Operator::I64Load32U { memarg } => ld!(memarg, 4, 64, false),
                Operator::I64Load32S { memarg } => ld!(memarg, 4, 64, true),
                Operator::I32Store { memarg } | Operator::I64Store32 { memarg } => st!(memarg, 4),
                Operator::I64Store { memarg } => st!(memarg, 8),
                Operator::I32Store8 { memarg } | Operator::I64Store8 { memarg } => st!(memarg, 1),
                Operator::I32Store16 { memarg } | Operator::I64Store16 { memarg } => st!(memarg, 2),
                op => {
                    out.push(End::Opaque { cond: s.cond, why: format!("the operator {op:?}") });
                    return;
                }
            }
        }
        out.push(End::Opaque { cond: s.cond, why: "more than 200000 steps".into() });
    }
}
