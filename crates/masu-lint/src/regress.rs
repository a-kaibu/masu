//! Rules made from bugs that were fixed once (2026-11-20, 「これまでに色々直してきたと思うけど再発防止の
//! ためにリンター整備してな」): each names the fix that set it up. What a type could forbid was made a
//! type; these are what is left, a shape of code that has already been wrong once.
use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;

use super::{Hits, Rel, Tree, items, rx, test_mod_files};

// --- catch-unwind-abort -----------------------------------------------------------------------

/// what sake.wasm is built from: crates/sake and every crate it reaches through [dependencies]
pub(super) const ABORT_ROOT: &str = "crates/sake";

static SECTION: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"^\s*\[([^\]]+)\]\s*$"));
static PATH_DEP: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r#"path\s*=\s*"\.\./([A-Za-z0-9_-]+)""#));

/// the crate directories (`crates/x`) a crate's Cargo.toml reaches by path in a section the build
/// links: [dependencies], [target.'..'.dependencies], [dependencies.x] (not dev- or build-)
fn path_deps(toml: &str) -> Vec<String> {
    let (Some(sec), Some(dep)) = (SECTION.as_ref(), PATH_DEP.as_ref()) else { return vec![] };
    let mut linked = false;
    let mut out = Vec::new();
    for l in toml.lines() {
        if let Some(c) = sec.captures(l) {
            let name = c.get(1).map_or("", |m| m.as_str());
            let last = name.rsplit('.').find(|p| p.ends_with("dependencies")).unwrap_or(name);
            linked = (name.ends_with("dependencies") || name.contains("dependencies.")) && last == "dependencies";
            continue;
        }
        if linked && let Some(c) = dep.captures(l) {
            out.extend(c.get(1).map(|m| format!("crates/{}", m.as_str())));
        }
    }
    out
}

/// every crate linked into the panic=abort build (crates/sake/build.sh: `-C panic=abort`)
pub(super) fn abort_crates(tree: &Tree) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut todo = vec![ABORT_ROOT.to_owned()];
    while let Some(c) = todo.pop() {
        if seen.insert(c.clone()) {
            todo.extend(path_deps(&tree.text(&Rel(format!("{c}/Cargo.toml")))));
        }
    }
    seen
}

pub(super) fn catch_unwind_abort(tree: &Tree) -> Hits {
    let crates = abort_crates(tree);
    let prefixes: Vec<String> = crates.iter().map(|c| format!("{c}/")).collect();
    let prefixes: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    let files = tree.rs(&prefixes);
    let tests = test_mod_files(tree, &files);
    let files: Vec<Rel> = files.into_iter().filter(|f| !tests.contains(f)).collect();
    let mut out = Hits::new();
    for (f, it) in items(tree, &files) {
        let lines: BTreeSet<usize> = it.idents.iter().filter(|(i, _)| i == "catch_unwind").map(|(_, l)| *l).collect();
        out.extend(lines.into_iter().map(|l| (f.clone(), l, "catch_unwind in a crate sake.wasm links (panic=abort: it never catches)".into())));
    }
    out
}

// --- shell-special-env ------------------------------------------------------------------------

/// bash's own variables: an assignment is dropped (GROUPS, the ids), replaced at once (RANDOM,
/// SECONDS, LINENO...) or changes how the shell itself works (OPTIND, PWD); IFS is left out:
/// `IFS=$'\t' read` is how a line is split
pub const BASH_SPECIAL: [&str; 31] = [
    "GROUPS", "RANDOM", "SRANDOM", "SECONDS", "LINENO", "BASHPID", "BASHOPTS", "SHELLOPTS", "EUID", "UID", "PPID", "FUNCNAME", "EPOCHSECONDS", "EPOCHREALTIME", "HISTCMD", "DIRSTACK",
    "PIPESTATUS", "BASH_COMMAND", "BASH_SUBSHELL", "BASH_LINENO", "BASH_SOURCE", "BASH_ARGC", "BASH_ARGV", "BASH_VERSINFO", "HOSTTYPE", "OSTYPE", "MACHTYPE", "OPTIND", "OPTARG", "PWD", "OLDPWD",
];

static SH_SET: LazyLock<Option<Regex>> = LazyLock::new(|| rx(&format!(r"(?:^|[^\w$./-])({})(?:\[[^\]]*\])?\+?=", BASH_SPECIAL.join("|"))));
static YAML_KEY: LazyLock<Option<Regex>> = LazyLock::new(|| rx(&format!(r"^\s*({})\s*:(?:\s|$)", BASH_SPECIAL.join("|"))));

/// the shell scripts and workflows the rule reads
pub(super) fn is_shell(p: &Rel) -> bool {
    let s = p.as_str();
    !s.starts_with("vendor/") && (s.ends_with(".sh") || (s.starts_with(".github/workflows/") && (s.ends_with(".yml") || s.ends_with(".yaml"))))
}

