//! Rust 1.97..1.100 の機能で型や設定に移した決まり (2026-11-21、こた「Rust 1.100 の新機能を全部
//! 入れて、変更箇所の退行がないように自作リンタも」)。基準値のない決まり: 例外は
//! tools/ci/arch-lint-sites.tsv に理由つきで書く。
//!
//! * `rust-lints`: crate の Cargo.toml はどれも `raw_borrows_via_references` (1.99) を持つ
//!   (`&x as *const T` は先に参照を作り、その別名規則を負う: `&raw const x`、`Box::as_ptr`)。
//! * `bin-dead-pub`: バイナリの根 (src/main.rs、src/bin/*.rs) は
//!   `#![cfg_attr(not(test), warn(dead_code_pub_in_binary))]` (1.97) を持つ: crate の階層でしか
//!   効かないので Cargo.toml でなく main に書く。テストは多くを使うので外す。
//! * `hand-bit-width`: `32 - x.leading_zeros()` を手で書かない: `x.bit_width()` (1.100)。
//! * `hand-utf16`: UTF-16LE のバイト列を `chunks_exact(2)` と `u16::from_le_bytes` で手で組まない:
//!   kome::le の units16 / utf16_lossy / utf16z_lossy (`String::from_utf16le_lossy`, 1.100)。
//! * `noreturn-handler`: 戻らない Windows API (ExitProcess、exit、abort、_assert..) の sake の
//!   実装は `fn(&mut Ctx) -> !` で、`noreturn!` で登録する (api::NoReturn が型で確かめる): 戻り値
//!   を返す形に戻すと、戻ったあとのゲストが落ちる道を型が止めない。
//! * `wildcard-unreachable`: テストの外で `_ => unreachable!(..)` の腕を書かない。来ない値は型で
//!   消す (enum、`!`、表を const で作る)。
use std::sync::LazyLock;

use regex::Regex;

use super::{Hits, Rel, Tree, rx, test_mod_files};

static BITS: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\b(?:8|16|32|64|128)\s*-\s*[\w.()\[\]&*| ]*?\.leading_zeros\s*\(\s*\)"));
static UTF16: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?s)chunks_exact\s*\(\s*2\s*\)[^;]{0,200}?u16\s*::\s*from_le_bytes|u16\s*::\s*from_le_bytes[^;]{0,120}?chunks_exact\s*\(\s*2\s*\)"));

#[cfg(test)]
pub(super) fn regexes() -> [&'static LazyLock<Option<Regex>>; 2] {
    [&BITS, &UTF16]
}

/// the one home of the UTF-16 readers
pub const LE: &str = "crates/kome/src/le.rs";

/// The sake handlers that do not come back (Windows documents each as not returning), by file:
/// each is `fn NAME(c: &mut Ctx) -> !`.
pub const NORETURN: &[(&str, &[&str])] = &[
    ("crates/sake/src/k32.rs", &["ExitProcess", "FatalAppExitA", "ExitThread", "_endthreadex", "_endthread", "FreeLibraryAndExitThread"]),
    ("crates/sake/src/crt.rs", &["exit", "_exit", "abort", "_purecall", "_invalid_parameter_noinfo_noreturn", "terminate"]),
    ("crates/sake/src/crt_io.rs", &["_assert", "_wassert", "_amsg_exit"]),
];

/// the Rust files rules read, test files and the lint's own fixtures left out
fn sources(tree: &Tree) -> Vec<Rel> {
    tree.rs(&["crates/", "tools/", "fuzz/", "host/", "tests/"]).into_iter().filter(|f| !f.as_str().starts_with("tools/lint-fixtures/")).collect()
}

/// a crate's manifest (it has a [package]) outside vendor/ and the lint's fixtures
fn manifests(tree: &Tree) -> Vec<Rel> {
    tree.paths()
        .iter()
        .filter(|p| p.as_str() == "Cargo.toml" || p.as_str().ends_with("/Cargo.toml"))
        .filter(|p| !p.as_str().starts_with("vendor/") && !p.as_str().starts_with("tools/lint-fixtures/"))
        .filter(|p| tree.text(p).lines().any(|l| l.trim() == "[package]"))
        .cloned()
        .collect()
}

