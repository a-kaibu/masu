//! The rules against writing an escape down instead of the behavior (user rule 2026-11-20:
//! no comment may excuse code from doing what Windows does).
//!
//! - `escape-comment`: a comment (or, in Rust outside tests, a string literal such as a log
//!   line) that names a shortcut: a to-do marker, "for now", a simplification, "not implemented",
//!   "unsupported", an approximation, "best effort", a stub that answers anything, a placeholder
//!   answer, in English or Japanese. "not supported" / "unsupported" / an approximation / an
//!   ignored flag pass only when the same comment says the answer is Windows' (winref, a golden,
//!   MEASURED, Wine's tests, the spec, the CPU's): then it is what Windows does, not a gap.
//! - `constant-handler`: a sake API handler that never reads its arguments (`_c: &mut Ctx`) and
//!   returns a constant: an empty success (or failure) unless its doc says which Windows answer
//!   it is.
//!
//! Each site already there is in tools/ci/arch-lint-sites.tsv as `escape:` work to do; the list
//! only goes down.
use std::sync::LazyLock;

use regex::Regex;

use crate::code::{code_only, line_of};
use crate::tree::{Rel, Tree};

type Hits = Vec<(Rel, usize, String)>;

/// words that are an escape whatever else the comment says
static ALWAYS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(?:TO", r"DO|FIX", r"ME|XX", r"X|HA", r"CK)\b",
        r"|(?i:\bfor\s+now\b|\bfor\s+the\s+moment\b|\bin\s+the\s+meantime\b)",
        r"|\bsimplif(?:y|ied|ies|ication|ications)\b|\bsimpler\s+(?:version|form|model)\b",
        r"|(?i:\bnot\s+(?:yet\s+)?(?:implemented|ported|modeled|modelled)\b|\bunimplemented\b|\bun-?implemented\b)",
        r"|(?i:\bbest[\s-]+effort\b|\bgood\s+enough\b|\bclose\s+enough\b|\bstubbed\b|\blogged\s+stub\b|\bstub\s+(?:returning|that\s+(?:logs|returns|fails|answers)))",
        r"|(?i:\bplaceholder\s+(?:value|answer|implementation|result)s?\b|\bhack(?:y|ish)?\b|\bworkaround\b|\bfake\s+(?:success|answer|result))",
        r"|\bonly\b[^.;:]{0,60}\bso\s+far\b",
        r"|とりあえず|簡略|未実装|未対応|暫定|ひとまず|手抜き|近似|仮実装|後で実装|あとで実装|ダミー実装",
    ))
    .ok()
});

/// words that are an escape unless the same comment gives Windows' evidence for the answer
static UNLESS_EVIDENCE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\bnot\s+(?:supported|handled)\b|\bunsupported\b|\bunhandled\s+(?:case|form|kind)s?\b",
        r"|(?i:\bapproximat(?:e|ed|es|ion|ions|ely)\b)",
        r"|\b(?:flags?|offsets?|attributes?|arguments?|parameters?|options?|the\s+rest)\s+(?:are\s+|is\s+)?ignored\b",
        r"|\b(?:taken|accepted),?\s+(?:and\s+)?(?:ignored|no\s+effect)\b|\baccepted,?\s+(?:and\s+)?(?:does\s+)?nothing\b",
    ))
    .ok()
});

/// what makes "not supported" Windows' answer and not a gap
static EVIDENCE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"Windows|winref|golden|MEASURED|[Mm]easured|Wine's test|\bas on\b|\bthe spec\b|functional spec|documented|MSDN|learn\.microsoft|\bx86\b|\bx87\b|\bfxc\b|\bCPUs?\b|vendor's|the hardware").ok()
});

/// the vocabulary itself, this rule's tests and the ratchet's fixtures (which mark to-do macros)
fn exempt(p: &str) -> bool {
    p.starts_with("vendor/") || p.contains("/testdata/") || p.starts_with("testdata/") || p.starts_with("tools/lint-fixtures/") || p == "crates/masu-lint/src/escape.rs" || p == "crates/masu-lint/src/escape_tests.rs"
}

