//! Generates api::outs from src/outptrs.txt (itself generated from Microsoft's Win32 metadata by
//! tools/win32md): for each function with pointer-sized out-parameters ("dll name i,j... k,l...":
//! the x64 argument indexes, then the x86 stack dwords, which differ after a GUID or a double
//! passed by value), a type `outs::<Name>` whose `Outs::MASK` / `MASK32` have those bits set, and
//! `outs::mask(name)` for the dispatcher. Handlers named after such a function take their arguments through
//! `c.args::<outs::Name>()`, whose `a::<I>()` does not compile for an out-parameter and whose
//! `out::<I>()` does not compile for anything else (review 2026-11-16: `c.a(i)` and `Out::addr`
//! let a handler take a pointer-sized out-parameter as a plain u32).
use std::collections::BTreeMap;
use std::fmt::Write as _;

type Error = Box<dyn std::error::Error>;

/// The bits of "i,j...", or why it is not that.
fn bits(args: &str) -> Result<u64, String> {
    let mut mask = 0u64;
    for a in args.split(',') {
        let i: u32 = a.parse().map_err(|e| format!("{a}: not an argument index: {e}"))?;
        mask |= 1u64.checked_shl(i).ok_or(format!("argument {i} past the mask"))?;
    }
    Ok(mask)
}

/// One line's (name, x64 mask, x86 mask), or why it is not one.
fn parse(line: &str) -> Result<(&str, u64, u64), String> {
    let mut f = line.split(' ');
    let (Some(_dll), Some(name), Some(x64), Some(x86), None) = (f.next(), f.next(), f.next(), f.next(), f.next()) else {
        return Err(format!("not \"dll name i,j... k,l...\": {line}"));
    };
    if !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(format!("{name}: not a name"));
    }
    Ok((name, bits(x64)?, bits(x86)?))
}

/// src/com_outptrs.txt (tools/win32md: the interfaces sake names): `com(iface, slot)`, a COM
/// method's pointer-sized out-parameters as `mask` gives a function's (`this` argument 0),
/// `com_dwords(iface)`, its vtable's methods' x86 stack dwords, which sake's vtables are
/// checked against, and for each method with such parameters a type
/// `iface::<Interface>::<Method>` (`<Method>_<slot>` for a second method of one name) whose
/// Outs are its, which its handler takes its arguments as.
fn com(o: &mut String) -> Result<(), Error> {
    println!("cargo::rerun-if-changed=src/com_outptrs.txt");
    let text = std::fs::read_to_string("src/com_outptrs.txt")?;
    let mut masks = BTreeMap::<(&str, u32), (&str, u64, u64)>::new();
    let mut dwords = BTreeMap::<&str, &str>::new();
    for (n, line) in (1u32..).zip(text.lines()) {
        let at = |e: String| format!("src/com_outptrs.txt:{n}: {e}");
        let f: Vec<&str> = line.split(' ').collect();
        let name = f.first().copied().unwrap_or_default();
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(at(format!("{name}: not a name")).into());
        }
        match f.as_slice() {
            [_, "#", d] if d.split(',').all(|x| x.parse::<u8>().is_ok()) => {
                dwords.insert(name, d);
            }
            [_, slot, method, x64, x86] if method.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') => {
                let slot: u32 = slot.parse().map_err(|e| at(format!("{slot}: not a slot: {e}")))?;
                masks.insert((name, slot), (method, bits(x64).map_err(at)?, bits(x86).map_err(at)?));
            }
            _ => return Err(at(format!("not \"iface slot method i,j... k,l...\" nor \"iface # d,d...\": {line}")).into()),
        }
    }
    for (name, _) in masks.keys() {
        if !dwords.contains_key(name) {
            return Err(format!("src/com_outptrs.txt: {name} has out-parameters and no vtable").into());
        }
    }
    *o += "/// The pointer-sized out-parameters of the COM method `slot` of `iface` (bit i: argument i,\n/// `this` 0), none when it has none or the metadata has no such interface.\npub fn com(iface: &str, slot: u32) -> super::OutMask {\n    match (iface, slot) {\n";
    for ((name, slot), (_, x64, x86)) in &masks {
        writeln!(o, "        (\"{name}\", {slot}) => super::OutMask {{ x64: {x64:#x}, x86: {x86:#x} }},")?;
    }
    *o += "        _ => super::OutMask::NONE,\n    }\n}\n";
    *o += "/// The name of the COM method `slot` of `iface` where it has pointer-sized out-parameters.\npub fn com_name(iface: &str, slot: u32) -> Option<&'static str> {\n    match (iface, slot) {\n";
    for ((name, slot), (method, _, _)) in &masks {
        writeln!(o, "        (\"{name}\", {slot}) => Some(\"{method}\"),")?;
    }
    *o += "        _ => None,\n    }\n}\n";
    *o += "/// Each COM method's arguments with pointer-sized out-parameters, by interface: its handler's\n/// `c.args::<outs::iface::IFoo::Method>()`.\n#[allow(non_snake_case, non_camel_case_types, dead_code)]\npub mod iface {\n";
    let mut open: Option<&str> = None;
    let mut seen = std::collections::BTreeSet::<&str>::new();
    for ((name, slot), (method, x64, x86)) in &masks {
        if open != Some(*name) {
            if open.is_some() {
                *o += "    }\n";
            }
            writeln!(o, "    pub mod {name} {{")?;
            open = Some(name);
            seen.clear();
        }
        let ty = if seen.insert(method) { (*method).to_string() } else { format!("{method}_{slot}") };
        writeln!(o, "        pub struct {ty};\n        impl super::super::super::Outs for {ty} {{\n            const MASK: u64 = {x64:#x};\n            const MASK32: u64 = {x86:#x};\n        }}")?;
    }
    if open.is_some() {
        *o += "    }\n";
    }
    *o += "}\n";
    *o += "/// The vtable of `iface` as the metadata has it: each method's x86 stack dwords, `this` and a\n/// hidden result pointer counted.\npub fn com_dwords(iface: &str) -> Option<&'static [u8]> {\n    match iface {\n";
    for (name, d) in &dwords {
        writeln!(o, "        \"{name}\" => Some(&[{d}]),")?;
    }
    *o += "        _ => None,\n    }\n}\n";
    Ok(())
}

