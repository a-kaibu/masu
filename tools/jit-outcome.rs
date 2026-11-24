use koji::{Cpu, Exit};
use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum RunError {
    Guest { exit: Exit, pc: u64 },
    Limit,
    Translation(String),
    HostTrap(String),
    Harness(String),
}

impl RunError {
    pub fn guest(exit: Exit, cpu: &Cpu) -> Self {
        Self::Guest { exit, pc: (u64::from(cpu.rip_hi) << 32) | u64::from(cpu.eip) }
    }

    pub fn wasm(error: wasmtime::Error) -> Self {
        if error.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::OutOfFuel) {
            Self::Limit
        } else {
            Self::HostTrap(format!("{error:?}"))
        }
    }
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Guest { exit, pc } => write!(f, "guest exit {exit:?} at {pc:#x}"),
            Self::Limit => f.write_str("execution limit reached"),
            Self::Translation(error) => write!(f, "translation: {error}"),
            Self::HostTrap(error) => write!(f, "host trap: {error}"),
            Self::Harness(error) => f.write_str(error),
        }
    }
}

pub fn compare_outcomes<T>(want: &Result<(), RunError>, got: &Result<T, RunError>, diffs: &mut Vec<String>) -> bool {
    match (want, got) {
        (Ok(()), Ok(_)) => true,
        (Err(want @ RunError::Guest { .. }), Err(got @ RunError::Guest { .. })) if want == got => true,
        _ => {
            let want = want.as_ref().err().map_or_else(|| "completed".into(), ToString::to_string);
            let got = got.as_ref().err().map_or_else(|| "completed".into(), ToString::to_string);
            diffs.push(format!("runs: interp {want}, jit {got}"));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_matching_completions_and_guest_exits_have_comparable_state() {
        let guest = |exit, pc| Err::<(), _>(RunError::Guest { exit, pc });
        let want = guest(Exit::DivideError, 0x1000);
        let mut diffs = Vec::new();
        assert!(compare_outcomes(&want, &guest(Exit::DivideError, 0x1000), &mut diffs));
        assert!(diffs.is_empty());

        for invalid in [
            guest(Exit::InvalidOpcode, 0x1000),
            guest(Exit::DivideError, 0x1_0000_1000),
            Err(RunError::Translation("module refused".into())),
            Err(RunError::HostTrap("unreachable".into())),
            Err(RunError::Limit),
            Err(RunError::Harness("unexpected status".into())),
            Ok(()),
        ] {
            let mut diffs = Vec::new();
            assert!(!compare_outcomes(&want, &invalid, &mut diffs), "{invalid:?}");
            assert_eq!(diffs.len(), 1);
        }
        assert!(!compare_outcomes(&Err(RunError::Limit), &Err::<(), _>(RunError::Limit), &mut diffs));
    }
}
