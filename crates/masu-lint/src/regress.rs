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

// --- clear-rounding --------------------------------------------------------------------------

/// the clear texels' file: Windows rounds a clear's small floats toward zero there
const CLEAR_RS: &str = "crates/kura/src/d3d11/clear.rs";

/// kura's clear.rs (outside its tests) calling format::f16_bits, the nearest-rounding one
pub(super) fn clear_rounding(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let f = Rel::new(CLEAR_RS);
    for (n, l) in tree.text(&f).lines().enumerate() {
        if l.trim_start().starts_with("mod tests") || l.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        let code = l.split("//").next().unwrap_or_default();
        if code.contains("f16_bits") {
            out.push((f.clone(), n.saturating_add(1), "f16_bits rounds to the nearest: a clear rounds toward zero (f16_rtz)".to_owned()));
        }
    }
    out
}

// --- ps-quads --------------------------------------------------------------------------------

/// the clip pass, the query that reads its counters, a line's count by the region WARP's line
/// rasterizer visits and a triangle's by its edge table's spans (2026-11-21: lines were counted
/// by the quads they light, winref d3d11gs 128 for Windows' 228; triangles by the quads with a
/// covered centre, 3764 for 3772; the WGSL's quad rows started from (0x7fff, 0) where the edge
/// table has none, which only shaderconf's ps_cases, the WGSL run against the Rust rule, showed)
const PS_SITES: [(&str, &str, &str); 10] = [
    ("crates/dxwgsl/src/wgsl/expand.rs", "ps_pairs(p,", "the clip pass does not add ps_pairs(p, ..) to the counters"),
    ("crates/kura/src/d3d11/query.rs", "c::PS_PAIRS", "PIPELINE_STATISTICS does not read c::PS_PAIRS"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "line_visits(snap(", "a line's invocations are not line_visits (the quads WARP visits)"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "return ps_line_visits(ps_snap(", "the clip pass does not count a line by ps_line_visits"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "spans(&v, r)", "a triangle's invocations are not its edge table's spans (spans)"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "lx = ps_edge_x(", "the clip pass does not count a triangle by WARP's edge DDA (ps_edge_x)"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "if wide { big_edge_x }", "a far triangle is not counted by BigEdgeIterator's exact x (big_edge_x)"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "lx = ps_big_edge_x(", "the clip pass does not count a far triangle by BigEdgeIterator (ps_big_edge_x)"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "#[path = \"ps_tests.rs\"]", "the spans are no longer checked against the geometric reference (ps_tests.rs)"),
    ("crates/shaderconf/src/compute.rs", "out.extend(ps_cases());", "the clip pass's WGSL is no longer run against the Rust rule on WebGPU (shaderconf ps_cases)"),
];

/// PSInvocations not taken from the clip pass's pairs, or a line's not from its visited quads
pub(super) fn ps_quads(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (path, needle, why) in PS_SITES {
        let f = Rel::new(path);
        if tree.is_made_up() && !tree.has(&f) {
            continue;
        }
        // (strings kept: the clip pass writes ps_quads in its WGSL text)
        let code = tree.code(&f, true);
        if !code.contains(needle) {
            out.push((f, 1, why.to_owned()));
        }
    }
    out
}

// --- logic-ops -------------------------------------------------------------------------------

