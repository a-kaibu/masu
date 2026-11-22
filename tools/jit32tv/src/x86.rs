//! What x86 defines for the integer instructions validated, from the Intel SDM (vol. 2, the
//! instructions' "Operation" and "Flags Affected"), written apart from koji: registers, and the
//! six status flags each with whether it is defined after the instruction (an undefined flag is
//! not compared).

use crate::expr::{self as x, E};
use iced_x86::{Instruction, Mnemonic, OpKind, Register};

#[derive(Clone)]
pub struct Flag {
    pub v: E,
    /// 1 bit: 1 when the SDM defines the flag's value here
    pub defined: E,
}

#[derive(Clone)]
pub struct State {
    /// EAX ECX EDX EBX ESP EBP ESI EDI
    pub regs: Vec<E>,
    pub cf: Flag,
    pub pf: Flag,
    pub af: Flag,
    pub zf: Flag,
    pub sf: Flag,
    pub of: Flag,
}

fn known(v: E) -> Flag {
    Flag { v, defined: x::c(1, 1) }
}

impl State {
    pub fn new(regs: Vec<E>, eflags: &E) -> State {
        let f = |b| known(x::bit(eflags, b));
        State { regs, cf: f(0), pf: f(2), af: f(4), zf: f(6), sf: f(7), of: f(11) }
    }
}

/// a register's place: (index of its 32-bit register, bit offset, width)
fn place(r: Register) -> Option<(usize, u32, u32)> {
    use Register::*;
    let i = |r: Register| r.full_register32().number();
    Some(match r {
        AL | CL | DL | BL => (i(r), 0, 8),
        AH | CH | DH | BH => (i(r), 8, 8),
        AX | CX | DX | BX | SP | BP | SI | DI => (i(r), 0, 16),
        EAX | ECX | EDX | EBX | ESP | EBP | ESI | EDI => (i(r), 0, 32),
        _ => return Option::None,
    })
}

fn read(s: &State, r: Register) -> Option<E> {
    let (i, off, w) = place(r)?;
    Some(x::extract(&s.regs[i], off + w - 1, off))
}

/// a write of `v` to `r`: the other bits of its 32-bit register kept (8 and 16 bits)
fn write(s: &mut State, r: Register, v: &E) -> Option<()> {
    let (i, off, w) = place(r)?;
    let old = &s.regs[i];
    let mut n = x::zext(v, 32);
    if w < 32 {
        let m = x::c(32, !(x::mask(w) << off));
        n = x::or(&x::and(old, &m), &x::shl(&n, &x::c(32, u64::from(off))));
    }
    s.regs[i] = n;
    Some(())
}

fn operand(s: &State, ins: &Instruction, k: u32, w: u32) -> Option<E> {
    match ins.op_kind(k) {
        OpKind::Register => read(s, ins.op_register(k)),
        OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate8to16 | OpKind::Immediate8to32 => {
            Some(x::c(w, ins.immediate(k)))
        }
        _ => None,
    }
}

fn parity(r: &E) -> E {
    // (PF: 1 when the low byte has an even number of ones)
    let mut p = x::bit(r, 0);
    for i in 1..8 {
        p = x::xor(&p, &x::bit(r, i));
    }
    x::bnot(&p)
}

fn szp(s: &mut State, r: &E) {
    let w = r.w;
    s.zf = known(x::eq(r, &x::c(w, 0)));
    s.sf = known(x::bit(r, w - 1));
    s.pf = known(parity(r));
}

fn undefined(v: &Flag) -> Flag {
    Flag { v: v.v.clone(), defined: x::c(1, 0) }
}

/// `a + b + carry` (carry 1 bit) with its flags
fn add(s: &mut State, a: &E, b: &E, carry: &E) -> E {
    let w = a.w;
    let wide = x::add(&x::add(&x::zext(a, w + 1), &x::zext(b, w + 1)), &x::zext(carry, w + 1));
    let r = x::extract(&wide, w - 1, 0);
    s.cf = known(x::bit(&wide, w));
    s.of = known(x::bit(&x::and(&x::xor(a, &r), &x::xor(b, &r)), w - 1));
    s.af = known(x::bit(&x::xor(&x::xor(a, b), &r), 4));
    szp(s, &r);
    r
}

