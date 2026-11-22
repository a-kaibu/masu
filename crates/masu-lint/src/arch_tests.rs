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
    "    implement(d, \"RtlInitUnicodeStringEx\", f); implement(d, \"RtlAppendAsciizToString\", f); implement(d, \"RtlAppendStringToString\", f);\n",
    "    implement(d, \"RtlAppendUnicodeStringToString\", f); implement(d, \"RtlAppendUnicodeToString\", f); implement(d, \"RtlCopyString\", f);\n",
    "    implement(d, \"RtlUpperChar\", f); implement(d, \"RtlUpperString\", f); implement(d, \"RtlUpcaseUnicodeString\", f);\n",
    "    implement(d, \"RtlDowncaseUnicodeString\", f); implement(d, \"RtlDuplicateUnicodeString\", f); implement(d, \"RtlFindCharInUnicodeString\", f);\n",
    "    implement(d, \"RtlCompareUnicodeStrings\", f); implement(d, \"RtlGUIDFromString\", f); implement(d, \"RtlStringFromGUID\", f);\n",
    "    implement(d, \"RtlUTF8ToUnicodeN\", f); implement(d, \"RtlUnicodeToUTF8N\", f); implement(d, \"RtlHashUnicodeString\", f);\n",
    "    implement(d, \"RtlUnicodeStringToAnsiString\", f); implement(d, \"RtlDowncaseUnicodeChar\", f);\n",
    "    implement(d, \"RtlFormatMessage\", f); implement(d, \"RtlFormatMessageEx\", f); implement(d, \"RtlIsTextUnicode\", f);\n",
    "    implement(d, \"_snprintf\", f); implement(d, \"_vsnprintf\", f); implement(d, \"_snwprintf\", f); implement(d, \"_vsnwprintf\", f);\n",
    "    implement(d, \"sscanf\", f); implement(d, \"sscanf_s\", f); implement(d, \"swscanf_s\", f);\n",
    "    implement(d, \"_allmul\", f); implement(d, \"_alldiv\", f); implement(d, \"_allrem\", f); implement(d, \"_alldvrm\", f); implement(d, \"_aulldiv\", f);\n",
    "    implement(d, \"_aullrem\", f); implement(d, \"_aulldvrm\", f); implement(d, \"_allshl\", f); implement(d, \"_allshr\", f); implement(d, \"_aullshr\", f);\n",
    "    implement(d, \"RtlExtendedMagicDivide\", f); implement(d, \"RtlLargeIntegerToChar\", f); implement(d, \"RtlInt64ToUnicodeString\", f);\n",
    "    implement(d, \"RtlSetCurrentDirectory_U\", f); implement(d, \"RtlDetermineDosPathNameType_U\", f); implement(d, \"RtlIsDosDeviceName_U\", f);\n",
    "    implement(d, \"RtlGetFullPathName_U\", f); implement(d, \"RtlGetFullPathName_UEx\", f); implement(d, \"RtlDosPathNameToNtPathName_U\", f);\n",
    "    implement(d, \"RtlDosPathNameToNtPathName_U_WithStatus\", f); implement(d, \"RtlIsNameLegalDOS8Dot3\", f);\n",
    "    implement(d, \"RtlOemStringToUnicodeString\", f); implement(d, \"RtlCompareString\", f);\n",
    "}\n",
);

/// what rebuild::MESSAGE_WIDTH wants, each function saying its part
const GOOD_IO_RESULT: Files = &[
    (
        "crates/sake/src/files.rs",
        "impl File {\n    pub fn read_into(&self, dst: u32, n: u32) -> Result<u32, IoFail> {}\n    pub fn write_from(&self, src: u32, n: u32) -> Result<u32, IoFail> {\n        let b = crate::mem::try_bytes(src, n).ok_or(IoFail::NoAccess)?;\n    }\n    pub fn read_at(&self, start: Option<SeekFrom>, keep: bool, buf: &mut [u8]) -> Result<usize, ReadAt> {}\n}\nfn read_full() { match x { Err(_) => return Err(IoFail::Read), } }\nfn open_shared() { let entry = share.map(|s| ShareEntry::take(&host, access, s)).transpose()?; }\n",
    ),
    ("crates/sake/src/k32.rs", "fn WriteFile(c: &mut Ctx) -> u32 { mem::try_bytes(buf, n).ok_or(files::IoFail::NoAccess) }\n"),
];

const GOOD_D2D_LAYOUT: Files = &[(
    "crates/sake/src/d2d.rs",
    "fn hwnd_swap_chain(cpu: &mut Cpu, hp: u32) { use x::{D2D1_HWND_RENDER_TARGET_PROPERTIES as P, DXGI_SWAP_CHAIN_DESC as D}; let o = (desc + size, desc + size + ps, desc + size + 2 * ps); crate::abi::call(cpu, create, &args); }\nfn CreateHwndRenderTarget(c: &mut Ctx) -> u32 { hwnd_swap_chain(c.ctx().cpu, hp) }\n",
)];

