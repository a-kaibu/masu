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

/// ntcrt.rs registering every name of win32::NTDLL_OWN_CRT (the classes by its two lists)
const GOOD_NTCRT: &str = concat!(
    "pub fn init() {\n    let d = \"ntdll\";\n",
    "    classes!(Upper \"upper\", Lower \"lower\", Digit \"digit\", Space \"space\", Punct \"punct\", Cntrl \"cntrl\", Xdigit \"xdigit\", Alpha \"alpha\", Alnum \"alnum\", Graph \"graph\", Print \"print\");\n",
    "    wclasses!(Lower \"lower\", Digit \"digit\", Space \"space\", Xdigit \"xdigit\", Alpha \"alpha\", Alnum \"alnum\", Graph \"graph\", Print \"print\");\n",
    "    implement(d, \"towlower\", f); implement(d, \"towupper\", f); implement(d, \"tolower\", f); implement(d, \"toupper\", f);\n",
    "    implement(d, \"_wcslwr\", f); implement(d, \"_wcsupr\", f); implement(d, \"_wcsicmp\", f); implement(d, \"_wcsnicmp\", f);\n",
    "    implement(d, \"iswctype\", f); implement(d, \"iswascii\", f);\n",
    "    implement(d, \"_itoa\", f); implement(d, \"_ltoa\", f); implement(d, \"_ultoa\", f); implement(d, \"_i64toa\", f); implement(d, \"_ui64toa\", f);\n",
    "    implement(d, \"_itow\", f); implement(d, \"_ltow\", f); implement(d, \"_ultow\", f); implement(d, \"_i64tow\", f); implement(d, \"_ui64tow\", f);\n",
    "    implement(d, \"atoi\", f); implement(d, \"atol\", f); implement(d, \"_wtoi\", f); implement(d, \"_wtol\", f); implement(d, \"_atoi64\", f); implement(d, \"_wtoi64\", f);\n",
    "    implement(d, \"strtol\", f); implement(d, \"strtoul\", f); implement(d, \"wcstol\", f); implement(d, \"wcstoul\", f); implement(d, \"_wcstoi64\", f); implement(d, \"_wcstoui64\", f);\n",
    "    implement(d, \"RtlAnsiCharToUnicodeChar\", f);\n",
    "}\n",
);

