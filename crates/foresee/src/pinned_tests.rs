//! The index readers before le-safe reads (the oracle): the same members from any bytes.
use super::*;
fn old_bgi(head: &[u8]) -> Vec<Member> {
    let (name_len, entry) = match head.get(..12) {
        Some(b"PackFile    ") => (16, 32),
        Some(b"BURIKO ARC20") => (96, 128),
        _ => return Vec::new(),
    };
    let Some(n) = head.get(12..16).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize) else { return Vec::new() };
    let base = 16 + n.saturating_mul(entry);
    let mut out = Vec::new();
    for k in 0..n.min(1 << 20) {
        let Some(e) = head.get(16 + k * entry..16 + (k + 1) * entry) else { break };
        let raw = &e[..name_len];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(name_len);
        let name: String = raw[..end].iter().map(|&b| if b < 0x80 { b as char } else { '_' }).collect();
        let off = u32::from_le_bytes(e[name_len..name_len + 4].try_into().unwrap()) as u64;
        let size = u32::from_le_bytes(e[name_len + 4..name_len + 8].try_into().unwrap()) as u64;
        if !name.is_empty() {
            out.push(Member { name, segments: vec![(base as u64 + off, size, false)], hash: 0 });
        }
    }
    out
}
proptest::proptest! {
    #[test]
    fn bgi_as_before(kind in 0u8..3, n in 0u32..5, body in proptest::collection::vec(proptest::num::u8::ANY, 0..700)) {
        let mut head = match kind { 0 => b"PackFile    ".to_vec(), 1 => b"BURIKO ARC20".to_vec(), _ => b"other thing!".to_vec() };
        head.extend_from_slice(&n.to_le_bytes());
        head.extend_from_slice(&body);
        proptest::prop_assert_eq!(bgi_index(&head), old_bgi(&head));
    }
}