pub(super) fn rust_lints(tree: &Tree) -> Hits {
    manifests(tree)
        .into_iter()
        .filter(|m| {
            !tree.text(m).lines().map(|l| l.split('#').next().unwrap_or_default().replace(' ', "")).any(|l| l.starts_with("raw_borrows_via_references=") && !l.contains("\"allow\""))
        })
        .map(|m| (m, 1, "[lints.rust] raw_borrows_via_references = \"warn\" (Rust 1.99) is missing".to_owned()))
        .collect()
}

/// a binary's root: src/main.rs, src/bin/x.rs
fn is_bin_root(p: &str) -> bool {
    p == "src/main.rs" || p.ends_with("/src/main.rs") || p.split_once("/src/bin/").is_some_and(|(_, f)| !f.contains('/') && f.ends_with(".rs"))
}

/// a crate attribute `#![..]` that warns (denies, forbids) dead_code_pub_in_binary
fn enables(file: &syn::File) -> bool {
    file.attrs.iter().filter(|a| matches!(a.style, syn::AttrStyle::Inner(_))).any(|a| {
        let t: String = quote::ToTokens::to_token_stream(&a.meta).to_string();
        let words: Vec<&str> = t.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
        words.contains(&"dead_code_pub_in_binary") && words.iter().any(|w| matches!(*w, "warn" | "deny" | "forbid")) && !words.iter().any(|w| matches!(*w, "allow" | "expect"))
    })
}

pub(super) fn bin_dead_pub(tree: &Tree) -> Hits {
    tree.paths()
        .iter()
        .filter(|p| p.is_rust() && is_bin_root(p.as_str()) && !p.as_str().starts_with("vendor/") && !p.as_str().starts_with("tools/lint-fixtures/"))
        .filter(|p| tree.syntax(p).is_some_and(|f| !enables(&f)))
        .map(|p| (p.clone(), 1, "`#![cfg_attr(not(test), warn(dead_code_pub_in_binary))]` is missing".to_owned()))
        .collect()
}

/// the matches of `r` in each file's code (comments and strings blanked)
fn matches(tree: &Tree, files: &[Rel], r: &LazyLock<Option<Regex>>, what: &str) -> Hits {
    let Some(r) = r.as_ref() else { return Hits::new() };
    let mut out = Hits::new();
    for f in files {
        let code = tree.code(f, false);
        for m in r.find_iter(&code) {
            out.push((f.clone(), crate::code::line_of(&code, m.start()), format!("{what}: `{}`", m.as_str().split_whitespace().collect::<Vec<_>>().join(" "))));
        }
    }
    out
}

pub(super) fn hand_bit_width(tree: &Tree) -> Hits {
    matches(tree, &sources(tree), &BITS, "write `.bit_width()` (Rust 1.100)")
}

pub(super) fn hand_utf16(tree: &Tree) -> Hits {
    let files: Vec<Rel> = sources(tree).into_iter().filter(|f| f.as_str() != LE).collect();
    matches(tree, &files, &UTF16, "read UTF-16LE with kome::le::units16 / utf16_lossy / utf16z_lossy")
}

pub(super) fn noreturn_handler(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (file, names) in NORETURN {
        let f = Rel::new(file);
        if tree.is_made_up() && !tree.has(&f) {
            continue;
        }
        let Some(syntax) = tree.syntax(&f) else {
            out.push((f, 1, "does not parse (or is gone): its noreturn handlers are not found".to_owned()));
            continue;
        };
        for name in *names {
            let sig = syntax.items.iter().find_map(|i| if let syn::Item::Fn(g) = i && g.sig.ident == name { Some(&g.sig) } else { None });
            match sig {
                Some(s) if matches!(&s.output, syn::ReturnType::Type(_, t) if matches!(**t, syn::Type::Never(_))) => {}
                Some(s) => out.push((f.clone(), crate::syntax::line(s.ident.span()), format!("{name} does not come back: `fn {name}(c: &mut Ctx) -> !`, registered with noreturn!"))),
                None => out.push((f.clone(), 1, format!("{name} (NORETURN) is not a fn of this file: move it to its file in modern::NORETURN"))),
            }
        }
    }
    out
}

struct Wild(Vec<usize>);

