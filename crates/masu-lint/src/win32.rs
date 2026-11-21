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

/// ntdll's C runtime functions that behave unlike msvcrt's (sake-re/ntdll/crt-case.md,
/// crt-numbers.md): without an ntdll handler of their own, find_impl falls back to msvcrt's
/// (2026-11-20: ntdll!towlower was msvcrt's full Unicode one, _wtoi saturated, _itoa checked
/// its base); plus RtlAnsiCharToUnicodeChar, which toupper is made of (it had no handler: Wine's
/// toupper oracle read 0 for every character)
pub const NTDLL_OWN_CRT: &[&str] = &[
    "towlower", "towupper", "tolower", "toupper", "_wcslwr", "_wcsupr", "_wcsicmp", "_wcsnicmp", "iswctype", "iswascii",
    "isalpha", "isdigit", "isspace", "isupper", "islower", "isxdigit", "isalnum", "ispunct", "iscntrl", "isgraph", "isprint",
    "iswalpha", "iswdigit", "iswspace", "iswlower", "iswxdigit", "iswalnum", "iswgraph", "iswprint",
    "_itoa", "_ltoa", "_ultoa", "_i64toa", "_ui64toa", "_itow", "_ltow", "_ultow", "_i64tow", "_ui64tow",
    "atoi", "atol", "_wtoi", "_wtol", "_atoi64", "_wtoi64", "strtol", "strtoul", "wcstol", "wcstoul", "_wcstoi64", "_wcstoui64",
    "RtlAnsiCharToUnicodeChar",
    // (rtlstr.rs, 2026-11-21: without them GetProcAddress gave Wine's ntdll:rtlstr NULL or a stub
    // answering 0, and it hung)
    "RtlInitUnicodeStringEx", "RtlAppendAsciizToString", "RtlAppendStringToString", "RtlAppendUnicodeStringToString",
    "RtlAppendUnicodeToString", "RtlCopyString", "RtlUpperChar", "RtlUpperString", "RtlUpcaseUnicodeString",
    "RtlDowncaseUnicodeString", "RtlDuplicateUnicodeString", "RtlFindCharInUnicodeString", "RtlCompareUnicodeStrings",
    "RtlGUIDFromString", "RtlStringFromGUID", "RtlUTF8ToUnicodeN", "RtlUnicodeToUTF8N", "RtlHashUnicodeString",
    "RtlUnicodeStringToAnsiString", "RtlDowncaseUnicodeChar",
];

static NTDLL_IMPL: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"implement\(\s*(?:"ntdll"|d)\s*,\s*"([A-Za-z0-9_]+)""#).ok());
/// ntcrt.rs's `classes!(..)` (is*) and `wclasses!(..)` (isw*) lists
static CLASSES: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"\b(w?)classes!\(([^;]*)\);"#).ok());

/// `ntdll-own-crt`: every NTDLL_OWN_CRT name has an `implement("ntdll", name, ..)` (or `d` bound
/// to "ntdll", or the classes! list of ntcrt.rs)
pub fn ntdll_own_crt(tree: &Tree) -> Hits {
    let mut have = std::collections::BTreeSet::new();
    let files = tree.rs(&["crates/sake/src/"]);
    for p in &files {
        let code = tree.code(p, true);
        // (`d` counts only in a file that binds it to "ntdll")
        let binds_d = code.contains(r#"let d = "ntdll";"#);
        if let Some(re) = NTDLL_IMPL.as_ref() {
            for c in re.captures_iter(&code) {
                let whole = c.get(0).map_or("", |m| m.as_str());
                if !whole.contains("\"ntdll\"") && !binds_d {
                    continue;
                }
                if let Some(n) = c.get(1) {
                    have.insert(n.as_str().to_string());
                }
            }
        }
        if binds_d && let Some(re) = CLASSES.as_ref() {
            for c in re.captures_iter(&code) {
                let w = c.get(1).map_or("", |m| m.as_str());
                for n in c.get(2).map_or("", |m| m.as_str()).split(',').filter_map(|e| e.split('"').nth(1)) {
                    have.insert(format!("is{w}{n}"));
                }
            }
        }
    }
    let at = Rel::new("crates/sake/src/ntcrt.rs");
    NTDLL_OWN_CRT.iter().filter(|n| !have.contains(**n)).map(|n| (at.clone(), 1, format!("ntdll!{n} has no handler of its own: msvcrt's (or the unimplemented trap) would answer"))).collect()
}

/// `utf8-one-decoder`: `utf8_chunks(` in sake's code only in rtlstr.rs (Windows' UTF-8 rule)
pub fn utf8_one_decoder(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for p in tree.rs(&["crates/sake/src/"]) {
        if p.as_str() == "crates/sake/src/rtlstr.rs" {
            continue;
        }
        let code = tree.code(&p, false);
        for (at, _) in code.match_indices("utf8_chunks(") {
            out.push((p.clone(), line_of(&code, at), "a UTF-8 decoder of its own: Windows' rule is crate::rtlstr::utf8_units".to_string()));
        }
    }
    out
}
