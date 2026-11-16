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

/// The text from `s[i]`, an opening bracket, to its closing one (strings skipped), or None.
fn bracketed(s: &str, i: usize) -> Option<&str> {
    let b = s.as_bytes();
    let (mut depth, mut j, mut quoted) = (0i32, i, false);
    while let Some(&ch) = b.get(j) {
        if quoted {
            if ch == b'\\' {
                j += 2;
                continue;
            }
            quoted = ch != b'"';
        } else {
            match ch {
                b'"' => quoted = true,
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return s.get(i..=j);
                    }
                }
                _ => {}
            }
        }
        j += 1;
    }
    None
}

/// `t` split at its top-level commas (line comments dropped).
fn top_level(t: &str) -> Vec<String> {
    let t: String = t.lines().map(|l| l.split("//").next().unwrap_or_default()).collect::<Vec<_>>().join("\n");
    let (mut out, mut cur, mut depth, mut quoted) = (vec![], String::new(), 0i32, false);
    for ch in t.chars() {
        if quoted {
            quoted = ch != '"';
        } else if ch == '"' {
            quoted = true;
        } else if "([{".contains(ch) {
            depth += 1;
        } else if ")]}".contains(ch) {
            depth -= 1;
        } else if ch == ',' && depth == 0 {
            out.push(std::mem::take(&mut cur).trim().to_string());
            continue;
        }
        cur.push(ch);
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// Every COM method with pointer-sized out-parameters (com_outptrs.txt) that a vtable of sake's
/// gives a handler: com::impls! lists by name, api::vtable's lists by slot where the list is
/// in the text (inline, or bound to a name in the same file). Its handler takes its arguments
/// as that method's, c.args::<outs::iface::I::M>() (or a method's with the same out-parameters),
/// first thing, or (a handler of several overloads) picks the one it is called as and takes
/// those; a `_` handler takes none (review 2026-11-16 stage 3: 290 COM handlers took theirs
/// through Ctx::a, the typelib's BSTRs and misc's QI written 32 bits wide). Lists made at run
/// time (vec! extended, by kind) are checked when they are called: Ctx::a of an out-parameter
/// fails a test, c.args of another method's too.
#[test]
fn com_handlers_take_their_methods_arguments() {
    // (interface, slot) -> (method, x64, x86); (interface, method) -> slots
    let mut masks: HashMap<(String, usize), (String, String, String)> = HashMap::new();
    let mut slots: HashMap<(String, String), Vec<usize>> = HashMap::new();
    let mut ifaces = std::collections::HashSet::new();
    for l in include_str!("com_outptrs.txt").lines() {
        let f: Vec<&str> = l.split(' ').collect();
        match f.as_slice() {
            [i, "#", _] => {
                ifaces.insert(i.to_string());
            }
            [i, s, m, a, b] => {
                let s: usize = s.parse().unwrap();
                masks.insert((i.to_string(), s), (m.to_string(), a.to_string(), b.to_string()));
                slots.entry((i.to_string(), m.to_string())).or_default().push(s);
            }
            _ => panic!("com_outptrs.txt: {l}"),
        }
    }
    // (a type's out-parameters: iface::I::M or iface::I::M_slot)
    let of_type = |t: &str| -> Option<(String, String)> {
        let rest = t.strip_prefix("crate::api::outs::iface::")?;
        let (i, m) = rest.split_once("::")?;
        let s = match m.rsplit_once('_') {
            Some((_, n)) if n.parse::<usize>().is_ok() && masks.contains_key(&(i.to_string(), n.parse().ok()?)) => n.parse().ok()?,
            _ => *slots.get(&(i.to_string(), m.to_string()))?.first()?,
        };
        masks.get(&(i.to_string(), s)).map(|x| (x.1.clone(), x.2.clone()))
    };
    let mut files = vec![];
    let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
    while let Some(d) = dirs.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("outptr_audit.rs") {
                files.push((p.clone(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    // (a handler's body by name: its file's, else its parent module's)
    let body_of = |file: &std::path::Path, name: &str| -> Option<Result<String, ()>> {
        let parent = file.parent().map(|d| d.with_extension("rs"));
        for f in std::iter::once(file.to_path_buf()).chain(parent) {
            let Some((_, s)) = files.iter().find(|x| x.0 == f) else { continue };
            for (k, _) in s.match_indices(&format!("fn {name}(")) {
                let head = &s[k + 3 + name.len()..];
                if head.starts_with("(_: &mut Ctx) -> u32") {
                    return Some(Err(()));
                }
                if let Some(b) = head.strip_prefix("(c: &mut Ctx) -> u32 {") {
                    return Some(Ok(b.lines().take(8).collect::<Vec<_>>().join("\n")));
                }
            }
        }
        None
    };
    let (mut bad, mut checked) = (vec![], 0);
    let mut check = |file: &std::path::Path, iface: &str, slot: usize, h: &str| {
        let Some((m, x64, x86)) = masks.get(&(iface.to_string(), slot)) else { return };
        let h = h.trim().trim_end_matches(" as Handler").trim_end_matches(" as crate::api::Handler");
        let h = h.strip_prefix("hr!(").and_then(|x| x.strip_suffix(')')).unwrap_or(h);
        let body = if h.starts_with('|') {
            Ok(h.to_string())
        } else {
            let name = h.rsplit("::").next().unwrap_or(h);
            match body_of(file, name) {
                Some(b) => b,
                None => return,
            }
        };
        checked += 1;
        let Ok(body) = body else { return };
        let typed: Vec<&str> = body.match_indices("c.args::<").filter_map(|(k, _)| body[k + 9..].split('>').next()).collect();
        let first = body.trim_start_matches(|c: char| c == '{' || c.is_whitespace()).starts_with("let c = &mut c.args::<") || h.starts_with('|');
        let ok = !typed.is_empty() && typed.iter().all(|t| of_type(t).is_some()) && (first || typed.len() > 1) && typed.iter().any(|t| of_type(t).as_ref() == Some(&(x64.clone(), x86.clone())));
        if !ok {
            bad.push(format!("{}: {iface}::{m} (slot {slot}): its handler {h} is to start `let c = &mut c.args::<crate::api::outs::iface::{iface}::{m}>();` (or a method's with the same out-parameters), it takes {typed:?}", file.display()));
        }
    };
    for (p, s) in &files {
        // com::impls!(IFoo { Method [@ slot]: handler, .. })
        for (k, _) in s.match_indices("impls!(") {
            let rest = &s[k + 7..];
            let iface: String = rest.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            let Some(open) = rest.find('{') else { continue };
            let Some(list) = bracketed(rest, open) else { continue };
            for e in top_level(&list[1..list.len() - 1]) {
                let Some((m, h)) = e.split_once(':') else { continue };
                let (m, at) = match m.split_once('@') {
                    Some((m, n)) => (m.trim(), n.trim().parse::<usize>().ok()),
                    None => (m.trim(), None),
                };
                let ss = match at {
                    Some(n) => vec![n],
                    None => slots.get(&(iface.clone(), m.to_string())).and_then(|v| v.first()).map(|&x| vec![x]).unwrap_or_default(),
                };
                for sl in ss {
                    check(p, &iface, sl, h);
                }
            }
        }
        // vtable("Name", &[(handler, bytes), ..]) or a list bound to a name before it
        for (k, _) in s.match_indices("vtable(") {
            if s[..k].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let Some(args) = bracketed(s, k + 6) else { continue };
            let a = top_level(&args[1..args.len() - 1]);
            let [name, list] = a.as_slice() else { continue };
            let names: Vec<&str> = name.split('"').skip(1).step_by(2).collect();
            let list = list.trim_start_matches('&').trim();
            let arr = if list.starts_with('[') {
                Some(list.to_string())
            } else if list.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                // (its binding: `let list[: T] = [` or `vec![`, the last before the call)
                s[..k].rmatch_indices(&format!("let {list}")).chain(s[..k].rmatch_indices(&format!("let mut {list}"))).map(|x| x.0).max().and_then(|b| {
                    let r = &s[b..k];
                    let eq = r.find('=')?;
                    let v = r[eq + 1..].trim_start().trim_start_matches('&').trim_start_matches("vec!");
                    let at = b + eq + 1 + (r[eq + 1..].len() - v.len());
                    (v.starts_with('[') && !r[..eq].contains(';') || v.starts_with('[')).then(|| bracketed(s, at).map(str::to_string)).flatten()
                })
            } else {
                None
            };
            let Some(arr) = arr else { continue };
            for n in names {
                let iface = n.split(['(']).next().unwrap_or(n).rsplit([' ', ':']).next().unwrap_or(n);
                if !ifaces.contains(iface) {
                    continue;
                }
                for (sl, e) in top_level(&arr[1..arr.len() - 1]).iter().enumerate() {
                    if let Some(inner) = e.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
                        let parts = top_level(inner);
                        if let Some(h) = parts.first() {
                            check(p, iface, sl, h);
                        }
                    }
                }
            }
        }
    }
    // (320 at 2026-11-16)
    assert!(checked > 250, "only {checked} COM methods found");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