/// Direct3D 11.1's logic ops (2026-11-22: kura answered OutputMergerLogicOp 0 and drew a
/// logic-op blend state as a plain write, known.txt holding WARP's 1 as a difference)
const LOGIC_SITES: [(&str, &str, &str); 19] = [
    ("crates/kura/src/d3d11/mod.rs", "for k in 0..10 {", "D3D11_OPTIONS does not answer every BOOL from OutputMergerLogicOp (0) on"),
    ("crates/kura/src/d3d11/mod.rs", "dxwgsl::translate_logic(", "the pixel shader of a draw with logic ops is not dxwgsl::translate_logic"),
    ("crates/kura/src/d3d11/mod.rs", "logic::targets(", "a draw's logic ops are not taken from its blend state (logic::targets)"),
    ("crates/kura/src/d3d11/mod.rs", "logic::split(&w, &copies, stash.as_ref(),", "a draw reading its target is not split by primitive after the target's copy (logic::split), with its stash passes"),
    ("crates/kura/src/d3d11/mod.rs", "dxwgsl::sample_rate(", "a multisampled target's op is not chosen by the pixel shader's rate (WARP: a pixel-rate shader stays one invocation a pixel under a logic op)"),
    ("crates/kura/src/d3d11/mod.rs", "page_multisamples(", "a target the page does not multisample is taken as multisampled for its logic op"),
    ("crates/kura/src/d3d11/logic.rs", "Some(op) if ms && !sample_rate && op.reads_dst() => Store::Stash { op, mark },", "a multisampled target's op under a pixel-rate shader is not a stash resolved per sample (WARP: per sample, the shader per pixel)"),
    ("crates/kura/src/d3d11/logic.rs", "scratch(d, (COPY_OF | l.target,", "a target's copy is not kept by its slot (two targets of one size and format shared one copy)"),
    ("crates/kura/src/d3d11/logic.rs", "f::P_RGBA8SI => ([8, 8, 8, 8], s),", "SINT targets get no logic ops (WARP applies them, though FORMAT_SUPPORT2 lists UINT formats only)"),
    ("crates/dxwgsl/src/logic.rs", "None => saturate(bits, l.sign, sv),", "an integer target's plain write is not saturated (WARP: min of the value and the channel's largest)"),
    ("crates/kura/src/d3d11/logic.rs", "dxwgsl::logic::carry(l.bits)", "eight multisampled targets have no location for a stash's mark and it is not carried by a stash (dxwgsl::logic::carry)"),
    ("crates/kura/src/d3d11/logic.rs", "stashes().next().map(|l| Mark::Slot(l.target))", "eight targets whose stashes cannot carry the mark get no mark (the shader would have to run per sample, Windows runs it per pixel): the first stash's location is the mark's, its source in LOGIC_SRC"),
    ("crates/dxwgsl/src/wgsl.rs", "var<storage, read_write> lg_src: array<u32>", "a stash at the mark's location does not write its source to LOGIC_SRC"),
    ("host/vngpu.js", "maxColorAttachmentBytesPerSample", "the page's device keeps WebGPU's 32 color bytes a sample (Direct3D 11 draws to 8 targets of 16 bytes, a stash's mark adds to them)"),
    ("crates/kura/src/render/d3d.rs", "LOGIC_RESOLVE => {", "the page does not do LOGIC_RESOLVE (a multisampled target's op per sample)"),
    ("crates/dxwgsl/src/wgsl.rs", "logic_output(l, r, k,", "a logic target's output does not go through logic_output"),
    ("crates/dxwgsl/src/wgsl.rs", "crate::logic::LOGIC_DST", "logic_output does not read the target's copy at LOGIC_DST"),
    ("crates/dxwgsl/src/logic.rs", "#[path = \"logic_tests.rs\"]", "the ops are no longer checked against D3D11_LOGIC_OP's table (logic_tests.rs)"),
    ("crates/kura/src/d3d11/logic.rs", "#[path = \"logic_tests.rs\"]", "the targets and primitives of a logic draw are no longer tested (logic_tests.rs)"),
];

/// logic ops refused or drawn as plain writes
pub(super) fn logic_ops(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (path, needle, why) in LOGIC_SITES {
        let f = Rel::new(path);
        if tree.is_made_up() && !tree.has(&f) {
            continue;
        }
        let code = tree.code(&f, true);
        if !code.contains(needle) {
            out.push((f, 1, why.to_owned()));
        }
    }
    out
}

// --- d3d9-status-codes ------------------------------------------------------------------------

/// d3d9.h's codes by name: MAKE_D3DHRESULT(code) = 0x8876_0000 | code (D3DERR_), MAKE_D3DSTATUS
/// (code) = 0x0876_0000 | code (D3DOK_, S_)
pub const D3D9_CODES: [(&str, u32); 31] = [
    ("D3DERR_WRONGTEXTUREFORMAT", 0x8876_0000 | 2072),
    ("D3DERR_UNSUPPORTEDCOLOROPERATION", 0x8876_0000 | 2073),
    ("D3DERR_UNSUPPORTEDCOLORARG", 0x8876_0000 | 2074),
    ("D3DERR_UNSUPPORTEDALPHAOPERATION", 0x8876_0000 | 2075),
    ("D3DERR_UNSUPPORTEDALPHAARG", 0x8876_0000 | 2076),
    ("D3DERR_TOOMANYOPERATIONS", 0x8876_0000 | 2077),
    ("D3DERR_CONFLICTINGTEXTUREFILTER", 0x8876_0000 | 2078),
    ("D3DERR_UNSUPPORTEDFACTORVALUE", 0x8876_0000 | 2079),
    ("D3DERR_CONFLICTINGRENDERSTATE", 0x8876_0000 | 2081),
    ("D3DERR_UNSUPPORTEDTEXTUREFILTER", 0x8876_0000 | 2082),
    ("D3DERR_CONFLICTINGTEXTUREPALETTE", 0x8876_0000 | 2086),
    ("D3DERR_DRIVERINTERNALERROR", 0x8876_0000 | 2087),
    ("D3DERR_NOTFOUND", 0x8876_0000 | 2150),
    ("D3DERR_MOREDATA", 0x8876_0000 | 2151),
    ("D3DERR_DEVICELOST", 0x8876_0000 | 2152),
    ("D3DERR_DEVICENOTRESET", 0x8876_0000 | 2153),
    ("D3DERR_NOTAVAILABLE", 0x8876_0000 | 2154),
    ("D3DERR_OUTOFVIDEOMEMORY", 0x8876_0000 | 380),
    ("D3DERR_INVALIDDEVICE", 0x8876_0000 | 2155),
    ("D3DERR_INVALIDCALL", 0x8876_0000 | 2156),
    ("D3DERR_DRIVERINVALIDCALL", 0x8876_0000 | 2157),
    ("D3DERR_WASSTILLDRAWING", 0x8876_0000 | 540),
    ("D3DOK_NOAUTOGEN", 0x0876_0000 | 2159),
    ("D3DERR_DEVICEREMOVED", 0x8876_0000 | 2160),
    ("S_NOT_RESIDENT", 0x0876_0000 | 2165),
    ("S_RESIDENT_IN_SHARED_MEMORY", 0x0876_0000 | 2166),
    ("S_PRESENT_MODE_CHANGED", 0x0876_0000 | 2167),
    ("S_PRESENT_OCCLUDED", 0x0876_0000 | 2168),
    ("D3DERR_DEVICEHUNG", 0x8876_0000 | 2164),
    ("D3DERR_UNSUPPORTEDOVERLAY", 0x8876_0000 | 2171),
    ("D3DERR_UNSUPPORTEDCRYPTO", 0x8876_0000 | 2173),
];