/// `text`'s assignments to bash's own variables: (line, name), comments left out
pub fn special_sets(text: &str, yaml: bool) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (k, l) in text.lines().enumerate() {
        let code = l.split(" #").next().unwrap_or(l);
        if code.trim_start().starts_with('#') {
            continue;
        }
        let hit = SH_SET.as_ref().and_then(|r| r.captures(code)).or_else(|| if yaml { YAML_KEY.as_ref().and_then(|r| r.captures(code)) } else { None });
        if let Some(m) = hit.and_then(|c| c.get(1)) {
            out.push((k.saturating_add(1), m.as_str().to_owned()));
        }
    }
    out
}

pub(super) fn shell_special_env(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for p in tree.paths().iter().filter(|p| is_shell(p)) {
        let yaml = !p.as_str().ends_with(".sh");
        out.extend(special_sets(&tree.text(p), yaml).into_iter().map(|(l, n)| (p.clone(), l, format!("sets bash's own {n}"))));
    }
    out
}

// --- depth-guard ------------------------------------------------------------------------------

/// The depth limits fixes put on recursion over game and file input (review b00/b04/b07: damaged
/// traces, MSI conditions, SQL and formats, shaders, scripts, .x files and menus nesting without
/// end blew the stack; b87128be: Rust 1.100's larger frames overflowed hlslc's): (file, const).
/// Each stays declared and is still compared against in its file. A new limit goes here.
pub const DEPTH_GUARDS: [(&str, &str); 15] = [
    ("crates/apitrace/src/trace.rs", "MAX_DEPTH"),
    ("crates/hlslc/src/parse.rs", "MAX_NEST"),
    ("crates/hlslc/src/pp.rs", "MAX_IF_DEPTH"),
    ("crates/msidb/src/cond.rs", "MAX_DEPTH"),
    ("crates/msidb/src/sql.rs", "MAX_DEPTH"),
    ("crates/msidb/src/format.rs", "MAX_DEPTH"),
    ("crates/koji/src/jit/mod.rs", "MAX_DEPTH"),
    ("crates/koji/src/jit/long.rs", "MAX_DEPTH"),
    ("crates/sake/src/jscript.rs", "MAX_DEPTH"),
    ("crates/sake/src/xof.rs", "MAX_DEPTH"),
    ("crates/sake/src/user.rs", "MENU_DEPTH"),
    ("crates/sake/src/msi_install.rs", "MAX_ACTION_DEPTH"),
    ("crates/sake/src/d3dreflect.rs", "MAX_TYPE_DEPTH"),
    ("crates/masu/src/evb.rs", "MAX_DEPTH"),
    ("crates/masu/src/asar.rs", "MAX_DEPTH"),
];

pub(super) fn depth_guard(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (file, name) in DEPTH_GUARDS {
        let f = Rel::new(file);
        if tree.is_made_up() && !tree.has(&f) {
            continue;
        }
        let Some(it) = tree.items(&f) else {
            out.push((f, 1, format!("{name}: the file is gone or does not parse (move the guard and this entry)")));
            continue;
        };
        if !it.consts.iter().any(|(c, _, _)| c == name) {
            out.push((f, 1, format!("const {name} is gone: the recursion it bounded is as deep as the input again")));
            continue;
        }
        // (its declaration is one identifier; a comparison or a call taking it is another)
        let uses = it.idents.iter().filter(|(i, _)| i == name).count();
        if uses < 2 {
            out.push((f, 1, format!("const {name} is declared but nothing compares against it")));
        }
    }
    out
}

// --- wgsl-load-bounds ------------------------------------------------------------------------

/// The WGSL dxwgsl writes reads a texel with textureLoad only beside the test of its address:
/// `select(zero, textureLoad(..), in bounds)` in one expression (2026-11-21, ld returned
/// WGSL's texel of an address outside the texture, where Direct3D returns 0). A line of
/// product code under crates/dxwgsl/src that writes `textureLoad(` and no `select(`.
pub(super) fn wgsl_load_bounds(tree: &Tree) -> Hits {
    let files = tree.rs(&["crates/dxwgsl/src/"]);
    let tests = test_mod_files(tree, &files);
    let mut out = Hits::new();
    for f in files.into_iter().filter(|f| !tests.contains(f)) {
        for (n, l) in tree.text(&f).lines().enumerate() {
            let code = l.split("//").next().unwrap_or_default();
            if code.contains("textureLoad(") && !code.contains("select(") {
                out.push((f.clone(), n.saturating_add(1), "textureLoad outside select(zero, .., in bounds)".to_owned()));
            }
        }
    }
    out
}

/// (for the tests: every regex here compiles)
#[cfg(test)]
pub(super) fn regexes() -> [&'static LazyLock<Option<Regex>>; 4] {
    [&SECTION, &PATH_DEP, &SH_SET, &YAML_KEY]
}

