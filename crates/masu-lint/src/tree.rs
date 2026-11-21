//! The files a rule reads: the repository's tracked files, or a made-up tree, with an overlay on
//! top (what the tests put a regression into).
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::code::code_only;

/// A file's path relative to the repository, '/'-separated, as `git ls-files` prints it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rel(pub String);

impl Rel {
    pub fn new(s: &str) -> Rel {
        Rel(s.to_owned())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_rust(&self) -> bool {
        self.0.ends_with(".rs")
    }
    /// tests/, benches/, *_tests.rs, tests.rs: free to cast, index and unwrap
    pub fn is_test_file(&self) -> bool {
        let s = self.as_str();
        s.starts_with("tests/") || s.starts_with("benches/") || s.contains("/tests/") || s.contains("/benches/") || s.ends_with("_tests.rs") || s.ends_with("_test.rs") || s == "tests.rs" || s == "test.rs" || s.ends_with("/tests.rs") || s.ends_with("/test.rs")
    }
}

impl std::fmt::Display for Rel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// what a tree's files are before the overlay
enum Base {
    /// the repository's tracked files (listed once, when the tree is made), read from the disk
    Repo(BTreeSet<Rel>),
    /// a made-up tree: path -> text
    MadeUp(BTreeMap<Rel, String>),
}

pub struct Tree {
    root: PathBuf,
    base: Base,
    /// path -> text (None: the file is gone)
    overlay: BTreeMap<Rel, Option<String>>,
    paths: OnceLock<Vec<Rel>>,
    code: std::sync::Mutex<BTreeMap<(Rel, bool), std::sync::Arc<str>>>,
    items: std::sync::Mutex<BTreeMap<Rel, Option<std::sync::Arc<crate::syntax::Items>>>>,
}

impl Tree {
    /// the repository's tracked files (Err: they could not be listed)
    pub fn repo(root: &Path) -> Result<Tree, String> {
        Ok(Tree { root: root.to_owned(), base: Base::Repo(git_files(root)?), overlay: BTreeMap::new(), paths: OnceLock::new(), code: Default::default(), items: Default::default() })
    }
    #[cfg(test)]
    pub fn made_up(files: &[(&str, &str)]) -> Tree {
        let base = files.iter().map(|(p, t)| (Rel::new(p), (*t).to_owned())).collect();
        Tree { root: PathBuf::new(), base: Base::MadeUp(base), overlay: BTreeMap::new(), paths: OnceLock::new(), code: Default::default(), items: Default::default() }
    }
    /// this tree with `path` replaced (None: deleted)
    #[cfg(test)]
    pub fn with(real: &Tree, path: &Rel, text: Option<String>) -> Tree {
        let base = match &real.base {
            Base::Repo(f) => Base::Repo(f.clone()),
            Base::MadeUp(m) => Base::MadeUp(m.clone()),
        };
        let mut t = Tree { root: real.root.clone(), base, overlay: BTreeMap::new(), paths: OnceLock::new(), code: Default::default(), items: Default::default() };
        t.overlay.insert(path.clone(), text);
        t
    }
    pub fn is_made_up(&self) -> bool {
        matches!(self.base, Base::MadeUp(_))
    }
    pub fn has(&self, p: &Rel) -> bool {
        self.paths().binary_search(p).is_ok()
    }

    pub fn paths(&self) -> &[Rel] {
        self.paths.get_or_init(|| {
            let mut ps: BTreeSet<Rel> = match &self.base {
                Base::MadeUp(b) => b.keys().cloned().collect(),
                Base::Repo(f) => f.clone(),
            };
            for (p, t) in &self.overlay {
                if t.is_some() {
                    ps.insert(p.clone());
                } else {
                    ps.remove(p);
                }
            }
            ps.into_iter().collect()
        })
    }

    pub fn text(&self, p: &Rel) -> String {
        if let Some(t) = self.overlay.get(p) {
            return t.clone().unwrap_or_default();
        }
        match &self.base {
            Base::MadeUp(b) => b.get(p).cloned().unwrap_or_default(),
            Base::Repo(_) => std::fs::read(self.root.join(p.as_str())).map_or_default(|b| String::from_utf8_lossy(&b).into_owned()),
        }
    }

    /// `p`'s code with comments (and strings unless `strings`) blanked
    pub fn code(&self, p: &Rel, strings: bool) -> std::sync::Arc<str> {
        let key = (p.clone(), strings);
        if let Some(c) = self.code.lock().ok().and_then(|m| m.get(&key).cloned()) {
            return c;
        }
        let c: std::sync::Arc<str> = code_only(&self.text(p), strings).into();
        if let Ok(mut m) = self.code.lock() {
            m.insert(key, c.clone());
        }
        c
    }

    /// the Rust files under `prefixes`, test files left out
    pub fn rs(&self, prefixes: &[&str]) -> Vec<Rel> {
        self.paths().iter().filter(|p| p.is_rust() && !p.is_test_file() && prefixes.iter().any(|x| p.as_str().starts_with(x))).cloned().collect()
    }

    /// `p`'s items, parsed once (None: it does not parse, which the compiler reports)
    pub fn items(&self, p: &Rel) -> Option<std::sync::Arc<crate::syntax::Items>> {
        if let Some(i) = self.items.lock().ok().and_then(|m| m.get(p).cloned()) {
            return i;
        }
        let i = self.syntax(p).map(|f| std::sync::Arc::new(crate::syntax::Items::of(&f)));
        if let Ok(mut m) = self.items.lock() {
            m.insert(p.clone(), i.clone());
        }
        i
    }

    /// `p` parsed (None: it does not parse, which the compiler reports)
    pub fn syntax(&self, p: &Rel) -> Option<syn::File> {
        syn::parse_file(&self.text(p)).ok()
    }
}

/// the tracked files, or why they could not be listed: a listing that failed is not an empty
/// repository (cargo-mutants' copy has no .git, `git ls-files` failed there and every rule read
/// no files; review e18bf44)
fn git_files(root: &Path) -> Result<BTreeSet<Rel>, String> {
    let o = std::process::Command::new("git").args(["ls-files", "-z"]).current_dir(root).output().map_err(|e| format!("masu-lint: git ls-files in {}: {e}", root.display()))?;
    if !o.status.success() {
        return Err(format!("masu-lint: git ls-files in {} failed ({}): {}", root.display(), o.status, String::from_utf8_lossy(&o.stderr).trim()));
    }
    let files: BTreeSet<Rel> = String::from_utf8_lossy(&o.stdout).split('\0').filter(|s| !s.is_empty()).map(Rel::new).collect();
    if files.is_empty() {
        return Err(format!("masu-lint: git ls-files in {} listed no files", root.display()));
    }
    Ok(files)
}
