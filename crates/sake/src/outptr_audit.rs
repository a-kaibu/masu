//! Out-parameters that point at something pointer-sized (pointers, handles, interfaces, SIZE_T,
//! SRWLOCK...) are 8 bytes on x64; a handler that writes them with put32/w32 leaves the upper half
//! as the caller had it (GetProcessAffinityMask's masks, InitializeSRWLock's lock, every
//! DirectDrawCreate/DirectSoundCreate interface: tests/winref/probes/outptr*.c). Such an argument
//! is an api::Out (`c.out(i)`): it is no address the 32-bit writers take, and its own writers
//! write it whole; this test checks that every handler of such a function takes its arguments as
//! the function's (api::Args, from the types build.rs makes of outptrs.txt), where taking one as
//! a value does not compile. outptrs.txt is
//! generated from Microsoft's Win32 metadata (tools/win32md): "dll name x64 x86" per function
//! (the x64 argument indices and the x86 stack dwords, "i,j").
//! This checks every `fn Name(c: &mut Ctx) -> u32` handler named after a function there.
use std::collections::HashMap;

/// (handler name, file, body) of every handler in src/.
fn handlers() -> Vec<(String, String, String)> {
    let mut v = vec![];
    let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let s = std::fs::read_to_string(&p).unwrap();
            let mut rest = s.as_str();
            while let Some(at) = rest.find("\nfn ") {
                rest = &rest[at + 4..];
                let head = rest;
                let Some(paren) = head.find('(') else { continue };
                let name = &head[..paren];
                if !head[paren..].starts_with("(c: &mut Ctx) -> u32 {\n") || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                let body = &head[paren..];
                let end = body.find("\n}\n").unwrap_or(body.len());
                v.push((name.to_string(), p.display().to_string(), body[..end].to_string()));
            }
        }
    }
    v
}

#[test]
fn pointer_sized_out_params_are_written_whole() {
    let mut table: HashMap<&str, Vec<usize>> = HashMap::new();
    for l in include_str!("outptrs.txt").lines() {
        let v: Vec<&str> = l.split(' ').collect();
        table.entry(v[1]).or_default().extend(v[2].split(',').map(|i| i.parse::<usize>().unwrap()));
    }
    let hs = handlers();
    let checked: Vec<_> = hs.iter().filter(|h| table.contains_key(h.0.as_str())).collect();
    // (the parse above found them: 324 handlers have such parameters at 2026-11-16)
    assert!(checked.len() > 300, "only {} handlers found", checked.len());
    // (a pointer-sized out-parameter is taken only as api::Out: each such handler takes its
    // arguments as its function's, c.args::<outs::Name>(), where a::<I>() of one does not
    // compile and the index-taking c.a(i) is not there; review 2026-11-16, the text search for
    // `c.a(i)` this was missed an index in a variable and a helper given `c`, which Ctx::a now
    // reports at run time)
    let mut bad = vec![];
    for (name, file, body) in checked {
        let first = format!("(c: &mut Ctx) -> u32 {{\n    let c = &mut c.args::<crate::api::outs::{name}>();\n");
        if !body.starts_with(&first) {
            bad.push(format!("{file}: {name} has pointer-sized out-parameters {:?}: its first line is to be `let c = &mut c.args::<crate::api::outs::{name}>();`", table[name.as_str()]));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// An Out becomes a u32 only as a word of a call (api::words!): the sealed trait behind it is
/// named nowhere else in sake (review 2026-11-16: Out::to_callee was a plain u32 that a 32-bit
/// writer could take; a crate cannot hide a module from itself, so this is the rest of the seal).
#[test]
fn an_out_is_a_u32_only_in_words() {
    let mut bad = vec![];
    let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    let mut seen = 0;
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") || p.ends_with("api.rs") || p.ends_with("outptr_audit.rs") {
                continue;
            }
            let s = std::fs::read_to_string(&p).unwrap();
            seen += s.matches("words![").count();
            for (n, l) in s.lines().enumerate() {
                if l.contains("__word") || l.contains("callee::") || l.contains("to_callee") {
                    bad.push(format!("{}:{}: {l}", p.display(), n + 1));
                }
            }
        }
    }
    // (the d3d11 device calls, CoCreateInstance's, MFGetService's...: 15 at 2026-11-16)
    assert!(seen >= 10, "only {seen} words! found");
    assert!(bad.is_empty(), "an Out made a u32 outside words!:\n{}", bad.join("\n"));
}
