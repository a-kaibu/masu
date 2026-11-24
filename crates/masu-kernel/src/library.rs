use alloc::vec::Vec;
use h5i_app_core::{Kernel, TenantId};

pub const MAX_NOTE_BYTES: usize = 100_000;
pub const MAX_METADATA_URL_BYTES: usize = 2_048;
pub const METADATA_STALE_MS: i64 = 10 * 60_000;

/// Library API identifiers are single, portable path components.
pub fn valid_game(id: &[u8]) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id.first() != Some(&b'.')
        && id
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

pub fn valid_note(bytes: usize) -> bool {
    bytes <= MAX_NOTE_BYTES
}

pub struct Profile;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileRecord {
    pub favorite: bool,
    pub note: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NoteTooLong;

impl Kernel for Profile {
    type Principal = ();
    type Snapshot = ProfileRecord;
    type Command = ProfileRecord;
    type WriteSet = ProfileRecord;
    type Reply = ();
    type Error = NoteTooLong;

    fn tenant(_: &Self::Principal) -> TenantId {
        TenantId(0)
    }

    fn transition(
        _: &Self::Principal,
        _: &ProfileRecord,
        cmd: &ProfileRecord,
    ) -> Result<(ProfileRecord, ()), NoteTooLong> {
        if valid_note(cmd.note.len()) {
            Ok((cmd.clone(), ()))
        } else {
            Err(NoteTooLong)
        }
    }

    fn apply(_: &ProfileRecord, replacement: &ProfileRecord) -> ProfileRecord {
        replacement.clone()
    }
}

pub fn valid_metadata_url(bytes: usize) -> bool {
    bytes <= MAX_METADATA_URL_BYTES
}

pub fn metadata_pending_expired(updated_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(updated_ms) > METADATA_STALE_MS
}

pub struct Metadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataCommand {
    Begin { job: u128, url_bytes: usize },
    Finish { job: u128 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataRefusal {
    UrlTooLong,
    SupersededJob,
}

pub fn metadata_transition(
    active_job: Option<u128>,
    cmd: MetadataCommand,
) -> Result<u128, MetadataRefusal> {
    match cmd {
        MetadataCommand::Begin { job, url_bytes } if valid_metadata_url(url_bytes) => Ok(job),
        MetadataCommand::Begin { .. } => Err(MetadataRefusal::UrlTooLong),
        MetadataCommand::Finish { job } if active_job == Some(job) => Ok(job),
        MetadataCommand::Finish { .. } => Err(MetadataRefusal::SupersededJob),
    }
}

impl Kernel for Metadata {
    type Principal = ();
    type Snapshot = Option<u128>;
    type Command = MetadataCommand;
    type WriteSet = u128;
    type Reply = ();
    type Error = MetadataRefusal;

    fn tenant(_: &Self::Principal) -> TenantId {
        TenantId(0)
    }

    fn transition(
        _: &Self::Principal,
        active_job: &Option<u128>,
        cmd: &MetadataCommand,
    ) -> Result<(u128, ()), MetadataRefusal> {
        match metadata_transition(*active_job, *cmd) {
            Ok(job) => Ok((job, ())),
            Err(reason) => Err(reason),
        }
    }

    fn apply(_: &Option<u128>, job: &u128) -> Option<u128> {
        Some(*job)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_replacements_are_subject_to_the_note_limit() {
        let current = ProfileRecord::default();
        let accepted = ProfileRecord {
            favorite: true,
            note: alloc::vec![b'n'; MAX_NOTE_BYTES],
        };
        let (write, ()) = Profile::transition(&(), &current, &accepted).unwrap();
        assert_eq!(Profile::apply(&current, &write), accepted);
        let rejected = ProfileRecord {
            favorite: false,
            note: alloc::vec![b'n'; MAX_NOTE_BYTES + 1],
        };
        assert_eq!(
            Profile::transition(&(), &current, &rejected),
            Err(NoteTooLong)
        );
    }

    #[test]
    fn library_limits_match_public_identifiers_and_byte_limits() {
        assert!(valid_game(b"A-z_0.1"));
        assert!(!valid_game(b".hidden"));
        assert!(!valid_game(b"a/b"));
        assert!(!valid_game(b""));
        assert!(!valid_game("ゲーム".as_bytes()));
        assert!(valid_game(&[b'a'; 200]));
        assert!(!valid_game(&[b'a'; 201]));
        assert!(valid_note(100_000));
        assert!(!valid_note(100_001));
        assert!(valid_metadata_url(2_048));
        assert!(!valid_metadata_url(2_049));
        assert!(!metadata_pending_expired(40, 600_040));
        assert!(metadata_pending_expired(40, 600_041));
        assert!(metadata_pending_expired(i64::MIN, i64::MAX));
    }

    #[test]
    fn a_new_metadata_job_rejects_the_previous_jobs_completion() {
        let (job, ()) = Metadata::transition(
            &(),
            &Some(1),
            &MetadataCommand::Begin {
                job: 2,
                url_bytes: 6,
            },
        )
        .unwrap();
        let state = Metadata::apply(&Some(1), &job);
        assert_eq!(
            Metadata::transition(&(), &state, &MetadataCommand::Finish { job: 1 }),
            Err(MetadataRefusal::SupersededJob)
        );
        assert_eq!(
            Metadata::transition(&(), &state, &MetadataCommand::Finish { job: 2 }),
            Ok((2, ()))
        );
        assert_eq!(
            Metadata::transition(&(), &None, &MetadataCommand::Finish { job: 2 }),
            Err(MetadataRefusal::SupersededJob)
        );
    }
}
