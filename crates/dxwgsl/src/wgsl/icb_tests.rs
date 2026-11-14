/// ps_4_0: dcl_output o0.xyzw; [dcl_immediateConstantBuffer of `icb` vec4s]; mov o0, icb[k]; ret
fn ps(icb: Option<&[[u32; 4]]>, k: u32) -> Vec<u8> {
    let mut t = vec![0x0000_0040, 0, 0x0300_0065, 0x0010_20f2, 0];
    if let Some(v) = icb {
        t.extend([0x35 | 3 << 11, 2 + 4 * v.len() as u32]);
        t.extend(v.iter().flatten());
    }
    t.extend([0x0500_0036, 0x0010_20f2, 0, 0x0010_9e46, k, 0x0100_003e]);
    t[1] = t.len() as u32;
    let shdr: Vec<u8> = t.iter().flat_map(|x| x.to_le_bytes()).collect();
    // OSGN: SV_Target0, float4, o0, mask xyzw
    let mut osgn = Vec::new();
    for x in [1u32, 8, 32, 0, 64, 3, 0, 0xf] {
        osgn.extend(x.to_le_bytes());
    }
    osgn.extend(b"SV_Target\0\0\0");
    let chunks: [(&[u8; 4], &[u8]); 2] = [(b"OSGN", &osgn), (b"SHDR", &shdr)];
    let mut d = b"DXBC".to_vec();
    d.extend([0; 16]);
    d.extend(1u32.to_le_bytes());
    d.extend(0u32.to_le_bytes()); // total size, set below
    d.extend((chunks.len() as u32).to_le_bytes());
    let mut at = d.len() + 4 * chunks.len();
    for (_, c) in chunks {
        d.extend((at as u32).to_le_bytes());
        at += 8 + c.len();
    }
    for (tag, c) in chunks {
        d.extend(*tag);
        d.extend((c.len() as u32).to_le_bytes());
        d.extend(c);
    }
    let n = d.len() as u32;
    d[24..28].copy_from_slice(&n.to_le_bytes());
    d
}

fn valid(w: &str) {
    let m = naga::front::wgsl::parse_str(w).unwrap_or_else(|e| panic!("{}\n{w}", e.emit_to_string(w)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&m)
        .unwrap_or_else(|e| panic!("{e:?}\n{w}"));
}

#[test]
fn icb_reads_its_declaration() {
    let v = [[1, 2, 3, 4], [5, 6, 7, 8]];
    for k in 0..2 {
        let w = crate::translate(&ps(Some(&v), k), true, None).expect("declared icb");
        assert!(w.code.contains(&format!("icb[{k}")), "{}", w.code);
        valid(&w.code);
    }
}

#[test]
fn icb_without_a_declaration_is_rejected() {
    // the fuzz's input read icb[3] with no dcl_immediateConstantBuffer: WGSL naming an
    // undefined icb went to WebGPU
    let e = crate::translate(&ps(None, 3), true, None).err().expect("rejected");
    assert_eq!(e, "icb is not declared");
    let e = crate::translate(&ps(Some(&[]), 0), true, None).err().expect("rejected");
    assert_eq!(e, "icb is not declared");
    let e = crate::translate(&ps(Some(&[[1, 2, 3, 4]]), 1), true, None).err().expect("rejected");
    assert!(e.contains("past its size"), "{e}");
}