/// where Direct3D 9's codes are spelled out
const D3D9_HOMES: [&str; 2] = ["crates/kura/src/d3d9/", "crates/sake/src/d3dx9"];

static D3D9_CONST: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"\bconst\s+((?:D3DERR|D3DOK|S)_[A-Z0-9_]+)\s*:\s*u32\s*=\s*(0x[0-9a-fA-F_]+)\s*;"));

/// a Direct3D 9 status const whose value is not d3d9.h's (or whose name the table lacks)
pub(super) fn d3d9_status(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let Some(re) = D3D9_CONST.as_ref() else { return out };
    for f in tree.paths().iter().filter(|p| p.is_rust() && D3D9_HOMES.iter().any(|h| p.as_str().starts_with(h))) {
        let code = tree.code(f, true);
        for c in re.captures_iter(&code) {
            let (Some(name), Some(v)) = (c.get(1), c.get(2)) else { continue };
            let got = v.as_str().strip_prefix("0x").and_then(|h| u32::from_str_radix(&h.replace('_', ""), 16).ok());
            let line = crate::code::line_of(&code, name.start());
            match D3D9_CODES.iter().find(|(n, _)| *n == name.as_str()) {
                Some(&(_, want)) if got != Some(want) => out.push((f.clone(), line, format!("{} is {:#010x} in d3d9.h, not {}", name.as_str(), want, v.as_str()))),
                Some(_) => {}
                None if !name.as_str().starts_with("S_") => out.push((f.clone(), line, format!("{} is not in regress::D3D9_CODES: add it from d3d9.h", name.as_str()))),
                None => {}
            }
        }
    }
    out
}

// --- console-lines ---------------------------------------------------------------------------

/// The program's console output written straight to stderr (`eprint!`) instead of through
/// k32::console_write, which keeps it a line at a time (2026-11-21: crt's printf went out with
/// eprint!, and a line split in two where the program waited for the GPU, winref d3d9tex's
/// "after a lock of the top left"). A line of product code under crates/sake/src with `eprint!(`.
pub(super) fn console_lines(tree: &Tree) -> Hits {
    let files = tree.rs(&["crates/sake/src/"]);
    let tests = test_mod_files(tree, &files);
    let mut out = Hits::new();
    for f in files.into_iter().filter(|f| !tests.contains(f)) {
        for (n, l) in tree.text(&f).lines().enumerate() {
            let code = l.split("//").next().unwrap_or_default();
            if code.contains("eprint!(") {
                out.push((f.clone(), n.saturating_add(1), "eprint!: the program's console goes through k32::console_write".to_owned()));
            }
        }
    }
    out
}

// --- known-diffs -----------------------------------------------------------------------------

/// winref's list of the differences from Windows compare.py lets pass
const KNOWN_TXT: &str = "tests/winref/known.txt";

/// Every pattern line of tests/winref/known.txt (not its comments): each one a difference from
/// Windows kept, so each one a site with its why in arch-lint-sites.tsv; a line added there
/// hides a new difference (2026-11-21: LogicOp).
pub(super) fn known_diffs(tree: &Tree) -> Hits {
    let f = Rel::new(KNOWN_TXT);
    let mut out = Hits::new();
    for (n, l) in tree.text(&f).lines().enumerate() {
        let t = l.trim();
        if !t.is_empty() && !t.starts_with('#') {
            out.push((f.clone(), n.saturating_add(1), "a difference from Windows let pass: make the behavior Windows', then drop the line".to_owned()));
        }
    }
    out
}

// --- load-every-level -------------------------------------------------------------------------

/// where `ld` by level reads every level's view: (file, what it must hold, why)
const LOAD_SITES: [(&str, &str, &str); 4] = [
    ("crates/dxwgsl/src/wgsl.rs", "format!(\"tl{t}\")", "ld does not read tl<t>, the view of every level"),
    ("crates/dxwgsl/src/wgsl.rs", "+ bv.l{t}.x", "ld does not add the SRV's first level (bv.l<t>.x)"),
    ("crates/kura/src/d3d11/mod.rs", "load_texture_entry(d, sv,", "kura does not bind LoadTexture with load_texture_entry"),
    ("crates/kura/src/d3d11/mod.rs", "load_texture_levels(d, sv)", "kura does not give LoadTexture's levels (load_texture_levels)"),
];

