use proptest::prelude::*;

fn shdr(words: &[u32]) -> Vec<u8> {
    let mut w = vec![0x0040_0050u32, u32::try_from(words.len() + 2).unwrap_or(u32::MAX)];
    w.extend(words);
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Review A-7: an OP_CUSTOMDATA length near 2^32 is an error (on wasm32 `p + n` wrapped
/// below the program's length and the slice panicked); a good one still decodes.
#[test]
fn customdata_lengths() {
    for n in [0u32, 1, 5, 0xffff_ffff, 0xffff_fffe, 0xffff_fffd, 0x8000_0000] {
        assert!(super::decode(&shdr(&[0x35, n, 1, 2])).is_err(), "{n:#x}");
    }
    let p = super::decode(&shdr(&[0x35, 4, 7, 8])).unwrap();
    assert_eq!(p.insts[0].data, [7, 8]);
}

proptest! {
    /// Any words: decode returns, no panic.
    #[test]
    fn any_words(words in proptest::collection::vec(prop_oneof![any::<u32>(), Just(0x35), 0u32..8, Just(u32::MAX)], 0..40)) {
        let _ = super::decode(&shdr(&words));
    }
}

/// (every compute instruction wgsl translates has its operand count checked: the fuzzer's
/// ld_structured with 3 operands indexed past them)
#[test]
fn compute_instructions_have_their_arity() {
    for (op, k) in [(0xa3, 3), (0xa4, 3), (0xa5, 3), (0xa6, 3), (0xa7, 4), (0xa8, 4), (0xbe, 0)] {
        assert_eq!(super::arity(super::Opcode::from_code(op)), Some(k), "{op:#x}");
    }
}

/// fxc's own bytecode (D3DCompile on Windows, winref d3d11's MSAA shaders, ps_4_1):
/// samplepos carries a zero word past its operands; ld2dms and sampleinfo as documented
#[test]
fn fxc_multisample_shaders() {
    for (name, code, want) in [
        ("samplepos", &include_bytes!("../fxc/ps41_samplepos.dxbc")[..], &["textureNumSamples", "d3d_samplepos"][..]),
        ("ld2dms", &include_bytes!("../fxc/ps41_ld2dms.dxbc")[..], &["textureLoad"][..]),
    ] {
        let w = crate::translate(code, true, None).unwrap_or_else(|e| panic!("{name}: {e}"));
        for s in want {
            assert!(w.code.contains(s), "{name}: no {s} in\n{}", w.code);
        }
    }
}
