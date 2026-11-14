//! The GL enum tables pinned, before the GL types and base formats become enums: every answer
//! of type_size, comp_size, blend_factor, blend_op, stencil_op, compare, base_format,
//! format_comps and packed_fields for every value 0..=0xffff (each one's non-default answers
//! listed, and a hash of all), comp8 of random bytes for each type, and to_rgba of random
//! pixels for every format, type and base format, with random unpack states.
//! format_golden.txt holds them; MASU_UPDATE_FORMATS=1 rewrites it.
use super::*;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h ^= u64::from(x);
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

fn table(name: &str, default: String, f: impl Fn(u32) -> String) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut s = format!("{name} (else {default}):");
    for v in 0..=0xffffu32 {
        let a = f(v);
        fnv(&mut h, a.as_bytes());
        if a != default {
            s += &format!(" {v:#x}={a}");
        }
    }
    s + &format!(" all={h:016x}\n")
}

#[test]
fn gl_tables_answer_as_pinned() {
    let mut got = String::new();
    got += &table("type_size", "4".into(), |v| type_size(v).to_string());
    got += &table("comp_size", "0".into(), |v| comp_size(v).to_string());
    got += &table("blend_factor", "1".into(), |v| blend_factor(v).to_string());
    got += &table("blend_op", "1".into(), |v| blend_op(v).to_string());
    got += &table("stencil_op", "1".into(), |v| stencil_op(v).to_string());
    got += &table("compare", "8".into(), |v| compare(v).to_string());
    got += &table("base_format", "0".into(), |v| base_format(v as i32).to_string());
    got += &table("format_comps", "0".into(), |v| format_comps(v).to_string());
    got += &table("packed_fields", "None".into(), |v| format!("{:?}", packed_fields(v)));
    // (base_format of the negative and the large internal formats)
    got += &format!("base_format_far: {:?}\n", [-1, i32::MIN, 0x1_0000, 0x7fff_ffff].map(base_format));
    let mut r = Rng(0x5eed_6c00_0000_0001);
    let types = [0x1400u32, 0x1401, 0x1402, 0x1403, 0x1404, 0x1405, 0x1406, 0x140A, 0x140B, 0x1407, 0];
    for ty in types {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for _ in 0..512 {
            let b = r.next().to_le_bytes();
            fnv(&mut h, &[unsafe { comp8(b.as_ptr(), ty) }]);
        }
        got += &format!("comp8 {ty:#x} {h:016x}\n");
    }
    let formats = [0x1903u32, 0x1904, 0x1905, 0x1906, 0x1907, 0x1908, 0x1909, 0x190A, 0x8227, 0x80E0, 0x80E1, 0x1902, 0];
    let ptypes = [0x1400u32, 0x1401, 0x1402, 0x1403, 0x1404, 0x1405, 0x1406, 0x140B, 0x8032, 0x8362, 0x8363, 0x8364, 0x8033, 0x8365, 0x8034, 0x8366, 0x8035, 0x8367, 0x8036, 0x8368];
    let src: Vec<u8> = (0..1 << 16).map(|_| r.next().to_le_bytes()[2]).collect();
    for format in formats {
        let mut line = format!("to_rgba {format:#x}");
        for ty in ptypes {
            let mut h = 0xcbf2_9ce4_8422_2325u64;
            for base in 0..=7u32 {
                for _ in 0..3 {
                    let (w, ht) = (1 + (r.next() % 13) as u32, 1 + (r.next() % 7) as u32);
                    let unpack = [[1, 2, 4, 8][(r.next() % 4) as usize], (r.next() % 20) as i32 - 2, (r.next() % 3) as i32, (r.next() % 3) as i32];
                    let px = unsafe { to_rgba(src.as_ptr(), w, ht, format, ty, base, unpack) };
                    fnv(&mut h, &px);
                }
            }
            line += &format!(" {ty:#x}={:08x}", h as u32);
        }
        got += &line;
        got.push('\n');
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gl/format_golden.txt");
    if std::env::var_os("MASU_UPDATE_FORMATS").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    if want != got {
        let diff: Vec<String> = want.lines().zip(got.lines()).filter(|(a, b)| a != b).map(|(a, b)| format!("- {a}\n+ {b}")).collect();
        panic!("gl tables differ from format_golden.txt (MASU_UPDATE_FORMATS=1 rewrites it):\n{}", diff.join("\n"));
    }
}