const GOOD_MESSAGE_WIDTH: Files = &[
    ("crates/sake/src/user.rs", "fn PostMessageA(c: &mut Ctx) { c.wparam(2), c.lparam(3); }
fn PostThreadMessageA(c: &mut Ctx) { c.wparam(2), l: c.lparam(3); }
fn send_message(c: &mut Ctx) { c.wparam(2), c.lparam(3); c.ret_lresult(; }
fn send_notify(c: &mut Ctx) { c.wparam(2), c.lparam(3); }
fn SendMessageTimeoutA(c: &mut Ctx) { res.put_size(r.bits()); }
fn SendDlgItemMessageA(c: &mut Ctx) { c.wparam(3), c.lparam(4); }
fn CallWindowProcA(c: &mut Ctx) { abi::call_wndproc(; c.ret_lresult(; }
fn DispatchMessageA(c: &mut Ctx) { c.ret_lresult(; }
fn call_proc(c: &mut Ctx) { abi::call_wndproc(; }
abi::write_slot(at + lay.wparam, self.w.guest_bits(arch), arch);
fn msg_at(c: &mut Ctx) { abi::read_slot(; }
fn GetWindowLongPtrA(c: &mut Ctx) { LongWidth::Ptr; c.ret_long_ptr(; }
fn SetWindowLongPtrA(c: &mut Ctx) { c.long_ptr(2); LongWidth::Ptr; }
fn GetClassLongPtrA(c: &mut Ctx) { LongWidth::Ptr; }
fn SetClassLongPtrA(c: &mut Ctx) { c.long_ptr(2); }
fn SetTimer(c: &mut Ctx) { c.wparam(1); }
fn EndDialog(c: &mut Ctx) { c.long_ptr(1); }
"),
    ("crates/sake/src/abi.rs", "fn call_wndproc(c: &mut Ctx) { Arg::handle(hwnd), Arg::dword(msg), w.into(), l.into(); }
"),
    ("crates/sake/src/kern.rs", "pub struct XMsg {\n    pub w: u64,\n    pub l: u64,\n}\n"),
];

/// what rebuild::RUN_PLAN wants, each file saying its part
const GOOD_RUN_PLAN: Files = &[
    ("tests/winref/run.sh", "{ echo \"# run.sh\"; } > \"$OUT/.plan/$runner\"\n"),
    ("tests/winref/run-browser.mjs", "writeFileSync(join(outDir, \".plan\", runner), text);\n"),
    ("tests/winref/compare.py", "plan_errors = plan.problems(plans, required, got)\nsys.exit(1 if failed or plan_errors else 0)\n"),
    ("tests/winetest/run.py", "all_jobs, unplanned = plan(listed, sel, expected, explicit)\nif not os.path.isfile(win_path):\n            raise FileNotFoundError(win_path)\nall_jobs, not_adopted, missing = adopt(all_jobs, read_tsv(win_path))\nunplanned += missing\nresults[k], judged[k] = combine(rs)\nif unplanned:\n    return 1\n"),
    (".github/workflows/winref.yml", "run: compare.py --require-plan node --require-plan browser-0of4\n"),
    (".github/workflows/conformance.yml", "run: python tests/winetest/run.py --suite rostests --native --repeat 3 --arch i386\n"),
];

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
        Rule::CrtStartup => Fixture {
            bad: &[("crates/sake/src/crt.rs", "fn init() {\n    reg!(malloc, free);\n}\n")],
            good: &[("crates/sake/src/crt.rs", "fn init() {\n    reg!(__C_specific_handler, __dllonexit, __getmainargs, __initenv, __iob_func, __lconv_init, __p__commode, __p__fmode, __set_app_type, __setusermatherr, _acmdln, _amsg_exit, _cexit, _commode, _fmode, _fpreset, _initterm, _iob, _lock, _onexit, _strdup, _unlock, abort, atoi, calloc, exit, fflush, fprintf, free, fwrite, malloc, memcpy, memmove, memset, setvbuf, signal, sprintf, strcmp, strcpy, strlen, strncmp, strrchr, vfprintf, wcscat, wcscmp, wcscpy);\n    implement(\"msvcrt\", \"_swprintf\", swprintf);\n}\n")],
            mutate: ("crates/sake/src/crt.rs", |t| Some(t.replace("_statusfp, _fpreset,", "_statusfp,"))),
        },
        Rule::NtdllOwnCrt => Fixture {
            bad: &[("crates/sake/src/ntcrt.rs", "pub fn init() {\n    let d = \"ntdll\";\n    implement(d, \"towlower\", towlower_h);\n}\n")],
            good: &[(
                "crates/sake/src/ntcrt.rs",
                GOOD_NTCRT,
            )],
            mutate: ("crates/sake/src/ntcrt.rs", |t| Some(t.replace("implement(d, \"_wtoi\",", "implement(d, \"_wtoi_gone\","))),
        },
        Rule::Utf8OneDecoder => Fixture {
            bad: &[("crates/sake/src/text.rs", "fn utf8_to_w(s: &[u8]) -> Vec<u16> {\n    for ch in s.utf8_chunks() {}\n}\n")],
            good: &[
                ("crates/sake/src/text.rs", "fn utf8_to_w(s: &[u8]) -> (Vec<u16>, bool) {\n    crate::rtlstr::utf8_units(s) // not utf8_chunks(\n}\n"),
                ("crates/sake/src/rtlstr.rs", "fn f(b: &[u8]) { for ch in b.utf8_chunks() {} }\n"),
            ],
            mutate: ("crates/sake/src/text.rs", |t| Some(t.replace("crate::rtlstr::utf8_units(s)", "{ let _ = s.utf8_chunks(); crate::rtlstr::utf8_units(s) }"))),
        },
        Rule::MessageOneFormatter => Fixture {
            bad: &[("crates/sake/src/fmtmsg.rs", "fn format(s: &[u16]) -> Vec<u16> {\n    crt::fmt::format(s, m, &mut a)\n}\n")],
            good: &[("crates/sake/src/fmtmsg.rs", "fn format(r: &Request) -> Outcome {\n    rtlfmt::format(r, &mut g) // not a printf\n}\n")],
            mutate: ("crates/sake/src/fmtmsg.rs", |t| Some(t.replace("rtlfmt::format(", "crate::misc::printf("))),
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
        Rule::WgslLoadBounds => Fixture {
            bad: &[("crates/dxwgsl/src/wgsl.rs", "fn load() -> String {\n    format!(\"textureLoad(t{t}, {a}, {l})\")\n}\n")],
            good: &[("crates/dxwgsl/src/wgsl.rs", "fn load() -> String {\n    format!(\"select(vec4<f32>(), textureLoad(t{t}, {a}, {l}), {ok})\")\n}\n// textureLoad( named in a comment\n")],
            mutate: ("crates/dxwgsl/src/wgsl.rs", |t| Some(t.replace("format!(\"select(vec4<{}>(), textureLoad({tv}, {args}), {ok})\", k.s())", "format!(\"textureLoad({tv}, {args})\")"))),
        },
        Rule::ClearRounding => Fixture {
            bad: &[("crates/kura/src/d3d11/clear.rs", "use super::format::f16_bits;\nfn texel(v: f32) -> u32 { f16_bits(v) }\n")],
            good: &[("crates/kura/src/d3d11/clear.rs", "// not f16_bits\nfn texel(v: f32) -> u32 { f16_rtz(v) }\n#[cfg(test)]\nmod tests {\n    fn t() { f16_bits(1.0); }\n}\n")],
            mutate: ("crates/kura/src/d3d11/clear.rs", |t| Some(t.replace("sign | rtz_bits(", "f16_bits(v) | rtz_bits("))),
        },
        Rule::PsQuads => Fixture {
            bad: &[("crates/kura/src/d3d11/query.rs", "const STATS: [u32; 3] = [c::GS_PRIMITIVES, c::C_INVOCATIONS, c::C_PRIMITIVES];\n// c::PS_PAIRS named in a comment\n")],
            good: &[("crates/kura/src/d3d11/query.rs", "const STATS: [u32; 4] = [c::GS_PRIMITIVES, c::C_INVOCATIONS, c::C_PRIMITIVES, c::PS_PAIRS];\n")],
            mutate: ("crates/dxwgsl/src/wgsl/expand.rs", |t| Some(t.replace("ps_pairs(p,", "clip_count(p,"))),
        },
        Rule::LogicOps => Fixture {
            bad: &[("crates/kura/src/d3d11/mod.rs", "fn options() {\n    for k in 1..10 {}\n    // dxwgsl::translate_views(&s.code, true, None, None, &key.1, &key.2, named in a comment\n}\n")],
            good: &[("crates/kura/src/d3d11/mod.rs", "fn options() {\n    for k in 0..10 {}\n    let s = dxwgsl::translate_views(&s.code, true, None, None, &key.1, &key.2, setup, &key.3);\n    let (o, k) = logic::targets(b, f, m, r);\n    let r = dxwgsl::sample_rate(c) && page_multisamples(p);\n    logic::split(&w, &copies, stash.as_ref(), topo, &cuts, flip);\n}\n")],
            mutate: ("crates/kura/src/d3d11/mod.rs", |t| Some(t.replace("for k in 0..10 {", "for k in 1..10 {"))),
        },
        Rule::SnormTargets => Fixture {
            bad: &[("crates/kura/src/d3d11/logic.rs", "fn snorm_twin(v: u32) -> Option<(u32, [u32; 4])> {\n    // 31 => Some((f::P_RGBA8SI, [8, 8, 8, 8])), named in a comment\n    None\n}\n")],
            good: &[("crates/kura/src/d3d11/logic.rs", "fn snorm_twin(v: u32) -> Option<(u32, [u32; 4])> {\n    match v {\n        31 => Some((f::P_RGBA8SI, [8, 8, 8, 8])),\n        51 => Some((f::P_RG8SI, [8, 8, 0, 0])),\n        63 => Some((f::P_R8SI, [8, 0, 0, 0])),\n        _ => None,\n    }\n}\nfn t() { let store = match b { Some(_) | None => Store::Snorm(b), }; }\nfn u() { let s = match b { Some(blend) if ms && !sample_rate && blend.reads_dst() => Store::SnormStash { blend, mark }, }; let r = vec![gpu::SNORM_RESOLVE, tex, copy, stash, mark_tex, mask]; }\n")],
            mutate: ("crates/kura/src/d3d11/mod.rs", |t| Some(t.replace("logic::twin_of(", "logic::copy_of("))),
        },
        Rule::Review6852be1 => Fixture {
            bad: &[
                ("crates/kura/src/jit.rs", "static SHARED_BYTES: AtomicUsize = AtomicUsize::new(0);\n// struct SharedCache named in a comment, jit::TABLE_SLOTS too\n"),
                ("crates/kura/src/vfs.rs", "fn with<R>(f: impl FnOnce(&mut Fs) -> R) -> R { f(fs) }\n"),
            ],
            good: &[
                ("crates/kura/src/jit.rs", "struct SharedCache { map: M, bytes: usize }\nfn init() { vnvm_jit_init(p, n, jit::TABLE_SLOTS); }\nlet i = es.filter(|&i| s.entries.get(i).is_some_and(|e| e.pages.runs_now())).max();\n"),
                ("crates/kura/src/vfs.rs", "fn change<R: Refused>(f: F) -> R { r }\nfn h_read() { x.allows(Use::Read, false)?; }\nfn load() { fs.base = fs.entries(); }\n"),
            ],
            mutate: ("crates/kura/src/vfs/shared.rs", |t| Some(t.replace("fs.undo();", "fs.undo = None;"))),
        },
        Rule::D3d9Status => Fixture {
            bad: &[
                ("crates/kura/src/d3d9/device.rs", "const D3DOK_NOAUTOGEN: u32 = 0x0876_087f;\n"),
                ("crates/sake/src/d3dx9fx.rs", "const D3DERR_MADEUP: u32 = 0x8876_0999;\n"),
            ],
            good: &[
                ("crates/kura/src/d3d9/device.rs", "const D3DOK_NOAUTOGEN: u32 = 0x0876_086f;\nconst S_PRESENT_OCCLUDED: u32 = 0x0876_0878;\n"),
                ("crates/sake/src/d3d7.rs", "const D3DERR_INVALIDCALL: u32 = 0x8007_0057;\n"),
            ],
            mutate: ("crates/kura/src/d3d9/device.rs", |t| Some(t.replace("0x0876_086f", "0x0876_087f"))),
        },
        Rule::D3d9DevFormats => Fixture {
            bad: &[
                ("tests/winref/probes/d3d9dev.c", "static const struct { D3DFORMAT f; const char *n; } fm[] = { { D3DFMT_L8, \"L8\" }, { (D3DFORMAT)MAKEFOURCC('N', 'V', '1', '2'), \"NV12\" } };\n"),
                ("crates/kura/src/d3d9/dev_tests.rs", "let named = |n: &str| match n { \"L8\" => f::L8, other => panic!() };\n"),
            ],
            good: &[
                ("tests/winref/probes/d3d9dev.c", "static const struct { D3DFORMAT f; const char *n; } fm[] = { { D3DFMT_L8, \"L8\" }, { (D3DFORMAT)MAKEFOURCC('N', 'V', '1', '2'), \"NV12\" } };\n"),
                ("crates/kura/src/d3d9/dev_tests.rs", "let named = |n: &str| match n { \"L8\" => f::L8, \"NV12\" => nv12, other => panic!() };\n"),
            ],
            mutate: ("crates/kura/src/d3d9/dev_tests.rs", |t| Some(t.replace("\"G32R32F\" => f::G32R32F,", ""))),
        },
        Rule::ConsoleLines => Fixture {
            bad: &[("crates/sake/src/crt.rs", "fn puts(s: &str) { eprint!(\"{s}\"); }\n")],
            good: &[("crates/sake/src/crt.rs", "fn puts(s: &str) { crate::k32::console_write(s); eprintln!(\"sake: puts\"); }\n// eprint!( in a comment\n")],
            mutate: ("crates/sake/src/crt.rs", |t| Some(t.replacen("crate::k32::console_write(&a2s(&s));", "eprint!(\"{}\", a2s(&s));", 1))),
        },
        Rule::KnownDiffs => Fixture {
            bad: &[("tests/winref/known.txt", "# a gap\nd3d11copy-* ^(want|got) +options logic_op \n")],
            good: &[("tests/winref/known.txt", "# only comments\n\n   # and blanks\n")],
            mutate: ("tests/winref/known.txt", |t| Some(format!("{t}d3d11-* ^(want|got) +made up \n"))),
        },
        Rule::LoadEveryLevel => Fixture {
            bad: &[("crates/dxwgsl/src/wgsl.rs", "fn load() { let tv = format!(\"t{t}\"); }\n// format!(\"tl{t}\") + bv.l{t}.x in a comment\n")],
            good: &[("crates/dxwgsl/src/wgsl.rs", "fn load() { let tv = format!(\"tl{t}\"); let a = format!(\"{lv} + bv.l{t}.x\"); }\n")],
            mutate: ("crates/dxwgsl/src/wgsl.rs", |t| Some(t.replace("format!(\"tl{t}\")", "format!(\"t{t}\")"))),
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
            bad: &[("crates/kura/src/x.rs", "fn lo16(x: u32) -> u16 {\n    x as u16\n}\npub trait Lock<T> {}\n"), ("crates/sake/src/y.rs", "const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] { [0; 16] }\nfn read_guid(p: u32) -> [u8; 16] { mem::array::<16>(p) }\nfn be32(d: &[u8], o: usize) -> Option<u32> {\n    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))\n}\n"), ("crates/sake/src/z_tests.rs", "fn h(b: &[u8]) -> u64 {\n    b.iter().fold(0xcbf29ce484222325, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100000001b3))\n}\n")],
            good: &[
                ("crates/kura/src/x.rs", "use kome::num::lo16;\nimpl R {\n    fn u32_at(&self, o: usize) -> u32 { 0 }\n}\nfn word(b: &[u8], o: usize, k: usize) -> u32 { u32::from_le_bytes([0; 4]) }\nfn len(b: &[u8], o: usize) -> usize { b.len() - o }\n"),
                ("crates/kome/src/num.rs", "pub fn lo16(x: u32) -> u16 { 0 }\n"),
                ("crates/kome/src/golden.rs", "pub const PRIME: u64 = 0x100_0000_01b3;\n"),
                ("crates/sake/src/z_tests.rs", "// (FNV-1a: 0x100_0000_01b3, kome's)\nfn h(b: &[u8]) -> u64 { kome::golden::Fnv::of(b) }\n"),
                (MEM, "pub fn guid_at(p: u32) -> Option<[u8; 16]> { None }\n"),
            ],
            mutate: ("crates/kura/src/audio.rs", |t| Some(t + "\nfn lo32(x: u64) -> u32 {\n    x as u32\n}\n")),
        },
        Rule::GoldenEnv => Fixture {
            bad: &[
                ("crates/sake/src/x.rs", "fn f() { if std::env::var_os(\"SAKE_UPDATE_GOLDEN\").is_some() {} }\n"),
                ("crates/kura/src/y.rs", "fn f() { std::env::var(\"MASU_UPDATE_GOLDEN\"); }\n"),
                ("tools/x.sh", "BLESS=1 cargo test\n"),
                ("crates/asf/tests/pin.rs", "fn f() { if std::env::var_os(\"PIN_RECORD\").is_some() {} }\n"),
                ("crates/masu/src/pack.rs", "fn f() { std::env::var_os(\"MASU_REGEN\"); }\n"),
            ],
            good: &[
                (GOLDEN_HOME, "pub const UPDATE: &str = \"MASU_UPDATE_GOLDEN\";\n"),
                ("crates/sake/src/x.rs", "fn f() { if kome::golden::updating() {} }\n// MASU_UPDATE_GOLDEN=1 rewrites it\n"),
                ("tests/x.sh", "MASU_UPDATE_GOLDEN=1 cargo test\n"),
                ("crates/masu/src/update.rs", "fn f() { std::env::var(\"MASU_UPDATE_URL\"); std::env::var(\"MASU_GOLDEN_GROUP\"); }\n"),
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
                (".github/workflows/ci.yml", "cargo run -q --release -p masu-lint -- arch\ncargo run -q --release -p masu-lint -- dup\ncargo test --workspace --release\ncargo run -q --release -p masu-lint -- ratchet\nrun: tools/ci/own-workspaces.sh\nrun: cargo test -p masu --bin masu -- --exact pack::tests::adding_a_small_game_takes_the_store_lock_file\n"),
                ("crates/sake/src/api.rs", "pub fn fallbacks(dll: &str) -> &'static [&'static str] {\n    if is_crt(dll) { &[\"msvcrt\"] } else { &[\"kernel32\"] }\n}\n/// ```compile_fail\n/// ```\npub type NoReturn = fn(&mut Ctx) -> !;\n"),
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
        Rule::RustLints => Fixture {
            bad: &[
                ("crates/kome/Cargo.toml", "[package]\nname = \"kome\"\n[lints.rust]\nunsafe_code = \"deny\"\n"),
                ("crates/koji/Cargo.toml", "[package]\nname = \"koji\"\n[lints.rust]\nraw_borrows_via_references = \"allow\"\n# raw_borrows_via_references = \"warn\"\n"),
            ],
            good: &[
                ("crates/kome/Cargo.toml", "[package]\nname = \"kome\"\n[lints.rust]\nraw_borrows_via_references = \"warn\"\n"),
                ("crates/koji/Cargo.toml", "[package]\n[lints.rust]\nraw_borrows_via_references = \"deny\" # (the JIT's tables)\n"),
                ("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n"),
                ("vendor/x/Cargo.toml", "[package]\nname = \"x\"\n"),
                ("tools/lint-fixtures/ratchet/Cargo.toml", "[package]\nname = \"fixture\"\n"),
            ],
            mutate: ("crates/kome/Cargo.toml", |t| Some(t.replace("raw_borrows_via_references = \"warn\"", "raw_borrows_via_references = \"allow\""))),
        },
        Rule::BinDeadPub => Fixture {
            bad: &[
                ("crates/masu/src/main.rs", "//! masu\nfn main() {}\n"),
                ("crates/x/src/bin/tool.rs", "#![allow(dead_code_pub_in_binary)]\nfn main() {}\n"),
                ("tools/y/src/main.rs", "// #![warn(dead_code_pub_in_binary)]\nfn main() {}\n"),
            ],
            good: &[
                ("crates/masu/src/main.rs", "//! masu\n#![cfg_attr(not(test), warn(dead_code_pub_in_binary))]\nfn main() {}\n"),
                ("crates/x/src/bin/tool.rs", "#![deny(dead_code_pub_in_binary)]\nfn main() {}\n"),
                ("crates/x/src/lib.rs", "pub fn f() {}\n"),
                ("crates/x/src/bin/tool/helper.rs", "pub fn g() {}\n"),
            ],
            mutate: ("crates/masu/src/main.rs", |t| Some(t.replace("#![cfg_attr(not(test), warn(dead_code_pub_in_binary))]", ""))),
        },
        Rule::HandBitWidth => Fixture {
            bad: &[
                ("crates/koji/src/round.rs", "fn w(n: u64) -> u32 { 64 - n.leading_zeros() }\n"),
                ("crates/kura/src/x.rs", "fn w(a: u32, b: u32) -> usize { (32 - (a | b).leading_zeros()) as usize }\n"),
                ("tools/win32md/src/x.rs", "fn w(v: &[u16]) -> u32 { 16 - v[3].leading_zeros() }\n"),
            ],
            good: &[
                ("crates/koji/src/round.rs", "fn w(n: u64) -> u32 { n.bit_width() }\nfn z(n: u32) -> u32 { n.leading_zeros() }\nfn top(n: u32) -> u32 { 31 - n.leading_zeros() }\n"),
                ("crates/kura/src/x.rs", "// 32 - x.leading_zeros() was written by hand here\nconst S: &str = \"64 - n.leading_zeros()\";\n"),
            ],
            mutate: ("crates/kome/src/num.rs", |t| Some(t + "\npub fn width(v: u32) -> u32 { 32 - v.leading_zeros() }\n")),
        },
        Rule::HandUtf16 => Fixture {
            bad: &[
                ("crates/sake/src/ini.rs", "fn s(b: &[u8]) -> String { let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect(); String::from_utf16_lossy(&u) }\n"),
                ("crates/asf/src/lib.rs", "fn s(b: &[u8]) -> Vec<u16> {\n    b.chunks_exact(2)\n        .map(|p| u16::from_le_bytes(p.try_into().unwrap_or_default()))\n        .collect()\n}\n"),
            ],
            good: &[
                ("crates/sake/src/ini.rs", "fn s(b: &[u8]) -> String { kome::le::utf16_lossy(b) }\nfn p(b: &[u8]) -> Vec<[u8; 2]> { b.chunks_exact(2).map(|c| [c[0], c[1]]).collect() }\nfn i(b: &[u8]) -> i16 { i16::from_le_bytes([b[0], b[1]]) }\n"),
                (modern::LE, "pub fn units16(b: &[u8]) -> impl Iterator<Item = u16> + '_ { b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])) }\n"),
            ],
            mutate: ("crates/sake/src/ini.rs", |t| Some(t + "\nfn hand(b: &[u8]) -> Vec<u16> { b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect() }\n")),
        },
        Rule::NoReturnHandler => Fixture {
            bad: &[
                ("crates/sake/src/k32.rs", "fn ExitProcess(c: &mut Ctx) -> u32 { exit_all(c.a(0)); 0 }\nfn FatalAppExitA(c: &mut Ctx) -> ! { exit_all(1) }\nfn ExitThread(c: &mut Ctx) -> ! { t(c) }\nfn _endthreadex(c: &mut Ctx) -> ! { t(c) }\nfn _endthread(c: &mut Ctx) -> ! { t(c) }\nfn FreeLibraryAndExitThread(c: &mut Ctx) -> ! { t(c) }\n"),
                ("crates/sake/src/crt_io.rs", "fn _assert(c: &mut Ctx) -> ! { a(c) }\nfn _wassert(c: &mut Ctx) -> ! { a(c) }\n"),
            ],
            good: &[
                ("crates/sake/src/k32.rs", "fn ExitProcess(c: &mut Ctx) -> ! { exit_all(c.a(0)) }\nfn FatalAppExitA(c: &mut Ctx) -> ! { exit_all(1) }\nfn ExitThread(c: &mut Ctx) -> ! { t(c) }\nfn _endthreadex(c: &mut Ctx) -> ! { t(c) }\nfn _endthread(c: &mut Ctx) -> ! { t(c) }\nfn FreeLibraryAndExitThread(c: &mut Ctx) -> ! { t(c) }\nfn Sleep(c: &mut Ctx) -> u32 { 0 }\n"),
            ],
            mutate: ("crates/sake/src/k32.rs", |t| Some(t.replace("fn ExitProcess(c: &mut Ctx) -> ! {", "fn ExitProcess(c: &mut Ctx) -> u32 {"))),
        },
        Rule::WildcardUnreachable => Fixture {
            bad: &[
                ("crates/sake/src/xof.rs", "fn f(t: &Ty, s: Option<u8>) -> u8 { match (t, s) { (_, Some(s)) => s, (Ty::Prim(p), _) => *p, _ => unreachable!() } }\n"),
                ("crates/shaderconf/src/ffstages.rs", "fn op(o: u32) -> f64 { match o { 2 => 1.0, _ => { unreachable!(\"op {o}\") } } }\n"),
                ("crates/kura/src/x.rs", "fn f(v: u8) -> u8 { match v { 0 => 1, _ => core::unreachable!() } }\n"),
            ],
            good: &[
                ("crates/sake/src/xof.rs", "fn f(b: &Body) -> u8 { match b { Body::Tpl(s) => s.0, Body::Prim(p) => *p } }\nfn g(v: u8) -> u8 { match v { 0 => 1, _ if v > 9 => unreachable!(), _ => 2 } }\n#[cfg(test)]\nmod tests {\n    fn t(v: u8) -> u8 { match v { 0 => 1, _ => unreachable!() } }\n}\n"),
                ("crates/kura/src/x_tests.rs", "fn t(v: u8) -> u8 { match v { 0 => 1, _ => unreachable!() } }\n"),
                ("crates/kura/src/y.rs", "fn f(v: Never) -> u8 { match v { Never::A => unreachable!() } }\n// _ => unreachable!()\n"),
            ],
            mutate: ("crates/sake/src/xof.rs", |t| Some(t + "\nfn back(t: &Ty) -> u8 { match t { Ty::Prim(_) => 0, _ => unreachable!() } }\n")),
        },
        Rule::GuestScratch => Fixture {
            bad: &[
                ("crates/sake/src/d3dx9.rs", "fn f(c: &mut Ctx) -> u32 {\n    let pt = mem::heap_alloc(8, true);\n    com_call(c, 1, 2, &[pt]);\n    let v = r32(pt);\n    mem::heap_free(pt);\n    v\n}\n"),
                ("crates/sake/src/w16api.rs", "fn init() {\n    reg(\"USER.DrawText\", |c| {\n        let r = crate::mem::heap_alloc(16, true);\n        if c.a(0) == 0 {\n            crate::mem::heap_free(r);\n            return 0;\n        }\n        crate::mem::heap_free(r);\n        0\n    });\n}\n"),
            ],
            good: &[
                ("crates/sake/src/d3dx9.rs", "fn f(c: &mut Ctx) -> u32 {\n    let pt_own = mem::Scratch::new(8, true);\n    let pt = pt_own.get();\n    com_call(c, 1, 2, &[pt]);\n    r32(pt)\n}\n"),
                ("crates/sake/src/w16.rs", "fn alloc(size: u32) -> u32 {\n    let b = crate::mem::heap_alloc(size, true);\n    if sel(b) == 0 {\n        crate::mem::heap_free(b);\n    }\n    b\n}\n"),
                ("crates/sake/src/mem.rs", "fn t() { let b = heap_alloc(4, true); heap_free(b); }\n"),
                ("crates/sake/src/x.rs", "#[cfg(test)]\nmod tests {\n    fn t() { let b = mem::heap_alloc(4, true); mem::heap_free(b); }\n}\n"),
            ],
            mutate: ("crates/sake/src/glu.rs", |t| Some(t + "\nfn hand() -> u32 {\n    let p = mem::heap_alloc(8, true);\n    let v = mem::r32(p);\n    mem::heap_free(p);\n    v\n}\n")),
        },
        Rule::HandStd => Fixture {
            bad: &[
                ("crates/sake/src/a.rs", "fn f(s: &mut S) { s.on = !s.on; }\n"),
                ("crates/sake/src/b.rs", "fn f(name: &str) -> &str { name.strip_prefix(\"\\\\?\\\\\").unwrap_or(name) }\n"),
                ("crates/kura/src/c.rs", "fn f(o: Option<u32>) -> String { o.map(|v| v.to_string()).unwrap_or_default() }\n"),
                ("crates/kura/src/d.rs", "fn f(o: Option<&str>) -> String { o.map_or(String::new(), |v| format!(\"{v}\")) }\n"),
                ("crates/masu/src/e.rs", "fn f(ok: bool) -> Result<(), E> { if ok { Ok(()) } else { Err(E::Bad) } }\n"),
                ("crates/koji/src/g.rs", "fn f(v: &[u8]) -> Vec<u32> { (0..v.len()).map(|i| u32::from(v[i])).collect::<Vec<u32>>() }\n"),
            ],
            good: &[
                ("crates/sake/src/a.rs", "fn f(s: &mut S) { s.on.toggle(); s.a = !s.b; }\n"),
                ("crates/sake/src/b.rs", "fn f(name: &str, d: &str) -> &str { name.trim_prefix(\"x\"); name.strip_prefix(\"y\").unwrap_or(d) }\n"),
                ("crates/kura/src/c.rs", "fn f(o: Option<u32>) -> String { o.map_or_default(|v| v.to_string()) }\nfn g(v: &[Option<u8>]) -> Vec<u8> { v.iter().map(|x| x.unwrap_or_default()).collect() }\n"),
                ("crates/masu/src/e.rs", "fn f(ok: bool) -> Result<(), E> { ok.ok_or(E::Bad) }\nfn g(ok: bool) -> Result<u8, E> { if ok { Ok(1) } else { Err(E::Bad) } }\n"),
                ("crates/koji/src/g.rs", "fn f(n: u32) -> Vec<u32> { (0..n).map(|i| i * 2).collect::<Vec<u32>>() }\nfn g(v: &[u8]) -> Vec<u32> { Vec::from_fn(v.len(), |i| u32::from(v[i])) }\n"),
            ],
            mutate: ("crates/kome/src/num.rs", |t| Some(t + "\npub fn flip(b: &mut bool) { *b = !*b; }\npub fn d(o: Option<u8>) -> u8 { o.map(|v| v + 1).unwrap_or_default() }\n")),
        },
        Rule::KaniFeatures => Fixture {
            bad: &[
                ("formal/kani.sh", "#!/bin/bash\nfor c in kura; do\n  (cd \"crates/$c\" && cargo kani)\ndone\n"),
                ("crates/kura/Cargo.toml", "[package]\nname = \"kura\"\n[dependencies]\nkome = { path = \"../kome\" }\n[dev-dependencies]\nproptest = \"1\"\n"),
                ("crates/kura/src/lib.rs", "//! kura\n#![cfg_attr(kani, feature(bool_toggle))]\nmod a;\n"),
                ("crates/kura/src/a.rs", "fn f(n: usize) -> Vec<u8> { Vec::<_>::from_fn(n, |i| i as u8) }\n"),
                ("crates/kome/src/lib.rs", "pub fn t(s: &str) -> &str { s.trim_suffix('\\r') }\n"),
            ],
            good: &[
                ("formal/kani.sh", "#!/bin/bash\nfor c in kura; do\n  (cd \"crates/$c\" && cargo kani)\ndone\n"),
                ("crates/kura/Cargo.toml", "[package]\nname = \"kura\"\n[dependencies]\nkome = { path = \"../kome\" }\n[dev-dependencies]\nshaderconf = { path = \"../shaderconf\" }\n"),
                ("crates/kura/src/lib.rs", "//! kura\n#![cfg_attr(kani, feature(vec_from_fn, bool_toggle))]\nmod a;\n"),
                ("crates/kura/src/a.rs", "fn f(n: usize, b: &mut bool) -> Vec<u8> { b.toggle(); Vec::<_>::from_fn(n, |i| i as u8) }\n#[cfg(test)]\nmod tests {\n    fn t(s: &str) -> &str { s.trim_prefix(\"x\") }\n}\n"),
                ("crates/kome/src/lib.rs", "pub fn t(s: &str) -> &str { s.strip_suffix('\\r').unwrap_or(s) }\n"),
                ("crates/shaderconf/src/lib.rs", "pub fn t(s: &str) -> &str { s.trim_prefix(\"x\") }\n"),
            ],
            mutate: ("crates/kura/src/lib.rs", |t| Some(t.replace("#![cfg_attr(kani, feature(vec_from_fn, bool_toggle, trim_prefix_suffix))]", ""))),
        },
        Rule::KaniCover => Fixture {
            bad: &[
                ("formal/kani.sh", "#!/bin/bash\nfor c in kura; do\n  (cd \"crates/$c\" && cargo kani)\ndone\n"),
                ("crates/kura/src/a.rs", "#[cfg(kani)]\nmod proofs {\n    #[kani::proof]\n    fn f() {\n        let x: u32 = kani::any();\n        kani::assume(x < 4 && x > 8);\n        assert!(x == 0);\n    }\n    #[kani::proof]\n    fn g() {\n        let Some(n) = std::num::NonZeroU32::new(kani::any()) else { return };\n        assert!(n.get() > 0);\n    }\n}\n"),
                ("crates/kome/src/num.rs", "#[cfg(kani)]\nmod proofs {\n    #[kani::proof]\n    fn h() {\n        let x: u8 = kani::any();\n        assert!(u32::from(x) < 256);\n    }\n}\n"),
            ],
            good: &[
                ("formal/kani.sh", "#!/bin/bash\nfor c in kura kome; do\n  (cd \"crates/$c\" && cargo kani)\ndone\n"),
                ("crates/kura/src/a.rs", "#[cfg(kani)]\nmod proofs {\n    #[kani::proof]\n    fn f() {\n        let x: u32 = kani::any();\n        kani::assume(x < 4);\n        assert!(x < 5);\n        kani::cover!(x == 3, \"the top\");\n    }\n    #[kani::proof]\n    fn g() {\n        let n: u8 = kani::any();\n        let returned = n.checked_add(1);\n        assert!(returned.is_none() == (n == 255));\n    }\n}\nfn not_a_proof(x: u32) -> u32 { if x == 0 { return 1 } x }\n"),
                ("crates/kome/src/num.rs", "#[cfg(kani)]\nmod proofs {\n    #[kani::proof]\n    fn h() {\n        let x: u8 = kani::any();\n        assert!(u32::from(x) < 256);\n    }\n}\n"),
            ],
            mutate: ("crates/sake/src/mem_proofs.rs", |t| Some(t.replace("    kani::cover!(n > u32::MAX - PAGE, \"a length near 4 GB (review A-13)\");\n    kani::cover!(touched && i == pages - 1 && off < PAGE && pages > 1, \"a range to the allocation's end\");\n    kani::cover!(!touched && i > first, \"a page after the range\");\n", ""))),
        },
        Rule::VerusMirror => Fixture {
            bad: &[
                ("formal/verus/cdc.rs", "verus! { pub fn next() {} }\nfn main() {}\n"),
                ("crates/masu/src/cdc.rs", "pub fn next() {}\n"),
            ],
            good: &[
                ("formal/verus/cdc.rs", "verus! { pub fn next() {} }\nfn main() {}\n"),
                ("crates/masu/src/cdc.rs", "pub fn next() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn same() { assert!(include_str!(\"../../../formal/verus/cdc.rs\").contains(\"fn next\")); }\n}\n"),
            ],
            mutate: ("crates/masu/src/cdc.rs", |t| Some(t.replace("include_str!(\"../../../formal/verus/cdc.rs\")", "\"\""))),
        },
        Rule::UncheckedOutput => Fixture {
            bad: &[
                ("crates/masu-lint/src/tree.rs", "fn files(root: &Path) -> Vec<String> {\n    let o = Command::new(\"git\").arg(\"ls-files\").current_dir(root).output();\n    o.map(|o| String::from_utf8_lossy(&o.stdout).lines().map(str::to_owned).collect()).unwrap_or_default()\n}\n"),
                ("tools/win32md/src/main.rs", "impl X {\n    fn rev(&self) -> String {\n        let o = Command::new(\"git\").arg(\"rev-parse\").output()?;\n        String::from_utf8_lossy(&o.stdout).into()\n    }\n}\n"),
            ],
            good: &[
                ("crates/masu-lint/src/tree.rs", "fn files(root: &Path) -> Result<Vec<String>, String> {\n    let o = Command::new(\"git\").arg(\"ls-files\").output().map_err(|e| e.to_string())?;\n    if !o.status.success() {\n        return Err(\"failed\".into());\n    }\n    Ok(String::from_utf8_lossy(&o.stdout).lines().map(str::to_owned).collect())\n}\n"),
                ("crates/masu/src/x.rs", "fn has() -> bool { Command::new(\"ffmpeg\").output().is_ok() }\n#[cfg(test)]\nmod tests {\n    fn t() -> Vec<u8> { Command::new(\"x\").output().unwrap().stdout }\n}\n"),
            ],
            mutate: ("crates/masu-lint/src/tree.rs", |t| Some(t.replace("o.status", "o.stderr"))),
        },
        Rule::HandLock => Fixture {
            bad: &[
                ("crates/masu/src/pack.rs", "fn acquire(store: &Path) -> Result<File> {\n    let file = File::open(store)?;\n    file.lock()?;\n    Ok(file)\n}\n"),
                ("crates/masu/src/webassets.rs", "fn prune(p: &Path) {\n    match std::fs::File::open(p.join(\".done\")) {\n        Ok(f) => if f.try_lock().is_ok() {},\n        Err(_) => {}\n    }\n    std::fs::OpenOptions::new().write(true).open(p).unwrap().lock_shared().unwrap();\n}\n"),
            ],
            good: &[
                ("crates/masu/src/lockfile.rs", "pub struct LockFile(File);\nimpl LockFile {\n    pub fn lock(&self) -> std::io::Result<()> {\n        let f = OpenOptions::new().write(true).open(\"x\")?;\n        f.lock()?;\n        self.0.lock()\n    }\n}\n"),
                ("crates/masu/src/pack.rs", "fn acquire(store: &Path) -> Result<LockFile> {\n    let file = LockFile::in_dir(store, STORE_LOCK)?;\n    file.lock()?;\n    let m = SHARED.lock();\n    let f = File::open(store)?;\n    let g = m.lock();\n    Ok(file)\n}\n"),
            ],
            mutate: ("crates/masu/src/pack.rs", |t| Some(t.replace("let file = crate::lockfile::LockFile::in_dir(store, STORE_LOCK)?;", "let file = File::open(store)?;"))),
        },
        Rule::StageRun => Fixture {
            bad: &[
                ("crates/kura/src/d3d11/expand.rs", "fn run(st: &State) { let g = st.shaders[0]; }\n"),
                ("crates/kura/src/d3d11/expand_tess.rs", "// st.shaders[1] st.shaders[2] patch_points(st.topology)\nfn stages() {}\n#[cfg(test)]\nmod tests { fn t(st: &State) { st.shaders[1]; st.shaders[2]; patch_points(st.topology); } }\n"),
                ("crates/kura/src/d3d11/mod.rs", "fn dispatch(st: &State) { let cs = st.shaders[3]; }\n"),
            ],
            good: &[
                ("crates/kura/src/d3d11/expand.rs", "fn run(st: &State) { let g = st.shaders[0]; }\n"),
                ("crates/kura/src/d3d11/expand_tess.rs", "fn stages(st: &State) { let (h, d) = (st.shaders[1], st.shaders[2]); let n = patch_points(st.topology); }\n"),
                ("crates/kura/src/d3d11/mod.rs", "fn dispatch(st: &State) { let cs = st.shaders[3]; }\n"),
            ],
            mutate: ("crates/kura/src/d3d11/expand_tess.rs", |t| Some(t.replace("let (hs, ds) = (st.shaders[1], st.shaders[2]);", "let (hs, ds) = (0, 0);"))),
        },
        Rule::NanMinMax => Fixture {
            bad: &[
                ("crates/dxwgsl/src/wgsl.rs", "fn t(&mut self) { match op {\n    Opcode::Min => self.cw(i, F, F, |a| format!(\"min({}, {})\", a[0], a[1]))?,\n    Opcode::Max => self.cw(i, F, F, |a| format!(\"max({}, {})\", a[0], a[1]))?,\n} }\n"),
                ("crates/hlslc/src/lower.rs", "// h.max_tess\nfn write_outputs() {}\n"),
                ("crates/kura/src/d3d11/expand_tess.rs", "fn factors() { match m { Some(m) if v > m => m, Some(_) | None => v } }\n#[cfg(test)]\nmod tests { fn t() { v.is_nan() => m } }\n"),
            ],
            good: &[
                ("crates/dxwgsl/src/wgsl.rs", "fn t(&mut self) { match op {\n    Opcode::Min => self.per_comp(i, &[F, F], F, |a| format!(\"d3d_min({}, {})\", a[0], a[1]))?,\n    Opcode::Max => self.per_comp(i, &[F, F], F, |a| format!(\"d3d_max({}, {})\", a[0], a[1]))?,\n    Opcode::Imin => self.cw(i, I, I, |a| format!(\"min({}, {})\", a[0], a[1]))?,\n} }\n"),
                ("crates/hlslc/src/lower.rs", "fn write_outputs(&mut self) { if let Some(m) = self.hs.as_ref().and_then(|h| h.max_tess) {} }\n"),
                ("crates/kura/src/d3d11/expand_tess.rs", "fn factors() { match m { Some(m) if v > m || v.is_nan() => m, Some(_) | None => v } }\n"),
            ],
            mutate: ("crates/dxwgsl/src/wgsl.rs", |t| Some(t.replace("format!(\"d3d_min({}, {})\"", "format!(\"min({}, {})\""))),
        },
        Rule::PcsgPack => Fixture {
            bad: &[("crates/hlslc/src/lower.rs", "// pc_slot(list, n) Lane::mask(n) << at\nfn sig() { let reg = 0; let mask = Lane::mask(n); }\n")],
            good: &[("crates/hlslc/src/lower.rs", "fn sig() { let packed = pc_slot(list, n); let mask = Lane::mask(n) << at; }\n")],
            mutate: ("crates/hlslc/src/lower.rs", |t| Some(t.replace("pc_slot(list, n)", "None::<(u32, u8)>"))),
        },
        Rule::RunPlan => Fixture {
            bad: &[
                ("tests/winref/compare.py", "        if not os.path.isdir(os.path.join(got, probe)):\n            continue  # (not run: a filtered run.sh)\n    sys.exit(1 if failed else 0)\n"),
                ("tests/winetest/run.py", "        if not os.path.exists(exe):\n            print(f\"missing {exe}\", file=sys.stderr)\n            continue\n"),
                ("tests/winetest/run.py", "windows = read_tsv(win_path) if os.path.exists(win_path) else {}\n"),
                ("tests/winetest/run.py", "def report(path):\n    with open(path, \"w\") as f:\n        f.write(msg)\n"),
                ("tests/winetest/run.py", "for k, rs in runs.items():\n    results[k] = rs[-1]\n"),
                (".github/workflows/conformance.yml", "run: python tests/winetest/run.py --suite rostests --native --arch i386\n"),
            ],
            good: GOOD_RUN_PLAN,
            mutate: ("tests/winref/compare.py", |t| Some(t.replace("sys.exit(1 if failed or plan_errors else 0)", "sys.exit(1 if failed else 0)"))),
        },
        Rule::MessageWidth => Fixture {
            bad: &[
                ("crates/sake/src/user.rs", "fn GetWindowLongPtrA(c: &mut Ctx) -> u32 {\n    GetWindowLongA(c)\n}\nfn PostMessageA(c: &mut Ctx) -> u32 {\n    post(c.a(0), c.a(1), c.a(2), c.a(3)) as u32\n}\n"),
                ("crates/sake/src/abi.rs", "impl From<u32> for Wparam { fn from(v: u32) -> Wparam { Wparam(v.into()) } }\n"),
                ("crates/sake/src/kern.rs", "pub struct XMsg {\n    pub w: u32,\n    pub l: u32,\n}\n"),
            ],
            good: GOOD_MESSAGE_WIDTH,
            mutate: ("crates/sake/src/user.rs", |t| Some(t.replace("let (h, m, w, l) = (c.a(0), c.a(1), c.wparam(2), c.lparam(3));", "let (h, m, w, l) = (c.a(0), c.a(1), c.a(2), c.a(3));"))),
        },
        Rule::NotDone => Fixture {
            bad: &[
                ("crates/sake/src/api.rs", "pub fn unimplemented_result(name: &str) -> u32 {\n    0\n}\nfn unimplemented_call(c: &mut Ctx) -> u32 {\n    let r = unimplemented_result(&n);\n    r\n}\n"),
                ("crates/sake/src/returns.txt", "kernel32 CreateFileA lasterror\nole32 CoCreateInstance hresult_\n"),
            ],
            good: &[
                ("crates/sake/src/api.rs", "fn returns() -> &'static R {\n    include_str!(\"returns.txt\")\n}\npub fn unimplemented_result(name: &str) -> NotDone {\n    if let Some(k) = returns().get(&(key.as_str(), f)) {}\n}\nfn unimplemented_call(c: &mut Ctx) -> u32 {\n    let (r, err) = unimplemented_result(&n).result();\n}\n"),
                ("crates/sake/src/returns.txt", "kernel32 CreateFileA lasterror\nole32 CoCreateInstance hresult\n"),
            ],
            mutate: ("crates/sake/src/api.rs", |t| Some(t.replace("let (r, err) = unimplemented_result(&n).result();", "let (r, err) = (0, None::<u32>);"))),
        },
        Rule::GdiLockOrder => Fixture {
            bad: &[
                ("crates/sake/src/gdi.rs", "fn bit_blt(dst: u32, src: u32) -> u32 {\n    u32::from(draw(dst, |t, ds, c| {\n        let Some(s) = source(t, src) else { return Rect::default() };\n        blit(c, s)\n    }))\n}\nthread_local! { static HELD: u32 = 0; }\n"),
                ("crates/sake/src/user.rs", "fn ensure_surface(w: &mut Window) {\n    if crate::gdi::holds(p) {}\n}\n"),
            ],
            good: &[(
                "crates/sake/src/gdi.rs",
                "fn draw_src(dc: u32, src: u32, mut f: impl FnMut(&Objs, &DcState, &mut Canvas, Option<&Source>) -> Rect) -> bool {\n    let from = if src != 0 && src != dc { source(t, src) } else { None };\n}\nfn AlphaBlend() -> u32 {\n    if src == dst && s.0 + s.2 > d.0 && s.0 < d.0 + d.2 && s.1 + s.3 > d.1 && s.1 < d.1 + d.3 {\n        crate::k32::last_error(87);\n        return 0;\n    }\n}\nfn bit_blt(dst: u32, src: u32) -> u32 {\n    u32::from(draw_src(dst, src, |t, ds, c, from| blit(c, from)))\n}\nfn redraw(x: u32) { x.draw(source(1)); }\n",
            )],
            mutate: ("crates/sake/src/gdi.rs", |t| Some(t.replace("u32::from(draw_src(dst, src, |_, _, cv, from| {", "u32::from(draw_src(dst, src, |t, _, cv, from| {\n        let _ = source(t, src);"))),
        },
        Rule::Lifetimes => Fixture {
            bad: &[
                ("crates/sake/src/pe.rs", "pub fn load_library(cpu: &mut Cpu, name: &str) -> u32 {\n    match load_dll_file(name) {\n        Ok(b) => {\n            init_dlls(cpu);\n            b\n        }\n    }\n}\n"),
                ("crates/sake/src/k32.rs", "pub fn exit_thread(code: u32) -> ! {\n    if t.tid == crate::process::first_tid(crate::pid()) {\n        // the main thread: the process\n    }\n}\n"),
            ],
            good: &[
                ("crates/sake/src/pe.rs", "pub fn init_dlls(cpu: &mut Cpu) -> Result<(), InitFailed> {\n    return Err(InitFailed(base));\n}\nfn init_loaded(cpu: &mut Cpu, b: u32) -> Result<u32, LoadFailure> {}\npub fn load_library_searching(cpu: &mut Cpu) -> Result<u32, LoadFailure> {\n    match x { Ok(b) => init_loaded(cpu, b), }\n}\n"),
                ("crates/sake/src/k32.rs", "pub fn exit_thread(code: u32) -> ! {\n    let last = crate::lives::ends(t.tid);\n}\n"),
                ("crates/sake/src/lib.rs", "fn start() {\n    if let Err(pe::InitFailed(b)) = pe::init_dlls(cpu) {}\n}\n"),
            ],
            mutate: ("crates/sake/src/k32.rs", |t| Some(t.replace("let last = crate::lives::ends(t.tid);", "let last = t.tid == crate::process::first_tid(crate::pid());"))),
        },
        Rule::IoResult => Fixture {
            bad: &[
                ("crates/sake/src/files.rs", "impl File {\n    pub fn write_from(&self, src: u32, n: u32) -> u32 {\n        let mut f = self.f.locked();\n        f.write_all(&crate::mem::bytes(src, n)).map_or(0, |()| n)\n    }\n}\n"),
                ("crates/sake/src/k32.rs", "fn WriteFile(c: &mut Ctx) -> u32 {\n    f.write_at(start, crate::iocp::is_overlapped(h), &mem::bytes(buf, n))\n}\n"),
            ],
            good: GOOD_IO_RESULT,
            mutate: ("crates/sake/src/files.rs", |t| Some(t.replace("let b = crate::mem::try_bytes(src, n).ok_or(IoFail::NoAccess)?;", "let b = crate::mem::bytes(src, n);"))),
        },
        Rule::HandleKey => Fixture {
            bad: &[
                ("crates/sake/src/obj.rs", "impl Objs {\n    pub fn key(&self, h: &u32) -> Option<Key> {\n        self.handles.get(h).copied().or_else(|| self.map.contains_key(&Key(*h)).then_some(Key(*h)))\n    }\n}\n"),
                ("crates/sake/src/k32.rs", "static GATE: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());\nfn ResumeThread(c: &mut Ctx) -> u32 {\n    if let Some((_, n)) = GATE.locked().iter_mut().find(|(x, n)| *x == h && *n > 0) {}\n}\n"),
            ],
            good: &[
                ("crates/sake/src/obj.rs", "impl Objs {\n    pub fn key(&self, h: &u32) -> Option<Key> {\n        self.handles.get(h).copied()\n    }\n}\n"),
                ("crates/sake/src/k32.rs", "static GATE: Mutex<Vec<(obj::Key, u32)>> = Mutex::new(Vec::new());\nfn gate_key(h: u32) -> Option<obj::Key> {\n    obj::key_of(h)\n}\n"),
                ("crates/sake/src/lib.rs", "pub fn spawn_gated(start: u32, param: u32, stack: u32, obj: u32, gate: Option<obj::Key>, kind: Lives) -> u32 {}\n"),
            ],
            mutate: ("crates/sake/src/obj.rs", |t| Some(t.replace("        self.handles.get(h).copied()\n    }", "        self.handles.get(h).copied().or_else(|| self.map.contains_key(&Key(*h)).then_some(Key(*h)))\n    }"))),
        },
        Rule::D2dLayout => Fixture {
            bad: &[("crates/sake/src/d2d.rs", "fn CreateHwndRenderTarget(c: &mut Ctx) -> u32 {\n    let hwnd = r32(hp);\n    let (mut w, mut h) = (r32(hp + 4), r32(hp + 8));\n    for (o, v) in [(0, w), (4, h), (44, hwnd)] { w32(desc + o, v); }\n    let (sc_out, dev_out, ctx_out) = (desc + 60, desc + 64, desc + 68);\n}\n")],
            good: GOOD_D2D_LAYOUT,
            mutate: ("crates/sake/src/d2d.rs", |t| Some(t.replace("(desc + size, desc + size + ps, desc + size + 2 * ps)", "(desc + size, desc + size + 4, desc + size + 8)"))),
        },
        Rule::WarpSetup => Fixture {
            bad: &[
                ("crates/kura/src/d3d11/expand.rs", "// setup_wanted(d, st) setup_draw(d, st, \nfn draw() {}\n"),
                ("crates/dxwgsl/src/wgsl.rs", "fn inputs() { let a = \"@location(0) @interpolate(linear) v0: vec4<f32>\"; }\n"),
                ("crates/dxwgsl/src/wgsl/setup.rs", "fn sx_plane() {}\nconst W: &str = \"let g = fma(d10, dy20, d20 * dy01);\";\n"),
            ],
            good: &[
                ("crates/kura/src/d3d11/expand.rs", "fn draw(d: &mut D11, st: &State) { let w = setup_wanted(d, st); setup_draw(d, st, &stv); }\n"),
                ("crates/dxwgsl/src/wgsl.rs", "fn ps() { expand::setup::inputs(code, samples); }\n"),
                ("crates/dxwgsl/src/wgsl/setup.rs", "const W: &str = \"fn sx_plane(a0: f32) -> vec3<f32> { let g = d10 * dy20 + d20 * dy01; }\";\n"),
            ],
            mutate: ("crates/kura/src/d3d11/expand.rs", |t| Some(t.replace("let wants_setup = setup_wanted(d, st);", "let wants_setup = false;"))),
        },
        Rule::WarpWire => Fixture {
            bad: &[
                ("crates/dxwgsl/src/wgsl/setup.rs", "fn wire() { let l = line(); }\n#[cfg(test)]\nmod t { fn f() { l.z.c += b.of(zmax, s); let w = \"sx_bias(sx_wz, 1.0)\"; } }\n"),
                ("crates/kura/src/d3d11/expand.rs", "fn raster_words() { put(w, x::xd::RASTER, 0, bits); let c = crate::render::gpu::has_feature(\"depth-clip-control\"); }\n"),
                ("crates/kura/src/d3d11/mod.rs", "unsafe fn wire_edges(d: &D11) {}\nfn triangle_edges(idx: &[u32]) {}\n"),
            ],
            good: &[
                ("crates/dxwgsl/src/wgsl/setup.rs", "fn wire() { l.z.c += b.of(zmax, s); }\nconst W: &str = \"pz.z = pz.z + sx_bias(sx_wz, m);\";\n"),
                ("crates/kura/src/d3d11/expand.rs", "fn raster_words() { put(w, x::xd::BIAS, 0, at(3)); }\nfn setup_wanted() { let wired = f.is_some_and(|f| Fill::of(f) == Fill::Wireframe); }\n"),
                ("crates/kura/src/d3d11/mod.rs", "fn of() { let wired = match fill { Fill::Wireframe => topo.triangles() && !sprite && !squares, Fill::Solid => false }; }\nfn draw_with() { let q = s.is_some_and(expand::SetupStore::squares); }\n"),
            ],
            mutate: ("crates/kura/src/d3d11/expand.rs", |t| Some(t.replace("    put(w, x::xd::BIAS, 0, at(3));\n", ""))),
        },
        Rule::DllSearch => Fixture {
            bad: &[
                ("crates/sake/src/pe.rs", "pub static DLL_DIRS: Mutex<Vec<String>> = Mutex::new(Vec::new());\nfn find_dll_file(dll: &str) {\n    let mut dirs = vec![exe_dir];\n    dirs.extend(crate::dllpath::user_dirs().into_iter().rev());\n    dirs.push(crate::files::cwd().trim_end_matches('\\\\').to_string());\n    dirs.push(\"C:\\\\windows\\\\system32\".into());\n}\n// for d in search.folders() image_resolver(search.of_dependency(&win))\n"),
                ("crates/sake/src/k32.rs", "fn load_library_ex(c: &mut Ctx) -> u32 { if flags & 0x62 == 0 { return LoadLibraryW(c); } 0 }\n#[cfg(test)]\nmod t { fn f() { crate::pe::LoadSearch { module: Some(n.clone()), flags, missing: crate::pe::OnMissing::Fails }; } }\n"),
            ],
            good: &[
                ("crates/sake/src/pe.rs", "impl LoadSearch { fn folders(&self) -> Vec<String> { crate::dllpath::dll_folders(self.module.as_deref(), self.flags).unwrap_or_else(|_| Vec::new()) } }\nfn find_dll_file(dll: &str, search: &LoadSearch) { for d in search.folders().iter() {} }\nfn load_dll_file() { load(data, &win, &mut image_resolver(search.of_dependency(&win))); }\n"),
                ("crates/sake/src/k32.rs", "#[cfg(test)]\nmod early { fn f() { let s = \"}\"; } }\nfn load_library_ex(c: &mut Ctx) -> u32 { let search = crate::pe::LoadSearch { module: Some(n.clone()), flags, missing: crate::pe::OnMissing::Fails }; }\n"),
            ],
            mutate: ("crates/sake/src/pe.rs", |t| Some(t.replace("image_resolver(search.of_dependency(&win))", "image_resolver(LoadSearch::default())"))),
        },
        Rule::MissingImportFails => Fixture {
            bad: &[
                ("crates/sake/src/pe.rs", "fn resolve_import_with() { Err(e) => { eprintln!(\"sake: import from {dll}: {e}\"); return api::export(dll, &name); } }\n// note_missing(match e { note_missing(LoadFailure::NoProc) ABANDONED.locked().push(base)\n"),
                ("crates/sake/src/k32.rs", "fn load_library_named(c: &mut Ctx, n: &str) -> u32 { let r = crate::pe::load_library(c.cpu, n); if r == 0 { last_error(126); } r }\n#[cfg(test)]\nmod t { fn f() { last_error(why.error()); } }\n"),
            ],
            good: &[
                ("crates/sake/src/pe.rs", "fn resolve_import_with() { note_missing(match e { DllError::Failed(why) => why, DllError::NotThere(_) => LoadFailure::NotFound }); note_missing(LoadFailure::NoProc); }\nfn load_dll_file() { ABANDONED.locked().push(base); }\nfn load_library_searching() { match r { Err(DllError::Failed(why)) => Err(abandon(cpu, why)), } }\n"),
                ("crates/sake/src/k32.rs", "fn load_library_named(c: &mut Ctx, n: &str) -> u32 { match crate::pe::load_library_searching(c.cpu, n, &s) { Err(why) => { last_error(why.error()); 0 } } }\n"),
            ],
            mutate: ("crates/sake/src/pe.rs", |t| Some(t.replace("ABANDONED.locked().push(base)", "drop(base)"))),
        },
        Rule::DrawPlan => Fixture {
            bad: &[
                ("crates/kura/src/d3d11/mod.rs", "fn draw_with() {\n    let topo = match st.topology { 1 => 0, _ => 3 };\n    let topo = if wire.is_some() { 1 } else { topo };\n}\n// DrawPlan::of(assembled, adjacent, fill, sprite.is_some(), store.is_some(), count) logic::primitives(plan.topo, drawn, &cuts)\n"),
                ("crates/kura/src/d3d11/logic.rs", "pub fn primitives(topo: u32, n: u32, cuts: &[u32]) -> Vec<Prim> { Vec::new() }\n"),
            ],
            good: &[
                ("crates/kura/src/d3d11/mod.rs", "fn draw_with() {\n    let plan = match DrawPlan::of(assembled, adjacent, fill, sprite.is_some(), store.is_some(), count) { _ => return };\n    let prims = logic::primitives(plan.topo, drawn, &cuts);\n}\n#[cfg(test)]\nmod t { fn f() { let topo = match st.topology { _ => 3 }; } }\n"),
                ("crates/kura/src/d3d11/logic.rs", "pub fn primitives(topo: super::PageTopo, n: u32, cuts: &[u32]) -> Vec<Prim> { match topo { PageTopo::Points => Vec::new() } }\n"),
            ],
            mutate: ("crates/kura/src/d3d11/mod.rs", |t| Some(t.replace("logic::primitives(plan.topo, drawn, &cuts)", "logic::primitives(PageTopo::Lines, count, &cuts)"))),
        },
        Rule::FullPathOneRule => Fixture {
            bad: &[("crates/sake/src/files.rs", "pub fn full_path(p: &str) -> String {\n    // crate::fullpath::process_env_full(&w, 0xfffe)\n    full_path_trim(p, true)\n}\n")],
            good: &[("crates/sake/src/files.rs", "pub fn full_path(p: &str) -> String { match FullMode::of(p) { FullMode::Win32 => win32_full(p), FullMode::Literal => full_path_trim(p, true) } }\nfn win32_full(p: &str) -> String { let t = crate::fullpath::process_env_full(&w, 0xfffe).text(); }\n")],
            mutate: ("crates/sake/src/files.rs", |t| Some(t.replace("FullMode::Win32 => win32_full(p)", "FullMode::Win32 => full_path_trim(p, true)"))),
        },
        Rule::ShortNameTable => Fixture {
            bad: &[
                ("crates/kura/src/fs.rs", "pub fn remove_file(p: &Path) -> io::Result<()> { vfs::remove_at(&at, false) }\n"),
                ("crates/sake/src/shortname.rs", "pub fn of_dir(host_dir: &str) { let names = kura::fs::read_dir(host_dir); assign(&names) }\n"),
                ("crates/sake/src/k32.rs", "fn RemoveDirectoryW(c: &mut Ctx) -> u32 { if kura::fs::read_dir(&h).is_ok_and(|mut d| d.next().is_some()) { return 0; } ok(kura::fs::remove_dir(&h).is_ok()) }\n"),
            ],
            good: &[
                ("crates/kura/src/fs.rs", "fn create_dir() { named(Named::Made(&at)); }\nfn remove_file() { named(Named::Gone(&at)); }\nfn rename() { named(Named::Gone(&a)); named(Named::Made(&b)); }\n"),
                ("crates/sake/src/shortname.rs", "pub fn watch() { kura::fs::watch_names(on_named); }\npub fn of_dir(host_dir: &str) { let mut t = read_table(host_dir); for e in kura::fs::read_dir(host_dir) {} }\n"),
                ("crates/sake/src/files.rs", "fn c_root() -> String { crate::shortname::watch(); c }\n"),
                ("crates/sake/src/k32.rs", "fn RemoveDirectoryW(c: &mut Ctx) -> u32 { ok(crate::shortname::remove_dir(&h).is_ok()) }\n#[cfg(test)]\nmod tests { fn t() { kura::fs::read_dir(\"/x\"); } }\n"),
            ],
            mutate: ("crates/sake/src/misc.rs", |t| Some(t.replacen("crate::shortname::read_dir(", "kura::fs::read_dir(", 1))),
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

/// the repository's tracked files (a listing that fails fails the test: it is not an empty tree)
fn real_tree() -> Tree {
    match Tree::repo(&repo()) {
        Ok(t) => t,
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn a_listing_that_fails_is_an_error_not_an_empty_tree() {
    // (cargo-mutants' copy of the tree has no .git: `git ls-files` fails there, and every rule
    // read no files, so a regression put back was not found; review e18bf44)
    let dir = std::env::temp_dir().join(format!("masu-lint-no-git-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let r = Tree::repo(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(r.is_err(), "a directory outside any repository listed as a tree");
    assert!(!real_tree().paths().is_empty());
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
    let real = real_tree();
    for r in Rule::ALL {
        let (file, mutate) = fixture(r).mutate;
        let f = Rel::new(file);
        let before = fresh(&check(&real, &[r]), &known).len();
        let tree = Tree::with(&real, &f, mutate(real.text(&f)));
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
    for r in [&BUILD_EXT, &BUILD_OK, &TARGET_DIR, &GOLDEN_ANY, &GOLDEN_READ, &FNV_PRIME, &GOLDEN_FILES, &JOB, &CHECKS, &DISALLOWED, &REG_BY_NUMBER, &REG_UNTYPED] {
        assert!(r.is_some());
    }
    for r in regress::regexes() {
        assert!(r.is_some());
    }
    for r in modern::regexes() {
        assert!(r.is_some());
    }
    for r in modern::std_regexes() {
        assert!(r.is_some());
    }
}

#[test]
fn the_new_rules_find_each_spelling_alone() {
    // (each bad file on its own: one spelling found does not hide another that is missed)
    for r in [Rule::HandStd, Rule::GuestScratch, Rule::RustLints, Rule::BinDeadPub, Rule::HandBitWidth, Rule::HandUtf16, Rule::NoReturnHandler, Rule::WildcardUnreachable, Rule::SafeRawPointerAbi, Rule::UncheckedJitMode, Rule::OutEscape, Rule::EscapeComment, Rule::ArcTryUnwrap, Rule::ArcCount, Rule::ShellSpecialEnv, Rule::DepthGuard, Rule::TypedRegisters] {
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
    // (a copy of a number conversion, a lock trait, a GUID builder, a guest GUID reader, a
    // big-endian byte reader and an FNV-1a loop)
    assert_eq!(Rule::KomeCopy.check(&Tree::made_up(fixture(Rule::KomeCopy).bad)).len(), 6);
    assert_eq!(Rule::SafeRawPointerAbi.check(&Tree::made_up(fixture(Rule::SafeRawPointerAbi).bad)).len(), 3);
    assert_eq!(Rule::OutEscape.check(&Tree::made_up(fixture(Rule::OutEscape).bad)).len(), 6);
    // (the ExitProcess that returns u32 and crt_io's missing _amsg_exit; the three wildcard arms)
    assert_eq!(Rule::NoReturnHandler.check(&Tree::made_up(fixture(Rule::NoReturnHandler).bad)).len(), 2);
    assert_eq!(Rule::WildcardUnreachable.check(&Tree::made_up(fixture(Rule::WildcardUnreachable).bad)).len(), 3);
    assert_eq!(Rule::RustLints.check(&Tree::made_up(fixture(Rule::RustLints).bad)).len(), 2);
}

#[test]
fn the_abort_build_is_sake_and_what_it_links() {
    // (the real tree: the crates whose catch_unwind never catches; a dev- or build-dependency is
    // not linked into sake.wasm)
    let c = regress::abort_crates(&real_tree());
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
fn each_fix_of_the_review_of_6852be1_is_found_taken_back() {
    let real = real_tree();
    assert!(regress::review_6852be1(&real).is_empty(), "{:?}", regress::review_6852be1(&real));
    for (file, needle, _) in regress::REVIEW_6852BE1_HAS {
        let f = Rel::new(file);
        let t = Tree::with(&real, &f, Some(real.text(&f).replace(needle, "")));
        assert!(!regress::review_6852be1(&t).is_empty(), "{file}: {needle:?} taken away is not found");
    }
    for (file, needle, _) in regress::REVIEW_6852BE1_NOT {
        let f = Rel::new(file);
        let t = Tree::with(&real, &f, Some(format!("{}\nfn back() {{ {needle} }}\n", real.text(&f))));
        assert!(!regress::review_6852be1(&t).is_empty(), "{file}: {needle:?} put back is not found");
    }
}

#[test]
fn each_depth_guard_is_in_the_real_tree() {
    let real = real_tree();
    assert!(regress::depth_guard(&real).is_empty());
    // (taking any one guard's comparison away is found)
    for (file, name) in regress::DEPTH_GUARDS {
        let f = Rel::new(file);
        let text = real.text(&f);
        let decl = text.lines().find(|l| l.contains(&format!("const {name}:"))).unwrap_or_default().to_owned();
        let only = text.replace(name, "OTHER").replace(&decl.replace(name, "OTHER"), &decl);
        let t = Tree::with(&real, &f, Some(only));
        assert!(!regress::depth_guard(&t).is_empty(), "{file} {name}: its uses taken away is not found");
    }
}


/// rebuild's rules read a Rust file without each of its inline test modules, wherever they are
/// (k32.rs has one near its top: cutting at the first lost the rest of the file)
#[test]
fn rebuild_reads_past_each_test_module() {
    let code = "fn a() {}\n#[cfg(test)]\nmod t {\n    fn x() { if y { z } }\n}\nfn b() {}\n#[cfg(test)]\nmod outside;\n#[cfg(test)]\npub(crate) mod u { fn w() {} }\nfn c() {}\n";
    let got = super::rebuild::without_tests(code);
    assert!(got.contains("fn a()") && got.contains("fn b()") && got.contains("fn c()") && got.contains("mod outside;"), "{got}");
    assert!(!got.contains("fn x()") && !got.contains("fn w()"), "{got}");
}
