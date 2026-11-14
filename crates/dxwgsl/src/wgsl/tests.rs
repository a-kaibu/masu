/// Every DXBC file in $MASU_DXBC_DIR (shaders taken from games) translates to WGSL that naga
/// accepts. Pixel or vertex comes from the program type; each vertex shader is linked to
/// every pixel shader in the directory.
#[test]
fn corpus() {
    let Ok(dir) = std::env::var("MASU_DXBC_DIR") else { return };
    let mut shaders = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        let code = std::fs::read(&p).unwrap();
        let Ok((_, prog)) = super::parse(&code) else { continue };
        shaders.push((p, code, prog.kind));
    }
    let mut fails = 0;
    let check = |name: &str, code: &[u8], pixel: bool, ps: Option<&[u8]>| -> bool {
        match super::translate(code, pixel, ps) {
            Ok(w) => match naga::front::wgsl::parse_str(&w.code) {
                Ok(m) => {
                    let v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m);
                    if let Err(e) = v {
                        eprintln!("{name}: invalid: {e:?}\n{}", w.code);
                        return false;
                    }
                    if std::env::var_os("MASU_DXBC_PRINT").is_some() {
                        eprintln!("{name}:\n{}", w.code);
                    }
                    true
                }
                Err(e) => {
                    eprintln!("{name}: {}\n{}", e.emit_to_string(&w.code), w.code);
                    false
                }
            },
            Err(e) => {
                eprintln!("{name}: {e}");
                false
            }
        }
    };
    for (p, code, kind) in &shaders {
        let name = p.display().to_string();
        match *kind {
            crate::tpf::PIXEL => fails += !check(&name, code, true, None) as u32,
            crate::tpf::VERTEX => {
                fails += !check(&name, code, false, None) as u32;
                for (q, ps, k) in &shaders {
                    if *k == crate::tpf::PIXEL {
                        fails += !check(&format!("{name} + {}", q.display()), code, false, Some(ps)) as u32;
                    }
                }
            }
            k => eprintln!("{name}: program type {k} skipped"),
        }
    }
    assert_eq!(fails, 0);
}
