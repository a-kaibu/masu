//! masu-lint: the repository's own lints (docs/lints.md).
//!
//!   cargo run -q --release -p masu-lint -- arch [--update]
//!       the architecture rules (src/arch.rs): seconds, no build; --update writes
//!       tools/ci/arch-lint-sites.tsv down (only down: an exception is added by hand, with its why)
//!   cargo run -q --release -p masu-lint -- ratchet [--update [--raise FILE...]]
//!       clippy's type-safety, robustness and masu-* lints per file and unsafe site (src/ratchet.rs)
//!
//!   cargo run -q --release -p masu-lint -- dup [--update | --list]
//!       the same code written twice (src/dup.rs); known groups with their why in
//!       tools/ci/dup-sites.tsv, --update drops the gone ones, --list prints every group
//!
//! `cargo test -p masu-lint` proves each rule and each counted lint fires on its regression.
// (Rust 1.97: a pub item nothing in the binary uses is dead; tests use more of it, so not under test)
#![cfg_attr(not(test), warn(dead_code_pub_in_binary))]
mod arch;
mod code;
mod dup;
mod escape;
mod ratchet;
mod syntax;
mod tree;
mod win32;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use tree::{Rel, Tree};

/// the repository this binary lints (it lives in it)
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub const ARCH_SITES: &str = "tools/ci/arch-lint-sites.tsv";

fn arch_main(update: bool) -> Result<String, String> {
    let root = root();
    let tree = Tree::repo(&root);
    let found = arch::check(&tree, &arch::Rule::ALL);
    let sites_path = root.join(ARCH_SITES);
    let text = std::fs::read_to_string(&sites_path).unwrap_or_default();
    let known = arch::load_sites(&text);
    let counts = arch::known_counts(&known);
    if update {
        // (a known line stays while its site is still there, as often as it is)
        let mut left: BTreeMap<&arch::Site, usize> = found.iter().map(|(s, v)| (s, v.len())).collect();
        let mut out = String::new();
        for line in text.lines() {
            let keep = match arch::load_sites(line).first() {
                None => true,
                Some((s, _)) => left.get_mut(s).is_some_and(|n| n.checked_sub(1).map(|m| *n = m).is_some()),
            };
            if keep {
                out.push_str(line);
                out.push('\n');
            }
        }
        std::fs::write(&sites_path, out).map_err(|e| e.to_string())?;
        return Ok(format!("masu-lint arch: wrote {ARCH_SITES}"));
    }
    let new = arch::fresh(&found, &counts);
    for f in &new {
        println!("masu-lint arch: {}:{}: {}: {}\n    ({})", f.file, f.line, f.rule.id(), f.what, f.rule.why());
    }
    let gone: usize = counts.iter().map(|(s, n)| n.saturating_sub(found.get(s).map_or(0, Vec::len))).sum();
    if gone > 0 {
        println!("masu-lint arch: {gone} known sites are gone (masu-lint arch --update writes them down)");
    }
    if !new.is_empty() {
        return Err(format!("masu-lint arch: {} sites break the rules above (each says what to write instead; crates/masu-lint/src/arch.rs)", new.len()));
    }
    Ok(format!("masu-lint arch: ok ({} rules, {} known sites)", arch::Rule::ALL.len(), known.len()))
}

pub const DUP_SITES: &str = "tools/ci/dup-sites.tsv";
/// what dup does not read: vendored code, lint fixtures (they repeat on purpose)
pub const DUP_SKIP: &[&str] = &["vendor/", "tools/lint-fixtures/"];

fn dup_main(update: bool, list: bool) -> Result<String, String> {
    let root = root();
    let tree = Tree::repo(&root);
    let found = dup::find(&dup_files(&tree));
    let path = root.join(DUP_SITES);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if list {
        print!("{}", dup::list(&found));
        return Ok(format!("masu-lint dup: {} groups", found.len()));
    }
    if update {
        std::fs::write(&path, dup::updated(&text, &found)).map_err(|e| e.to_string())?;
        return Ok(format!("masu-lint dup: wrote {DUP_SITES}"));
    }
    let known = dup::load_known(&text);
    let new = dup::fresh(&found, &known);
    if !new.is_empty() {
        print!("{}", dup::report(&new, DUP_SITES));
        return Err(format!("masu-lint dup: {} groups of copied code (crates/masu-lint/src/dup.rs; kome is where crates share code)", new.len()));
    }
    Ok(format!("masu-lint dup: ok ({} groups known)", known.len()))
}

/// the (path, text) of every tracked Rust file dup reads
fn dup_files(tree: &Tree) -> Vec<(String, String)> {
    tree.paths().iter().filter(|p| p.is_rust() && !DUP_SKIP.iter().any(|s| p.as_str().starts_with(s))).map(|p| (p.as_str().to_owned(), tree.text(p))).collect()
}

fn ratchet_main(update: bool, raise: BTreeSet<Rel>) -> Result<String, String> {
    let root = root();
    let repo = ratchet::Repo { root: root.clone(), dirs: vec!["crates"], conf: root.clone(), target: None };
    ratchet::run(&repo, &ratchet::repo_paths(&root), update, &raise)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let update = args.iter().any(|a| a == "--update");
    let raise: BTreeSet<Rel> = args.iter().skip_while(|a| *a != "--raise").skip(1).map(|a| Rel::new(a)).collect();
    let r = match args.first().map(String::as_str) {
        Some("arch") => arch_main(update),
        Some("ratchet") => ratchet_main(update, raise),
        Some("dup") => dup_main(update, args.iter().any(|a| a == "--list")),
        Some(_) | None => Err("usage: masu-lint arch [--update] | masu-lint ratchet [--update [--raise FILE...]] | masu-lint dup [--update | --list]".into()),
    };
    match r {
        Ok(m) => {
            println!("{m}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// dup on the repository itself: every group it finds is in dup-sites.tsv (what CI runs)
#[cfg(test)]
#[test]
fn dup_the_repository_has_no_unknown_copies() {
    let root = root();
    let found = dup::find(&dup_files(&Tree::repo(&root)));
    let known = dup::load_known(&std::fs::read_to_string(root.join(DUP_SITES)).unwrap_or_default());
    let new = dup::fresh(&found, &known);
    assert!(new.is_empty(), "{}", dup::report(&new, DUP_SITES));
}
