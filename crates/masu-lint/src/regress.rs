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

// --- apt-bundle -------------------------------------------------------------------------------

/// what may run apt itself: the installer of the bundles and the script that makes them
pub const APT_OWN: [&str; 2] = ["tools/ci/apt.sh", "tools/ci/apt-bundle.sh"];
/// the sets ci-bundles.yml uploads, one per line
pub const APT_SETS: &str = "tools/ci/apt-sets.txt";
const APT_CALL: &str = "tools/ci/apt.sh";
/// what reads an apt mirror, or the cache of .debs per runner image that did
const APT_MIRROR: [&str; 4] = ["apt-get ", "apt install", "apt update", ".apt-cache"];

/// the set a line gives tools/ci/apt.sh: its words up to the end of that command
pub fn apt_set(code: &str) -> Option<String> {
    let rest = code.match_indices(APT_CALL).filter_map(|(at, _)| code.get(at.saturating_add(APT_CALL.len())..)).find(|r| r.is_empty() || r.starts_with(char::is_whitespace) || r.starts_with(')'))?;
    let mut words = Vec::new();
    for w in rest.split_whitespace() {
        if matches!(w, "&&" | "||" | ";" | "|" | ")" | "\\") {
            break;
        }
        let word = w.trim_end_matches([';', ')']);
        if !word.is_empty() {
            words.push(word);
        }
        if word.len() < w.len() {
            break;
        }
    }
    Some(words.join(" "))
}

