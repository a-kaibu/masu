//! LdrGetDllPath / RtlGetSearchPath / RtlGetExePath orders (sake-re/ntdll/dll-path.md),
//! with Wine kernel32:path as the Windows reference
use super::*;
use proptest::prelude::*;

fn parts<'a>(module: Option<&'a str>, dd: Option<&'a str>, user: &'a [String]) -> Parts<'a> {
    Parts { app: "C:\\game", module, dll_directory: dd, user_dirs: user, path_var: "C:\\bin" }
}

#[test]
fn safe_search_puts_the_current_folder_after_windows() {
    assert_eq!(compute(&SAFE, &parts(None, None, &[])), "C:\\game;C:\\windows\\system32;C:\\windows\\system;C:\\windows;.;C:\\bin");
    assert_eq!(compute(&UNSAFE, &parts(None, None, &[])), "C:\\game;.;C:\\windows\\system32;C:\\windows\\system;C:\\windows;C:\\bin");
    assert_eq!(compute(&EXE_NO_CUR, &parts(None, None, &[])), "C:\\game;C:\\windows\\system32;C:\\windows\\system;C:\\windows;C:\\bin");
}

#[test]
fn a_dll_directory_takes_the_place_of_the_current_folder() {
    assert_eq!(compute(&WITH_DLL_DIRECTORY, &parts(None, Some("D:\\x"), &[])), "C:\\game;D:\\x;C:\\windows\\system32;C:\\windows\\system;C:\\windows;C:\\bin");
}

#[test]
fn empty_parts_leave_no_separator() {
    let p = Parts { app: "C:\\game", module: None, dll_directory: None, user_dirs: &[], path_var: "" };
    assert_eq!(compute(&SAFE, &p), "C:\\game;C:\\windows\\system32;C:\\windows\\system;C:\\windows;.");
}

#[test]
fn altered_search_takes_the_module_folder_up_to_its_last_backslash() {
    for (m, want) in [("c:\\temp\\foo", Some("c:\\temp")), (".\\foo\\foobar", Some(".\\foo")), ("temp/foo", None), ("foo", None), ("\\\\?\\c:\\test.dll", Some("\\\\?\\c:"))] {
        let p = plan(Some(m), ALTERED, 0, false, true).ok();
        assert_eq!(p.as_ref().map(|p| p.elems.clone()), Some(SAFE.to_vec()), "{m}");
        assert_eq!(p.and_then(|p| p.module).and_then(module_dir), want, "{m}");
    }
}

#[test]
fn dll_load_dir_needs_an_absolute_module() {
    assert_eq!(plan(Some("foo"), DLL_LOAD_DIR, 0, false, true), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(plan(Some("foo"), ALTERED | DLL_LOAD_DIR, 0, false, true), Err(STATUS_INVALID_PARAMETER));
    for m in ["C:\\\\foo", "\\\\?\\C:\\foo", "\\\\srv\\share\\x"] {
        assert_eq!(plan(Some(m), DLL_LOAD_DIR, 0, false, true).map(|p| p.elems), Ok(vec![ModuleOrApp, Packages]), "{m}");
    }
}

#[test]
fn default_directories_stand_in_for_no_flags() {
    assert_eq!(plan(None, 0, SYSTEM32, false, true).map(|p| p.elems), Ok(vec![System32, Packages]));
    assert_eq!(plan(None, DEFAULT_DIRS, 0, true, true).map(|p| p.elems), Ok(vec![App, UserDirs, DllDirectory, System32, Packages]));
    assert_eq!(plan(None, 0, 0, true, true).map(|p| p.elems), Ok(WITH_DLL_DIRECTORY.to_vec()));
}

#[test]
fn set_default_refuses_dll_load_dir_and_unknown_bits() {
    assert_eq!(set_default(0), STATUS_INVALID_PARAMETER);
    assert_eq!(set_default(DLL_LOAD_DIR | SYSTEM32), STATUS_INVALID_PARAMETER);
    assert_eq!(set_default(0x8000), STATUS_INVALID_PARAMETER);
}

proptest! {
    /// any flags: either refused or a list of folders ending in the (empty) packages element,
    /// with no element twice
    #[test]
    fn plans_are_sane(flags in any::<u32>(), dd in any::<bool>(), m in prop::sample::select(vec!["C:\\a\\b.dll", "b.dll", ".\\x\\y"])) {
        if let Ok(p) = plan(Some(m), flags, 0, dd, true) {
            prop_assert_eq!(p.elems.last(), Some(&Packages));
            for (i, e) in p.elems.iter().enumerate() {
                prop_assert!(!p.elems.get(i + 1..).unwrap_or(&[]).contains(e));
            }
        }
    }

    /// the text never starts or ends with ';' and has no empty element
    #[test]
    fn computed_paths_have_no_empty_elements(app in "[A-Z]:\\\\[a-z]{0,4}", pv in "([A-Z]:\\\\[a-z]{1,3})?", dd in proptest::option::of("[A-Z]:\\\\[a-z]{1,3}")) {
        let user = vec!["C:\\u1".to_string(), "C:\\u2".to_string()];
        let p = Parts { app: &app, module: None, dll_directory: dd.as_deref(), user_dirs: &user, path_var: &pv };
        for elems in [SAFE.to_vec(), UNSAFE.to_vec(), WITH_DLL_DIRECTORY.to_vec(), with_options(0x7f00, true)] {
            let s = compute(&elems, &p);
            prop_assert!(!s.starts_with(';') && !s.ends_with(';') && !s.contains(";;"), "{}", s);
        }
    }
}
