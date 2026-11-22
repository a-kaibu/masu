//! GetFileInformationByHandleEx's class table (kernelbase's, Wine kernel32:file
//! test_GetFileInformationByHandleEx)
use super::{info_query, InfoQuery};
use proptest::prelude::*;

/// The classes only SetFileInformationByHandle takes, and FileIoPriorityHintInfo (12),
/// FileDispositionInfoEx (21), FileRenameInfoEx (22)
const SET_ONLY: [u32; 7] = [3, 4, 5, 6, 12, 21, 22];

proptest! {
    /// a class is asked of ntdll exactly when it is a known query class (else
    /// ERROR_INVALID_PARAMETER, test's 0xdeadbeef)
    #[test]
    fn only_the_query_classes_go_to_ntdll(class in prop_oneof![0u32..32, any::<u32>()]) {
        prop_assert_eq!(info_query(class).is_some(), class <= 24 && !SET_ONLY.contains(&class));
    }
}

/// A directory class's least size holds one entry's fixed part as ntdll writes it, and the
/// Restart twin of each asks the same with the scan restarted
#[test]
fn directory_classes_fit_ntdlls_entries() {
    for class in 0..=24 {
        let Some((q, least)) = info_query(class) else { continue };
        if let InfoQuery::Dir(k, restart) = q {
            let (fixed, ..) = crate::nt::dir_class_layout(k).expect("ntdll answers the class");
            assert!(least >= fixed, "class {class}: {least} < {fixed}");
            if !restart {
                assert_eq!(info_query(class + 1), Some((InfoQuery::Dir(k, true), least)), "class {class}'s restart twin");
            }
        }
    }
    // (the test's checks: FileIdBothDirectoryInfo needs 112 bytes, ERROR_BAD_LENGTH below)
    assert_eq!(info_query(10), Some((InfoQuery::Dir(37, false), 112)));
    assert_eq!(info_query(2), Some((InfoQuery::File(9), 8)));
}
