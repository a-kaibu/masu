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
            // (`fn`, `pub fn`, `pub(crate) fn`: D3DReflect was a pub one, which this missed)
            while let Some(at) = ["\nfn ", "\npub fn ", "\npub(crate) fn "].iter().filter_map(|k| rest.find(k).map(|i| (i, k.len()))).min() {
                rest = &rest[at.0 + at.1..];
                let head = rest;
                let Some(paren) = head.find('(') else { continue };
                let name = &head[..paren];
                // (one-line handlers too: CryptGetUserKey's `{ add_rsa(c, false) }` passed its
                // out-parameter on as c.a(2), which this missed and MASU_OUTS_STRICT found)
                if !head[paren..].starts_with("(c: &mut Ctx) -> u32 {") || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    continue;
                }
                let body = &head[paren..];
                let one_line = !body.starts_with("(c: &mut Ctx) -> u32 {\n");
                let end = if one_line { body.find('\n') } else { body.find("\n}\n") }.unwrap_or(body.len());
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

/// A closure registered for such a function (`implement(dll, "Name", |c| ..)`) takes its
/// arguments as the function's too, first thing; a named handler registered under another
/// name has that name's out-parameters (review 2026-11-16 follow-up: 24 closures took theirs
/// through Ctx::a, SHCreateItemFromParsingName's written 32 bits wide).
#[test]
fn registered_closures_take_their_functions_arguments() {
    let mut table: HashMap<String, (String, String)> = HashMap::new();
    for l in include_str!("outptrs.txt").lines() {
        let v: Vec<&str> = l.split(' ').collect();
        table.insert(v[1].to_string(), (v[2].to_string(), v[3].to_string()));
    }
    // (a handler's typed first line: the function whose arguments it takes)
    let typed_as: HashMap<String, String> = handlers()
        .into_iter()
        .filter_map(|(n, _, body)| {
            let rest = body.strip_prefix("(c: &mut Ctx) -> u32 {\n    let c = &mut c.args::<crate::api::outs::")?;
            Some((n, rest.split('>').next()?.to_string()))
        })
        .collect();
    let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    let (mut bad, mut seen) = (vec![], 0);
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "rs") || p.ends_with("outptr_audit.rs") {
                continue;
            }
            let s = std::fs::read_to_string(&p).unwrap();
            let mut rest = s.as_str();
            while let Some(at) = rest.find("implement(") {
                rest = &rest[at + 10..];
                // implement(<dll>, "<Name>", <handler>
                let Some(q) = rest.find('"') else { break };
                let Some(len) = rest[q + 1..].find('"') else { break };
                let name = &rest[q + 1..q + 1 + len];
                if rest[..q].contains([')', ';', '\n']) || !table.contains_key(name) {
                    continue;
                }
                let h = rest[q + 2 + len..].trim_start_matches([',', ' ']);
                if h.starts_with('|') {
                    seen += 1;
                    let typed = format!("c.args::<crate::api::outs::{name}>()");
                    let head: String = h.chars().take(160).collect();
                    if !head.contains(&typed) {
                        bad.push(format!("{}: implement(.., \"{name}\", |c| ..): its first thing is to be {typed}", p.display()));
                    }
                } else {
                    let f: String = h.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':').collect();
                    let f = f.rsplit("::").next().unwrap_or_default();
                    // (its handler takes the arguments of a function with the same out-parameters)
                    let as_ = typed_as.get(f).map(String::as_str);
                    if f != name && as_.is_none_or(|t| table.get(t) != table.get(name)) {
                        bad.push(format!("{}: implement(.., \"{name}\", {f}): {f} is to start `let c = &mut c.args::<crate::api::outs::{name}>();` (or a function's with the same out-parameters), it takes {as_:?}'s", p.display()));
                    }
                }
            }
            // ("Name", handler) tables (gdiplus's, d3dx9's...): the same rule as a named handler's
            let mut rest = s.as_str();
            while let Some(at) = rest.find("(\"") {
                rest = &rest[at + 2..];
                let Some(len) = rest.find('"') else { break };
                let name = &rest[..len];
                let Some(h) = rest[len + 1..].strip_prefix(", ") else { continue };
                let f: String = h.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == ':').collect();
                if !table.contains_key(name) || !f.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') || !h[f.len()..].starts_with(')') {
                    continue;
                }
                let f = f.rsplit("::").next().unwrap_or_default();
                let as_ = typed_as.get(f).map(String::as_str);
                if f != name && as_.is_none_or(|t| table.get(t) != table.get(name)) {
                    bad.push(format!("{}: (\"{name}\", {f}): {f} is to start `let c = &mut c.args::<crate::api::outs::{name}>();` (or a function's with the same out-parameters), it takes {as_:?}'s", p.display()));
                }
            }
        }
    }
    // (13 on one line at 2026-11-16)
    assert!(seen >= 10, "only {seen} closures found");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
