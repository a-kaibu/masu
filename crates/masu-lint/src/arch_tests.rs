//! Each rule against its regression: in a made-up tree (it must find it), the right spelling (it
//! must not), and the regression put back into a real file of the repository (it must find it
//! beyond the known sites). A rule without a fixture here does not compile.
use super::*;

/// a made-up tree: (path, text)
type Files = &'static [(&'static str, &'static str)];

struct Fixture {
    bad: Files,
    good: Files,
    /// (a real file, its text -> the text with the regression; None deletes it)
    mutate: (&'static str, fn(String) -> Option<String>),
}

fn fixture(r: Rule) -> Fixture {
    match r {
        Rule::BuildOutput => Fixture {
            bad: &[("crates/masu/target/x86_64-pc-windows-gnu/release/masu.exe", "x"), ("web/public/sake/sake.wasm", "x")],
            good: &[("crates/kura/testdata/idec/planes.bin", "x"), ("testdata/apitrace/a.trace.zst", "x"), ("crates/kura/src/target.rs", "x")],
            mutate: ("crates/masu/target/release/masu", |_| Some("stub\n".into())),
        },
        Rule::HresultCopy => Fixture {
            bad: &[(HRESULT, "pub const E_FAIL: u32 = 0x8000_4005;\n"), ("crates/sake/src/vmr9.rs", "const E_FAIL: u32 = 0x8000_4005;\n")],
            good: &[
                (HRESULT, "pub const E_FAIL: u32 = 0x8000_4005;\n"),
                ("crates/sake/src/vmr9.rs", "use crate::hresult::E_FAIL;\nconst VFW_E_X: u32 = 0x8004_0200;\n#[cfg(test)]\nmod tests {\n    const E_FAIL: u32 = 1;\n}\n"),
                ("crates/sake/src/x_tests.rs", "const E_FAIL: u32 = 1;\n"),
            ],
            mutate: ("crates/sake/src/k32.rs", |t| Some(t + "\nconst E_FAIL: u32 = 0x8000_4005;\n")),
        },
        Rule::KomeCopy => Fixture {
            bad: &[("crates/kura/src/x.rs", "fn lo16(x: u32) -> u16 {\n    x as u16\n}\npub trait Lock<T> {}\n")],
            good: &[("crates/kura/src/x.rs", "use kome::num::lo16;\nimpl R {\n    fn u32_at(&self, o: usize) -> u32 { 0 }\n}\n"), ("crates/kome/src/num.rs", "pub fn lo16(x: u32) -> u16 { 0 }\n")],
            mutate: ("crates/kura/src/audio.rs", |t| Some(t + "\nfn lo32(x: u64) -> u32 {\n    x as u32\n}\n")),
        },
        Rule::GoldenEnv => Fixture {
            bad: &[
                ("crates/sake/src/x.rs", "fn f() { if std::env::var_os(\"SAKE_UPDATE_GOLDEN\").is_some() {} }\n"),
                ("crates/kura/src/y.rs", "fn f() { std::env::var(\"MASU_UPDATE_GOLDEN\"); }\n"),
                ("tools/x.sh", "BLESS=1 cargo test\n"),
            ],
            good: &[
                (GOLDEN_HOME, "pub const UPDATE: &str = \"MASU_UPDATE_GOLDEN\";\n"),
                ("crates/sake/src/x.rs", "fn f() { if kome::golden::updating() {} }\n// MASU_UPDATE_GOLDEN=1 rewrites it\n"),
                ("tests/x.sh", "MASU_UPDATE_GOLDEN=1 cargo test\n"),
            ],
            mutate: ("crates/sake/src/gl.rs", |t| Some(t + "\nfn regen() -> bool { std::env::var_os(\"SAKE_UPDATE_GOLDEN\").is_some() }\n")),
        },
        Rule::DeadApi => Fixture {
            bad: &[(MEM, "pub fn view<R>(a: u32, n: u32, f: impl FnOnce(&[u8]) -> R) -> Option<R> { None }\n"), (API, "impl Out {\n    pub fn to_callee(self) -> u32 { 0 }\n}\n")],
            good: &[(MEM, "pub unsafe fn view_unchecked<R>(a: u32) {}\npub fn bytes(a: u32, n: u32) -> Vec<u8> { vec![] }\n"), (API, "pub fn out(&self) {}\n")],
            mutate: (MEM, |t| Some(t + "\npub fn guest_mut(a: u32, n: u32) -> &'static mut [u8] { todo!() }\n")),
        },
        Rule::UntypedWayBack => Fixture {
            bad: &[(MEM, "impl Ga {\n    pub fn r32(self) -> u32 { 0 }\n}\n"), (API, "impl<F> std::ops::Deref for Args<F> {\n    type Target = Ctx;\n}\n")],
            good: &[(MEM, "impl Ga {\n    pub fn since(self, b: Ga) -> u32 { 0 }\n}\npub fn r32(a: impl Into<Ga>) -> u32 { 0 }\n"), (API, "impl<F> Args<F> {\n    pub fn ctx(&self) {}\n}\n")],
            mutate: (MEM, |t| Some(t + "\nimpl Ga {\n    pub fn w32(self, v: u32) {}\n}\n")),
        },
        Rule::HomeOnly => Fixture {
            bad: &[
                ("crates/sake/src/gdi.rs", "fn f() { let v = unsafe { mem::view_unchecked(a, n, |s| s[0]) }; }\n"),
                ("crates/kura/src/vfs.rs", "fn f() { unsafe { AtomicU32::from_ptr(p) }.load(Relaxed); X.locked_anyway(); }\n"),
                ("crates/kura/src/log.rs", "fn f() { println!(\"{}\", core::sync::atomic::AtomicU8::from_ptr(p).load(Relaxed)); }\n"),
            ],
            good: &[
                ("crates/sake/src/k32.rs", "fn f() { unsafe { mem::view_mut_unchecked(a, n, |s| ()) }; }\n"),
                ("crates/kura/src/crashlog.rs", "fn f() { unsafe { AtomicU8::from_ptr(p) }.load(Relaxed); X.locked_anyway(); }\n"),
                ("crates/kura/src/vfs.rs", "fn f() { X.locked(); let a = AtomicU32::new(0); }\n// AtomicU32::from_ptr in a comment\n"),
            ],
            mutate: ("crates/sake/src/gdi/mod.rs", |t| Some(t + "\nfn peek(a: u32) -> u8 { unsafe { crate::mem::view_unchecked(a, 1, |s| s[0]) }.unwrap_or(0) }\n")),
        },
        Rule::ReprCLayout => Fixture {
            bad: &[("crates/kura/src/x.rs", "#[repr(C)]\npub struct Ctl {\n    pub magic: u32,\n}\n#[repr(C, packed)]\npub struct P(u8);\nconst _: () = assert!(size_of::<Ctl>() > 0 || true);\n")],
            good: &[
                ("crates/kura/src/x.rs", "#[repr(C)]\npub struct Ctl {\n    pub magic: u32,\n}\nconst _: () = assert!(std::mem::size_of::<Ctl>() == 4);\n#[repr(u8)]\nenum E { A }\n"),
                ("crates/sake/src/nt.rs", "#[cfg(test)]\nmod tests {\n    #[repr(C)]\n    struct Head { next: u32 }\n}\n"),
            ],
            mutate: ("crates/kura/src/snap.rs", |t| Some(t + "\n#[repr(C)]\npub struct Extra {\n    pub a: u32,\n}\n")),
        },
        Rule::ReleaseBuildOnly => Fixture {
            bad: &[(RELEASE, "jobs:\n  version:\n    runs-on: x\n  test:\n    steps:\n      - run: cargo test --workspace\n")],
            good: &[(RELEASE, "jobs:\n  version:\n    runs-on: x\n  release:\n    steps:\n      - run: cargo build --release\n")],
            mutate: (RELEASE, |t| Some(t + "  lint:\n    runs-on: ubuntu-latest\n    steps:\n      - run: cargo clippy\n")),
        },
        Rule::Required => Fixture {
            bad: &[("tests/host-tests.sh", "#!/bin/sh\ncargo test\n")],
            good: &[
                ("tests/host-tests.sh", "export MASU_OUTS_STRICT=1\n"),
                ("crates/sake/src/lib.rs", "#[cfg(test)]\nmod outptr_audit;\n"),
                (".github/workflows/ci.yml", "cargo run -q --release -p masu-lint -- arch\ncargo test --workspace --release\ncargo run -q --release -p masu-lint -- ratchet\n"),
            ],
            mutate: ("tests/host-tests.sh", |t| Some(t.replace("export MASU_OUTS_STRICT=1", ""))),
        },
        Rule::DisallowedNamed => Fixture {
            bad: &[("clippy.toml", "disallowed-methods = [\n    { path = \"core::mem::zeroed\", reason = \"use Default\" },\n]\n")],
            good: &[("clippy.toml", "disallowed-methods = [\n    { path = \"core::mem::zeroed\", reason = \"masu-uninit: Default / a value\" },\n]\n")],
            mutate: ("clippy.toml", |t| Some(t.replace("reason = \"masu-relay: ", "reason = \"relay: "))),
        },
    }
}

fn repo() -> std::path::PathBuf {
    crate::root()
}

#[test]
fn each_rule_finds_its_regression_in_a_fixture() {
    for r in Rule::ALL {
        let f = fixture(r);
        assert!(!r.check(&Tree::made_up(f.bad)).is_empty(), "{}: finds nothing in its regression fixture", r.id());
    }
}

#[test]
fn each_rule_lets_the_right_spelling_through() {
    for r in Rule::ALL {
        let f = fixture(r);
        let hits = r.check(&Tree::made_up(f.good));
        assert!(hits.is_empty(), "{}: flags its good fixture: {hits:?}", r.id());
    }
}

#[test]
fn each_rule_finds_its_regression_put_back_into_the_real_tree() {
    let root = repo();
    let known = known_counts(&load_sites(&std::fs::read_to_string(root.join(crate::ARCH_SITES)).unwrap_or_default()));
    let real = Tree::repo(&root);
    for r in Rule::ALL {
        let (file, mutate) = fixture(r).mutate;
        let f = Rel::new(file);
        let before = fresh(&check(&real, &[r]), &known).len();
        let tree = Tree::with(&root, &f, mutate(real.text(&f)));
        let after = fresh(&check(&tree, &[r]), &known).len();
        assert!(after > before, "{}: misses its regression put into {file}", r.id());
    }
}

#[test]
fn every_rule_is_listed_once() {
    let ids: BTreeSet<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
    assert_eq!(ids.len(), Rule::ALL.len());
}

#[test]
fn a_known_site_covers_as_many_as_it_is_listed() {
    let tree = Tree::made_up(&[("crates/kura/src/x.rs", "fn lo16(x: u32) -> u16 { 0 }\n")]);
    let found = check(&tree, &[Rule::KomeCopy]);
    let one = known_counts(&load_sites("kome-copy\tcrates/kura/src/x.rs\tfn lo16(x: u32) -> u16 { 0 }\twhy\n"));
    assert!(fresh(&found, &one).is_empty());
    assert_eq!(fresh(&found, &BTreeMap::new()).len(), 1);
    // (an edited line is another site)
    let edited = known_counts(&load_sites("kome-copy\tcrates/kura/src/x.rs\tfn lo16(x: u32) -> u16 { 1 }\twhy\n"));
    assert_eq!(fresh(&found, &edited).len(), 1);
}

#[test]
fn every_regex_compiles() {
    for r in [&BUILD_EXT, &BUILD_OK, &TARGET_DIR, &GOLDEN_ANY, &GOLDEN_FILES, &JOB, &CHECKS, &DISALLOWED] {
        assert!(r.is_some());
    }
}