/// ld by level reading the SRV's own view
pub(super) fn load_every_level(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (path, needle, why) in LOAD_SITES {
        let f = Rel::new(path);
        if tree.is_made_up() && !tree.has(&f) {
            continue;
        }
        // (strings kept: the WGSL is written in format strings)
        if !tree.code(&f, true).contains(needle) {
            out.push((f, 1, why.to_owned()));
        }
    }
    out
}

/// (for the tests: every regex here compiles)
#[cfg(test)]
pub(super) fn regexes() -> [&'static LazyLock<Option<Regex>>; 7] {
    [&SECTION, &PATH_DEP, &SH_SET, &YAML_KEY, &KANI_FOR, &KANI_GATE, &D3D9_CONST]
}


// --- kani-features ----------------------------------------------------------------------------

/// the proofs' script: its `for c in ...; do` names the crates Kani compiles
pub(super) const KANI_SCRIPT: &str = "formal/kani.sh";

/// Rust 1.100 library features Kani 0.68's compiler (nightly 2026-08-20) has still unstable: a
/// crate Kani compiles (or links) that uses one names it in `#![cfg_attr(kani, feature(..))]`
/// in its lib.rs (the Kani job stopped at Vec::from_fn in kura and proved nothing, 2026-11-21;
/// review e18bf44). (feature, what uses it)
pub const KANI_UNSTABLE: [(&str, &str); 3] = [
    ("vec_from_fn", r"\bVec\s*(?:::\s*<[^>]*>\s*)?::\s*from_fn\b"),
    ("bool_toggle", r"\.\s*toggle\s*\(\s*\)"),
    ("trim_prefix_suffix", r"\.\s*trim_(?:prefix|suffix)\s*\("),
];

/// the identifier each feature's use names (the line it is on is then read with its pattern)
const KANI_NAMES: [(&str, &str); 4] = [("vec_from_fn", "from_fn"), ("bool_toggle", "toggle"), ("trim_prefix_suffix", "trim_prefix"), ("trim_prefix_suffix", "trim_suffix")];

static KANI_FOR: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(?m)^\s*for\s+c\s+in\s+([A-Za-z0-9_ -]+);\s*do"));
static KANI_GATE: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"#!\[\s*cfg_attr\s*\(\s*kani\s*,\s*feature\s*\(([^)]*)\)\s*\)\s*\]"));
static KANI_USES: LazyLock<Vec<(&'static str, Option<Regex>)>> = LazyLock::new(|| KANI_UNSTABLE.iter().map(|(f, r)| (*f, rx(r))).collect());

/// the crates `cargo kani` compiles: the script's and every crate they link by path
pub(super) fn kani_crates(tree: &Tree) -> BTreeSet<String> {
    let script = tree.text(&Rel::new(KANI_SCRIPT));
    let roots: Vec<String> = KANI_FOR.as_ref().and_then(|r| r.captures(&script)).and_then(|c| c.get(1)).map_or_default(|m| m.as_str().split_whitespace().map(|c| format!("crates/{c}")).collect());
    let mut seen = BTreeSet::new();
    let mut todo = roots;
    while let Some(c) = todo.pop() {
        if seen.insert(c.clone()) {
            todo.extend(path_deps(&tree.text(&Rel(format!("{c}/Cargo.toml")))));
        }
    }
    seen
}

/// the features `lib` turns on for Kani
pub fn kani_gated(lib: &str) -> BTreeSet<String> {
    let Some(r) = KANI_GATE.as_ref() else { return BTreeSet::new() };
    r.captures_iter(lib).filter_map(|c| c.get(1)).flat_map(|m| m.as_str().split(',').map(|f| f.trim().to_owned()).filter(|f| !f.is_empty()).collect::<Vec<_>>()).collect()
}

pub(super) fn kani_features(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    if tree.is_made_up() && !tree.has(&Rel::new(KANI_SCRIPT)) {
        return out;
    }
    let crates = kani_crates(tree);
    if crates.is_empty() {
        out.push((Rel::new(KANI_SCRIPT), 1, "names no crates (`for c in ...; do`): the rule reads nothing".into()));
    }
    for c in crates {
        let lib = Rel(format!("{c}/src/lib.rs"));
        let gated = kani_gated(&tree.text(&lib));
        let prefix = format!("{c}/src/");
        let files = tree.rs(&[prefix.as_str()]);
        let tests = test_mod_files(tree, &files);
        let files: Vec<Rel> = files.into_iter().filter(|f| !tests.contains(f)).collect();
        // (the names outside #[cfg(test)] modules: Kani builds the library alone)
        for (f, it) in items(tree, &files) {
            let code = tree.code(&f, false);
            for (feature, r) in KANI_USES.iter() {
                let Some(r) = r else { continue };
                if gated.contains(*feature) {
                    continue;
                }
                let lines: BTreeSet<usize> = it.idents.iter().filter(|(i, _)| KANI_NAMES.iter().any(|(ff, n)| ff == feature && n == i)).map(|(_, l)| *l).collect();
                if let Some(l) = lines.into_iter().find(|l| r.is_match(&crate::code::line_text(&code, *l))) {
                    out.push((f.clone(), l, format!("uses {feature}, unstable in Kani's compiler: {lib} needs `#![cfg_attr(kani, feature({feature}))]`")));
                }
            }
        }
    }
    out
}

// --- unchecked-output -------------------------------------------------------------------------

/// a fn whose body reads a Command's `.output()` stdout and never its status
struct OutputUse {
    hits: Vec<(String, usize)>,
}

impl OutputUse {
    fn body(&mut self, name: &syn::Ident, block: &syn::Block) {
        let t = quote::ToTokens::to_token_stream(block).to_string().replace(' ', "");
        if t.contains(".output()") && t.contains(".stdout") && !t.contains("status") {
            self.hits.push((name.to_string(), crate::syntax::line(name.span())));
        }
    }
}

impl<'a> syn::visit::Visit<'a> for OutputUse {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !f.attrs.iter().any(|a| a.path().is_ident("test")) {
            self.body(&f.sig.ident, &f.block);
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        self.body(&f.sig.ident, &f.block);
        syn::visit::visit_impl_item_fn(self, f);
    }
}

/// what a fn reading a child's output takes for "it worked": its stdout alone (an empty listing
/// from a git that failed was "no files", review e18bf44)
pub(super) fn unchecked_output(tree: &Tree) -> Hits {
    let files: Vec<Rel> = tree.rs(&["crates/", "tools/"]).into_iter().filter(|f| !f.as_str().starts_with("tools/lint-fixtures/")).collect();
    let tests = test_mod_files(tree, &files);
    let mut out = Hits::new();
    for f in files.iter().filter(|f| !tests.contains(*f)) {
        let Some(file) = tree.syntax(f) else { continue };
        let mut v = OutputUse { hits: Vec::new() };
        syn::visit::Visit::visit_file(&mut v, &file);
        out.extend(v.hits.into_iter().map(|(n, l)| (f.clone(), l, format!("{n} reads a command's stdout and never its exit status: a failed command is not an empty answer"))));
    }
    out
}

/// a lock taken on a file opened by hand: `let f = File::open(..)`, `match File::open(..) { Ok(f) }`
/// or the open in the receiver itself, then `.lock()` and the like on it
struct HandLock {
    opened: BTreeSet<String>,
    hits: Vec<(String, usize)>,
}

const LOCK_METHODS: [&str; 4] = ["lock", "try_lock", "lock_shared", "try_lock_shared"];

fn opens_a_file(e: &impl quote::ToTokens) -> bool {
    let t = quote::ToTokens::to_token_stream(e).to_string().replace(' ', "");
    // (a File's own open, not LockFile::open)
    static OPEN: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r"(^|[^A-Za-z0-9_])(File::(open|create)\(|OpenOptions::new\(\))"));
    OPEN.as_ref().is_some_and(|r| r.is_match(&t))
}