/// `unreachable!(..)` / `core::unreachable!` / a block of only that
fn is_unreachable(e: &syn::Expr) -> bool {
    let named = |m: &syn::Macro| m.path.segments.last().is_some_and(|s| s.ident == "unreachable");
    if let syn::Expr::Macro(m) = e {
        return named(&m.mac);
    }
    let syn::Expr::Block(b) = e else { return false };
    if let [syn::Stmt::Macro(m)] = b.block.stmts.as_slice() {
        return named(&m.mac);
    }
    matches!(b.block.stmts.as_slice(), [syn::Stmt::Expr(e, _)] if is_unreachable(e))
}

impl<'a> syn::visit::Visit<'a> for Wild {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !crate::syntax::is_cfg_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_arm(&mut self, a: &'a syn::Arm) {
        if matches!(a.pat, syn::Pat::Wild(_)) && a.guard.is_none() && is_unreachable(&a.body) {
            self.0.push(crate::syntax::line(a.fat_arrow_token.spans[0]));
        }
        syn::visit::visit_arm(self, a);
    }
}

pub(super) fn wildcard_unreachable(tree: &Tree) -> Hits {
    let files = sources(tree);
    let tests = test_mod_files(tree, &files);
    let mut out = Hits::new();
    for f in files.iter().filter(|f| !tests.contains(*f)) {
        let Some(file) = tree.syntax(f) else { continue };
        let mut w = Wild(Vec::new());
        syn::visit::Visit::visit_file(&mut w, &file);
        out.extend(w.0.into_iter().map(|l| (f.clone(), l, "`_ => unreachable!()`: take the value that cannot come out by a type (an enum, `!`, a const-built table)".to_owned())));
    }
    out
}

/// `let x = heap_alloc(..);` and, later in the same block, the statement `heap_free(x);`
struct Pairs(Vec<(usize, String)>);

/// the last segment of a call's function path (`mem::heap_alloc(..)` -> heap_alloc)
fn called(e: &syn::Expr) -> Option<(String, &syn::punctuated::Punctuated<syn::Expr, syn::token::Comma>)> {
    let syn::Expr::Call(c) = e else { return None };
    let syn::Expr::Path(p) = &*c.func else { return None };
    Some((p.path.segments.last()?.ident.to_string(), &c.args))
}

impl<'a> syn::visit::Visit<'a> for Pairs {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !crate::syntax::is_cfg_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_block(&mut self, b: &'a syn::Block) {
        for (k, s) in b.stmts.iter().enumerate() {
            let syn::Stmt::Local(l) = s else { continue };
            let (syn::Pat::Ident(name), Some(init)) = (&l.pat, &l.init) else { continue };
            if called(&init.expr).is_none_or(|(f, _)| f != "heap_alloc") {
                continue;
            }
            let freed = b.stmts.iter().skip(k.saturating_add(1)).any(|t| {
                let syn::Stmt::Expr(e, Some(_)) = t else { return false };
                called(e).is_some_and(|(f, args)| f == "heap_free" && args.len() == 1 && matches!(args.first(), Some(syn::Expr::Path(p)) if p.path.is_ident(&name.ident)))
            });
            if freed {
                self.0.push((crate::syntax::line(name.ident.span()), name.ident.to_string()));
            }
        }
        syn::visit::visit_block(self, b);
    }
}

/// the one home of heap_alloc / heap_free and Scratch
pub const MEM: &str = "crates/sake/src/mem.rs";

pub(super) fn guest_scratch(tree: &Tree) -> Hits {
    let files: Vec<Rel> = tree.rs(&["crates/sake/"]).into_iter().filter(|f| f.as_str() != MEM).collect();
    let tests = test_mod_files(tree, &files);
    let mut out = Hits::new();
    for f in files.iter().filter(|f| !tests.contains(*f)) {
        let Some(file) = tree.syntax(f) else { continue };
        let mut v = Pairs(Vec::new());
        syn::visit::Visit::visit_file(&mut v, &file);
        out.extend(v.0.into_iter().map(|(l, n)| (f.clone(), l, format!("`{n}` is freed by hand in its own block: `let {n}_own = mem::Scratch::new(..); let {n} = {n}_own.get();` frees it on every way out"))));
    }
    out
}

// --- hand-std: the shapes Rust 1.98..1.100 named (2026-11-21) ---------------------------------

