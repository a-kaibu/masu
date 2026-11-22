//! Bit-vector terms: what the wasm computes and what x86 defines, folded where their inputs are
//! known, written out as SMT-LIB for Z3 where they are not.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    Const,
    Var,
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
    /// (counts of the width or more give 0, as SMT-LIB's bvshl)
    Shl,
    Lshr,
    Ashr,
    Not,
    /// 1 bit: 1 when equal
    Eq,
    Ult,
    Slt,
    /// (cond 1 bit, then, else)
    Ite,
    /// bits hi..=lo
    Extract(u32, u32),
    Zext,
    Sext,
    /// (high, low)
    Concat,
}

#[derive(Debug)]
pub struct Node {
    pub op: Op,
    pub w: u32,
    pub k: u64,
    pub name: String,
    pub args: Vec<E>,
}

pub type E = Rc<Node>;

pub fn mask(w: u32) -> u64 {
    if w >= 64 { u64::MAX } else { (1u64 << w) - 1 }
}

fn sx(v: u64, w: u32) -> i64 {
    if w >= 64 { v as i64 } else { ((v << (64 - w)) as i64) >> (64 - w) }
}

pub fn c(w: u32, v: u64) -> E {
    Rc::new(Node { op: Op::Const, w, k: v & mask(w), name: String::new(), args: Vec::new() })
}

pub fn var(name: &str, w: u32) -> E {
    Rc::new(Node { op: Op::Var, w, k: 0, name: name.into(), args: Vec::new() })
}

pub fn konst(e: &E) -> Option<u64> {
    (e.op == Op::Const).then_some(e.k)
}

fn node(op: Op, w: u32, args: Vec<E>) -> E {
    Rc::new(Node { op, w, k: 0, name: String::new(), args })
}

/// A binary operation of two terms of one width, folded when both are known.
pub fn bin(op: Op, a: &E, b: &E) -> E {
    assert_eq!(a.w, b.w, "{op:?} of widths {} and {}", a.w, b.w);
    let w = a.w;
    if let (Some(x), Some(y)) = (konst(a), konst(b)) {
        let m = mask(w);
        let v = match op {
            Op::Add => x.wrapping_add(y),
            Op::Sub => x.wrapping_sub(y),
            Op::Mul => x.wrapping_mul(y),
            Op::And => x & y,
            Op::Or => x | y,
            Op::Xor => x ^ y,
            Op::Shl => if y >= u64::from(w) { 0 } else { x << y },
            Op::Lshr => if y >= u64::from(w) { 0 } else { x >> y },
            Op::Ashr => (sx(x, w) >> y.min(u64::from(w) - 1)) as u64,
            Op::Eq => return c(1, u64::from(x == y)),
            Op::Ult => return c(1, u64::from(x < y)),
            Op::Slt => return c(1, u64::from(sx(x, w) < sx(y, w))),
            _ => panic!("{op:?} is not binary"),
        };
        return c(w, v & m);
    }
    // (identities that keep known-zero terms small)
    match (op, konst(a), konst(b)) {
        (Op::And, Some(0), _) | (Op::And, _, Some(0)) => return c(w, 0),
        (Op::And, Some(m), _) if m == mask(w) => return b.clone(),
        (Op::And, _, Some(m)) if m == mask(w) => return a.clone(),
        (Op::Or | Op::Xor | Op::Add, Some(0), _) => return b.clone(),
        (Op::Or | Op::Xor | Op::Add | Op::Sub | Op::Shl | Op::Lshr | Op::Ashr, _, Some(0)) => return a.clone(),
        _ => {}
    }
    let rw = match op {
        Op::Eq | Op::Ult | Op::Slt => 1,
        _ => w,
    };
    node(op, rw, vec![a.clone(), b.clone()])
}

pub fn add(a: &E, b: &E) -> E { bin(Op::Add, a, b) }
pub fn sub(a: &E, b: &E) -> E { bin(Op::Sub, a, b) }
pub fn and(a: &E, b: &E) -> E { bin(Op::And, a, b) }
pub fn or(a: &E, b: &E) -> E { bin(Op::Or, a, b) }
pub fn xor(a: &E, b: &E) -> E { bin(Op::Xor, a, b) }
pub fn shl(a: &E, b: &E) -> E { bin(Op::Shl, a, b) }
pub fn lshr(a: &E, b: &E) -> E { bin(Op::Lshr, a, b) }
pub fn ashr(a: &E, b: &E) -> E { bin(Op::Ashr, a, b) }
pub fn eq(a: &E, b: &E) -> E { bin(Op::Eq, a, b) }
pub fn ult(a: &E, b: &E) -> E { bin(Op::Ult, a, b) }
pub fn slt(a: &E, b: &E) -> E { bin(Op::Slt, a, b) }

pub fn not(a: &E) -> E {
    match konst(a) {
        Some(x) => c(a.w, !x),
        None => node(Op::Not, a.w, vec![a.clone()]),
    }
}