fn pat_idents(p: &syn::Pat, out: &mut BTreeSet<String>) {
    struct V<'o>(&'o mut BTreeSet<String>);
    impl<'a> syn::visit::Visit<'a> for V<'_> {
        fn visit_pat_ident(&mut self, i: &'a syn::PatIdent) {
            self.0.insert(i.ident.to_string());
            syn::visit::visit_pat_ident(self, i);
        }
    }
    syn::visit::Visit::visit_pat(&mut V(out), p);
}

impl<'a> syn::visit::Visit<'a> for HandLock {
    fn visit_local(&mut self, l: &'a syn::Local) {
        if l.init.as_ref().is_some_and(|i| opens_a_file(&i.expr)) {
            pat_idents(&l.pat, &mut self.opened);
        }
        syn::visit::visit_local(self, l);
    }
    fn visit_expr_match(&mut self, m: &'a syn::ExprMatch) {
        if opens_a_file(&m.expr) {
            for a in &m.arms {
                pat_idents(&a.pat, &mut self.opened);
            }
        }
        syn::visit::visit_expr_match(self, m);
    }
    fn visit_expr_let(&mut self, l: &'a syn::ExprLet) {
        if opens_a_file(&l.expr) {
            pat_idents(&l.pat, &mut self.opened);
        }
        syn::visit::visit_expr_let(self, l);
    }
    fn visit_expr_method_call(&mut self, c: &'a syn::ExprMethodCall) {
        let m = c.method.to_string();
        if c.args.is_empty() && LOCK_METHODS.contains(&m.as_str()) {
            let named = if let syn::Expr::Path(p) = &*c.receiver { p.path.get_ident().is_some_and(|i| self.opened.contains(&i.to_string())) } else { false };
            if named || opens_a_file(&c.receiver) {
                self.hits.push((m, crate::syntax::line(c.method.span())));
            }
        }
        syn::visit::visit_expr_method_call(self, c);
    }
}

/// locks between processes taken on a file (or directory) opened by hand, not through
/// crates/masu/src/lockfile.rs: Windows opens no directory with File::open, and the store's
/// lock did so (`masu add` failed there; review e18bf44)
pub(super) fn hand_lock(tree: &Tree) -> Hits {
    let files: Vec<Rel> = tree.rs(&["crates/", "tools/"]).into_iter()
        .filter(|f| !f.as_str().starts_with("tools/lint-fixtures/") && f.as_str() != "crates/masu/src/lockfile.rs")
        .collect();
    let mut out = Hits::new();
    for f in &files {
        let Some(file) = tree.syntax(f) else { continue };
        let mut v = HandLock { opened: BTreeSet::new(), hits: Vec::new() };
        syn::visit::Visit::visit_file(&mut v, &file);
        out.extend(v.hits.into_iter().map(|(m, l)| (f.clone(), l, format!("`.{m}()` on a file opened by hand: take it through crate::lockfile::LockFile (a lock file, never a directory)"))));
    }
    out
}

