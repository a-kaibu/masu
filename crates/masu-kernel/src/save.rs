use h5i_app_core::{Kernel, TenantId};

pub struct Save;

pub const SAVE_REVISIONS: i64 = 8;

pub fn oldest_retained_revision(current: i64) -> i64 {
    current.saturating_sub(SAVE_REVISIONS - 1).max(1)
}

pub fn revision_expired(current: i64, revision: i64) -> bool {
    revision <= current.saturating_sub(SAVE_REVISIONS)
}

pub fn older_than_grace(written_ms: i64, now_ms: i64, grace_ms: u128) -> bool {
    let age_ms = i128::from(now_ms).saturating_sub(i128::from(written_ms));
    u128::try_from(age_ms).is_ok_and(|age| age > grace_ms)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    StaleRevision,
    RevisionExhausted,
}

/// Given the persisted revision and the client's expected revision, decide
/// which revision may be committed. Zero denotes a save that has never been written.
pub fn transition(current: i64, expected: i64) -> Result<i64, Refusal> {
    if current != expected {
        Err(Refusal::StaleRevision)
    } else if current < 0 {
        Err(Refusal::RevisionExhausted)
    } else {
        current.checked_add(1).ok_or(Refusal::RevisionExhausted)
    }
}

impl Kernel for Save {
    type Principal = ();
    type Snapshot = i64;
    type Command = i64;
    type WriteSet = i64;
    type Reply = ();
    type Error = Refusal;

    fn tenant(_: &Self::Principal) -> TenantId {
        TenantId(0)
    }

    fn transition(
        _: &Self::Principal,
        current: &i64,
        expected: &i64,
    ) -> Result<(i64, ()), Refusal> {
        match transition(*current, *expected) {
            Ok(next) => Ok((next, ())),
            Err(reason) => Err(reason),
        }
    }

    fn apply(_: &i64, new_revision: &i64) -> i64 {
        *new_revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committing_a_save_advances_only_the_current_revision() {
        let snap = 4;
        assert_eq!(
            Save::transition(&(), &snap, &3),
            Err(Refusal::StaleRevision)
        );
        assert_eq!(
            Save::transition(&(), &snap, &5),
            Err(Refusal::StaleRevision)
        );
        let (write, ()) = Save::transition(&(), &snap, &snap).unwrap();
        assert_eq!(Save::apply(&snap, &write), 5);
        assert_eq!(transition(0, 0), Ok(1));
    }

    #[test]
    fn invalid_revisions_cannot_wrap_or_restart_the_counter() {
        assert_eq!(transition(i64::MAX - 1, i64::MAX - 1), Ok(i64::MAX));
        assert_eq!(
            transition(i64::MAX, i64::MAX),
            Err(Refusal::RevisionExhausted)
        );
        assert_eq!(transition(-1, -1), Err(Refusal::RevisionExhausted));
    }

    #[test]
    fn retention_preserves_the_last_eight_revisions_and_the_grace_boundary() {
        assert_eq!(oldest_retained_revision(0), 1);
        assert_eq!(oldest_retained_revision(20), 13);
        assert!(revision_expired(20, 12));
        assert!(!revision_expired(20, 13));
        assert!(!older_than_grace(90, 100, 10));
        assert!(older_than_grace(89, 100, 10));
        assert!(!older_than_grace(101, 100, 0));
        assert!(!older_than_grace(i64::MIN, i64::MAX, u128::MAX));
    }
}