static TOGGLE: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?:^|[^\w.])([\w.]+(?:\[[^\]\n]+\])?)\s*=\s*!\s*([\w.]+(?:\[[^\]\n]+\])?)\s*;"));
static TRIM: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?:^|[^\w.])([\w.]+)\.strip_prefix\([^()]*\)\s*\.unwrap_or\(\s*([\w.]+)\s*\)"));
static MAP_OR_DEFAULT: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\.map_or\(\s*(?:Default::default\(\)|String::new\(\)|Vec::new\(\))\s*,"));
static OK_OR: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\bif [^{};\n]+ \{\s*Ok\(\(\)\)\s*\}\s*else\s*\{\s*Err\("));
static FROM_FN: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\(0\.\.[\w.()]*\.len\(\)\)\.map\(\|\w+\|"));

#[cfg(test)]
pub(super) fn std_regexes() -> [&'static LazyLock<Option<Regex>>; 5] {
    [&TOGGLE, &TRIM, &MAP_OR_DEFAULT, &OK_OR, &FROM_FN]
}

/// the byte of the `)` closing the `(` at `open` (None: unbalanced)
fn closing(code: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in code.get(open..)?.char_indices() {
        match c {
            '(' | '[' | '{' => depth = depth.saturating_add(1),
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open.saturating_add(i));
                }
            }
            _ => {}
        }
    }
    None
}

/// the hand-written shapes of one file's code: (byte, what to write)
fn hand_std_in(code: &str) -> Vec<(usize, &'static str)> {
    let mut out = Vec::new();
    if let Some(r) = TOGGLE.as_ref() {
        out.extend(r.captures_iter(code).filter(|c| c.get(1).map(|m| m.as_str()) == c.get(2).map(|m| m.as_str())).filter_map(|c| c.get(1)).map(|m| (m.start(), "`x = !x` is `x.toggle()` (Rust 1.100)")));
    }
    if let Some(r) = TRIM.as_ref() {
        out.extend(r.captures_iter(code).filter(|c| c.get(1).map(|m| m.as_str()) == c.get(2).map(|m| m.as_str())).filter_map(|c| c.get(1)).map(|m| (m.start(), "`s.strip_prefix(p).unwrap_or(s)` is `s.trim_prefix(p)` (Rust 1.100)")));
    }
    if let Some(r) = MAP_OR_DEFAULT.as_ref() {
        out.extend(r.find_iter(code).map(|m| (m.start(), "`.map_or(Default::default(), f)` is `.map_or_default(f)` (Rust 1.98)")));
    }
    // (`.map(f).unwrap_or_default()`: the map's parentheses balanced by hand, a regex cannot)
    for (at, _) in code.match_indices(".map(") {
        let Some(close) = closing(code, at.saturating_add(4)) else { continue };
        if code.get(close.saturating_add(1)..).is_some_and(|rest| rest.trim_start().starts_with(".unwrap_or_default()")) {
            out.push((at, "`.map(f).unwrap_or_default()` is `.map_or_default(f)` (Rust 1.98)"));
        }
    }
    if let Some(r) = OK_OR.as_ref() {
        out.extend(r.find_iter(code).map(|m| (m.start(), "`if c { Ok(()) } else { Err(e) }` is `c.ok_or(e)` / `c.ok_or_else(|| e)` (Rust 1.98)")));
    }
    if let Some(r) = FROM_FN.as_ref() {
        for m in r.find_iter(code) {
            // (the `(` of the `.map(` the match ends in)
            let Some(open) = code.get(..m.end()).and_then(|s| s.rfind(".map(")).map(|k| k.saturating_add(4)) else { continue };
            let Some(close) = closing(code, open) else { continue };
            if code.get(close.saturating_add(1)..).is_some_and(|rest| rest.trim_start().starts_with(".collect::<Vec<")) {
                out.push((m.start(), "`(0..v.len()).map(f).collect::<Vec<_>>()` is `Vec::from_fn(v.len(), f)` (Rust 1.100)"));
            }
        }
    }
    out
}

pub(super) fn hand_std(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for f in sources(tree) {
        let code = tree.code(&f, false);
        out.extend(hand_std_in(&code).into_iter().map(|(at, what)| (f.clone(), crate::code::line_of(&code, at), what.to_owned())));
    }
    out
}