// --- stage-run -------------------------------------------------------------------------------

/// Where kura's Direct3D 11 draws and dispatches read each bound stage's shader (State::shaders:
/// GS, HS, DS, CS): (file, what its code outside the tests says)
pub const STAGE_READS: [(&str, &str); 5] = [
    ("crates/kura/src/d3d11/expand.rs", "st.shaders[0]"),
    ("crates/kura/src/d3d11/expand_tess.rs", "st.shaders[1]"),
    ("crates/kura/src/d3d11/expand_tess.rs", "st.shaders[2]"),
    ("crates/kura/src/d3d11/expand_tess.rs", "patch_points(st.topology)"),
    ("crates/kura/src/d3d11/mod.rs", "st.shaders[3]"),
];

/// Each of STAGE_READS missing from its file's code (comments left out, and an inline
/// #[cfg(test)] module and what follows it): a stage bound and never run.
pub(super) fn stage_run(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (file, want) in STAGE_READS {
        let f = Rel::new(file);
        if !product_code(tree, &f).contains(want) {
            out.push((f, 1, format!("no `{want}` in the code: the stage it binds is never run")));
        }
    }
    out
}

/// A file's code without its comments and without its inline #[cfg(test)] modules (each
/// blanked from its attribute to its closing brace, wherever it stands in the file).
pub(super) fn product_code(tree: &Tree, f: &Rel) -> String {
    let kept: Vec<char> = tree.code(f, true).chars().collect();
    let bare: Vec<char> = tree.code(f, false).chars().collect();
    let text: String = bare.iter().collect();
    let mut out = kept.clone();
    let mut from = 0usize;
    while let Some(rel) = text.get(from..).and_then(|t| t.find("#[cfg(test)]")) {
        let at = from.saturating_add(rel);
        let after = at.saturating_add(12);
        let rest = text.get(after..).unwrap_or_default();
        let lead = rest.len() - rest.trim_start().len();
        let rest = rest.trim_start();
        let open = rest.find('{');
        let inline = rest.starts_with("mod ") && open.is_some_and(|b| rest.find(';').is_none_or(|s| b < s));
        from = after;
        let Some(open) = open.filter(|_| inline) else { continue };
        // (the module's braces, in the code with its strings blanked)
        let start = text.get(..after.saturating_add(lead).saturating_add(open)).map_or(0, |t| t.chars().count());
        let mut depth = 0i64;
        let mut end = bare.len();
        for (k, c) in bare.iter().enumerate().skip(start) {
            match c {
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = k;
                        break;
                    }
                }
                _ => {}
            }
        }
        let first = text.get(..at).map_or(0, |t| t.chars().count());
        for c in out.iter_mut().take(end.saturating_add(1)).skip(first) {
            if *c != '\n' {
                *c = ' ';
            }
        }
        let byte_end = bare.iter().take(end.saturating_add(1)).map(|c| c.len_utf8()).sum::<usize>();
        from = from.max(byte_end);
    }
    out.into_iter().collect()
}

// --- nan-min-max -----------------------------------------------------------------------------

/// Direct3D's min/max of a NaN (the other operand) where masu makes or runs them: (file, what
/// its code outside the tests says). dxwgsl's float min/max through its helpers (WGSL leaves a
/// NaN to the implementation), hlslc's [maxtessfactor] clamp of the tess factors (a min, as
/// d3dcompiler emits it), kura's clamp giving the maximum for a NaN (winref d3d11tess).
pub const NAN_MIN_MAX: [(&str, &str); 4] = [
    ("crates/dxwgsl/src/wgsl.rs", "d3d_min({}, {})"),
    ("crates/dxwgsl/src/wgsl.rs", "d3d_max({}, {})"),
    ("crates/hlslc/src/lower.rs", "h.max_tess"),
    ("crates/kura/src/d3d11/expand_tess.rs", "v.is_nan() => m"),
];

/// What NAN_MIN_MAX's files must not say: a DXBC float min/max as WGSL's own.
pub const NAN_MIN_MAX_NOT: [(&str, &str); 2] = [
    ("crates/dxwgsl/src/wgsl.rs", "Opcode::Min => self.cw("),
    ("crates/dxwgsl/src/wgsl.rs", "Opcode::Max => self.cw("),
];

/// Each of NAN_MIN_MAX missing from its file's code, each of NAN_MIN_MAX_NOT in it.
pub(super) fn nan_min_max(tree: &Tree) -> Hits {
    says(tree, &NAN_MIN_MAX, &NAN_MIN_MAX_NOT, "a NaN through min/max not as Direct3D gives it (the other operand)", "WGSL's min/max leave a NaN to the implementation; Direct3D gives the other operand (d3d_min/d3d_max)")
}