/// `a - b - borrow` with its flags
fn sub(s: &mut State, a: &E, b: &E, borrow: &E) -> E {
    let w = a.w;
    let r = x::sub(&x::sub(a, b), &x::zext(borrow, w));
    s.cf = known(x::ult(&x::zext(a, w + 1), &x::add(&x::zext(b, w + 1), &x::zext(borrow, w + 1))));
    s.of = known(x::bit(&x::and(&x::xor(a, b), &x::xor(a, &r)), w - 1));
    s.af = known(x::bit(&x::xor(&x::xor(a, b), &r), 4));
    szp(s, &r);
    r
}

fn logic(s: &mut State, r: &E) {
    s.cf = known(x::c(1, 0));
    s.of = known(x::c(1, 0));
    s.af = undefined(&s.af);
    szp(s, r);
}

/// flag `f` as `new` where `cnd`, else as it was
fn keep_unless(cnd: &E, new: &Flag, old: &Flag) -> Flag {
    Flag { v: x::ite(cnd, &new.v, &old.v), defined: x::ite(cnd, &new.defined, &old.defined) }
}

/// The shifts and rotates this reference defines
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shift {
    Shl,
    Shr,
    Sar,
    Rol,
    Ror,
}

fn shift(s: &mut State, m: Shift, a: &E, count: &E) -> E {
    let w = a.w;
    // (the count masked to 5 bits, for every operand size; 0 changes nothing, flags included)
    let kw = x::zext(&x::and(count, &x::c(8, 0x1f)), 32);
    let zero = x::eq(&kw, &x::c(32, 0));
    let one = x::eq(&kw, &x::c(32, 1));
    let a32 = x::zext(a, 64);
    let k64 = x::zext(&kw, 64);
    let old = s.clone();
    let (r, cf, of_v) = match m {
        Shift::Shl => {
            let wide = x::shl(&a32, &k64);
            let r = x::extract(&wide, w - 1, 0);
            let cf = x::bit(&wide, w);
            (r.clone(), cf.clone(), x::xor(&x::bit(&r, w - 1), &cf))
        }
        Shift::Shr => {
            let r = x::extract(&x::lshr(&a32, &k64), w - 1, 0);
            let cf = x::bit(&x::lshr(&a32, &x::sub(&k64, &x::c(64, 1))), 0);
            (r, cf, x::bit(a, w - 1))
        }
        Shift::Sar => {
            let s64 = x::sext(a, 64);
            let r = x::extract(&x::ashr(&s64, &k64), w - 1, 0);
            let cf = x::bit(&x::ashr(&s64, &x::sub(&k64, &x::c(64, 1))), 0);
            (r, cf, x::c(1, 0))
        }
        Shift::Rol | Shift::Ror => {
            // (rotation by the masked count modulo the width; CF and OF only)
            let n = x::extract(&x::and(&kw, &x::c(32, u64::from(w) - 1)), w - 1, 0);
            let back = x::and(&x::sub(&x::c(w, u64::from(w)), &n), &x::c(w, u64::from(w) - 1));
            let left = m == Shift::Rol;
            let r = if left { x::or(&x::shl(a, &n), &x::lshr(a, &back)) } else { x::or(&x::lshr(a, &n), &x::shl(a, &back)) };
            let r = x::ite(&x::eq(&n, &x::c(w, 0)), a, &r);
            let cf = if left { x::bit(&r, 0) } else { x::bit(&r, w - 1) };
            let of = if left { x::xor(&x::bit(&r, w - 1), &cf) } else { x::xor(&x::bit(&r, w - 1), &x::bit(&r, w - 2)) };
            s.cf = keep_unless(&x::bnot(&zero), &known(cf), &old.cf);
            s.of = keep_unless(&x::bnot(&zero), &Flag { v: of, defined: one.clone() }, &old.of);
            return x::ite(&zero, a, &r);
        }
    };
    // (CF: the last bit shifted out, defined for counts below the width; OF for a count of 1;
    // AF undefined; SZP from the result)
    let below = x::ult(&kw, &x::c(32, u64::from(w)));
    let new_cf = Flag { v: cf, defined: below };
    let new_of = Flag { v: of_v, defined: one };
    let mut n = old.clone();
    szp(&mut n, &r);
    let ch = x::bnot(&zero);
    s.cf = keep_unless(&ch, &new_cf, &old.cf);
    s.of = keep_unless(&ch, &new_of, &old.of);
    s.af = keep_unless(&ch, &undefined(&old.af), &old.af);
    s.zf = keep_unless(&ch, &n.zf, &old.zf);
    s.sf = keep_unless(&ch, &n.sf, &old.sf);
    s.pf = keep_unless(&ch, &n.pf, &old.pf);
    x::ite(&zero, a, &r)
}

