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
                ("crates/kura/src/gpu.rs", "fn f() { waits::locked::pair(&a, &b, p, |v| None); }\n"),
                ("crates/kura/src/audio.rs", "struct T;\nunsafe impl crate::pool::Brand for T {}\n"),
                ("crates/sake/src/gdi_geom.rs", "fn f(r: Range) { let _ = unsafe { exclusive_slice(r, |s| s[0]) }; }\n"),
                ("crates/kura/src/vfs2.rs", "fn f() { let _ = unsafe { crate::jit::LatePart::adopt(p, 1, q, 1) }; }\n"),
            ],
            good: &[
                ("crates/sake/src/k32.rs", "fn f() { unsafe { mem::view_mut_unchecked(a, n, |s| ()) }; }\n"),
                ("crates/kura/src/crashlog.rs", "fn f() { unsafe { AtomicU8::from_ptr(p) }.load(Relaxed); X.locked_anyway(); }\n"),
                ("crates/kura/src/vfs.rs", "fn f() { X.locked(); let a = AtomicU32::new(0); }\n// AtomicU32::from_ptr in a comment\n"),
                ("crates/kura/src/cpu.rs", "fn f() { waits::locked::pair(&a, &b, p, |v| None); }\n"),
                ("crates/kura/src/gpu.rs", "fn f() { let p = vec.pair(); crate::pool!(mod r: u8 as R); }\n"),
                ("crates/kura/src/jit.rs", "fn f() { let _ = unsafe { LatePart::adopt(p, 1, q, 1) }; }\n"),
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
        Rule::SafeRawPointerAbi => Fixture {
            bad: &[(
                "crates/kura/src/x.rs",
                "#[unsafe(no_mangle)]\npub extern \"C\" fn take(p: *const u8, n: u32) -> i32 { 0 }\n#[no_mangle]\nextern \"C\" fn opt(p: Option<*mut u32>) {}\nimpl X {\n    #[unsafe(export_name = \"x_m\")]\n    pub extern \"C\" fn m(&self, out: *mut u32) {}\n}\n",
            )],
            good: &[
                (
                    "crates/kura/src/x.rs",
                    "/// # Safety\n/// `p` points at `n` readable bytes\n#[unsafe(no_mangle)]\npub unsafe extern \"C\" fn take(p: *const u8, n: u32) -> i32 { 0 }\npub extern \"C\" fn val(a: u32) -> u32 { a }\npub extern \"C\" fn cb(f: extern \"C\" fn(*const u8)) {}\nfn plain(p: *const u8) {}\n#[cfg(test)]\nmod tests {\n    extern \"C\" fn t(p: *const u8) {}\n}\n",
                ),
                ("crates/kura/src/x_tests.rs", "#[no_mangle]\nextern \"C\" fn t(p: *const u8) {}\n"),
                // (a thread's entry, handed over as a fn pointer: not callable by name from outside)
                ("crates/kura/src/thread.rs", "fn start() {\n    extern \"C\" fn main(_: *mut c_void) -> *mut c_void { p }\n}\n#[cfg(all(test, not(target_os = \"emscripten\")))]\nmod stubs {\n    #[unsafe(no_mangle)]\n    extern \"C\" fn vnvm_fetch(n: *const c_char) {}\n}\n"),
                // (a module only tests build: the page's model for native tests)
                ("crates/kura/src/render/mod.rs", "pub mod gpu;\n#[cfg(test)]\npub mod model;\n"),
                ("crates/kura/src/render/model.rs", "#[unsafe(no_mangle)]\npub extern \"C\" fn vngpu_has_feature(_p: *const u8, _n: usize) -> i32 { 0 }\n"),
            ],
            mutate: ("crates/kura/src/audio.rs", |t| Some(t + "\n#[unsafe(no_mangle)]\npub extern \"C\" fn vnhost_regression(p: *const u8, n: u32) {}\n")),
        },
        Rule::UncheckedJitMode => Fixture {
            bad: &[
                (NOCHECK_HOME, "pub static NOCHECK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);\n"),
                ("crates/sake/src/x.rs", "fn f() { koji::jit::NOCHECK.store(true, Relaxed); }\n"),
                ("crates/masu/src/y.rs", "fn f() { unsafe { std::env::set_var(\"MASU_JIT_NOCHECK\", \"1\") } }\n"),
                (".github/workflows/bench.yml", "env:\n  MASU_JIT_NOCHECK: 1\n"),
                ("tests/x.sh", "MASU_JIT_NOCHECK=1 cargo test\n"),
                ("web/src/play.ts", "const env = { MASU_JIT_NOCHECK: '1' };\n"),
            ],
            good: &[
                (NOCHECK_HOME, "/// Measuring only (MASU_JIT_NOCHECK=1)\npub static NOCHECK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);\nfn nocheck() -> bool { NOCHECK.load(Relaxed) }\n"),
                (NOCHECK_SETTER, "fn init() { if std::env::var(\"MASU_JIT_NOCHECK\").is_ok() { jit::NOCHECK.store(true, Relaxed); } }\n"),
                ("crates/sake/src/x.rs", "// koji::jit::NOCHECK and MASU_JIT_NOCHECK=1 are for measuring\nfn f() {}\n"),
                ("docs/translator.md", "MASU_JIT_NOCHECK=1: no range checks\n"),
                ("tests/x.sh", "MASU_JIT_NOCHECK=0 cargo test\n"),
            ],
            mutate: (NOCHECK_HOME, |t| Some(t.replace("AtomicBool::new(false);\n/// Pushes", "AtomicBool::new(true);\n/// Pushes"))),
        },
        Rule::OutEscape => Fixture {
            bad: &[
                ("crates/sake/src/k32.rs", "fn f(c: &mut Ctx) { mem::w32(c.out::<0>().lock_word().key(), 0); }\n"),
                ("crates/sake/src/gdiplus.rs", "fn f(c: &mut Ctx) { let m = c.out::<0>().object().key(); let v = crate::mem::r32(m + 4); }\n"),
                ("crates/sake/src/ws2.rs", "fn f(o: Out) { mem::copy(d, (o.object().key()), 4); let a = Ga::from(o.lock_word().key()); w8(ga(o.object().key()), 1); }\n"),
            ],
            good: &[
                ("crates/sake/src/k32.rs", "fn f(c: &mut Ctx) { SYNC.cv_wake(c.out::<0>().lock_word().key()); srw(c.out::<0>().lock_word().atomic()); }\n"),
                ("crates/sake/src/ws2name.rs", "fn f(res: Out) { wp(ov + 2 * k, res.object().key()); }\nfn g(o: Out) { let m = o.object().key(); }\nfn h(m: u32) { mem::r32(m); }\n"),
                ("crates/sake/src/x.rs", "fn f(o: Out) { let m = o.object().key(); let m = 5; mem::r32(m); }\n#[cfg(test)]\nmod tests {\n    fn t(o: Out) { mem::w32(o.object().key(), 0); }\n}\n"),
            ],
            mutate: ("crates/sake/src/gdiplus_brush.rs", |t| Some(t + "\nfn escape(c: &mut Ctx) { crate::mem::w32(c.out::<0>().object().key(), 0); }\n")),
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

#[test]
fn the_new_rules_find_each_spelling_alone() {
    // (each bad file on its own: one spelling found does not hide another that is missed)
    for r in [Rule::SafeRawPointerAbi, Rule::UncheckedJitMode, Rule::OutEscape] {
        for one in fixture(r).bad.chunks(1) {
            assert!(!r.check(&Tree::made_up(one)).is_empty(), "{}: finds nothing in {}", r.id(), one[0].0);
        }
    }
    // (and every site in the bad files, not only the first: w8(ga(key)) is two)
    assert_eq!(Rule::SafeRawPointerAbi.check(&Tree::made_up(fixture(Rule::SafeRawPointerAbi).bad)).len(), 3);
    assert_eq!(Rule::OutEscape.check(&Tree::made_up(fixture(Rule::OutEscape).bad)).len(), 6);
}