fn fixture(r: Rule) -> Fixture {
    match r {
        Rule::MemoryPointer => Fixture {
            bad: &[(boundaries::MEMORY, "trait Memory { fn host(&mut self) -> Option<*mut u8>; }")],
            good: &[(boundaries::MEMORY, "trait Memory { fn copy(&mut self) -> bool; } trait Other { fn raw() -> *const u8; }")],
            mutate: (boundaries::MEMORY, |t| Some(t.replace("pub trait Memory {", "pub trait Memory { fn raw(&self) -> *const u8;"))),
        },
        Rule::PagedReadCast => Fixture {
            bad: &[(boundaries::PAGES, "impl Pages { fn read() { let n = buf.len().min((len - off) as usize); } }")],
            good: &[(boundaries::PAGES, "impl Pages { fn read() { let n = usize::try_from(len - off); } }")],
            mutate: (boundaries::PAGES, |t| Some(t.replace("usize::try_from(self.len - off).unwrap_or(usize::MAX)", "(self.len - off) as usize"))),
        },
        Rule::ShaderLayoutPanic => Fixture {
            bad: &[(boundaries::GLSL, "impl Parser { fn parse_array_specifier_single() { layout.update(ctx).expect(\"layout\"); } }")],
            good: &[(boundaries::GLSL, "impl Parser { fn parse_array_specifier_single() { self.expect(f, tok)?; layout.update(ctx)?; } }")],
            mutate: (boundaries::GLSL, |t| Some(t.replace("frontend.layouter.update(ctx.module.to_ctx()).map_err", "frontend.layouter.update(ctx.module.to_ctx()).unwrap(); frontend.layouter.update(ctx.module.to_ctx()).map_err"))),
        },
        Rule::StoreDeletion => Fixture {
            bad: &[(boundaries::STORAGE, "impl Store { fn merge(&self) { self.delete(id); } }")],
            good: &[(boundaries::STORAGE, "impl Store { fn gc(&self) { self.delete(id); } fn delete(&self) { self.bucket.delete_object(id); } }")],
            mutate: (boundaries::STORAGE, |t| Some(t + "\nimpl Storage { async fn erase(&self, id: Uuid) -> Result<()> { self.delete(id).await } }")),
        },
        Rule::CatchUnwindAbort => Fixture {
            bad: &[
                ("crates/sake/Cargo.toml", "[package]\nname = \"sake\"\n[dependencies]\nglwgsl = { path = \"../glwgsl\" }\n"),
                ("crates/glwgsl/Cargo.toml", "[package]\nname = \"glwgsl\"\n[target.'cfg(unix)'.dependencies]\nkome = { path = \"../kome\" }\n"),
                ("crates/glwgsl/src/lib.rs", "pub fn f() -> Option<u32> { std::panic::catch_unwind(|| naga()).ok() }\n"),
                ("crates/kome/src/lib.rs", "use std::panic::catch_unwind;\n"),
            ],
            good: &[
                ("crates/sake/Cargo.toml", "[package]\nname = \"sake\"\n[dependencies]\nkome = { path = \"../kome\" }\n[dev-dependencies]\nshaderconf = { path = \"../shaderconf\" }\n[build-dependencies]\nhlslc = { path = \"../hlslc\" }\n"),
                ("crates/sake/src/api.rs", "fn f() {}\n#[cfg(test)]\nmod tests {\n    fn t() { let _ = std::panic::catch_unwind(|| 1); }\n}\n"),
                ("crates/sake/src/x_tests.rs", "fn t() { let _ = std::panic::catch_unwind(|| 1); }\n"),
                ("crates/shaderconf/src/oracle.rs", "fn f() { let _ = std::panic::catch_unwind(|| 1); }\n"),
                ("crates/hlslc/src/lib.rs", "fn f() { let _ = std::panic::catch_unwind(|| 1); }\n"),
                ("crates/kome/src/lib.rs", "// (no catch_unwind: panic=abort)\npub fn f() {}\n"),
            ],
            mutate: ("crates/glwgsl/src/lib.rs", |t| Some(t + "\nfn caught() -> bool { std::panic::catch_unwind(|| 1).is_ok() }\n")),
        },
        Rule::ShellSpecialEnv => Fixture {
            bad: &[
                (".github/workflows/bench.yml", "jobs:\n  plan:\n    steps:\n      - env:\n          GROUPS: ${{ inputs.groups }}\n        run: echo\n"),
                ("tests/x.sh", "#!/bin/sh\nexport RANDOM=4\nSECONDS=0 cargo test\nfor x in a; do PIPESTATUS+=(1); done\n"),
                (".github/workflows/ci.yml", "      - run: |\n          UID=1000 ./run.sh\n"),
            ],
            good: &[
                (".github/workflows/bench.yml", "      - env:\n          ASKED: ${{ inputs.groups }}\n          BENCH_GROUPS: x\n        run: |\n          G=${ASKED:-all}\n          # GROUPS=1 would be dropped\n          echo \"$SECONDS $UID\"\n          while IFS=$'\\t' read -r a; do :; done\n"),
                ("tests/x.sh", "MY_GROUPS=1\nSRANDOM_SEED=2 echo $RANDOM\n[ \"$x\" = GROUPS ]\n"),
                ("vendor/naga/x.sh", "GROUPS=1\n"),
                ("docs/x.md", "GROUPS: 1\n"),
            ],
            mutate: (".github/workflows/bench.yml", |t| Some(t.replace("          ASKED: ${{ inputs.groups }}", "          GROUPS: ${{ inputs.groups }}"))),
        },
        Rule::NtdllOwnCrt => Fixture {
            bad: &[("crates/sake/src/ntcrt.rs", "pub fn init() {\n    let d = \"ntdll\";\n    implement(d, \"towlower\", towlower_h);\n}\n")],
            good: &[(
                "crates/sake/src/ntcrt.rs",
                GOOD_NTCRT,
            )],
            mutate: ("crates/sake/src/ntcrt.rs", |t| Some(t.replace("implement(d, \"_wtoi\",", "implement(d, \"_wtoi_gone\","))),
        },
        Rule::BooleanArg => Fixture {
            bad: &[
                ("crates/sake/src/k32.rs", "fn RtlAnsiStringToUnicodeString(c: &mut Ctx) -> u32 {\n    counted_out(c.a(0), &w, 2, c.a(2) != 0)\n}\n"),
                ("crates/sake/src/x.rs", "fn wait(c: &mut Ctx) -> u32 {\n    alertable(c.a(1))\n}\nfn init() { implement(\"ntdll\", \"NtWaitForSingleObject\", wait); }\n"),
            ],
            good: &[
                ("crates/sake/src/k32.rs", "fn RtlAnsiStringToUnicodeString(c: &mut Ctx) -> u32 {\n    counted_out(c.a(0), &w, 2, c.boolean(2))\n}\nfn RtlInitAnsiString(c: &mut Ctx) -> u32 {\n    c.a(2)\n}\n"),
                ("crates/sake/src/x.rs", "fn wait(c: &mut Ctx) -> u32 {\n    alertable(c.boolean(1)) + c.a(0)\n}\nfn init() { implement(\"ntdll\", \"NtWaitForSingleObject\", wait); }\n"),
            ],
            mutate: ("crates/sake/src/k32.rs", |t| Some(t.replace("counted_out(c.a(0), &w, 2, c.boolean(2))", "counted_out(c.a(0), &w, 2, c.a(2) != 0)"))),
        },
        Rule::DepthGuard => Fixture {
            bad: &[
                ("crates/apitrace/src/trace.rs", "pub const MAX_DEPTH: u32 = 64;\nfn value(&mut self) { self.depth += 1; }\n"),
                ("crates/msidb/src/cond.rs", "fn deeper(&mut self) -> Option<()> { self.depth += 1; Some(()) }\n"),
                ("crates/hlslc/src/parse.rs", "#[cfg(test)]\nmod tests {\n    const MAX_NEST: u32 = 512;\n    fn t() { assert!(1 < MAX_NEST); }\n}\n"),
            ],
            good: &[
                ("crates/apitrace/src/trace.rs", "pub const MAX_DEPTH: u32 = 64;\nfn value(&mut self) -> Result<(), String> { if self.depth >= MAX_DEPTH { return Err(String::new()); } Ok(()) }\n"),
                ("crates/msidb/src/cond.rs", "pub const MAX_DEPTH: usize = 200;\nfn deeper(&mut self) -> Option<()> { self.depth += 1; (self.depth <= MAX_DEPTH).then_some(()) }\n"),
            ],
            mutate: ("crates/msidb/src/sql.rs", |t| Some(t.replace("pub const MAX_DEPTH: usize = 64;", "pub const MAX_NESTING: usize = 64;"))),
        },
        Rule::TypedRegisters => Fixture {
            bad: &[
                ("crates/sake/src/x.rs", "fn f(cpu: &mut Cpu) { cpu.regs[koji::Gpr::ALL[3]] = 0; cpu.set_gpr(koji::Reg64::ALL[ 8 ], 1); }\n"),
                ("crates/koji/tests/t.rs", "fn f(c: &Cpu) -> u32 { c.regs.values()[4] + c.sbase[Seg::ALL[2]] }\n"),
                (KOJI_LIB, "pub struct Cpu { pub regs: [u32; 8], pub sreg: [u16; 6] }\npub struct Gprs(pub [u32; 8]);\nimpl core::ops::Index<usize> for Gprs {}\nimpl From<u32> for Gpr {}\n"),
            ],
            good: &[
                ("crates/sake/src/x.rs", "fn f(cpu: &mut Cpu, i: usize) { cpu.regs[koji::EBX] = 0; cpu.set_gpr(koji::Reg64::R8, 1); cpu.set_gpr(koji::Reg64::ALL[i], 2); }\n// Gpr::ALL[3] in a comment\n"),
                (KOJI_LIB, "pub struct Cpu { pub regs: Gprs, pub sreg: Segs<u16> }\npub struct Gprs([u32; 8]);\nimpl core::ops::Index<Gpr> for Gprs {}\nimpl From<Gpr> for Reg64 {}\nfn at(i: usize) -> Option<Gpr> { if i < 8 { Some(Gpr::ALL[i]) } else { None } }\n"),
                ("crates/dxwgsl/src/d3d9ff.rs", "fn f(regs: &mut [[f32; 4]; 32]) { regs[16] = [0.0; 4]; }\n"),
            ],
            mutate: (KOJI_LIB, |t| Some(t.replacen("    pub regs: Gprs,", "    pub regs: [u32; 8],", 1))),
        },
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
            bad: &[("crates/kura/src/x.rs", "fn lo16(x: u32) -> u16 {\n    x as u16\n}\npub trait Lock<T> {}\n"), ("crates/sake/src/y.rs", "const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] { [0; 16] }\nfn read_guid(p: u32) -> [u8; 16] { mem::array::<16>(p) }\nfn be32(d: &[u8], o: usize) -> Option<u32> {\n    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))\n}\n")],
            good: &[
                ("crates/kura/src/x.rs", "use kome::num::lo16;\nimpl R {\n    fn u32_at(&self, o: usize) -> u32 { 0 }\n}\nfn word(b: &[u8], o: usize, k: usize) -> u32 { u32::from_le_bytes([0; 4]) }\nfn len(b: &[u8], o: usize) -> usize { b.len() - o }\n"),
                ("crates/kome/src/num.rs", "pub fn lo16(x: u32) -> u16 { 0 }\n"),
                (MEM, "pub fn guid_at(p: u32) -> Option<[u8; 16]> { None }\n"),
            ],
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
        Rule::EscapeComment => Fixture {
            bad: &[
                ("crates/sake/src/x.rs", "// TO\x44O: the rest\nfn f() {} // flags ignored\n/// D3DX: everything else is a logged stub returning 0.\nfn g() { eprintln!(\"x not implemented\"); }\n"),
                ("crates/kura/src/y.rs", "/* an approximation of replacing it */\nfn h() {}\n// とりあえず 0 を返す\n"),
                ("tests/z.py", "x = 1  # for now\n"),
            ],
            good: &[
                ("crates/sake/src/x.rs", "// GAA_FLAG_SKIP_FRIENDLY_NAME is ignored, as in Windows\n// (binary labels: not supported, as on Windows)\nfn f() { let s = \"# for nothing\"; }\n/// Simplify(geometry): D2D's own method\n"),
                ("crates/koji/src/sse.rs", "// RCP: approximations, each vendor's own (x86)\n"),
                ("vendor/ring/src/a.rs", "// TO\x44O upstream\n"),
                ("tests/z.sh", "echo \"#for now\"  # what Windows answers\n"),
            ],
            mutate: ("crates/sake/src/k32.rs", |t| Some(t + "\n// for now: 0\nfn later() {}\n")),
        },
        Rule::ConstantHandler => Fixture {
            bad: &[("crates/sake/src/x.rs", "fn SetWorldTransform(_c: &mut Ctx) -> u32 {\n    1\n}\n/// accepted\nfn ok(_: &mut crate::api::Ctx) -> u32 { S_OK }\n")],
            good: &[(
                "crates/sake/src/x.rs",
                "/// GetDoubleClickTime: 500, Windows' default (winref user)\nfn GetDoubleClickTime(_c: &mut Ctx) -> u32 {\n    500\n}\nfn GetDC(c: &mut Ctx) -> u32 {\n    c.a(0)\n}\nfn f(_c: &mut Ctx) -> u32 {\n    let x = 1;\n    x\n}\n",
            )],
            mutate: ("crates/sake/src/k32.rs", |t| Some(t + "\nfn Regression(_c: &mut Ctx) -> u32 {\n    1\n}\n")),
        },
        Rule::ConstantMethod => Fixture {
            bad: &[("crates/kura/src/d3d11/d3d10.rs", "fn device_call(d: u32, m: u32) -> u32 {\n    match m {\n        49 | 51 => {}\n        // GetData\n        7 => S_OK,\n        _ => 0,\n    }\n}\n")],
            good: &[(
                "crates/kura/src/d3d11/d3d10.rs",
                "fn context_call(m: u32, a: u32) -> u32 {\n    match m {\n        // SetMarkerInt: annotations, which Windows' runtime drops without a tool attached (documented)\n        141 => {}\n        12 => S_OK, // AddRef as Windows (winref d3d11api)\n        13 => a,\n        _ => E_NOTIMPL,\n    }\n}\nfn new_texture(dim: u32) -> u32 {\n    match dim {\n        3 => 1,\n        _ => 0,\n    }\n}\nfn blend_state(f: u32) -> u32 {\n    match f {\n        8 | 11 => 1,\n        f => f,\n    }\n}\n",
            )],
            mutate: ("crates/kura/src/d3d11/mod.rs", |t| Some(t + "\nfn regression_call(m: u32) -> u32 {\n    match m {\n        51 | 52 => {}\n        _ => 1,\n    }\n}\n")),
        },
        Rule::ArcTryUnwrap => Fixture {
            bad: &[
                ("crates/sake/src/mf/x.rs", "fn f(d: Arc<Data>) { if let Ok(d) = Arc::try_unwrap(d) { release(d); } }\n"),
                ("crates/masu/src/x.rs", "fn f(d: std::sync::Arc<u32>) -> u32 { std::sync::Arc::try_unwrap(d).unwrap_or(0) }\n"),
            ],
            good: &[
                ("crates/sake/src/mf/x.rs", "fn f(d: Arc<Data>) { if let Some(d) = Arc::into_inner(d) { release(d); } }\nfn g(r: Rc<u8>) { let _ = Rc::try_unwrap(r); }\n"),
                ("crates/sake/src/y.rs", "#[cfg(test)]\nmod tests {\n    fn t(d: Arc<u8>) { let _ = Arc::try_unwrap(d); }\n}\n"),
            ],
            mutate: ("crates/sake/src/mf/source.rs", |t| Some(t + "\nfn unwrap_again(d: Arc<Data>) -> Option<Data> { Arc::try_unwrap(d).ok() }\n")),
        },
        Rule::ArcCount => Fixture {
            bad: &[
                ("crates/sake/src/x.rs", "fn last(d: &Arc<Data>) -> bool { Arc::strong_count(d) == 1 }\n"),
                ("crates/masu/src/x.rs", "fn f(d: &std::sync::Arc<u8>) -> usize { std::sync::Arc::weak_count(d) }\n"),
            ],
            good: &[
                ("crates/sake/src/x.rs", "fn f(d: Arc<Data>) -> Option<Data> { Arc::into_inner(d) }\nfn g(r: &Rc<u8>) -> usize { Rc::strong_count(r) }\n"),
                ("crates/sake/src/y.rs", "#[cfg(test)]\nmod tests {\n    fn t(d: &Arc<u8>) { assert_eq!(Arc::strong_count(d), 1); }\n}\n"),
            ],
            mutate: ("crates/sake/src/mf/source.rs", |t| Some(t + "\nfn last(d: &Arc<Data>) -> bool { Arc::strong_count(d) == 1 }\n")),
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
    for r in [&BUILD_EXT, &BUILD_OK, &TARGET_DIR, &GOLDEN_ANY, &GOLDEN_FILES, &JOB, &CHECKS, &DISALLOWED, &REG_BY_NUMBER, &REG_UNTYPED] {
        assert!(r.is_some());
    }
    for r in regress::regexes() {
        assert!(r.is_some());
    }
}

#[test]
fn the_new_rules_find_each_spelling_alone() {
    // (each bad file on its own: one spelling found does not hide another that is missed)
    for r in [Rule::SafeRawPointerAbi, Rule::UncheckedJitMode, Rule::OutEscape, Rule::EscapeComment, Rule::ArcTryUnwrap, Rule::ArcCount, Rule::ShellSpecialEnv, Rule::DepthGuard, Rule::TypedRegisters] {
        for one in fixture(r).bad.chunks(1) {
            assert!(!r.check(&Tree::made_up(one)).is_empty(), "{}: finds nothing in {}", r.id(), one[0].0);
        }
    }
    // (two numbered registers in each made-up file, the untyped lib.rs's five ways back)
    assert_eq!(Rule::TypedRegisters.check(&Tree::made_up(fixture(Rule::TypedRegisters).bad)).len(), 9);
    // (both silent arms of the bad file: the empty one and the S_OK one whose comment above
    // names the method, not Windows' answer)
    assert_eq!(Rule::ConstantMethod.check(&Tree::made_up(fixture(Rule::ConstantMethod).bad)).len(), 2);
    // (and every site in the bad files, not only the first: w8(ga(key)) is two)
    // (a copy of a number conversion, a lock trait, a GUID builder, a guest GUID reader and a
    // big-endian byte reader)
    assert_eq!(Rule::KomeCopy.check(&Tree::made_up(fixture(Rule::KomeCopy).bad)).len(), 5);
    assert_eq!(Rule::SafeRawPointerAbi.check(&Tree::made_up(fixture(Rule::SafeRawPointerAbi).bad)).len(), 3);
    assert_eq!(Rule::OutEscape.check(&Tree::made_up(fixture(Rule::OutEscape).bad)).len(), 6);
}

#[test]
fn the_abort_build_is_sake_and_what_it_links() {
    // (the real tree: the crates whose catch_unwind never catches; a dev- or build-dependency is
    // not linked into sake.wasm)
    let c = regress::abort_crates(&Tree::repo(&repo()));
    for k in ["crates/sake", "crates/kura", "crates/koji", "crates/kome", "crates/glwgsl", "crates/hlslc", "crates/msidb"] {
        assert!(c.contains(k), "{k} not in {c:?}");
    }
    assert!(!c.contains("crates/shaderconf") && !c.contains("crates/masu"), "{c:?}");
    // (and the switch that makes it so is still there)
    assert!(std::fs::read_to_string(repo().join("crates/sake/build.sh")).unwrap_or_default().contains("-C panic=abort"));
}

#[test]
fn bash_drops_an_assignment_to_groups() {
    // (the reason for shell-special-env, shown on this machine's bash: a script's GROUPS=.. is
    // dropped; an env value gets through or not by the variable and the bash version)
    if let Ok(o) = std::process::Command::new("bash").args(["-c", "GROUPS=gdi; echo \"$GROUPS\""]).output() {
        assert_ne!(String::from_utf8_lossy(&o.stdout).trim(), "gdi");
    }
    assert_eq!(regress::special_sets("A=1\nGROUPS=2\n  export SECONDS=3 # x\n# RANDOM=4\nX_UID=5\n", false), vec![(2, "GROUPS".to_owned()), (3, "SECONDS".to_owned())]);
}

#[test]
fn each_depth_guard_is_in_the_real_tree() {
    let real = Tree::repo(&repo());
    assert!(regress::depth_guard(&real).is_empty());
    // (taking any one guard's comparison away is found)
    for (file, name) in regress::DEPTH_GUARDS {
        let f = Rel::new(file);
        let text = real.text(&f);
        let decl = text.lines().find(|l| l.contains(&format!("const {name}:"))).unwrap_or_default().to_owned();
        let only = text.replace(name, "OTHER").replace(&decl.replace(name, "OTHER"), &decl);
        let t = Tree::with(&repo(), &f, Some(only));
        assert!(!regress::depth_guard(&t).is_empty(), "{file} {name}: its uses taken away is not found");
    }
}
