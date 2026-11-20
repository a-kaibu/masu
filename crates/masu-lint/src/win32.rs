//! Rules on how sake's handlers read Win32 arguments.
//!
//! - `boolean-arg`: a BOOLEAN (UCHAR) parameter is its low byte only; the rest of the stack slot
//!   or register is whatever the caller left there. A handler reads it with `c.boolean(i)`, never
//!   `c.a(i)` (RtlAnsiStringToUnicodeString read `c.a(2) != 0`, 2026-11-20: Windows' ntdll tests
//!   the low byte, sake-re/ntdll/rtl-integer-strings.md). BOOLEAN_ARGS lists the parameters, from
//!   their Microsoft Learn pages; a handler registered under another name is followed through
//!   `implement("dll", "Name", handler)`.
use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::code::line_of;
use crate::tree::{Rel, Tree};

type Hits = Vec<(Rel, usize, String)>;

/// (function, the 0-based index of a BOOLEAN parameter)
pub const BOOLEAN_ARGS: &[(&str, u32)] = &[
    ("RtlAnsiStringToUnicodeString", 2),
    ("RtlUnicodeStringToAnsiString", 2),
    ("RtlOemStringToUnicodeString", 2),
    ("RtlUnicodeStringToOemString", 2),
    ("RtlUpcaseUnicodeString", 2),
    ("RtlDowncaseUnicodeString", 2),
    ("RtlUpcaseUnicodeStringToAnsiString", 2),
    ("RtlUpcaseUnicodeStringToOemString", 2),
    ("RtlEqualUnicodeString", 2),
    ("RtlCompareUnicodeString", 2),
    ("RtlEqualString", 2),
    ("RtlCompareString", 2),
    ("RtlPrefixUnicodeString", 2),
    ("RtlPrefixString", 2),
    ("RtlHashUnicodeString", 1),
    ("RtlCompareUnicodeStrings", 4),
    ("RtlAdjustPrivilege", 1),
    ("RtlAdjustPrivilege", 2),
    ("NtWaitForSingleObject", 1),
    ("ZwWaitForSingleObject", 1),
    ("NtWaitForMultipleObjects", 3),
    ("NtDelayExecution", 0),
    ("ZwDelayExecution", 0),
    ("NtCreateEvent", 4),
];

static IMPLEMENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"implement\(\s*"[a-z0-9_.]+"\s*,\s*"([A-Za-z0-9_]+)"\s*,\s*(?:crate::api::handler\()?([A-Za-z0-9_:]+)"#).ok());
static FN: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub(?:\([a-z]+\))?\s+)?fn\s+([A-Za-z0-9_]+)\s*\(\s*c\s*:\s*&mut\s+(?:crate::api::)?Ctx\s*\)").ok());

/// the handler functions of each BOOLEAN parameter: its own name and the ones `implement` names
fn handlers(tree: &Tree, files: &[Rel]) -> BTreeMap<String, Vec<u32>> {
    let mut m: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    for (f, i) in BOOLEAN_ARGS {
        m.entry((*f).to_string()).or_default().push(*i);
    }
    let Some(re) = IMPLEMENT.as_ref() else { return m };
    for p in files {
        let code = tree.code(p, true);
        for c in re.captures_iter(&code) {
            let (Some(api), Some(h)) = (c.get(1), c.get(2)) else { continue };
            let h = h.as_str().rsplit("::").next().unwrap_or_default().to_string();
            for (f, i) in BOOLEAN_ARGS {
                if *f == api.as_str() && h != api.as_str() {
                    m.entry(h.clone()).or_default().push(*i);
                }
            }
        }
    }
    m
}

pub fn boolean_arg(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let files = tree.rs(&["crates/sake/src/"]);
    let want = handlers(tree, &files);
    let Some(fnre) = FN.as_ref() else { return out };
    for p in &files {
        let code = tree.code(p, false);
        let starts: Vec<(usize, String)> = fnre.captures_iter(&code).filter_map(|c| Some((c.get(0)?.start(), c.get(1)?.as_str().to_string()))).collect();
        for (k, (start, name)) in starts.iter().enumerate() {
            let Some(idx) = want.get(name) else { continue };
            let end = starts.get(k.saturating_add(1)).map_or(code.len(), |s| s.0);
            let body = code.get(*start..end).unwrap_or_default();
            for i in idx {
                let raw = format!("c.a({i})");
                let mut from = 0;
                while let Some(at) = body.get(from..).and_then(|b| b.find(&raw)) {
                    let pos = start.saturating_add(from).saturating_add(at);
                    out.push((p.clone(), line_of(&code, pos), format!("fn {name}: {raw} reads the BOOLEAN argument {i} whole: c.boolean({i})")));
                    from = from.saturating_add(at).saturating_add(raw.len());
                }
            }
        }
    }
    out
}
