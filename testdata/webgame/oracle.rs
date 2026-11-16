// The server's former /tree/ rules (crates/masu/src/treefiles.rs range + with_hook before the
// files moved to the browser, verbatim but for the header lookup) over seeded random inputs, as
// JSON lines: testdata/webgame/tree-rules.jsonl, which web/src/treeread.test.ts replays.
//   rustc -O --edition 2021 oracle.rs && ./oracle | sort -u > tree-rules.jsonl
fn range(v: Option<&str>, size: u64) -> Option<Option<(u64, u64)>> {
    let v = v?;
    let parse = || -> Option<(u64, u64)> {
        let v = v.trim().strip_prefix("bytes=")?;
        let (a, b) = v.split_once('-')?;
        let last = size.checked_sub(1)?;
        let (a, b) = match (a.trim(), b.trim()) {
            ("", n) => (size.saturating_sub(n.parse::<u64>().ok().filter(|&n| n > 0)?), last),
            (a, "") => (a.parse().ok()?, last),
            (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(last)),
        };
        (a <= b).then_some((a, b))
    };
    Some(parse())
}
fn with_hook(page: Vec<u8>) -> Vec<u8> {
    const HOOK: &[u8] = b"<script src=\"/frame.js\"></script>";
    let body = page.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&page);
    if body.iter().find(|c| !c.is_ascii_whitespace()) != Some(&b'<') {
        return page;
    }
    let low = page.to_ascii_lowercase();
    let at = low
        .windows(5)
        .position(|w| w == b"<head")
        .and_then(|i| low[i..].iter().position(|&c| c == b'>').map(|j| i + j + 1))
        .unwrap_or(0);
    [&page[..at], HOOK, &page[at..]].concat()
}
struct R(u64);
impl R { fn n(&mut self, m: u64) -> u64 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; self.0 % m } }
fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn js(s: &str) -> String { let mut o = String::from("\""); for c in s.chars() { match c { '"' => o.push_str("\\\""), '\\' => o.push_str("\\\\"), c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)), c => o.push(c) } } o.push('"'); o }
fn main() {
    let mut r = R(0x9e3779b97f4a7c15);
    let pieces = ["bytes=", "bytes= ", " ", "-", "0", "1", "7", "12", "99", "+3", ",", "x", "18446744073709551615", "9007199254740991", "bytes=-", "BYTES="];
    for _ in 0..1000 {
        let mut v = String::new();
        for _ in 0..r.n(6) { v.push_str(pieces[r.n(pieces.len() as u64) as usize]); }
        let size = [0, 1, 2, 10, 100, 5000][r.n(6) as usize];
        let got = match range(Some(&v), size) { None => "null".into(), Some(None) => "\"bad\"".into(), Some(Some((a, b))) => format!("[{a},{b}]") };
        println!("{{\"range\":{},\"size\":{size},\"out\":{got}}}", js(&v));
    }
    for _ in 0..1500 {
        let size = [0, 1, 2, 10, 100, 5000, 9007199254740991][r.n(7) as usize];
        let num = |r: &mut R| -> String { match r.n(5) { 0 => String::new(), 1 => r.n(3).to_string(), 2 => r.n(size.max(1) + 2).to_string(), 3 => r.n(20000).to_string(), _ => (size.saturating_sub(r.n(3))).to_string() } };
        let sp = |r: &mut R| -> &str { if r.n(6) == 0 { " " } else { "" } };
        let v = format!("{}bytes={}{}{}-{}{}{}", sp(&mut r), sp(&mut r), num(&mut r), sp(&mut r), sp(&mut r), num(&mut r), sp(&mut r));
        let got = match range(Some(&v), size) { None => "null".into(), Some(None) => "\"bad\"".into(), Some(Some((a, b))) => format!("[{a},{b}]") };
        println!("{{\"range\":{},\"size\":{size},\"out\":{got}}}", js(&v));
    }
    let bits: [&[u8]; 14] = [b" ", b"\t", b"\n", b"\r", b"\x0b", b"\x0c", b"\xef\xbb\xbf", b"<", b">", b"<head", b"<HEAD", b"<html>", b"abc", b"<head x=1>"];
    for _ in 0..1200 {
        let mut p = Vec::new();
        for _ in 0..r.n(7) { p.extend_from_slice(bits[r.n(bits.len() as u64) as usize]); }
        println!("{{\"page\":\"{}\",\"out\":\"{}\"}}", hex(&p), hex(&with_hook(p.clone())));
    }
}
