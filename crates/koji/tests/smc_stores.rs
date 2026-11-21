//! Stores in a block whose code may be rewritten (koji::jit::set_watch: kura's W|X pages of the
//! main executable, where packers and protectors patch code): every kind of store the JIT
//! makes is followed by the check that leaves for the host when the page has translated code
//! (Cpu::jit_smc). Only write_op's and the packed `stored` ones had it; PUSH, FST/FSTP/FIST,
//! FNSTCW, MOVLPS/MOVHPS, MOVSS, MOVQ/MOVNTQ and the cached 128-bit moves stored without
//! (review b02). WATCH is a static: this test is alone in its binary.

use koji::jit::{self, wasm};


/// The JIT code of `bytes` then RET, as a region: how many smc exits (stores of an address
/// into Cpu::jit_smc) it has, or None if nothing was translated. Each exit stores the store's
/// length too (Cpu::jit_smc_len, the `widths` given: the host meets the whole span with the
/// code's, review e18bf44).
fn smc_exits(bytes: &[u8]) -> Option<usize> {
    let mut v = bytes.to_vec();
    v.push(0xc3);
    let mut m = wasm::Module::default();
    jit::translate_region(&mut m, &mut koji::Rom::<0x10_0000>(v), &[0x10_0000], 16, |_| true)?;
    let b = m.finish();
    let count = |off: usize| {
        let mut c = wasm::Code::default();
        c.store32(off as u32);
        b.windows(c.b.len()).filter(|w| *w == &c.b[..]).count()
    };
    let exits = count(std::mem::offset_of!(koji::Cpu, jit_smc));
    assert_eq!(count(std::mem::offset_of!(koji::Cpu, jit_smc_len)), exits, "an smc exit without the store's length");
    Some(exits)
}

/// The lengths an smc exit of `bytes` stores (the i32.const before each store to jit_smc_len).
fn smc_lengths(bytes: &[u8]) -> Vec<u32> {
    let mut v = bytes.to_vec();
    v.push(0xc3);
    let mut m = wasm::Module::default();
    jit::translate_region(&mut m, &mut koji::Rom::<0x10_0000>(v), &[0x10_0000], 16, |_| true).unwrap();
    let b = m.finish();
    let mut out = Vec::new();
    for n in [1u32, 2, 4, 8, 10, 16] {
        let mut c = wasm::Code::default();
        c.u32c(n).store32(std::mem::offset_of!(koji::Cpu, jit_smc_len) as u32);
        out.extend(std::iter::repeat_n(n, b.windows(c.b.len()).filter(|w| *w == &c.b[..]).count()));
    }
    out.sort();
    out
}

#[test]
fn an_smc_exit_says_how_many_bytes_were_stored() {
    jit::set_watch(|_| true);
    for (name, bytes, want) in [
        ("mov [ebx], al", &[0x88, 0x03][..], 1),
        ("mov [ebx], ax", &[0x66, 0x89, 0x03], 2),
        ("mov [ebx], eax", &[0x89, 0x03], 4),
        ("fld1; fstp qword [ebx]", &[0xd9, 0xe8, 0xdd, 0x1b], 8),
        ("movups [ebx], xmm1", &[0x0f, 0x11, 0x0b], 16),
    ] {
        let got = smc_lengths(bytes);
        assert!(!got.is_empty() && got.iter().all(|&n| n == want), "{name}: {got:?}");
    }
}

#[test]
fn every_store_is_checked_in_watched_code() {
    jit::set_watch(|_| true);
    let cases: &[(&str, &[u8])] = &[
        ("mov [ebx], eax", &[0x89, 0x03]),
        ("push eax", &[0x50]),
        ("push imm32", &[0x68, 0x78, 0x56, 0x34, 0x12]),
        ("push dword [ebx]", &[0xff, 0x33]),
        ("fld1; fstp dword [ebx]", &[0xd9, 0xe8, 0xd9, 0x1b]),
        ("fld1; fst qword [ebx]", &[0xd9, 0xe8, 0xdd, 0x13]),
        ("fld1; fistp dword [ebx]", &[0xd9, 0xe8, 0xdb, 0x1b]),
        ("fnstcw [ebx]", &[0xd9, 0x3b]),
        ("movlps [ebx], xmm0", &[0x0f, 0x13, 0x03]),
        ("movhps [ebx], xmm0", &[0x0f, 0x17, 0x03]),
        ("movss [ebx], xmm0", &[0xf3, 0x0f, 0x11, 0x03]),
        ("movq [ebx], mm0", &[0x0f, 0x7f, 0x03]),
        ("movntq [ebx], mm0", &[0x0f, 0xe7, 0x03]),
        ("movaps xmm0, [ecx]; movaps [ebx], xmm0", &[0x0f, 0x28, 0x01, 0x0f, 0x29, 0x03]),
        ("movups [ebx], xmm1", &[0x0f, 0x11, 0x0b]),
        ("movdqa [ebx], xmm0", &[0x66, 0x0f, 0x7f, 0x03]),
        ("movsd [ebx], xmm0", &[0xf2, 0x0f, 0x11, 0x03]),
        ("add [ebx+ecx*4+8], eax", &[0x01, 0x44, 0x8b, 0x08]),
        ("mov fs:[0], eax", &[0x64, 0xa3, 0, 0, 0, 0]),
    ];
    for (name, bytes) in cases {
        let n = smc_exits(bytes).unwrap_or_else(|| panic!("{name}: not translated"));
        assert!(n >= 1, "{name}: a store without the self-modifying code check");
    }
    // (and none where nothing is stored)
    assert_eq!(smc_exits(&[0x8b, 0x03]), Some(0), "mov eax, [ebx]");
}