pub(super) fn apt_bundle(tree: &Tree) -> Hits {
    let sets_file = Rel::new(APT_SETS);
    let sets: BTreeSet<String> = tree.text(&sets_file).lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_owned).collect();
    let mut out = Hits::new();
    for p in tree.paths().iter().filter(|p| is_shell(p) && !APT_OWN.contains(&p.as_str())) {
        for (k, l) in tree.text(p).lines().enumerate() {
            if l.trim_start().starts_with('#') {
                continue;
            }
            let code = l.split(" #").next().unwrap_or(l);
            let line = k.saturating_add(1);
            if let Some(m) = APT_MIRROR.iter().find(|m| code.contains(*m)) {
                out.push((p.clone(), line, format!("`{}`: apt's mirror at CI time; tools/ci/apt.sh installs a set of tools/ci/apt-sets.txt from the release ci-bundles", m.trim())));
            }
            match apt_set(code) {
                Some(set) if set.is_empty() => out.push((p.clone(), line, "tools/ci/apt.sh given no package".into())),
                Some(set) if !sets.contains(&set) => out.push((p.clone(), line, format!("`{set}` is not a line of {APT_SETS}: ci-bundles has no tarball for it"))),
                Some(_) | None => {}
            }
        }
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
    ("crates/kura/src/d3d11/mod.rs", "dxwgsl::translate_views(&s.code, true, None, None, &key.1, &key.2,", "the pixel shader of a draw with logic ops is not translated with its targets' logic (dxwgsl::translate_views)"),
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

// --- snorm-targets ------------------------------------------------------------------------------

/// 8-bit SNORM render targets (2026-11-22: kura dropped every clear of and draw to them,
/// WebGPU rendering to no SNORM format)
const SNORM_SITES: [(&str, &str, &str); 40] = [
    ("crates/kura/src/d3d11/mod.rs", "logic::snorm_ms_page(fmt, dim, layers, mips, samples, bind)", "a multisampled SNORM texture is not its SINT twin on the page (WebGPU multisamples no SNORM format: 4x SNORM would be one sample)"),
    ("crates/kura/src/d3d11/logic.rs", "Some(blend) if ms && !sample_rate && blend.reads_dst() => Store::SnormStash { blend, mark },", "a multisampled SNORM blend that reads its target under a pixel-rate shader is not done per sample in the resolve pass"),
    ("crates/kura/src/d3d11/logic.rs", "gpu::SNORM_RESOLVE, tex, copy, stash, mark_tex, mask", "an SNORM stash's draw has no resolve pass after it"),
    ("crates/kura/src/d3d11/mod.rs", "q.cmd(&[gpu::RESOLVE_SNORM, twin, src]);", "ResolveSubresource of a multisampled SNORM texture is not WARP's resolve of its bytes"),
    ("crates/kura/src/d3d11/mod.rs", "let views = snorm_ms_srvs(d, st, 5);", "a compute shader's SRV of a multisampled SNORM texture is not loaded as its bytes' floats"),
    ("crates/kura/src/d3d11/mod.rs", "let ps_views = snorm_ms_srvs(d, st, 1);", "a pixel shader's SRV of a multisampled SNORM texture is not loaded as its bytes' floats"),
    ("crates/dxwgsl/src/wgsl.rs", "v.load_wgsl(&format!(\"textureLoad({tv}, {args})\"))", "a load of a multisampled SNORM SRV does not convert the page's texels to the format's floats (snorm::View::load_wgsl)"),
    ("crates/kura/src/d3d11/mod.rs", "logic::twin_of(", "a draw to an SNORM target is not drawn to its SINT twin (logic::twin_of)"),
    ("crates/kura/src/d3d11/mod.rs", "logic::snorm_twin(*fmt).is_some()", "ClearRenderTargetView of an SNORM view does not write its texels (the page clears no SNORM pass)"),
    ("crates/kura/src/d3d11/mod.rs", "logic::split(&w, &copies, stash.as_ref(), &twins,", "a draw's SNORM twins are not loaded before its primitives and stored back after (logic::split)"),
    ("crates/kura/src/d3d11/logic.rs", "31 => Some((f::P_RGBA8SI, [8, 8, 8, 8])),", "R8G8B8A8_SNORM views have no SINT twin"),
    ("crates/kura/src/d3d11/logic.rs", "51 => Some((f::P_RG8SI, [8, 8, 0, 0])),", "R8G8_SNORM views have no SINT twin"),
    ("crates/kura/src/d3d11/logic.rs", "63 => Some((f::P_R8SI, [8, 0, 0, 0])),", "R8_SNORM views have no SINT twin"),
    ("crates/kura/src/d3d11/logic.rs", "Some(_) | None => Store::Snorm(b),", "an SNORM target's store does not carry its blend (logic::targets)"),
    ("crates/dxwgsl/src/wgsl.rs", "Store::Snorm(blend) =>", "an SNORM target's output does not go through logic_output's store and blend"),
    ("crates/dxwgsl/src/snorm.rs", "kome::num::trunc_i32((clamp_src(v) * w.scale()).round())", "an SNORM store no longer rounds WARP's way (the f32 product, halves away from zero; winref d3d11snorm)"),
    ("crates/dxwgsl/src/snorm.rs", "if v.is_nan() { -1.0 } else { v.clamp(-1.0, 1.0) }", "a NaN output to an SNORM target is not -127 (WARP writes NaN as -127; winref d3d11snorm)"),
    ("crates/kura/src/d3d11/clear.rs", "let q = trunc_i64(f64::from((v.clamp(-1.0, 1.0) * max).round()));", "an SNORM clear no longer rounds halves away from zero (winref d3d11snorm: 2.5 / 127 clears to 3)"),
    ("crates/dxwgsl/src/snorm.rs", "format!(\"sn_r1({s} * {}, sn_z)\", e.src.wgsl(c))", "an SNORM blend's products are no longer kept from fusing with the add (an FMA rounds once: 1.9 * -5/127 blends to -10, WARP to -9)"),
    ("crates/dxwgsl/src/snorm.rs", "const SN_LOAD = array<u32, 256>(", "the shader's SNORM load divides (WGSL's division is not exact: the blend's last bit differs from WARP's); it reads load()'s table"),
    ("crates/dxwgsl/src/wgsl.rs", "\"let sn_k = sn_clamp(sn_kv);\"", "an SNORM blend's constant is not clamped to -1..1 in the shader (WARP blends 1.5 as 1; winref d3d11snorm)"),
    ("crates/kura/src/d3d11/mod.rs", "and_then(dxwgsl::snorm::Blend::of).is_some_and(|b| b.takes()))?;", "CreateBlendState takes MIN / MAX with a dual-source factor (WARP: E_INVALIDARG; winref d3d11snorm)"),
    ("crates/dxwgsl/src/snorm.rs", "kome::num::to_f32(q.max(w.top().wrapping_neg())) * resolve_rcp(w)", "ResolveSubresource of SNORM reads its samples as the quotient, not WARP's product with the f32 nearest 1/127 (winref d3d11snorm's seed rows: 20 of 960 differ)"),
    ("crates/dxwgsl/src/snorm.rs", "s = s + sn_r(vec4<f32>(b) * bitcast<f32>(0x{rcp:08x}u),", "the resolve pass's SNORM samples are not WARP's product with 1/127, kept from fusing with the sum"),
    ("crates/kura/src/d3d11/clear.rs", "trunc_u32(f64::from((v.clamp(0.0, 1.0) * max).round_ties_even()))", "a UNORM clear is not rounded from the f32 product (WARP clears 0.5 / 255's nearest f32 to 0; winref d3d11snorm)"),
    ("crates/kura/src/d3d11/logic.rs", "13 => Some((f::P_RGBA16SI, [16, 16, 16, 16])),", "R16G16B16A16_SNORM views have no SINT twin (a draw to one is not clamped, rounded and blended as WARP does)"),
    ("crates/kura/src/d3d11/logic.rs", "37 => Some((f::P_RG16SI, [16, 16, 0, 0])),", "R16G16_SNORM views have no SINT twin"),
    ("crates/kura/src/d3d11/logic.rs", "58 => Some((f::P_R16SI, [16, 0, 0, 0])),", "R16_SNORM views have no SINT twin"),
    ("crates/dxwgsl/src/snorm.rs", "Width::Bits16 => (kome::num::to_f32(q) * resolve_rcp(w)).max(-1.0),", "a 16-bit SNORM value is read as the quotient, not WARP's product with the f32 nearest 1/32767 (winref d3d11snorm2's load rows: 24 of 1024 differ)"),
    ("crates/dxwgsl/src/snorm.rs", "return max(sn_r(vec4<f32>(b) * bitcast<f32>(0x{:08x}u), vec4<u32>(b + vec4<i32>(32768)) >> vec4<u32>(16u)), vec4<f32>(-1.0f));", "the shader's 16-bit SNORM load is not WARP's product with 1/32767, kept from fusing"),
    ("crates/kura/src/d3d11/format.rs", "dxwgsl::snorm::load_in(dxwgsl::snorm::Width::Bits16, i32::from(v.cast_signed()))", "16-bit SNORM texels uploaded to the page are other floats than WARP reads (the quotient, not load_in's product)"),
    ("crates/sake/src/k32.rs", "const CONSOLE_PIECE: usize = 65536;", "a console line of more than 4096 bytes is cut in two (winref d3d11snorm2's R16G16B16A16 sweep then differs from Windows')"),
    ("crates/dxwgsl/src/snorm_tests.rs", "fn warp_answers16()", "the 16-bit SNORM, ClearView and typeless MSAA answers of winref d3d11snorm2 are no longer checked"),
    ("crates/kura/src/d3d9/gpu.rs", "Convert::Sint16ToSnorm16 => format!(\"sn16_load({v})\"),", "a 16-bit SNORM twin's values go back to the page as other floats than their quotients"),
    ("crates/kura/src/d3d11/logic.rs", "let load = twin_copy(tex, sub, twin, 0, w, h, own, page, ms)?;", "an SNORM twin is not loaded by twin_copy (a 16-bit page's floats or a multisampled typeless page's bytes copied as bits)"),
    ("crates/kura/src/d3d11/logic.rs", "let tex = match twins.iter().find(|t| t.target == l.target) {", "an SNORM stash's resolve writes the target's page texture, not the twin the draw loads and stores"),
    ("crates/kura/src/d3d11/mod.rs", "dxwgsl::snorm::Source::Unorm8", "an SNORM SRV of a multisampled typeless texture loads the UNORM page's floats, not the bytes' SNORM values"),
    ("crates/kura/src/d3d11/op.rs", "clear_ms_rects(d, *view, *color, rects)", "ClearView with rectangles of a multisampled texture's view writes texels the page does not keep (it clears nothing)"),
    ("crates/kura/src/d3d11/mod.rs", "logic::twin_copy(bg, 0, ms, 0, w, h, b_page, page, true)", "ResolveSubresource as SNORM of a multisampled typeless texture resolves its UNORM floats"),
    ("crates/dxwgsl/src/snorm.rs", "#[path = \"snorm_tests.rs\"]", "the SNORM conversions and blend are no longer checked against WARP's answers (snorm_tests.rs)"),
];

/// SNORM targets dropped
pub(super) fn snorm_targets(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (path, needle, why) in SNORM_SITES {
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

// --- kura-review-6852be1 ------------------------------------------------------------------------

/// What the review of kura at 6852be1 found wrong, fixed (2026-11-22), each by its spelling in
/// the code (comments left out): (file, text it must hold, what is wrong without it).
pub(super) const REVIEW_6852BE1_HAS: [(&str, &str, &str); 43] = [
    ("crates/kura/src/d3d11/hazards.rs", "out.extend(st.so.iter()", "stream output targets are not outputs: a vertex buffer of one stays bound (D3D11 unbinds it)"),
    ("crates/kura/src/d3d11/hazards.rs", "for vb in st.vbs.iter_mut()", "the vertex buffers checked are not all 32 slots"),
    ("crates/kura/src/d3d11/op.rs", "if let Uavs::Set { start, views, counts } = uavs {", "OMSetRenderTargetsAndUnorderedAccessViews with KEEP render targets does not resolve its new UAVs"),
    ("crates/kura/src/d3d11/mod.rs", "ContextOp::read(d, m, a)", "the immediate context reads a state call its own way, not as the ContextOp a deferred context records"),
    ("crates/kura/src/d3d11/deferred.rs", "ContextOp::read(d, m, a)", "a deferred context records a call its own way, not as the ContextOp the immediate context applies"),
    ("crates/kura/src/d3d11/deferred.rs", "recs: std::sync::Arc<Vec<ContextOp<'static>>>", "a command list keeps raw calls (replayed through guest memory) instead of ContextOps"),
    ("crates/kura/src/d3d11/mod.rs", "Some((Data::Deferred(_) | Data::Device(_), _)) => true,", "a deferred context let go of is never destroyed (its state and list hold their objects for good)"),
    ("crates/kura/src/jit.rs", "struct SharedCache", "the shared JIT cache's byte count is kept apart from its map (it drifted)"),
    ("crates/kura/src/jit.rs", "jit::TABLE_SLOTS", "the JIT table's size is not koji's (the loader's 20-bit index)"),
    ("host/vnjit.js", "VNJIT.slots", "vnjitRoom does not refuse a table index past the slots the loader encodes"),
    ("crates/kura/src/vfs.rs", "fn change<R: Refused>", "a change of the shared namespace answers success whether or not it was published"),
    ("crates/kura/src/vfs/shared.rs", "fs.undo();", "a change the shared buffer could not take is kept here (lost at the next snapshot)"),
    ("crates/kura/src/vfs.rs", "x.allows(Use::Read, false)?", "a handle is read without the access it was opened with"),
    ("crates/kura/src/d3d9/gpu.rs", "pub struct Readback(u32)", "readback ids are a number each device counts (two devices waited on each other's)"),
    ("crates/kura/src/snap.rs", "fn wait_resume(", "a stop the page gave up on waits for RESUME for ever"),
    ("web/src/savestate.ts", "word(CANCEL)", "the page does not cancel a stop request it gave up on"),
    ("crates/kura/src/vfs/saves.rs", "Out::Gone(was.clone())", "a store file deleted or renamed is not in the saves (it comes back at the next start)"),
    ("crates/kura/src/vfs.rs", "Over(Box<over::Over>),", "a store file written to is copied into memory whole (copy_up) instead of keeping the pages written over the store's bytes"),
    ("crates/kura/src/vfs/over.rs", "self.base = self.base.min(n);", "a store file cut and made long again shows the store's bytes past the cut again"),
    ("crates/kura/src/vfs/saves.rs", "Out::Over { mode: n.mode, mtime: n.mtime, over: o.clone() }", "a store file written to is left out of the saves, or saved whole"),
    ("crates/kura/src/vfs/shared.rs", "const I_OVER: u8 = 4;", "the other processes are sent a store file written to whole on every write"),
    ("crates/kura/src/vfs.rs", "fs.base = fs.entries();", "the saves are not told against the namespace the store gave"),
    ("web/src/file-records.ts", "(first & TAG) >>> 0 === TAG", "the page does not read kura's tagged records (gone names, directories, store files)"),
    ("crates/koji/src/jit/mod.rs", "global_get(PAGES).add().load8u(0)", "translated code reads the page marks at an address of the run that made it (a saved translation could not run with the checks on)"),
    ("crates/koji/src/jit/saved.rs", "pages: super::Pages", "the saved index does not say whether a translation was made with the page checks"),
    ("crates/kura/src/jit.rs", ".is_some_and(|e| e.pages.runs_now())", "saved translations are not taken by what they were made with (with the page checks on, none is used)"),
    ("host/vnjit.js", "pages: pages >>> 0,", "the modules' \"pages\" global is not given the page marks of this run"),
    ("crates/kura/src/d3d11/mod.rs", "let dev = d.new_obj(IF_DEVICE, Data::Device(Dev { level, ..Dev::NONE }));", "D3D11CreateDevice gives back a device made before, not one of its own"),
    ("crates/kura/src/d3d11/mod.rs", "d.enter(own);", "a call runs on the device last made, not on the device of the object it is called on"),
    ("crates/kura/src/d3d11/mod.rs", "    Context(Box<Imm>),", "the immediate context's state is not the context's own (one state for every device)"),
    ("crates/kura/src/gl/mod.rs", "g.objs = g.groups.remove(&c.group).unwrap_or_default();", "a GL call sees every context's objects, not its share group's"),
    ("crates/kura/src/gl/mod.rs", "pub extern \"C\" fn vngl_share_lists(", "wglShareLists does nothing (every context sharing everything)"),
    ("crates/sake/src/gl.rs", "(\"wglShareLists\", wglShareLists),", "wglShareLists answers TRUE without sharing"),
    ("crates/kura/src/gl/mod.rs", "Fbos::Core => c.fbos.get(&f.name),", "GL 3.0 / ARB framebuffers are the share group's, not each context's own"),
    ("crates/kura/src/gl/mod.rs", "bind_fbo(Fbos::Ext, target, f)", "EXT_framebuffer_object's framebuffers are each context's own, not the share group's"),
    ("crates/kura/src/d3d11/mod.rs", "let alias = d.new_obj(iface, Data::Opened(id));", "OpenSharedResource on another device gives back the other device's object"),
    ("crates/kura/src/d3d9/mod.rs", "let mine = std::mem::replace(&mut self.d8, std::mem::take(&mut next.d8));", "Direct3D 8 shader and state block handles are one table for every device"),
    ("crates/kura/src/d3d9/gpu.rs", "pub fn readback(&mut self, what: Read, args: &[u32]) -> Readback {", "a readback's id made apart from the command it answers"),
    ("crates/kura/src/d3d9/gpu.rs", "    fn issue() -> Self {", "readback ids anyone can make, not only the queue"),
    ("crates/kura/src/d3d11/mod.rs", "    pub content: crate::content::Content,", "where a D3D11 buffer's newest bytes are kept as flags that can say two things at once"),
    ("crates/kura/src/pages.rs", "let page = Arc::make_mut(&mut self.pages[i]);", "a file's pages copied whole for a save instead of shared until written"),
    ("crates/kura/src/vfs/saves.rs", "File { mtime: i64, data: crate::pages::Pages },", "a save's files borrowed from the VFS: written out only while its lock is held"),
    ("crates/kura/src/vfs.rs", "    let saved = {", "a save written out while the VFS lock is held"),
];

/// (file, text it must not hold, what is wrong with it)
pub(super) const REVIEW_6852BE1_NOT: [(&str, &str, &str); 18] = [
    ("crates/kura/src/d3d11/hazards.rs", "0..16", "a fixed slot count: the slots past it are not checked"),
    ("crates/kura/src/d3d11/hazards.rs", "0..128", "a fixed slot count: the slots past it are not checked"),
    ("crates/kura/src/jit.rs", "SHARED_BYTES", "a byte count of the shared cache apart from what the map holds"),
    ("crates/kura/src/vfs.rs", "fn copy_up(", "a store file copied from the store into memory whole before it is written"),
    ("host/vnjit.js", "1 << 20", "a table limit of its own, not the loader's (VNJIT.slots)"),
    ("crates/kura/src/vfs.rs", "fn with<R>(", "an operation whose result does not say that its change was not published"),
    ("crates/kura/src/snap.rs", "wait_cmd(CMD_RESUME)", "a wait for RESUME the page cannot cancel"),
    ("crates/kura/src/adec.rs", "LIST_FROM", "the stream list's start kept between calls (a page of it taken for the whole)"),
    ("crates/koji/src/jit/mod.rs", "PAGE_MAP", "the page marks' address as a static of the translator: translations hold it as an immediate"),
    ("crates/kura/src/jit.rs", "profiling() || jit::page_check()", "saved translations refused whenever the page checks are on, the checked ones too"),
    ("crates/kura/src/d3d11/deferred.rs", "fn pointed(", "a table of what each method's pointers point to, beside the one that reads them"),
    ("crates/kura/src/d3d11/deferred.rs", "fn objects(m: u32)", "a table of which arguments are objects, beside the one that reads them"),
    ("crates/kura/src/d3d11/deferred.rs", "guest_alloc(lo32(b.len()))", "a recorded call put back into guest memory to be read again"),
    ("crates/kura/src/d3d11/mod.rs", "made_by: HashMap<u32, u32>", "which device made a child kept apart from the child (one device per process, the others its side-faces)"),
    ("crates/kura/src/d3d11/mod.rs", "if d.device == 0 {", "a device made only when there is none yet"),
    ("crates/kura/src/gl/mod.rs", "fn vngl_context_create(_share: u32)", "the share context of a new context not looked at"),
    ("crates/kura/src/d3d9/d3d8.rs", "let old = std::mem::take(&mut d.d8);", "a new Direct3D 8 device takes the handles of the device before"),
    ("crates/kura/src/d3d11/expand.rs", "b.newest = ", "a buffer's newest copy set by hand, not by its versions"),
];

/// a fix of the review of 6852be1 taken back
pub(super) fn review_6852be1(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let sites = REVIEW_6852BE1_HAS.iter().map(|x| (x, true)).chain(REVIEW_6852BE1_NOT.iter().map(|x| (x, false)));
    for ((path, needle, why), has) in sites {
        let f = Rel::new(path);
        if tree.is_made_up() && !tree.has(&f) {
            continue;
        }
        let code = tree.code(&f, true);
        if code.contains(needle) != has {
            let line = if has { 1 } else { code.split(needle).next().map_or(1, |b| b.lines().count().max(1)) };
            out.push((f, line, (*why).to_owned()));
        }
    }
    out.extend(host_files_held(tree));
    out
}

/// A kura test that opens files under the host's temporary directory through kura's own file
/// functions (fs.rs, routed through the VFS when a path is in its tree) without
/// `vfs::tests::host_files()`: run with the VFS tests, which mount "/", its files were looked
/// for in their tree (fs::head_copy_tests failed so). Each fn that names `temp_dir()` holds
/// the guard, or takes it held (`&HostFiles`); vfs.rs's own tests hold TEST_FS through their
/// trees.
fn host_files_held(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for f in tree.rs(&["crates/kura/src/"]) {
        if f.as_str().starts_with("crates/kura/src/vfs") {
            continue;
        }
        let code = tree.code(&f, true);
        let mut from = 0;
        while let Some(k) = code.get(from..).and_then(|c| c.find("temp_dir()")) {
            let at = from.saturating_add(k);
            let start = code.get(..at).and_then(|c| c.rfind("fn ")).unwrap_or(0);
            let end = code.get(at..).and_then(|c| c.find("\n    fn ").or_else(|| c.find("\nfn "))).map_or(code.len(), |e| at.saturating_add(e));
            if !code.get(start..end).is_some_and(|body| body.contains("tests::host_files();") || body.contains("&crate::vfs::tests::HostFiles")) {
                let line = code.get(..at).map_or(1, |b| b.lines().count().max(1));
                out.push((f.clone(), line, "a test's host file opened without vfs::tests::host_files() held (a VFS test's tree takes the path)".to_owned()));
            }
            from = at.saturating_add("temp_dir()".len());
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

// --- d3d9dev-formats --------------------------------------------------------------------------

/// the probe and the test that reads its golden
const D3D9DEV_PROBE: &str = "tests/winref/probes/d3d9dev.c";
const D3D9DEV_TEST: &str = "crates/kura/src/d3d9/dev_tests.rs";

static D3D9DEV_ROW: LazyLock<Option<Regex>> = LazyLock::new(|| rx(r#"\{\s*(?:D3DFMT_[A-Z0-9_]+|\(D3DFORMAT\)MAKEFOURCC\([^)]*\))\s*,\s*"([A-Za-z0-9_]+)"\s*\}"#));

/// a format of the d3d9dev probe's table that d3d9_answers_as_the_golden has no name for
pub(super) fn d3d9dev_formats(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let Some(re) = D3D9DEV_ROW.as_ref() else { return out };
    let (probe, test) = (Rel::new(D3D9DEV_PROBE), Rel::new(D3D9DEV_TEST));
    let named = tree.text(&test);
    let text = tree.text(&probe);
    for c in re.captures_iter(&text) {
        let Some(n) = c.get(1) else { continue };
        if !named.contains(&format!("\"{}\" =>", n.as_str())) {
            let line = crate::code::line_of(&text, n.start());
            out.push((probe.clone(), line, format!("d3d9dev asks {} but {D3D9DEV_TEST}'s d3d9_answers_as_the_golden has no \"{}\" => arm", n.as_str(), n.as_str())));
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

/// the crates formal/kani.sh runs `cargo kani` in (`crates/x`)
fn kani_roots(tree: &Tree) -> Vec<String> {
    let script = tree.text(&Rel::new(KANI_SCRIPT));
    KANI_FOR.as_ref().and_then(|r| r.captures(&script)).and_then(|c| c.get(1)).map_or_default(|m| m.as_str().split_whitespace().map(|c| format!("crates/{c}")).collect())
}

/// the crates `cargo kani` compiles: the script's and every crate they link by path
pub(super) fn kani_crates(tree: &Tree) -> BTreeSet<String> {
    let roots = kani_roots(tree);
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

// --- kani-cover -------------------------------------------------------------------------------

/// The `#[kani::proof]` fns of a file (in any module, `#[cfg(kani)]` ones included): (name,
/// line, whether it narrows its inputs, whether it has a `kani::cover!`)
fn kani_proofs(file: &syn::File) -> Vec<(String, usize, bool, bool)> {
    use quote::ToTokens;
    use syn::visit::Visit;
    struct V(Vec<(String, usize, bool, bool)>);
    impl<'a> Visit<'a> for V {
        fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
            let proof = f.attrs.iter().any(|a| {
                let p: Vec<String> = a.path().segments.iter().map(|s| s.ident.to_string()).collect();
                p == ["kani", "proof"]
            });
            if proof {
                let toks = f.block.to_token_stream().to_string();
                let words: Vec<&str> = toks.split_whitespace().collect();
                let body: String = words.concat();
                let narrows = body.contains("kani::assume(") || words.contains(&"return");
                let covers = body.contains("kani::cover!(");
                self.0.push((f.sig.ident.to_string(), crate::syntax::line(f.sig.ident.span()), narrows, covers));
            }
            syn::visit::visit_item_fn(self, f);
        }
    }
    let mut v = V(Vec::new());
    v.visit_file(file);
    v.0
}

pub(super) fn kani_cover(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    // (the script's own crates: `cargo kani` in a crate checks that crate's proofs, not those of
    // the crates it links)
    let run: BTreeSet<String> = kani_roots(tree).into_iter().collect();
    for f in tree.rs(&["crates/"]) {
        let Ok(file) = syn::parse_file(&tree.text(&f)) else { continue };
        let proofs = kani_proofs(&file);
        for (name, l, narrows, covers) in &proofs {
            if *narrows && !covers {
                out.push((f.clone(), *l, format!("Kani proof `{name}` narrows its inputs (assume / return / let-else) but has no `kani::cover!`: show the cases it is about are reached")));
            }
        }
        // (crates/<c>/src/..: the crate is crates/<c>)
        let mut parts = f.0.splitn(3, '/');
        let (Some(_), Some(c)) = (parts.next(), parts.next()) else { continue };
        let krate = format!("crates/{c}");
        if let Some((name, l, ..)) = proofs.first()
            && !run.contains(&krate)
        {
            out.push((f.clone(), *l, format!("Kani proof `{name}` is in {krate}, which formal/kani.sh does not run (`for c in ...; do`)")));
        }
    }
    out
}

// --- verus-mirror -----------------------------------------------------------------------------

pub(super) fn verus_mirror(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let read: String = tree.rs(&["crates/"]).iter().map(|f| tree.text(f)).filter(|t| t.contains("formal/verus/")).collect();
    for f in tree.rs(&["formal/verus/"]) {
        let Some(name) = f.as_str().rsplit('/').next() else { continue };
        if !read.contains(&format!("formal/verus/{name}\")")) {
            out.push((f.clone(), 1, format!("no test reads {f} (`include_str!(\"../../../formal/verus/{name}\")` in a test of the code it is a copy of, comparing the two)")));
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
        let lead = rest.len().saturating_sub(rest.trim_start().len());
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
/// where WARP's plane equations round after the multiply and after the add); the slot header's
/// rhw or depth plane (one vec4 (gx, gy, c, _) each) read as an input's (three vec4s down: the
/// pixel shader's SV_Position.z and .w and every perspective input came out wrong, winref
/// d3d11wire on the page).
pub const WARP_SETUP_NOT: [(&str, &str); 4] = [
    ("crates/dxwgsl/src/wgsl/setup.rs", "fma("),
    ("crates/dxwgsl/src/wgsl/setup.rs", "mul_add("),
    ("crates/dxwgsl/src/wgsl/setup.rs", "sx_at(sx_o + 3u"),
    ("crates/dxwgsl/src/wgsl/setup.rs", "sx_at(sx_o + 4u"),
];

/// Each of WARP_SETUP missing from its file's code, each of WARP_SETUP_NOT in it.
pub(super) fn warp_setup(tree: &Tree) -> Hits {
    says(tree, &WARP_SETUP, &WARP_SETUP_NOT, "interpolated inputs left to WebGPU's rasterizer, not WARP's setup", "one rounding where WARP rounds the product and the sum each")
}

// --- warp-wire -------------------------------------------------------------------------------

/// FillMode WIREFRAME as WARP draws it (winref d3d11wire): the edges the setup's aliased lines,
/// culled and biased there, (file, what its code outside the tests says).
pub const WARP_WIRE: [(&str, &str); 6] = [
    ("crates/dxwgsl/src/wgsl/setup.rs", "b.of(zmax, "),
    ("crates/dxwgsl/src/wgsl/setup.rs", "sx_bias(sx_wz, "),
    ("crates/kura/src/d3d11/expand.rs", "put(w, x::xd::BIAS, 0, "),
    ("crates/kura/src/d3d11/expand.rs", "Fill::of(f) == Fill::Wireframe"),
    ("crates/kura/src/d3d11/mod.rs", "expand::SetupStore::squares"),
    ("crates/kura/src/d3d11/mod.rs", "Fill::Wireframe => topo.triangles() && !sprite && !squares"),
];

/// What WARP_WIRE's files must not say: the edges of the draw's own triangles as a line list
/// (no culling, no bias: review b5dacf4 3.2); a device feature asked from the draw's side (the
/// device is the page's: the sake worker has none, and asking threw there); an f32 literal of
/// the 38th power in the hand-written WGSL (Tint reads it as a double and refused
/// -3.40282347e38f, past f32's largest: `bitcast<f32>(0xff7fffffu)` or dxwgsl::f32_text).
pub const WARP_WIRE_NOT: [(&str, &str); 8] = [
    ("crates/kura/src/d3d11/mod.rs", "fn wire_edges("),
    ("crates/kura/src/d3d11/mod.rs", "fn triangle_edges("),
    ("crates/kura/src/d3d11/mod.rs", "has_feature("),
    ("crates/kura/src/d3d11/expand.rs", "has_feature("),
    ("crates/dxwgsl/src/wgsl/setup.rs", "e38f"),
    ("crates/dxwgsl/src/wgsl/ps.rs", "e38f"),
    ("crates/dxwgsl/src/wgsl/expand.rs", "e38f"),
    ("crates/dxwgsl/src/wgsl.rs", "e38f"),
];

/// Each of WARP_WIRE missing from its file's code, each of WARP_WIRE_NOT in it.
pub(super) fn warp_wire(tree: &Tree) -> Hits {
    says(tree, &WARP_WIRE, &WARP_WIRE_NOT, "wireframe edges not the setup's culled and biased lines", "a wireframe drawn as the draw's own edges, unculled and unbiased")
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
    ("crates/kura/src/d3d11/mod.rs", "DrawPlan::of(assembled, adjacent, fill, sprite.is_some(), squares)"),
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

// --- jit-exit-contract -----------------------------------------------------------------------

/// koji's exit contract (jit::ExitAt): at an exit before or after an instruction every status
/// flag is exact (a guest exception handler reads EFLAGS: `xor eax,eax / mov ecx,[0]` left
/// 0x202 where the interpreter has 0x246), at a jump only those code there reads. (file, what its
/// code outside the tests says): each exit checks the contract, the liveness sees the bails of
/// an ESP not checked (1063 translations redone in 600 jit32test programs), a callee reading no
/// flags is recorded so (calls +22% with all of them kept for it).
pub const JIT_EXIT_CONTRACT: [(&str, &str); 5] = [
    ("crates/koji/src/jit/mod.rs", "self.exit_contract(at);"),
    ("crates/koji/src/jit/mod.rs", "self.exit_contract(ExitAt::Jump(need));"),
    ("crates/koji/src/jit/mod.rs", "self.exit_contract(ExitAt::Before);"),
    ("crates/koji/src/jit/mod.rs", "let esp = esp_unknown(list);"),
    ("crates/koji/src/jit/mod.rs", "call_reads.insert(k, r);"),
];

/// What JIT_EXIT_CONTRACT's files must not say: all flags taken live at a loop's head for its
/// hoisted checks (they leave for that same head: ExitAt::Jump with its live-in; sort +30%).
pub const JIT_EXIT_CONTRACT_NOT: [(&str, &str); 1] = [("crates/koji/src/jit/mod.rs", "g.hoist.contains_key(&k) ||")];

/// A line of koji's JIT emitting a constant made of iced's RflagsBits (STATUS_BITS among them):
/// those are the liveness' bits (CF 0x10, OF 0x01), not EFLAGS' (crate::flags: CF 0x01, OF
/// 0x800); rotate_flags wrote EFLAGS from them once.
fn rflags_emitted(line: &str) -> bool {
    (line.contains("RflagsBits") || line.contains("STATUS_BITS")) && ["u32c(", "i32c(", "i64c(", "u64c("].iter().any(|c| line.contains(c))
}

/// Each of JIT_EXIT_CONTRACT missing from its file's code, each of JIT_EXIT_CONTRACT_NOT in it,
/// each line of the JIT emitting RflagsBits.
/// The statics koji's translations are made under (jit::context): only TranslationContext::now
/// reads them, in context.rs
pub const JIT_MODES: [&str; 19] = ["SSA", "MX_EXACT", "MX_SWITCH", "PAGE_CHECK", "NOCHECK", "LAYOUT", "PROFILE", "COUNT", "HOIST", "SLOTS_MAX", "FN_ENTRIES", "FN", "FN64", "STRUCTURED", "DEBUG_CHECK", "DEBUG_ALL_REGS", "DEBUG_NO_SIDE", "MEMCOUNT", "PROBES"];

/// The functions that read them (a free call: a method of the context is the context's)
pub const JIT_MODE_READS: [&str; 3] = ["mx_exact()", "page_check()", "Pages::now()"];

/// What of `l` reads a mode of [`JIT_MODES`] / [`JIT_MODE_READS`] outside the context
fn mode_read(l: &str) -> Option<&'static str> {
    let free = |l: &str, at: usize| at == 0 || !l[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == ':') || l[..at].ends_with("super::") || l[..at].ends_with("jit::");
    for n in JIT_MODES {
        for (at, _) in l.match_indices(n) {
            let rest = &l[at + n.len()..];
            if free(l, at) && (rest.starts_with(".load(") || rest.starts_with(".swap(") || rest.starts_with(".iter()")) {
                return Some(n);
            }
        }
    }
    for n in JIT_MODE_READS {
        for (at, _) in l.match_indices(n) {
            if free(l, at) && !l[..at].trim_end().ends_with("fn") {
                return Some(n);
            }
        }
    }
    None
}

/// koji's generators read the modes from their TranslationContext only (see Rule::JitContext)
pub(super) fn jit_context(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for f in tree.rs(&["crates/koji/src/jit/"]) {
        if f.as_str() == "crates/koji/src/jit/context.rs" {
            continue;
        }
        for (k, l) in product_code(tree, &f).lines().enumerate() {
            if let Some(n) = mode_read(l) {
                out.push((f.clone(), k.saturating_add(1), format!("`{n}` read while translating: the translation's TranslationContext has it (read once, when it began)")));
            }
        }
    }
    out
}

/// The budget statics jit32test varies: koji::jit::<static>.store(mode.<field> (see
/// Rule::JitBudgetHarness)
pub const JIT_BUDGET_HARNESS: [(&str, &str); 3] = [("SSA", "mode.ssa"), ("HOIST", "mode.hoist"), ("SLOTS_MAX", "mode.slots")];

/// Budget statics jit32test leaves as they are, and why
pub const JIT_BUDGET_UNVARIED: [(&str, &str); 1] = [("FN_ENTRIES", "function mode's entry blocks: kura's, the only setting jit32test's function mode runs")];

/// The statics TranslationContext::now reads into its Budget
fn budget_statics(context: &str) -> Vec<String> {
    let Some(at) = context.find("budget: Budget {") else { return Vec::new() };
    let body = &context[at..];
    let body = &body[..body.find('}').unwrap_or(body.len())];
    body.match_indices("super::").filter_map(|(k, _)| {
        let rest = &body[k + "super::".len()..];
        let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
        rest[end..].starts_with(".load(").then(|| rest[..end].to_string())
    }).collect()
}

/// Each budget static set by jit32test from its Mode, or named unvaried (Rule::JitBudgetHarness)
pub(super) fn jit_budget_harness(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    let ctx = Rel::new("crates/koji/src/jit/context.rs");
    let harness = Rel::new("tools/jit32test/src/main.rs");
    let statics = budget_statics(&product_code(tree, &ctx));
    if statics.is_empty() {
        out.push((ctx.clone(), 1, "no `budget: Budget {` reading its statics (super::X.load) in TranslationContext::now".into()));
    }
    let t = product_code(tree, &harness);
    for n in &statics {
        if JIT_BUDGET_UNVARIED.iter().any(|(u, _)| u == n) {
            continue;
        }
        match JIT_BUDGET_HARNESS.iter().find(|(h, _)| h == n) {
            None => out.push((ctx.clone(), 1, format!("budget static `{n}` neither varied by jit32test (JIT_BUDGET_HARNESS) nor named unvaried (JIT_BUDGET_UNVARIED)"))),
            Some((_, field)) => {
                let want = format!("koji::jit::{n}.store({field}");
                if !t.contains(&want) {
                    out.push((harness.clone(), 1, format!("no `{want}`: jit32test translates with `{n}` as it found it, never the other way")));
                }
            }
        }
    }
    out
}

/// What koji's generators must not spell (Rule::JitSimdAccess): SIMD memory opcodes emitted
/// raw (0 load, 7-10 splats, 11 store, 0x5c/0x5d zero-extending loads), and the YMM half swap
pub const JIT_SIMD_RAW: [&str; 11] = [".simd(0)", ".simd(7)", ".simd(8)", ".simd(9)", ".simd(10)", ".simd(11)", ".simd(0x5c)", ".simd(0x5d)", ".simd(92)", ".simd(93)", "swap_hi("];

pub(super) fn jit_simd_access(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for f in tree.rs(&["crates/koji/src/jit/"]) {
        for (k, l) in product_code(tree, &f).lines().enumerate() {
            if let Some(b) = JIT_SIMD_RAW.iter().find(|b| l.contains(*b)) {
                out.push((f.clone(), k.saturating_add(1), format!("`{b}`: a SIMD access not through Code::simd_access (MEMCOUNT misses it), or the YMM halves swapped in the Cpu")));
            }
        }
    }
    out
}

/// What jit32test must say: a module that does not load fails the test (Made, not a refusal),
/// and a run in which no translation loaded fails
pub const JIT_HARNESS_LOADED: [(&str, &str); 2] = [("tools/jit32test/src/main.rs", "fn translate(&mut self, base: usize, eip: u32) -> Result<Made, String>"), ("tools/jit32test/src/main.rs", "if loaded == 0 && count > 0")];

/// koji's test harnesses: each that defines env.table defines env.pages (see Rule::JitHarnessImports)
pub(super) fn jit_harness_imports(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for f in tree.rs(&["tools/", "fuzz/"]) {
        let t = product_code(tree, &f);
        if let Some(k) = t.lines().position(|l| l.contains("\"env\", \"table\"")) {
            if !t.contains("\"env\", \"pages\"") {
                out.push((f.clone(), k.saturating_add(1), "env.table defined, env.pages not: every module of koji imports the page marks and none loads".into()));
            }
        }
    }
    for (file, want) in JIT_HARNESS_LOADED {
        let f = Rel::new(file);
        if !product_code(tree, &f).contains(want) {
            out.push((f, 1, format!("no `{want}`: jit32test could pass without running a translation")));
        }
    }
    out
}

pub(super) fn jit_exit_contract(tree: &Tree) -> Hits {
    let mut out = Hits::new();
    for (file, want) in JIT_EXIT_CONTRACT {
        let f = Rel::new(file);
        if !product_code(tree, &f).contains(want) {
            out.push((f, 1, format!("no `{want}` in the code: an exit whose status flags are not checked to be exact (jit::ExitAt)")));
        }
    }
    for (file, bad) in JIT_EXIT_CONTRACT_NOT {
        let f = Rel::new(file);
        if product_code(tree, &f).contains(bad) {
            out.push((f, 1, format!("`{bad}`: a loop's hoisted checks leave for its own head, which reads what its live-in says (ExitAt::Jump)")));
        }
    }
    for f in tree.rs(&["crates/koji/src/jit/"]) {
        for (k, l) in product_code(tree, &f).lines().enumerate() {
            if rflags_emitted(l) {
                out.push((f.clone(), k.saturating_add(1), "iced's RflagsBits emitted into the code: EFLAGS' bits are crate::flags'".into()));
            }
        }
    }
    out
}
