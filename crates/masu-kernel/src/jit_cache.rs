use alloc::string::{String, ToString};

pub const JIT_NAME_MAX: usize = 200;

/// The name of a trace, chunk list, or compiled region upload.
#[derive(Debug, PartialEq, Eq)]
pub enum JitName {
    Trace(String),
    Chunks(String),
    Regions { prog: String, ver: String },
}

impl JitName {
    pub fn parse(name: &str) -> Option<Self> {
        if !crate::library::valid_game(name.as_bytes()) {
            return None;
        }
        if name.starts_with("trace.") {
            return Some(Self::Trace(name.to_string()));
        }
        if name.ends_with(".chunks") {
            return Some(Self::Chunks(name.to_string()));
        }
        let (prog, ver) = name.rsplit_once('-')?;
        regions_name(name).then(|| Self::Regions {
            prog: prog.to_string(),
            ver: ver.to_string(),
        })
    }
}

/// Accept names of the form `<program>-<build>`. Build names contain only
/// letters and digits so uploads cannot alias the files maintained by the server.
pub fn regions_name(name: &str) -> bool {
    let Some((prog, ver)) = name.rsplit_once('-') else {
        return false;
    };
    !prog.is_empty()
        && !ver.is_empty()
        && ver.bytes().all(|b| b.is_ascii_alphanumeric())
        && name.len() <= JIT_NAME_MAX
        && !name.starts_with('.')
}

pub fn upload_fits(modules: u64, on_disk: u64, fresh: u64, max: (u64, u64)) -> bool {
    modules.checked_add(fresh).is_some_and(|size| size <= max.0)
        && on_disk.checked_add(fresh).is_some_and(|size| size <= max.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_upload_checks_all_limits_and_rejects_overflow() {
        assert!(upload_fits(4, 9, 1, (5, 10)));
        assert!(!upload_fits(4, 8, 2, (5, 10)));
        assert!(!upload_fits(3, 9, 2, (5, 10)));
        assert!(!upload_fits(u64::MAX, 0, 1, (u64::MAX, u64::MAX)));
        assert!(!upload_fits(0, u64::MAX, 1, (u64::MAX, u64::MAX)));
    }
}
