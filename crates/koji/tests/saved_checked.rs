//! A translation made with the page checks on (PAGE_NOACCESS / PAGE_GUARD marked: kura's
//! fault_pages) is saved and used in a later process with the checks on: it reads the page
//! marks through the "pages" global it imports, given at instantiation, not through an address
//! of the run that made it; it says it was made with the checks (saved::Entry::pages, VNJ3),
//! and only such ones are taken once the checks are on (review of 6852be1: a game with stack
//! guard pages never used saved code). PAGE_CHECK is a static: this test is alone in its binary.

use koji::jit::{self, saved, wasm, Pages};
use proptest::prelude::*;
use std::sync::atomic::Ordering;

/// Held while a test sets PAGE_CHECK (the tests of this binary run at once).
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `bytes` then RET at 0x100000, translated as a region: the module's bytes and the region.
fn translate(bytes: &[u8]) -> Option<(Vec<u8>, jit::Region)> {
    let mut v = bytes.to_vec();
    v.push(0xc3);
    let mut m = wasm::Module::default();
    let r = jit::translate_region(&mut m, &mut koji::Rom::<0x10_0000>(v), &[0x10_0000], 16, |_| true)?;
    Some((m.finish(), r))
}

/// What the module imports as globals ((module, name, mutable)), and how many global.get of
/// global 0 its code has; Err if it is not valid wasm.
fn globals(b: &[u8]) -> Result<(Vec<(String, String, bool)>, usize), String> {
    let mut f = wasmparser::WasmFeatures::default();
    f.insert(wasmparser::WasmFeatures::THREADS | wasmparser::WasmFeatures::TAIL_CALL);
    wasmparser::Validator::new_with_features(f).validate_all(b).map_err(|e| e.to_string())?;
    let (mut imports, mut gets) = (Vec::new(), 0);
    for p in wasmparser::Parser::new(0).parse_all(b) {
        match p.map_err(|e| e.to_string())? {
            wasmparser::Payload::ImportSection(s) => {
                for i in s.into_imports() {
                    let i = i.map_err(|e| e.to_string())?;
                    if let wasmparser::TypeRef::Global(g) = i.ty {
                        assert_eq!(g.content_type, wasmparser::ValType::I32);
                        imports.push((i.module.to_string(), i.name.to_string(), g.mutable));
                    }
                }
            }
            wasmparser::Payload::CodeSectionEntry(body) => {
                let mut r = body.get_operators_reader().map_err(|e| e.to_string())?;
                while !r.eof() {
                    if let wasmparser::Operator::GlobalGet { global_index: 0 } = r.read().map_err(|e| e.to_string())? {
                        gets += 1;
                    }
                }
            }
            _ => {}
        }
    }
    Ok((imports, gets))
}

/// The entry kura saves for a region (save(): roots, starts, code, its text, the pages).
fn entry(r: &jit::Region, root: u32) -> saved::Entry {
    saved::Entry { roots: vec![root], starts: r.starts.clone(), code: r.code.clone(), text: r.text.clone().unwrap_or_default(), build: 0, sites: r.sites.clone(), pages: r.pages }
}

/// Loads and stores through ECX/EDX, each of the widths: what the checks guard (a POP to
/// memory is the interpreter's: it leaves the block).
const ACCESSES: &[&[u8]] = &[
    &[0x8b, 0x01],             // mov eax, [ecx]
    &[0x89, 0x02],             // mov [edx], eax
    &[0x0f, 0xb6, 0x01],       // movzx eax, byte [ecx]
    &[0x66, 0x89, 0x02],       // mov [edx], ax
    &[0xff, 0x31],             // push dword [ecx]
    &[0x01, 0x01],             // add [ecx], eax
    &[0xf3, 0x0f, 0x7e, 0x01], // movq xmm0, [ecx]
];

#[test]
fn a_checked_translation_is_saved_and_used_with_the_checks_on() {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    // (a run without the checks: its translations say so, and read no page marks)
    jit::PAGE_CHECK.store(false, Ordering::SeqCst);
    let code: Vec<u8> = ACCESSES.concat();
    let (plain, pr) = translate(&code).expect("translated");
    assert_eq!(pr.pages, Pages::Unchecked);
    let (imports, gets) = globals(&plain).expect("valid");
    // (every module imports the global, whether it reads it or not: a bundle merges them)
    assert_eq!(imports, [("env".to_string(), "pages".to_string(), false)]);
    assert_eq!(gets, 0, "an unchecked translation reads the page marks");

    // (the first PAGE_NOACCESS: the checks on; the translation made now is saved)
    jit::PAGE_CHECK.store(true, Ordering::SeqCst);
    let (checked, cr) = translate(&code).expect("translated");
    assert_eq!(cr.pages, Pages::Checked);
    let (imports, gets) = globals(&checked).expect("valid");
    assert_eq!(imports, [("env".to_string(), "pages".to_string(), false)]);
    // (two page bytes per access: its first and its last)
    assert_eq!(gets, 2 * ACCESSES.len(), "{gets} reads of the page marks for {} accesses", ACCESSES.len());
    // (the same code made again is the same module: nothing of this run's addresses in it)
    assert_eq!(translate(&code).expect("translated").0, checked);

    let idx = saved::encode(&[entry(&pr, 0x10_0000), entry(&cr, 0x10_0000)]);
    let merged = wasm::merge(&[&plain[..], &checked[..]], &saved::func_names(&saved::decode(&idx).expect("index"))).expect("merged");
    let (imports, _) = globals(&merged).expect("the bundle is valid");
    assert_eq!(imports, [("env".to_string(), "pages".to_string(), false)]);

    // (the second launch: the checks come on again with the game's guard pages; what reads
    // back of the index is what may run: the checked translation only)
    let back = saved::decode(&idx).expect("index");
    assert_eq!(back.iter().map(|e| e.pages).collect::<Vec<_>>(), [Pages::Unchecked, Pages::Checked]);
    let runs: Vec<bool> = back.iter().map(|e| e.pages.runs_now()).collect();
    assert_eq!(runs, [false, true]);
    // (and before the checks come on, both may: a checked one finds no page marked)
    jit::PAGE_CHECK.store(false, Ordering::SeqCst);
    assert!(back.iter().all(|e| e.pages.runs_now()));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]
    /// Any code: translated with the checks on or off, the module imports only the "pages"
    /// global, reads it only when made with the checks, and says so in the index it is saved
    /// with (a translation that reads page marks never says Unchecked, nor one that reads
    /// none Checked unless it has no access).
    #[test]
    fn what_a_translation_says_is_how_it_was_made(bytes in prop::collection::vec(any::<u8>(), 1..40), on: bool) {
        let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
        jit::PAGE_CHECK.store(on, Ordering::SeqCst);
        let Some((b, r)) = translate(&bytes) else { return Ok(()) };
        prop_assert_eq!(r.pages, if on { Pages::Checked } else { Pages::Unchecked });
        let (imports, gets) = globals(&b).map_err(TestCaseError::fail)?;
        prop_assert_eq!(imports, vec![("env".to_string(), "pages".to_string(), false)]);
        if !on {
            prop_assert_eq!(gets, 0);
        }
        // (code read past the image has no text: kura shares and saves no such region)
        let e = entry(&r, 0x10_0000);
        if r.text.is_none() || !e.consistent() { return Ok(()) }
        let back = saved::decode(&saved::encode(&[e])).expect("index");
        prop_assert_eq!(back[0].pages, r.pages);
        prop_assert!(back[0].pages.runs_now());
    }
}
