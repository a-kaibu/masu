//! Rules from the sake ABI / state redesign (review of 6852be1f, 2026-11-22): each table lists
//! what a fixed file must keep saying (the fix) and what it must not say again (the bug), with
//! the bug it stands for. Rust files are read without comments and without their inline test
//! modules (each one, wherever it is: k32.rs has one near its top); other files (Python, shell,
//! JavaScript, workflows) as they are.
use super::{Hits, Rel, Tree};

/// (file, text that must be there / must not be there, what its absence / presence means)
pub type Needle = (&'static str, &'static str, &'static str);

fn body(tree: &Tree, f: &Rel) -> String {
    if f.is_rust() { without_tests(&tree.code(f, true)) } else { tree.text(f) }
}

/// `code` without each `#[cfg(test)] mod name { .. }` (its braces matched; comments are gone
/// already, braces in strings are rare enough in a test module's head not to matter: a module
/// whose braces do not close is cut to the end).
pub(super) fn without_tests(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find("#[cfg(test)]") {
        let (head, tail) = rest.split_at(at);
        out.push_str(head);
        let after = tail.get(12..).unwrap_or_default();
        let t = after.trim_start();
        let inline = (t.starts_with("mod ") || t.starts_with("pub mod ") || t.starts_with("pub(crate) mod ")) && t.find('{').is_some_and(|b| t.find(';').is_none_or(|s| b < s));
        if !inline {
            out.push_str("#[cfg(test)]");
            rest = after;
            continue;
        }
        let skipped = after.len().saturating_sub(t.len());
        let Some(open) = t.find('{') else { return out };
        let mut depth = 0usize;
        let mut end = None;
        for (i, ch) in t.char_indices().skip_while(|&(i, _)| i < open) {
            match ch {
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(i.saturating_add(1));
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { return out };
        rest = after.get(skipped.saturating_add(end)..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// Each `want` missing from its file, each `not` in it.
pub(super) fn needles(tree: &Tree, want: &[Needle], not: &[Needle]) -> Hits {
    let mut out = Hits::new();
    for (file, needle, why) in want {
        let f = Rel::new(file);
        if !body(tree, &f).contains(needle) {
            out.push((f, 1, format!("no `{needle}`: {why}")));
        }
    }
    for (file, needle, why) in not {
        let f = Rel::new(file);
        let text = body(tree, &f);
        if let Some(at) = text.find(needle) {
            let line = text.get(..at).map_or(1, |b| b.lines().count().max(1));
            out.push((f, line, format!("`{needle}`: {why}")));
        }
    }
    out
}

// --- run-plan --------------------------------------------------------------------------------

/// The runners write their plan before running, compare.py and run.py fail on a plan problem.
pub const RUN_PLAN: [Needle; 13] = [
    ("tests/winref/run.sh", "> \"$OUT/.plan/$runner\"", "run.sh writes no plan: a run that ran nothing compares 0 probes and passes"),
    ("tests/winref/run-browser.mjs", "writeFileSync(join(outDir, \".plan\", runner)", "run-browser.mjs writes no plan: a lost shard passes"),
    ("tests/winref/compare.py", "sys.exit(1 if failed or plan_errors else 0)", "compare.py does not fail on a plan problem (0 compared, a planned probe without a result, a lost shard)"),
    ("tests/winref/compare.py", "plan.problems(plans, required, got)", "compare.py does not check the run's plan"),
    ("tests/winetest/run.py", "all_jobs, unplanned = plan(", "run.py does not plan its tests before sharding: a missing test program passes with 0 run"),
    ("tests/winetest/run.py", "if unplanned:", "run.py does not fail on what it could not plan"),
    ("tests/winetest/run.py", "all_jobs, not_adopted, missing = adopt(all_jobs, read_tsv(win_path))", "rostests run on sake without Windows' answers: a ReactOS expectation Windows itself fails would be held against sake"),
    ("tests/winetest/run.py", "unplanned += missing", "a rostests test Windows was never asked about runs on sake as if Windows passed it"),
    ("tests/winetest/run.py", "if not os.path.isfile(win_path):\n            raise FileNotFoundError(win_path)", "rostests without tests/rostests/windows-<arch>.tsv pass with no Windows answer to hold sake to"),
    ("tests/winetest/run.py", "results[k], judged[k] = combine(rs)", "--repeat records a test by one of its runs, not by its worst and judged by its best: a test Windows passes only sometimes is adopted, or one bad Windows run fails CI"),
    (".github/workflows/conformance.yml", "run.py --suite rostests --native --repeat 3", "Windows runs each ReactOS test once in CI: a test Windows passes only sometimes (RegEnumKey, EmptyClipboard) turns windows-<arch>.tsv into a coin toss"),
    (".github/workflows/conformance.yml", "run.py --suite rostests --native", "Windows no longer runs ReactOS's tests in CI: tests/rostests/windows-<arch>.tsv is never checked against Windows"),
    (".github/workflows/winref.yml", "--require-plan browser-0of4", "the final winref comparison does not require each browser shard's plan: a lost shard passes"),
];

/// The two spellings that made a run without results pass.
pub const RUN_PLAN_NOT: [Needle; 4] = [
    ("tests/winref/compare.py", "continue  # (not run: a filtered run.sh)", "a probe without a result skipped as if filtered out: an empty result directory passed"),
    ("tests/winetest/run.py", "print(f\"missing {exe}\", file=sys.stderr)\n            continue", "a missing test program printed and skipped: the run passed with nothing run"),
    ("tests/winetest/run.py", "read_tsv(win_path) if os.path.exists(win_path)", "Windows' answers read only when present: without them every ReactOS test ran on sake unchecked"),
    ("tests/winetest/run.py", "with open(path, \"w\") as f:", "a result file written in the locale's encoding: on Windows (cp1252) the report stopped at the first test message outside it and the --native run failed after its results"),
];

/// The body of `fn name(` in `code` (from its `{` to the matching `}`), None when the file has
/// no such function.
pub(super) fn fn_body<'a>(code: &'a str, name: &str) -> Option<&'a str> {
    let at = code.find(&format!("fn {name}("))?;
    let rest = code.get(at..)?;
    let open = rest.find('{')?;
    let mut depth = 0usize;
    for (i, ch) in rest.char_indices().skip(open) {
        match ch {
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return rest.get(open..=i);
                }
            }
            _ => {}
        }
    }
    None
}

/// (file, function, text its body must hold / must not hold, why)
pub type FnNeedle = (&'static str, &'static str, &'static str, &'static str);

/// Each function of `want` that lacks its text (or is gone), each of `not` that holds it.
pub(super) fn fn_needles(tree: &Tree, want: &[FnNeedle], not: &[FnNeedle]) -> Hits {
    let mut out = Hits::new();
    for (file, name, needle, why) in want {
        let f = Rel::new(file);
        let code = body(tree, &f);
        if !fn_body(&code, name).is_some_and(|b| b.contains(needle)) {
            out.push((f, 1, format!("fn {name} without `{needle}`: {why}")));
        }
    }
    for (file, name, needle, why) in not {
        let f = Rel::new(file);
        let code = body(tree, &f);
        if fn_body(&code, name).is_some_and(|b| b.contains(needle)) {
            out.push((f, 1, format!("fn {name} with `{needle}`: {why}")));
        }
    }
    out
}

// --- message-width ---------------------------------------------------------------------------

const USER: &str = "crates/sake/src/user.rs";

/// The entry points that take a WPARAM/LPARAM or answer an LRESULT/LONG_PTR read and return
/// them whole (abi.rs), the LONG_PTR functions use their own width.
pub const MESSAGE_WIDTH: [FnNeedle; 21] = [
    (USER, "PostMessageA", "c.wparam(2), c.lparam(3)", "PostMessage reads wParam/lParam as 32-bit slots: x64 loses their upper half"),
    (USER, "PostThreadMessageA", "c.wparam(2), l: c.lparam(3)", "PostThreadMessage reads wParam/lParam as 32-bit slots"),
    (USER, "send_message", "c.wparam(2), c.lparam(3)", "SendMessage reads wParam/lParam as 32-bit slots"),
    (USER, "send_message", "c.ret_lresult(", "SendMessage answers the LRESULT's low half only"),
    (USER, "send_notify", "c.wparam(2), c.lparam(3)", "SendNotifyMessage reads wParam/lParam as 32-bit slots"),
    (USER, "SendMessageTimeoutA", "res.put_size(r.bits())", "SendMessageTimeout's DWORD_PTR result written as 32 bits"),
    (USER, "SendDlgItemMessageA", "c.wparam(3), c.lparam(4)", "SendDlgItemMessage reads wParam/lParam as 32-bit slots"),
    (USER, "CallWindowProcA", "abi::call_wndproc(", "CallWindowProc calls with 32-bit arguments"),
    (USER, "CallWindowProcA", "c.ret_lresult(", "CallWindowProc answers the LRESULT's low half only"),
    (USER, "DispatchMessageA", "c.ret_lresult(", "DispatchMessage answers the LRESULT's low half only"),
    (USER, "call_proc", "abi::call_wndproc(", "window procedures called with 32-bit arguments"),
    (USER, "msg_at", "abi::read_slot(", "MSG's wParam/lParam read as 32 bits"),
    (USER, "GetWindowLongPtrA", "LongWidth::Ptr", "GetWindowLongPtr is GetWindowLong (review 6852be1f)"),
    (USER, "GetWindowLongPtrA", "c.ret_long_ptr(", "GetWindowLongPtr answers 32 bits"),
    (USER, "SetWindowLongPtrA", "c.long_ptr(2)", "SetWindowLongPtr takes dwNewLong as 32 bits"),
    (USER, "SetWindowLongPtrA", "LongWidth::Ptr", "SetWindowLongPtr is SetWindowLong (review 6852be1f)"),
    (USER, "GetClassLongPtrA", "LongWidth::Ptr", "GetClassLongPtr is GetClassLong"),
    (USER, "SetClassLongPtrA", "c.long_ptr(2)", "SetClassLongPtr takes the new value as 32 bits"),
    (USER, "SetTimer", "c.wparam(1)", "SetTimer's nIDEvent (UINT_PTR) read as 32 bits"),
    (USER, "EndDialog", "c.long_ptr(1)", "EndDialog's nResult (INT_PTR) read as 32 bits"),
    ("crates/sake/src/abi.rs", "call_wndproc", "Arg::handle(hwnd), Arg::dword(msg), w.into(), l.into()", "window procedures' arguments not at their width"),
];

/// What must not come back: the LONG_PTR functions handing over to the 32-bit ones.
pub const MESSAGE_WIDTH_NOT: [FnNeedle; 4] = [
    (USER, "GetWindowLongPtrA", "GetWindowLongA(c)", "GetWindowLongPtr handed to GetWindowLong: 32 bits on x64"),
    (USER, "SetWindowLongPtrA", "SetWindowLongA(c)", "SetWindowLongPtr handed to SetWindowLong: 32 bits on x64"),
    (USER, "GetClassLongPtrA", "GetClassLongA(c)", "GetClassLongPtr handed to GetClassLong"),
    (USER, "SetClassLongPtrA", "SetClassLongA(c)", "SetClassLongPtr handed to SetClassLong"),
];

/// abi.rs's types are made by kind, never from a bare u32 (no From<u32>), and the cross-process
/// message keeps its parameters whole.
pub const MESSAGE_WIDTH_FILES_NOT: [Needle; 2] = [
    ("crates/sake/src/abi.rs", "impl From<u32> for", "a WPARAM/LPARAM made from a u32 without saying how it widens (dword, int, addr, handle)"),
    ("crates/sake/src/kern.rs", "    pub w: u32,\n    pub l: u32,", "another process's message keeps wParam/lParam as 32 bits"),
];

/// Msg::write puts the MSG down from its layout (not 4-byte offsets by hand)
pub const MESSAGE_WIDTH_FILES: [Needle; 1] = [(USER, "abi::write_slot(at + lay.wparam, self.w.guest_bits(arch), arch);", "MSG's wParam written by hand instead of from abi::msg_layout")];

pub(super) fn message_width(tree: &Tree) -> Hits {
    let mut out = fn_needles(tree, &MESSAGE_WIDTH, &MESSAGE_WIDTH_NOT);
    out.extend(needles(tree, &MESSAGE_WIDTH_FILES, &MESSAGE_WIDTH_FILES_NOT));
    out
}

// --- d2d-layout ------------------------------------------------------------------------------

const D2D: &str = "crates/sake/src/d2d.rs";

/// The HWND render target's swap chain is made from the header layouts, each output in a
/// pointer-sized slot of its own, the call made with typed arguments at their width.
pub const D2D_LAYOUT: [FnNeedle; 5] = [
    (D2D, "hwnd_swap_chain", "D2D1_HWND_RENDER_TARGET_PROPERTIES as P", "the HWND properties read at hand-written offsets"),
    (D2D, "hwnd_swap_chain", "DXGI_SWAP_CHAIN_DESC as D", "the swap chain description written at hand-written offsets"),
    (D2D, "hwnd_swap_chain", "(desc + size, desc + size + ps, desc + size + 2 * ps)", "the three outputs not a pointer apart: on x64 they overlap"),
    (D2D, "hwnd_swap_chain", "crate::abi::call(cpu, create, &args)", "D3D11CreateDeviceAndSwapChain called with 32-bit words"),
    (D2D, "CreateHwndRenderTarget", "hwnd_swap_chain(", "CreateHwndRenderTarget makes its swap chain by hand"),
];

/// The x86 spellings that were the bug.
pub const D2D_LAYOUT_NOT: [FnNeedle; 4] = [
    (D2D, "CreateHwndRenderTarget", "(44, hwnd)", "OutputWindow written at its x86 offset"),
    (D2D, "CreateHwndRenderTarget", "desc + 60", "the outputs placed after a 60-byte (x86) description"),
    (D2D, "CreateHwndRenderTarget", "r32(hp + 4)", "pixelSize read at its x86 offset"),
    (D2D, "hwnd_swap_chain", "r32(hp + 4)", "pixelSize read at its x86 offset"),
];

pub(super) fn d2d_layout(tree: &Tree) -> Hits {
    fn_needles(tree, &D2D_LAYOUT, &D2D_LAYOUT_NOT)
}

// --- handle-key ------------------------------------------------------------------------------

const OBJ: &str = "crates/sake/src/obj.rs";
const K32: &str = "crates/sake/src/k32.rs";

/// The handle table alone answers for a handle; the suspended-start gate is kept by object.
pub const HANDLE_KEY: [Needle; 4] = [
    (OBJ, "pub fn key(&self, h: &u32) -> Option<Key> {\n        self.handles.get(h).copied()\n    }", "Objs::key answers for more than the open handles"),
    (K32, "static GATE: Mutex<Vec<(obj::Key, u32)>>", "the CREATE_SUSPENDED gate kept by a handle number: ResumeThread through a duplicate misses it"),
    (K32, "fn gate_key(h: u32) -> Option<obj::Key> {\n    obj::key_of(h)\n}", "the gate not looked up by the handle's object"),
    ("crates/sake/src/lib.rs", "gate: Option<obj::Key>, kind: Lives) -> u32 {", "a new thread waits for ResumeThread on a handle number"),
];

/// The spellings that were the bug.
pub const HANDLE_KEY_NOT: [Needle; 3] = [
    (OBJ, "self.map.contains_key(&Key(*h)).then_some(Key(*h))", "a closed first handle's number taken as its object's key"),
    (K32, "static GATE: Mutex<Vec<(u32, u32)>>", "the CREATE_SUSPENDED gate kept by a handle number"),
    (K32, "find(|(x, n)| *x == h && *n > 0)", "the gate searched by the handle number passed in"),
];

pub(super) fn handle_key(tree: &Tree) -> Hits {
    needles(tree, &HANDLE_KEY, &HANDLE_KEY_NOT)
}

// --- io-result -------------------------------------------------------------------------------

const FILES: &str = "crates/sake/src/files.rs";

/// The reads and writes of a File say why they moved less; the share check enters its entry.
pub const IO_RESULT: [Needle; 7] = [
    (FILES, "pub fn read_into(&self, dst: u32, n: u32) -> Result<u32, IoFail> {", "read_into answers a count whatever happened: a host error reads as the end of the file"),
    (FILES, "pub fn write_from(&self, src: u32, n: u32) -> Result<u32, IoFail> {", "write_from answers a count whatever happened"),
    (FILES, "let b = crate::mem::try_bytes(src, n).ok_or(IoFail::NoAccess)?;", "write_from writes an empty copy of a buffer that is not guest memory and answers n"),
    (FILES, "pub fn read_at(&self, start: Option<SeekFrom>, keep: bool, buf: &mut [u8]) -> Result<usize, ReadAt> {", "read_at's failure is one Err(()) for the end of the file and a host error"),
    (FILES, "Err(_) => return Err(IoFail::Read),", "a host read error ends the read as if at the end of the file"),
    (FILES, "let entry = share.map(|s| ShareEntry::take(&host, access, s)).transpose()?;", "CreateFile's share check and its entry two locks apart: exclusive opens racing both get in"),
    (K32, "mem::try_bytes(buf, n).ok_or(files::IoFail::NoAccess)", "an overlapped WriteFile from a buffer that is not guest memory writes an empty copy"),
];

/// The spellings that were the bug.
pub const IO_RESULT_NOT: [Needle; 4] = [
    (FILES, "f.write_all(&crate::mem::bytes(src, n)).map_or(0, |()| n)", "a write of an empty copy answered as the count asked"),
    (FILES, "Ok(0) | Err(_) => break,", "a host read error taken as the end of the file"),
    (FILES, "OPEN.locked().push((id, host.to_lowercase(), access_bits(access), s));", "the share entry made after the check, under another lock"),
    (K32, "f.write_at(start, crate::iocp::is_overlapped(h), &mem::bytes(buf, n))", "an overlapped write of an empty copy of a bad buffer"),
];

pub(super) fn io_result(tree: &Tree) -> Hits {
    needles(tree, &IO_RESULT, &IO_RESULT_NOT)
}

// --- lifetimes -------------------------------------------------------------------------------

const PE: &str = "crates/sake/src/pe.rs";

/// What makes a DLL's failed init and a process's end what Windows does.
pub const LIFETIMES: [Needle; 6] = [
    (PE, "pub fn init_dlls(cpu: &mut Cpu) -> Result<(), InitFailed> {", "init_dlls does not say that a DllMain answered FALSE"),
    (PE, "return Err(InitFailed(base));", "a DllMain that answered FALSE is passed over"),
    (PE, "fn init_loaded(cpu: &mut Cpu, b: u32) -> Result<u32, LoadFailure> {", "LoadLibrary cannot fail for a DllMain's FALSE"),
    (PE, "Ok(b) => init_loaded(cpu, b),", "LoadLibrary of a DLL file answers it whatever its DllMain said"),
    (K32, "let last = crate::lives::ends(t.tid);", "a thread's end does not ask whether it was the program's last"),
    ("crates/sake/src/lib.rs", "if let Err(pe::InitFailed(b)) = pe::init_dlls(cpu) {", "the process starts though a DLL it imports answered FALSE"),
];

/// The spellings that were the bug.
pub const LIFETIMES_NOT: [Needle; 3] = [
    (K32, "if t.tid == crate::process::first_tid(crate::pid()) {\n        // the main thread: the process", "the process ends with its first thread, whatever else runs"),
    (PE, "Ok(b) => {\n            init_dlls(cpu);\n            b\n        }", "LoadLibrary answers the DLL whatever its DllMain said"),
    (PE, "Ok(b) => {\n            init_dlls(cpu);\n            Ok(b)\n        }", "LoadLibrary answers the DLL whatever its DllMain said"),
];

pub(super) fn lifetimes(tree: &Tree) -> Hits {
    needles(tree, &LIFETIMES, &LIFETIMES_NOT)
}

// --- gdi-lock-order --------------------------------------------------------------------------

const GDI: &str = "crates/sake/src/gdi.rs";

/// What a draw op's closure must not do with the destination's surface locked: find a window
/// (user32's lock), lock a surface or another lock, read a source DC (which does both).
pub const DRAW_CLOSURE_NOT: [(&str, &str); 5] = [
    ("source(", "reads a source DC: pass it to draw_src, which copies it before locking the destination"),
    ("user::", "calls user32 (its lock) with a surface locked"),
    (".lock()", "takes a lock with a surface locked"),
    (".locked()", "takes a lock with a surface locked"),
    ("target_of(", "finds a window's target (user32's lock) with a surface locked"),
];

/// The span of the argument list of each call of `draw(` / `draw_src(` in `code` (not their
/// definitions): (offset of the call, its arguments).
fn draw_calls(code: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    for name in ["draw(", "draw_src("] {
        let mut from = 0;
        while let Some(k) = code.get(from..).and_then(|r| r.find(name)) {
            let at = from.saturating_add(k);
            from = at.saturating_add(name.len());
            let before = code.get(..at).unwrap_or_default();
            // (a word of its own, not `fn draw(` nor `x.draw(` nor `redraw(`)
            let prev = before.chars().next_back();
            if prev.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') || before.trim_end().ends_with("fn") {
                continue;
            }
            let open = from.saturating_sub(1);
            let mut depth = 0usize;
            let mut end = None;
            for (i, ch) in code.get(open..).unwrap_or_default().char_indices() {
                match ch {
                    '(' => depth = depth.saturating_add(1),
                    ')' => {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            end = Some(open.saturating_add(i));
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if let Some(e) = end {
                out.push((at, code.get(open..e).unwrap_or_default()));
            }
        }
    }
    out
}

/// draw_src is there, the surface re-entry flag the old order needed is gone, and
/// no draw op's closure takes a lock.
pub const GDI_LOCKS: [Needle; 3] = [
    (GDI, "if src == dst && s.0 + s.2 > d.0 && s.0 < d.0 + d.2 && s.1 + s.3 > d.1 && s.1 < d.1 + d.3 {\n        crate::k32::last_error(87);\n        return 0;", "AlphaBlend within one DC with overlapping rectangles blends (Windows: FALSE, ERROR_INVALID_PARAMETER; Wine's test_GdiAlphaBlend)"),
    (GDI, "fn draw_src(dc: u32, src: u32, mut f: impl FnMut(&Objs, &DcState, &mut Canvas, Option<&Source>) -> Rect) -> bool {", "no draw op that is handed its source"),
    (GDI, "let from = if src != 0 && src != dc { source(t, src) } else { None };", "draw_src does not copy the source before it locks the destination"),
];
pub const GDI_LOCKS_NOT: [Needle; 2] = [
    (GDI, "static HELD:", "a surface held while user32 is called (the reentry flag of the old lock order)"),
    ("crates/sake/src/user.rs", "crate::gdi::holds(", "user32 asks whether this thread holds a surface: it never does when it takes user32's lock"),
];

pub(super) fn gdi_lock_order(tree: &Tree) -> Hits {
    let mut out = needles(tree, &GDI_LOCKS, &GDI_LOCKS_NOT);
    let f = Rel::new(GDI);
    let code = body(tree, &f);
    for (at, args) in draw_calls(&code) {
        for (needle, why) in DRAW_CLOSURE_NOT {
            if args.contains(needle) {
                let line = code.get(..at).map_or(1, |b| b.lines().count().max(1));
                out.push((f.clone(), line, format!("a draw op's closure with `{needle}`: {why}")));
            }
        }
    }
    out
}

// --- not-done --------------------------------------------------------------------------------

const API: &str = "crates/sake/src/api.rs";

/// An export sake has no code for answers from the metadata's table as an api::NotDone and sets the
/// last error that names.
pub const NOT_DONE_SIG: [Needle; 1] = [
    (API, "pub fn unimplemented_result(name: &str) -> NotDone {", "the unimplemented result a bare number: 0 answers success"),
];
pub const NOT_DONE: [FnNeedle; 3] = [
    (API, "returns", "include_str!(\"returns.txt\")", "the metadata's table of declared results not read"),
    (API, "unimplemented_result", "returns().get(&(key.as_str(), f))", "the declared result not looked up"),
    (API, "unimplemented_call", "let (r, err) = unimplemented_result(&n).result();", "the unimplemented call does not answer the declared failure or set its last error"),
];

/// The spellings that were the bug.
pub const NOT_DONE_NOT: [Needle; 1] = [
    (API, "pub fn unimplemented_result(name: &str) -> u32 {", "the unimplemented result a bare number: 0 answers success"),
];

pub(super) fn not_done(tree: &Tree) -> Hits {
    let mut out = needles(tree, &NOT_DONE_SIG, &NOT_DONE_NOT);
    out.extend(fn_needles(tree, &NOT_DONE, &[]));
    let f = Rel::new("crates/sake/src/returns.txt");
    if !tree.has(&f) {
        out.push((f, 1, "returns.txt missing: run tools/win32md".into()));
        return out;
    }
    for (i, l) in tree.text(&f).lines().enumerate() {
        let w: Vec<&str> = l.split(' ').collect();
        if !matches!(w.as_slice(), [_, _, "hresult" | "ntstatus" | "win32" | "lasterror"]) {
            out.push((f.clone(), i + 1, format!("`{l}`: not \"dll name hresult|ntstatus|win32|lasterror\" (sake reads it as 0)")));
        }
    }
    out
}

// --- wait-deadline ---------------------------------------------------------------------------

const SNAP: &str = "crates/sake/src/snap.rs";

/// The waits a restored copy calls again take their end from snap::wait_end, and the state
/// keeps it with the call.
pub const WAIT_DEADLINE: [Needle; 6] = [
    (SNAP, "crate::k32::wait_gate_start(k, r[1] as u32, r[2] as u32);", "a restored thread not started yet waits for ResumeThread where it cannot stand still for the next state"),
    (SNAP, "park_as(RESTART, trap as u64, esp as u64, Deadline::word(WAIT_END.get()));", "a state keeps a wait's call but not its end: the restored copy waits the whole time again"),
    (SNAP, "RESUMED_END.set(Deadline::of_word(r[3]));", "a restored copy's wait called again does not take the end the state kept"),
    (SNAP, "let end = resumed.unwrap_or_else(|| Deadline::after(now, ms));", "wait_end does not keep a restored wait's end"),
    (K32, "let end = crate::snap::wait_end(ms);", "Sleep's end made from its timeout again in a restored copy"),
    (OBJ, "let deadline = crate::snap::wait_end(ms);", "an object wait's end made from its timeout again in a restored copy"),
];

/// The spellings that were the bug.
pub const WAIT_DEADLINE_NOT: [Needle; 4] = [
    (SNAP, "crate::k32::wait_gate(", "a restored thread not started yet waits for ResumeThread where it cannot stand still for the next state"),
    (SNAP, "park_as(RESTART, trap as u64, esp as u64, 0);", "a state keeps a wait's call but not its end"),
    (K32, "let end = (ms != u32::MAX).then(|| kome::time::from_now(std::time::Duration::from_millis(u64::from(ms))));", "Sleep's end made from its timeout, again in a restored copy"),
    (OBJ, "let deadline = (ms != 0xffff_ffff).then(|| kome::time::from_now(Duration::from_millis(u64::from(ms))));", "an object wait's end made from its timeout, again in a restored copy"),
];

pub(super) fn wait_deadline(tree: &Tree) -> Hits {
    needles(tree, &WAIT_DEADLINE, &WAIT_DEADLINE_NOT)
}