fn main() -> Result<(), Error> {
    println!("cargo::rerun-if-changed=src/outptrs.txt");
    let text = std::fs::read_to_string("src/outptrs.txt")?;
    // (by name: a function some DLLs share, as the forwarders do, has one signature)
    let mut by_name = BTreeMap::<&str, (u64, u64)>::new();
    for (n, line) in (1u32..).zip(text.lines()) {
        let (name, x64, x86) = parse(line).map_err(|e| format!("src/outptrs.txt:{n}: {e}"))?;
        let m = by_name.entry(name).or_default();
        m.0 |= x64;
        m.1 |= x86;
    }
    let mut o = String::from("// Generated by build.rs from src/outptrs.txt: do not edit.\n");
    for (name, (x64, x86)) in &by_name {
        writeln!(o, "#[allow(non_camel_case_types, dead_code)]\npub struct {name};\nimpl super::Outs for {name} {{\n    const MASK: u64 = {x64:#x};\n    const MASK32: u64 = {x86:#x};\n}}")?;
    }
    o += "/// The pointer-sized out-parameters of the function `name` (bit i: argument i), none when it has none.\npub fn mask(name: &str) -> super::OutMask {\n    match name {\n";
    for (name, (x64, x86)) in &by_name {
        writeln!(o, "        \"{name}\" => super::OutMask {{ x64: {x64:#x}, x86: {x86:#x} }},")?;
    }
    o += "        _ => super::OutMask::NONE,\n    }\n}\n";
    com(&mut o)?;
    let dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR")?);
    std::fs::write(dir.join("outs.rs"), o)?;
    std::fs::write(dir.join("layouts.rs"), layouts()?)?;
    Ok(())
}

/// The headers' structure layouts (crates/apitrace/src/layouts.txt, tests/replay/layouts.py:
/// clang's record layouts of mingw-w64's headers for i686 and x86_64) as abi::Off constants,
/// one module per record: `SIZE` and each member by its name, a member of a nested record as
/// `outer_inner` (review 6852be1f: D2D built DXGI_SWAP_CHAIN_DESC with x86's offsets on x64).
fn layouts() -> Result<String, Error> {
    const PATH: &str = "../apitrace/src/layouts.txt";
    println!("cargo::rerun-if-changed={PATH}");
    let text = std::fs::read_to_string(PATH)?;
    let ident = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s.starts_with(|c: char| c.is_ascii_digit());
    // (name -> (size32, size64, members (off32, off64, name, type)))
    let mut recs: BTreeMap<String, ((u32, u32), Vec<(u32, u32, String, String)>)> = BTreeMap::new();
    let mut cur: Option<String> = None;
    for (n, line) in (1u32..).zip(text.lines()) {
        let bad = || format!("{PATH}:{n}: {line:?}");
        if let Some(rest) = line.strip_prefix("R ") {
            let w: Vec<&str> = rest.split(' ').collect();
            let [name, s32, s64, ..] = w[..] else { return Err(bad().into()) };
            recs.insert(name.into(), ((s32.parse().map_err(|_| bad())?, s64.parse().map_err(|_| bad())?), Vec::new()));
            cur = Some(name.into());
        } else if let Some(rest) = line.strip_prefix(' ') {
            let Some(r) = cur.as_ref().and_then(|c| recs.get_mut(c)) else { continue };
            let w: Vec<&str> = rest.split(' ').collect();
            if let [o32, o64, name, ty] = w[..] {
                r.1.push((o32.parse().map_err(|_| bad())?, o64.parse().map_err(|_| bad())?, name.into(), ty.into()));
            }
        } else {
            cur = None;
        }
    }
    // (a member named as a Rust keyword: a raw identifier)
    const KEYWORDS: [&str; 38] = ["as", "async", "await", "break", "const", "continue", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "static", "struct", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do"];
    let raw = |s: &str| if KEYWORDS.contains(&s) { format!("r#{s}") } else { s.to_owned() };
    let mut o = String::from("// Generated by build.rs from crates/apitrace/src/layouts.txt: do not edit.\n");
    for (name, ((s32, s64), members)) in &recs {
        if !ident(name) {
            continue;
        }
        writeln!(o, "#[allow(non_snake_case, non_upper_case_globals, dead_code)]\npub mod {name} {{\n    use crate::abi::Off;\n    pub const SIZE: Off = Off::new({s32}, {s64});")?;
        for (o32, o64, m, ty) in members {
            if !ident(m) || m == "SIZE" || ["self", "super", "crate", "Self"].contains(&m.as_str()) {
                continue;
            }
            writeln!(o, "    pub const {}: Off = Off::new({o32}, {o64});", raw(m))?;
            // (one level into a nested record: its members at their place in this one)
            if let Some(inner) = ty.strip_prefix("S:").and_then(|t| recs.get(t)) {
                for (i32_, i64_, im, _) in &inner.1 {
                    if ident(im) {
                        writeln!(o, "    pub const {m}_{im}: Off = Off::new({}, {});", o32.saturating_add(*i32_), o64.saturating_add(*i64_))?;
                    }
                }
            }
        }
        o += "}\n";
    }
    Ok(o)
}