// --- pcsg-pack -------------------------------------------------------------------------------

/// hlslc packs a patch constant that is not a tess factor into the free components of a
/// register already taken (d3dcompiler's PCSG, winref d3d11tess: PID at r0.y): (file, what its
/// code outside the tests says).
pub const PCSG_PACK: [(&str, &str); 2] = [("crates/hlslc/src/lower.rs", "pc_slot(list, n)"), ("crates/hlslc/src/lower.rs", "Lane::mask(n) << at")];

/// Each of PCSG_PACK missing from its file's code.
pub(super) fn pcsg_pack(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (file, want) in PCSG_PACK {
        let f = Rel::new(file);
        if !product_code(tree, &f).contains(want) {
            out.push((f, 1, format!("no `{want}` in the code: patch constants each in a register of their own, not packed as d3dcompiler packs them")));
        }
    }
    out
}

// --- warp-setup ------------------------------------------------------------------------------

/// Direct3D 11's interpolated pixel shader inputs as WARP makes them (its clipper, snap and
/// planes in a compute pass, the planes evaluated per fragment; winref d3d11logic, d3d11interp):
/// (file, what its code outside the tests says).
pub const WARP_SETUP: [(&str, &str); 4] = [
    ("crates/kura/src/d3d11/expand.rs", "setup_wanted(d, st)"),
    ("crates/kura/src/d3d11/expand.rs", "setup_draw(d, st, "),
    ("crates/dxwgsl/src/wgsl.rs", "expand::setup::inputs("),
    ("crates/dxwgsl/src/wgsl/setup.rs", "fn sx_plane("),
];

/// What WARP_SETUP's files must not say: a fused multiply-add in the setup's WGSL (one rounding
/// where WARP's plane equations round after the multiply and after the add).
pub const WARP_SETUP_NOT: [(&str, &str); 2] = [("crates/dxwgsl/src/wgsl/setup.rs", "fma("), ("crates/dxwgsl/src/wgsl/setup.rs", "mul_add(")];

/// Each of WARP_SETUP missing from its file's code, each of WARP_SETUP_NOT in it.
pub(super) fn warp_setup(tree: &Tree) -> Hits {
    says(tree, &WARP_SETUP, &WARP_SETUP_NOT, "interpolated inputs left to WebGPU's rasterizer, not WARP's setup", "one rounding where WARP rounds the product and the sum each")
}

/// Each `want` missing from its file's code and each `bad` in it, with the reason given.
fn says(tree: &Tree, want: &[(&str, &str)], bad: &[(&str, &str)], missing: &str, present: &str) -> Hits {
    let mut out = Hits::new();
    for (file, w) in want {
        let f = Rel::new(file);
        if !product_code(tree, &f).contains(w) {
            out.push((f, 1, format!("no `{w}` in the code: {missing}")));
        }
    }
    for (file, b) in bad {
        let f = Rel::new(file);
        if product_code(tree, &f).contains(b) {
            out.push((f, 1, format!("`{b}`: {present}")));
        }
    }
    out
}

// --- dll-search ------------------------------------------------------------------------------

/// The loader looks for a DLL named without a path in LdrGetDllPath's folders for that load's
/// module and flags, and a DLL's imports as the load that found it (review b5dacf4 2.1: an order
/// of its own, every by-path DLL's folder in it, LoadLibraryEx's flags dropped): (file, what its
/// code outside the tests says).
pub const DLL_SEARCH: [(&str, &str); 4] = [
    ("crates/sake/src/pe.rs", "crate::dllpath::dll_folders(self.module.as_deref(), self.flags)"),
    ("crates/sake/src/pe.rs", "for d in search.folders()"),
    ("crates/sake/src/pe.rs", "image_resolver(search.of_dependency(&win))"),
    ("crates/sake/src/k32.rs", "crate::pe::LoadSearch { module: Some(n.clone()), flags, missing: crate::pe::OnMissing::Fails }"),
];

/// What the loader must not say: folders of its own.
pub const DLL_SEARCH_NOT: [(&str, &str); 4] = [
    ("crates/sake/src/pe.rs", "DLL_DIRS"),
    ("crates/sake/src/pe.rs", "dllpath::user_dirs()"),
    ("crates/sake/src/pe.rs", "files::cwd()"),
    ("crates/sake/src/pe.rs", "\"C:\\\\windows\\\\system32\".into()"),
];

pub(super) fn dll_search(tree: &Tree) -> Hits {
    says(tree, &DLL_SEARCH, &DLL_SEARCH_NOT, "a DLL looked for outside LdrGetDllPath's folders", "the loader's own folders, not LdrGetDllPath's")
}

// --- missing-import-fails --------------------------------------------------------------------

/// A LoadLibrary whose DLLs import from a DLL file that is not there, or a function a DLL file
/// lacks, fails with that error and leaves nothing mapped (review b5dacf4 2.1: bound to a trap,
/// the DLL loaded): (file, what its code outside the tests says).
pub const MISSING_IMPORT: [(&str, &str); 5] = [
    ("crates/sake/src/pe.rs", "note_missing(match e {"),
    ("crates/sake/src/pe.rs", "note_missing(LoadFailure::NoProc)"),
    ("crates/sake/src/pe.rs", "Err(DllError::Failed(why)) => Err(abandon(cpu, why))"),
    ("crates/sake/src/pe.rs", "ABANDONED.locked().push(base)"),
    ("crates/sake/src/k32.rs", "last_error(why.error())"),
];

