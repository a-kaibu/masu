//! Far JMP / CALL outside 16-bit mode (found by the golden's random programs: `jmp far
//! ptr16:32` panicked in the interpreter, reading the far pointer as an immediate; the m16:32
//! form jumped to the offset and ignored the selector). User-mode code here has no other code
//! segment to go to: every form is #UD (Exit::InvalidOpcode) with nothing changed, EIP at the
//! instruction, nothing pushed.

use koji::{step, Cpu, Exit, Fault, Memory};
use proptest::prelude::*;

struct Flat(Vec<u8>);
impl Memory for Flat {
    fn read8(&mut self, a: u32) -> Result<u8, Fault> {
        self.0.get(usize::try_from(a).unwrap()).copied().ok_or(Fault { addr: a, write: false })
    }
    fn write8(&mut self, a: u32, v: u8) -> Result<(), Fault> {
        match self.0.get_mut(usize::try_from(a).unwrap()) {
            Some(b) => {
                *b = v;
                Ok(())
            }
            None => Err(Fault { addr: a, write: true }),
        }
    }
}

const CODE: u32 = 0x1000;
const PTR: u32 = 0x3000;
const SP: u32 = 0x8000;

/// The encodings: (bytes, valid in long mode)
fn form(kind: u8, sel: u16, off: u32, rexw: bool) -> (Vec<u8>, bool) {
    let mut v = Vec::new();
    match kind {
        // jmp far ptr16:32 / call far ptr16:32 (not encodable in long mode)
        0 | 1 => {
            v.push(if kind == 0 { 0xea } else { 0x9a });
            v.extend(off.to_le_bytes());
            v.extend(sel.to_le_bytes());
            (v, false)
        }
        // jmp far [ebx] / call far [ebx] (FF /5, FF /3), m16:32 or with REX.W m16:64
        _ => {
            if rexw {
                v.push(0x48);
            }
            v.extend([0xff, if kind == 2 { 0x2b } else { 0x1b }]);
            (v, true)
        }
    }
}

proptest! {
    #[test]
    fn far_transfers_are_ud(kind in 0u8..4, sel: u16, off: u32, long: bool, rexw: bool, regs in proptest::array::uniform8(any::<u32>())) {
        let (bytes, in_long) = form(kind, sel, off, rexw && long);
        prop_assume!(!long || in_long);
        let mut m = Flat(vec![0; 0x10000]);
        let (code, ptr) = (usize::try_from(CODE).unwrap(), usize::try_from(PTR).unwrap());
        m.0[code..code + bytes.len()].copy_from_slice(&bytes);
        m.0[ptr..ptr + 8].copy_from_slice(&u64::from(off).to_le_bytes());
        m.0[ptr + 8..ptr + 10].copy_from_slice(&sel.to_le_bytes());
        let mut cpu = Cpu { mode64: u32::from(long), eip: CODE, ..Cpu::default() };
        for (i, r) in regs.iter().enumerate() {
            cpu.set_gpr(i, u64::from(*r));
        }
        cpu.set_gpr(3, u64::from(PTR));
        cpu.set_gpr(4, u64::from(SP));
        let before = (0..16).map(|i| cpu.gpr(i)).collect::<Vec<_>>();
        let mem = m.0.clone();
        prop_assert_eq!(step(&mut cpu, &mut m), Exit::InvalidOpcode, "{:02x?}", bytes);
        prop_assert_eq!(cpu.eip, CODE);
        prop_assert_eq!((0..16).map(|i| cpu.gpr(i)).collect::<Vec<_>>(), before);
        prop_assert!(m.0 == mem, "memory changed");
    }
}