/// A shift or rotate of `dst` by an immediate or CL
fn shift_step(s: &mut State, ins: &Instruction, dst: Register, op: Shift) -> Option<()> {
    let a = read(s, dst)?;
    let count = match ins.op_kind(1) {
        OpKind::Register => read(s, ins.op1_register())?,
        OpKind::Immediate8 => x::c(8, ins.immediate(1)),
        _ => return None,
    };
    let r = shift(s, op, &a, &x::extract(&count, 7, 0));
    write(s, dst, &r)
}

/// One instruction; None for one this reference does not define.
pub fn step(s: &mut State, ins: &Instruction) -> Option<()> {
    let m = ins.mnemonic();
    let dst = ins.op0_register();
    let w = place(dst).map(|p| p.2);
    match m {
        Mnemonic::Add | Mnemonic::Adc | Mnemonic::Sub | Mnemonic::Sbb | Mnemonic::Cmp => {
            let w = w?;
            let a = read(s, dst)?;
            let b = operand(s, ins, 1, w)?;
            let cf = s.cf.v.clone();
            let r = match m {
                Mnemonic::Add => add(s, &a, &b, &x::c(1, 0)),
                Mnemonic::Adc => add(s, &a, &b, &cf),
                Mnemonic::Sub | Mnemonic::Cmp => sub(s, &a, &b, &x::c(1, 0)),
                _ => sub(s, &a, &b, &cf),
            };
            if m != Mnemonic::Cmp {
                write(s, dst, &r)?;
            }
        }
        Mnemonic::And | Mnemonic::Or | Mnemonic::Xor | Mnemonic::Test => {
            let w = w?;
            let a = read(s, dst)?;
            let b = operand(s, ins, 1, w)?;
            let r = match m {
                Mnemonic::And | Mnemonic::Test => x::and(&a, &b),
                Mnemonic::Or => x::or(&a, &b),
                _ => x::xor(&a, &b),
            };
            logic(s, &r);
            if m != Mnemonic::Test {
                write(s, dst, &r)?;
            }
        }
        Mnemonic::Inc | Mnemonic::Dec => {
            let w = w?;
            let a = read(s, dst)?;
            let cf = s.cf.clone();
            let r = if m == Mnemonic::Inc { add(s, &a, &x::c(w, 1), &x::c(1, 0)) } else { sub(s, &a, &x::c(w, 1), &x::c(1, 0)) };
            s.cf = cf;
            write(s, dst, &r)?;
        }
        Mnemonic::Neg => {
            let w = w?;
            let a = read(s, dst)?;
            let r = sub(s, &x::c(w, 0), &a, &x::c(1, 0));
            write(s, dst, &r)?;
        }
        Mnemonic::Not => {
            let a = read(s, dst)?;
            write(s, dst, &x::not(&a))?;
        }
        Mnemonic::Mov => {
            let w = w?;
            let v = operand(s, ins, 1, w)?;
            write(s, dst, &v)?;
        }
        Mnemonic::Movzx | Mnemonic::Movsx => {
            let w = w?;
            let v = read(s, ins.op1_register())?;
            let v = if m == Mnemonic::Movzx { x::zext(&v, w) } else { x::sext(&v, w) };
            write(s, dst, &v)?;
        }
        Mnemonic::Shl => shift_step(s, ins, dst, Shift::Shl)?,
        Mnemonic::Shr => shift_step(s, ins, dst, Shift::Shr)?,
        Mnemonic::Sar => shift_step(s, ins, dst, Shift::Sar)?,
        Mnemonic::Rol => shift_step(s, ins, dst, Shift::Rol)?,
        Mnemonic::Ror => shift_step(s, ins, dst, Shift::Ror)?,
        _ => return None,
    }
    Some(())
}
