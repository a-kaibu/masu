//! Every counted lint against tools/lint-fixtures/ratchet: a small workspace whose lines each
//! carry the regression one lint is there for, marked `//~ lint...` (`lint@group` for a lint
//! counted through its clippy group). Each mark must be found on its line and nothing unmarked
//! may be: a lint renamed or dropped by a clippy upgrade, a clippy.toml path that stops
//! resolving, a regex that stops matching, all fail here instead of silently counting 0. And
//! every lint in the lists has a mark.
use super::*;

fn fixture() -> Repo {
    let root = crate::root();
    Repo { root: root.join("tools/lint-fixtures/ratchet"), dirs: vec!["crates"], conf: root.clone(), target: Some(root.join("target/lint-fixtures")) }
}

#[test]
fn every_counted_lint_fires_on_its_fixture_line() {
    let repo = fixture();
    let mut want = BTreeSet::new();
    let mut groups = BTreeSet::new();
    for (rel, text) in sources(&repo) {
        for (k, line) in text.split('\n').enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let Some((_, marks)) = line.split_once("//~") else { continue };
            for m in marks.split_whitespace() {
                let (l, g) = m.split_once('@').map_or((m, None), |(l, g)| (l, Some(g)));
                want.insert((rel.clone(), k.saturating_add(1), l.to_owned()));
                groups.extend(g.map(str::to_owned));
            }
        }
    }
    let disallowed = crate::arch::disallowed(&std::fs::read_to_string(repo.conf.join("clippy.toml")).unwrap_or_default());
    let Hits { all: hits, product } = clippy(&repo, &disallowed).unwrap_or_else(|e| panic!("clippy on the fixture: {e}"));
    // (the fixture's library unwraps and expects, its cfg(test) code too: NEVER finds the
    // library's only)
    let nev = never(&product);
    let lints: BTreeSet<&str> = nev.iter().map(|k| k.1.as_str()).collect();
    assert_eq!(lints, NEVER.into_iter().collect(), "NEVER lints in the fixture's library");
    let lib = std::fs::read_to_string(repo.root.join("crates/masu/src/lib.rs")).unwrap_or_default();
    let test_only = lib.lines().position(|l| l.contains("fn test_only")).map(|i| i.saturating_add(2));
    assert!(test_only.is_some(), "the fixture's cfg(all(test, ..)) function");
    assert!(!product.iter().any(|h| Some(h.line) == test_only && h.file.as_str().ends_with("masu/src/lib.rs")), "a cfg(test) unwrap taken for product code: {product:#?}");
    let srcs = sources(&repo);
    let got: BTreeSet<(Rel, usize, String)> = hits.into_iter().chain(allow_attrs(&srcs)).chain(custom(&srcs)).map(|h| (h.file, h.line, h.lint)).collect();
    let missed: Vec<_> = want.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want).collect();
    assert!(missed.is_empty(), "marked but did not fire: {missed:#?}");
    assert!(extra.is_empty(), "fired where no mark says it should: {extra:#?}");
    let names: BTreeSet<&str> = want.iter().map(|(_, _, l)| l.as_str()).collect();
    let every: BTreeSet<String> = LINTS
        .iter()
        .chain(ROBUST.iter())
        .map(|s| (*s).to_owned())
        .chain([UNSAFE_OP.1.to_owned(), "allow-attr".to_owned()])
        .chain(CUSTOM.iter().chain(RAW.iter()).map(|(n, _)| (*n).to_owned()))
        .chain(disallowed.values().flatten().cloned())
        .collect();
    let unmarked: Vec<_> = every.iter().filter(|l| !names.contains(l.as_str())).collect();
    assert!(unmarked.is_empty(), "lints without a fixture line in tools/lint-fixtures/ratchet: {unmarked:?}");
    let ungrouped: Vec<_> = GROUPS.iter().filter(|g| !groups.contains(**g)).collect();
    assert!(ungrouped.is_empty(), "clippy groups without a fixture line (mark it `<lint>@<group>`): {ungrouped:?}");
}

#[test]
fn never_names_each_file_and_lint_once() {
    let h = |f: &str, line: usize, lint: &str| Hit { file: Rel::new(f), line, col: 1, lint: lint.to_owned() };
    let product: BTreeSet<Hit> = [h("a.rs", 1, "unwrap_used"), h("a.rs", 2, "unwrap_used"), h("b.rs", 3, "expect_used"), h("c.rs", 4, "as_conversions")].into_iter().collect();
    let nev = never(&product);
    let got: Vec<(&str, &str)> = nev.iter().map(|k| (k.0.as_str(), k.1.as_str())).collect();
    assert_eq!(got, [("a.rs", "unwrap_used"), ("b.rs", "expect_used")]);
}

#[test]
fn a_site_traded_for_another_is_new() {
    let k = |t: &str| (Rel::new("f.rs"), "masu-transmute".to_owned(), t.to_owned());
    let known: BTreeMap<_, _> = [(k("x"), 1)].into_iter().collect();
    let now: BTreeMap<_, _> = [(k("y"), 1)].into_iter().collect();
    assert_eq!(fresh_sites(&now, &known, &BTreeSet::new()), vec![&k("y")]);
    assert!(fresh_sites(&known, &known, &BTreeSet::new()).is_empty());
    assert!(fresh_sites(&now, &known, &[Rel::new("f.rs")].into_iter().collect()).is_empty());
}

#[test]
fn paths_of_path_includes_are_one_file() {
    assert_eq!(normalize("crates/masu/src/../../kura/src/vnv.rs"), "crates/kura/src/vnv.rs");
    assert_eq!(normalize("./a/b.rs"), "a/b.rs");
    assert_eq!(normalize("/r/x/../y.rs"), "/r/y.rs");
}

#[test]
fn the_lists_read_back_as_written() {
    let c: BTreeMap<_, _> = [((Rel::new("a.rs"), "unwrap_used".to_owned()), 3)].into_iter().collect();
    assert_eq!(load_counts(&save_counts(&c)), c);
    let s: BTreeMap<_, _> = [((Rel::new("a.rs"), "masu-transmute".to_owned(), "let f = x;".to_owned()), 2)].into_iter().collect();
    assert_eq!(load_sites(&save_sites(&s)), s);
}

#[test]
fn every_regex_compiles() {
    for r in [&TRACKED, &ALLOW, &BACKTICK] {
        assert!(r.is_some());
    }
    assert!(CUSTOM.iter().chain(RAW.iter()).all(|(_, r)| Regex::new(r).is_ok()));
}

#[test]
fn a_pinned_site_copied_or_moved_to_another_file_is_new() {
    // (review 2026-11-17: a site is its file, lint and line text, so the same text moved within
    // its file stays known, a second copy of it is new, and the same text in another file is new)
    let k = |f: &str| (Rel::new(f), "masu-transmute".to_owned(), "unsafe { transmute(x) }".to_owned());
    let known: BTreeMap<_, _> = [(k("a.rs"), 1)].into_iter().collect();
    let moved: BTreeMap<_, _> = [(k("a.rs"), 1)].into_iter().collect();
    assert!(fresh_sites(&moved, &known, &BTreeSet::new()).is_empty());
    let copied: BTreeMap<_, _> = [(k("a.rs"), 2)].into_iter().collect();
    assert_eq!(fresh_sites(&copied, &known, &BTreeSet::new()), vec![&k("a.rs")]);
    let elsewhere: BTreeMap<_, _> = [(k("b.rs"), 1)].into_iter().collect();
    assert_eq!(fresh_sites(&elsewhere, &known, &BTreeSet::new()), vec![&k("b.rs")]);
}
