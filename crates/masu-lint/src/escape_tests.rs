//! The vocabulary of escape-comment, word by word: each escape alone is found, each Windows
//! answer with its evidence lets through.
use super::*;

#[test]
fn each_escape_word_is_found_alone() {
    let words = [
        concat!("TO", "DO: later"), concat!("FIX", "ME"), concat!("XX", "X odd"), concat!("HA", "CK around"), "zero for now", "for the moment",
        "a simplified model", "the simplification", "not implemented", "not yet implemented", "unimplemented here",
        "not ported", "best effort", "best-effort install", "good enough", "close enough", "stubbed out",
        "a logged stub", "a stub returning 0", "placeholder value", "a workaround", "a fake success",
        "only PNG's chunks are answered so far", "とりあえず", "簡略化", "未実装", "未対応", "暫定", "ひとまず", "手抜き", "近似", "仮実装",
        "dropped: unsupported", "not supported", "an approximation of replacing it", "(flags ignored)", "offsets ignored", "attributes ignored", "taken, and ignored", "accepted, no effect",
    ];
    for w in words {
        assert!(is_escape(w, w), "{w:?} is not found");
    }
}

#[test]
fn windows_answers_with_evidence_pass() {
    for w in [
        "binary labels: not supported, as on Windows",
        "DDERR_UNSUPPORTED (winref ddraw)",
        "RCP/RSQRT: approximations, each vendor's own (x86)",
        "the flags are ignored (MEASURED, golden gdi-32)",
        "D2D1_GEOMETRY_SIMPLIFICATION_OPTION_LINES",
        "Simplify(options, transform, tolerance, sink)",
        "ignored, as in Windows",
        "the ASCII case ignored",
        "cargo test -- --ignored --nocapture",
    ] {
        assert!(!is_escape(w, w), "{w:?} is flagged");
    }
}

#[test]
fn evidence_elsewhere_in_the_block_counts() {
    assert!(!is_escape("not supported", "Windows answers FALSE: not supported"));
    assert!(is_escape(concat!("TO", "DO"), "Windows answers FALSE"));
}

#[test]
fn comments_of_each_kind() {
    assert_eq!(comments("a.rs", "let s = \"// no\"; // yes\n").first().map(String::as_str), Some("// yes"));
    assert_eq!(comments("a.py", "x = '# no'  # yes\n").first().map(String::as_str), Some("# yes"));
    assert_eq!(comments("a.sh", "#!/bin/sh\necho ${#x} # c\n"), vec!["".to_owned(), "# c".to_owned(), "".to_owned()]);
    assert!(comments("a.md", "TODO").is_empty());
}

#[test]
fn a_log_line_naming_the_gap_is_found() {
    let t = Tree::made_up(&[("crates/kura/src/d.rs", "fn f() { eprintln!(\"d3d9: {} not implemented\", m); }\n")]);
    assert_eq!(escape_comment(&t).len(), 1);
    // (FormatMessage's own text)
    let t = Tree::made_up(&[("crates/sake/src/f.rs", "const M: &str = \"This function is not supported on this system.\\r\\n\";\n")]);
    assert!(escape_comment(&t).is_empty());
}

#[test]
fn a_constant_handler_needs_its_evidence() {
    let t = Tree::made_up(&[("crates/sake/src/u.rs", "/// carets: accepted\nfn CreateCaret(_c: &mut Ctx) -> u32 {\n    1\n}\n")]);
    assert_eq!(constant_handler(&t).len(), 1);
    let t = Tree::made_up(&[("crates/sake/src/u.rs", "/// TRUE, as Windows answers (winref caret)\nfn CreateCaret(_c: &mut Ctx) -> u32 {\n    1\n}\n")]);
    assert!(constant_handler(&t).is_empty());
}