/// 1-bit boolean helpers
pub fn bnot(a: &E) -> E { xor(a, &c(1, 1)) }
pub fn band(a: &E, b: &E) -> E { and(a, b) }
pub fn bor(a: &E, b: &E) -> E { or(a, b) }

pub fn ite(cnd: &E, t: &E, f: &E) -> E {
    assert_eq!(cnd.w, 1);
    assert_eq!(t.w, f.w);
    match konst(cnd) {
        Some(1) => t.clone(),
        Some(_) => f.clone(),
        None if Rc::ptr_eq(t, f) => t.clone(),
        None => node(Op::Ite, t.w, vec![cnd.clone(), t.clone(), f.clone()]),
    }
}

pub fn extract(a: &E, hi: u32, lo: u32) -> E {
    assert!(hi < a.w && lo <= hi, "extract {hi}..{lo} of {}", a.w);
    if lo == 0 && hi + 1 == a.w {
        return a.clone();
    }
    match konst(a) {
        Some(x) => c(hi - lo + 1, x >> lo),
        None => node(Op::Extract(hi, lo), hi - lo + 1, vec![a.clone()]),
    }
}

pub fn zext(a: &E, w: u32) -> E {
    assert!(w >= a.w);
    if w == a.w {
        return a.clone();
    }
    match konst(a) {
        Some(x) => c(w, x),
        None => node(Op::Zext, w, vec![a.clone()]),
    }
}

pub fn sext(a: &E, w: u32) -> E {
    assert!(w >= a.w);
    if w == a.w {
        return a.clone();
    }
    match konst(a) {
        Some(x) => c(w, sx(x, a.w) as u64),
        None => node(Op::Sext, w, vec![a.clone()]),
    }
}

pub fn concat(hi: &E, lo: &E) -> E {
    match (konst(hi), konst(lo)) {
        (Some(h), Some(l)) if hi.w + lo.w <= 64 => c(hi.w + lo.w, h << lo.w | l),
        _ => node(Op::Concat, hi.w + lo.w, vec![hi.clone(), lo.clone()]),
    }
}

/// 1 when `a` is not 0
pub fn nonzero(a: &E) -> E {
    bnot(&eq(a, &c(a.w, 0)))
}

/// bit `i` of `a`, 1 bit
pub fn bit(a: &E, i: u32) -> E {
    extract(a, i, i)
}

/// Writes the terms as SMT-LIB definitions (shared subterms once) and returns the name of each.
pub struct Smt {
    pub text: String,
    names: HashMap<*const Node, String>,
    pub vars: Vec<(String, u32)>,
}

impl Smt {
    pub fn new() -> Smt {
        Smt { text: String::new(), names: HashMap::new(), vars: Vec::new() }
    }

    pub fn name(&mut self, e: &E) -> String {
        if let Some(n) = self.names.get(&Rc::as_ptr(e)) {
            return n.clone();
        }
        let s = match e.op {
            Op::Const => format!("(_ bv{} {})", e.k, e.w),
            Op::Var => {
                if !self.vars.iter().any(|(n, _)| *n == e.name) {
                    let _ = writeln!(self.text, "(declare-const {} (_ BitVec {}))", e.name, e.w);
                    self.vars.push((e.name.clone(), e.w));
                }
                e.name.clone()
            }
            op => {
                let a: Vec<String> = e.args.iter().map(|x| self.name(x)).collect();
                let f = |s: &str| format!("({s} {})", a.join(" "));
                let body = match op {
                    Op::Add => f("bvadd"),
                    Op::Sub => f("bvsub"),
                    Op::Mul => f("bvmul"),
                    Op::And => f("bvand"),
                    Op::Or => f("bvor"),
                    Op::Xor => f("bvxor"),
                    Op::Shl => f("bvshl"),
                    Op::Lshr => f("bvlshr"),
                    Op::Ashr => f("bvashr"),
                    Op::Not => f("bvnot"),
                    Op::Eq => format!("(ite (= {} {}) #b1 #b0)", a[0], a[1]),
                    Op::Ult => format!("(ite (bvult {} {}) #b1 #b0)", a[0], a[1]),
                    Op::Slt => format!("(ite (bvslt {} {}) #b1 #b0)", a[0], a[1]),
                    Op::Ite => format!("(ite (= {} #b1) {} {})", a[0], a[1], a[2]),
                    Op::Extract(h, l) => format!("((_ extract {h} {l}) {})", a[0]),
                    Op::Zext => format!("((_ zero_extend {}) {})", e.w - e.args[0].w, a[0]),
                    Op::Sext => format!("((_ sign_extend {}) {})", e.w - e.args[0].w, a[0]),
                    Op::Concat => f("concat"),
                    Op::Const | Op::Var => unreachable!(),
                };
                let n = format!("t{}", self.names.len());
                let _ = writeln!(self.text, "(define-fun {n} () (_ BitVec {}) {body})", e.w);
                n
            }
        };
        self.names.insert(Rc::as_ptr(e), s.clone());
        s
    }
}