const SLASH: [&str; 11] = [".rs", ".c", ".h", ".cpp", ".js", ".mjs", ".ts", ".tsx", ".wgsl", ".hlsl", ".fx"];
const HASH: [&str; 5] = [".py", ".sh", ".yml", ".yaml", ".toml"];

/// the comment text of each line of `text` (empty where there is none), by the file's kind
pub fn comments(path: &str, text: &str) -> Vec<String> {
    if SLASH.iter().any(|e| path.ends_with(e)) {
        let code = code_only(text, true);
        // (what code_only blanked and was not blank: comments and char literals)
        text.split('\n').zip(code.split('\n')).map(|(t, c)| t.chars().zip(c.chars().chain(std::iter::repeat(' '))).map(|(a, b)| if a != b { a } else { ' ' }).collect::<String>().trim().to_owned()).collect()
    } else if HASH.iter().any(|e| path.ends_with(e)) {
        text.split('\n').map(|l| hash_comment(l).unwrap_or_default().trim().to_owned()).collect()
    } else {
        vec![]
    }
}

/// a `#` comment outside quotes (not a shebang's `#!`, not a `${#x}` or `$#`)
fn hash_comment(l: &str) -> Option<&str> {
    let (mut q, mut prev) = (None::<char>, ' ');
    for (i, ch) in l.char_indices() {
        match q {
            Some(o) if ch == o && prev != '\\' => q = None,
            Some(_) => {}
            None if ch == '"' || ch == '\'' => q = Some(ch),
            None if ch == '#' && (prev.is_whitespace() || i == 0) && !l.get(i..).is_some_and(|r| r.starts_with("#!")) => return l.get(i..),
            None => {}
        }
        prev = ch;
    }
    None
}

/// the string literals of each line of a Rust file (empty where there is none)
fn rust_strings(text: &str) -> Vec<String> {
    let with = code_only(text, true);
    let without = code_only(text, false);
    with.split('\n').zip(without.split('\n')).map(|(a, b)| a.chars().zip(b.chars().chain(std::iter::repeat(' '))).map(|(x, y)| if x != y { x } else { ' ' }).collect::<String>().trim().to_owned()).collect()
}

fn hit(re: &LazyLock<Option<Regex>>, s: &str) -> bool {
    re.as_ref().is_some_and(|r| r.is_match(s))
}

/// whether comment `s` (one line, with the comment block it is in as `block`) is an escape
pub fn is_escape(s: &str, block: &str) -> bool {
    hit(&ALWAYS, s) || (hit(&UNLESS_EVIDENCE, s) && !hit(&EVIDENCE, block))
}

pub fn escape_comment(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for p in tree.paths() {
        let s = p.as_str();
        if exempt(s) {
            continue;
        }
        let text = tree.text(p);
        let lines = comments(s, &text);
        // (a comment block: consecutive commented lines, for the evidence)
        let mut k = 0;
        while k < lines.len() {
            let mut e = k;
            while e < lines.len() && lines.get(e).is_some_and(|l| !l.is_empty()) {
                e = e.saturating_add(1);
            }
            let block = lines.get(k..e).map(|b| b.join(" ")).unwrap_or_default();
            for (i, l) in lines.iter().enumerate().take(e).skip(k) {
                if is_escape(l, &block) {
                    out.push((p.clone(), i.saturating_add(1), l.clone()));
                }
            }
            k = e.saturating_add(1);
        }
        // (string literals: log lines and errors; not raw strings, the patterns a regex matches
        // against other tools' output, and not the lint's own names of lints)
        if p.is_rust() && !p.is_test_file() && !s.starts_with("crates/masu-lint/") {
            let strings = rust_strings(&text);
            for (i, l) in strings.iter().enumerate() {
                // (the evidence for a string: the comment lines above it, up to the item)
                let mut why = l.clone();
                let mut j = i;
                while let Some(k) = j.checked_sub(1).filter(|k| i.saturating_sub(*k) <= 12) {
                    match lines.get(k) {
                        Some(c) if !c.is_empty() => why.push_str(c),
                        _ => {}
                    }
                    j = k;
                }
                if !l.is_empty() && !l.starts_with("r\"") && !l.starts_with("r#") && is_escape(l, &why) && !hit(&ALWAYS_TEST_STRING, l) {
                    out.push((p.clone(), i.saturating_add(1), l.clone()));
                }
            }
        }
    }
    out
}