/// What LoadLibrary must not say: one error for every failure.
pub const MISSING_IMPORT_NOT: [(&str, &str); 1] = [("crates/sake/src/k32.rs", "let r = crate::pe::load_library(c.cpu, n);")];

pub(super) fn missing_import(tree: &Tree) -> Hits {
    says(tree, &MISSING_IMPORT, &MISSING_IMPORT_NOT, "a missing import of a DLL LoadLibrary loads does not fail the load", "LoadLibrary answers ERROR_MOD_NOT_FOUND whatever failed")
}

// --- draw-plan -------------------------------------------------------------------------------

/// A D3D11 draw's page topology, fill and indices are one typed plan, the logic-op split takes
/// its topology and the count of the indices it draws (review b5dacf4 3.1): (file, what its code
/// outside the tests says).
pub const DRAW_PLAN: [(&str, &str); 3] = [
    ("crates/kura/src/d3d11/mod.rs", "DrawPlan::of(assembled, adjacent, fill, sprite.is_some(), store.is_some(), count)"),
    ("crates/kura/src/d3d11/mod.rs", "logic::primitives(plan.topo, drawn, &cuts)"),
    ("crates/kura/src/d3d11/logic.rs", "pub fn primitives(topo: super::PageTopo"),
];

/// What the draw must not say: page topology numbers matched or chosen by hand.
pub const DRAW_PLAN_NOT: [(&str, &str); 3] = [
    ("crates/kura/src/d3d11/mod.rs", "let topo = match st.topology {"),
    ("crates/kura/src/d3d11/mod.rs", "let topo = if wired { 1 } else { topo };"),
    ("crates/kura/src/d3d11/mod.rs", "let topo = if wire.is_some() { 1 } else { topo };"),
];

pub(super) fn draw_plan(tree: &Tree) -> Hits {
    says(tree, &DRAW_PLAN, &DRAW_PLAN_NOT, "a D3D11 draw's topology or indices chosen outside DrawPlan", "page topology numbers chosen by hand, not by DrawPlan")
}

// --- full-path-one-rule ----------------------------------------------------------------------

/// Every file operation opens the full path GetFullPathName gives (review b5dacf4 2.2: `D:x`
/// opened D:\x while GetFullPathName answered from the `=D:` variable): files::full_path makes
/// it with fullpath's RtlGetFullPathName_Ustr.
pub const FULL_PATH: [(&str, &str); 2] = [
    ("crates/sake/src/files.rs", "crate::fullpath::process_env_full(&w, 0xfffe)"),
    ("crates/sake/src/files.rs", "FullMode::Win32 => win32_full(p)"),
];

pub(super) fn full_path_one_rule(tree: &Tree) -> Hits {
    says(tree, &FULL_PATH, &[], "files::full_path makes a path of its own, not GetFullPathName's", "")
}

// --- short-name-table ------------------------------------------------------------------------

/// A name's 8.3 name is given when the name is made and kept in its folder's table (review
/// b5dacf4 2.3: made again from each listing, so deleting a file renamed another): kura::fs
/// tells the watcher each name made and removed, sake watches from its first guest path on.
pub const SHORT_NAMES: [(&str, &str); 7] = [
    ("crates/kura/src/fs.rs", "named(Named::Made(&at));"),
    ("crates/kura/src/fs.rs", "named(Named::Gone(&at));"),
    ("crates/kura/src/fs.rs", "named(Named::Gone(&a));"),
    ("crates/kura/src/fs.rs", "named(Named::Made(&b));"),
    ("crates/sake/src/shortname.rs", "kura::fs::watch_names(on_named);"),
    ("crates/sake/src/shortname.rs", "let mut t = read_table(host_dir);"),
    ("crates/sake/src/files.rs", "crate::shortname::watch();"),
];

/// sake lists and removes a guest folder through shortname (its table is no guest entry, and a
/// folder that holds only it is empty): kura::fs::read_dir and remove_dir anywhere else, outside
/// tests.
pub(super) fn short_name_table(tree: &Tree) -> Hits {
    let mut out = says(tree, &SHORT_NAMES, &[], "a short name made again from a listing, not kept from when its name was made", "");
    let files = tree.rs(&["crates/sake/src/"]);
    let tests = test_mod_files(tree, &files);
    for f in files.iter().filter(|f| !tests.contains(*f) && f.as_str() != "crates/sake/src/shortname.rs") {
        let code = product_code(tree, f);
        for bad in ["kura::fs::read_dir(", "kura::fs::remove_dir("] {
            for (at, _) in code.match_indices(bad) {
                out.push((f.clone(), crate::code::line_of(&code, at), format!("`{bad}`: a guest folder listed or removed past its short-name table (shortname::read_dir / remove_dir)")));
            }
        }
    }
    out
}