/// string literals that name the words without being an escape (FormatMessage's and the C
/// library's own texts, Windows' answers)
static ALWAYS_TEST_STRING: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"^"(?:Function not implemented|function not supported)"|"[^"]*\\r\\n"$"#).ok());

static CONST_FN: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?m)^[ \t]*(?:pub(?:\([a-z]+\))?\s+)?fn\s+(\w+)\s*\(\s*_c?\s*:\s*&mut\s+(?:crate::api::)?Ctx\s*\)\s*->\s*u(?:32|64)\s*\{\s*([A-Za-z_0-9:]+)\s*\}").ok());

pub fn constant_handler(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let Some(re) = CONST_FN.as_ref() else { return out };
    for p in tree.rs(&["crates/sake/src/"]) {
        let code = tree.code(&p, false);
        let text = tree.text(&p);
        let doc = comments(p.as_str(), &text);
        for m in re.captures_iter(&code) {
            let (Some(all), Some(name), Some(v)) = (m.get(0), m.get(1), m.get(2)) else { continue };
            let start = all.start().saturating_add(all.as_str().len().saturating_sub(all.as_str().trim_start().len()));
            let line = line_of(&code, start);
            // (its doc: the comment lines right above it)
            let mut above = Vec::new();
            let mut j = line.saturating_sub(1);
            while j > 0 && doc.get(j.saturating_sub(1)).is_some_and(|l| !l.is_empty()) {
                above.push(doc.get(j.saturating_sub(1)).cloned().unwrap_or_default());
                j = j.saturating_sub(1);
            }
            if !hit(&EVIDENCE, &above.join(" ")) {
                out.push((p.clone(), line, format!("fn {} returns {} whatever it is given", name.as_str(), v.as_str())));
            }
        }
    }
    out
}

/// a COM method's arm (by number, or sake's `Method { .. }`) that answers a constant
static CONST_ARM: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\s*(?:[0-9][0-9a-fx_ |.=]*|Method \{[^}]*\})\s*=>\s*(?:\{\s*\}|S_OK|0|1|E_NOTIMPL|E_FAIL|D3DERR_INVALIDCALL|return [A-Z_0-9]+;?)\s*,?\s*(//.*)?$").ok());

/// The files of the Direct3D objects' method tables: kura's d3d9 and d3d11 (not their
/// format tables) and sake's Direct3D 9 glue.
const METHOD_TABLES: [&str; 9] = [
    "crates/kura/src/d3d9/device.rs",
    "crates/kura/src/d3d9/d3d8.rs",
    "crates/kura/src/d3d9/mod.rs",
    "crates/kura/src/d3d11/mod.rs",
    "crates/kura/src/d3d11/d3d10.rs",
    "crates/kura/src/d3d11/deferred.rs",
    "crates/kura/src/d3d11/d2d.rs",
    "crates/sake/src/d3d9.rs",
    "crates/sake/src/d3d11.rs",
];

/// `constant-method`: a method of a Direct3D object that does nothing and answers a constant
/// (an empty arm, S_OK, 0, E_NOTIMPL...) whatever it is given, with no Windows evidence in its
/// comment or the comment lines right above it: the method's behavior is not there.
pub fn constant_method(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let Some(re) = CONST_ARM.as_ref() else { return out };
    for p in tree.rs(&METHOD_TABLES) {
        let text = tree.text(&p);
        let notes = comments(p.as_str(), &text);
        for (i, l) in text.split('\n').enumerate() {
            if !re.is_match(l) {
                continue;
            }
            // (the evidence: its own comment and the comment block right above)
            let mut why = notes.get(i).cloned().unwrap_or_default();
            let mut j = i;
            while let Some(k) = j.checked_sub(1).filter(|k| notes.get(*k).is_some_and(|c| !c.is_empty()) && text.split('\n').nth(*k).is_some_and(|t| t.trim_start().starts_with("//"))) {
                why.push(' ');
                why.push_str(notes.get(k).map_or("", String::as_str));
                j = k;
            }
            if !hit(&EVIDENCE, &why) {
                out.push((p.clone(), i.saturating_add(1), l.trim().to_owned()));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "escape_tests.rs"]
mod tests;
