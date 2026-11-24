use alloc::vec::Vec;
use h5i_app_core::{Kernel, TenantId};

pub struct Session;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub game: Vec<u8>,
    pub started_ms: i64,
    pub elapsed_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub record: Record,
    /// A live heartbeat has a clock bound. A portable import supplies
    /// historical records already checked by the package decoder.
    pub now_ms: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    InvalidTime,
    IdentityConflict,
}

pub fn transition(previous: &Option<Record>, cmd: &Command) -> Result<Record, Refusal> {
    const SKEW_MS: i128 = 60_000;
    let incoming = &cmd.record;
    if let Some(now) = cmd.now_ms {
        let age = i128::from(now).saturating_sub(i128::from(incoming.started_ms));
        if incoming.started_ms <= 0
            || age < -SKEW_MS
            || incoming.elapsed_ms < 0
            || i128::from(incoming.elapsed_ms) > age.max(0).saturating_add(SKEW_MS)
        {
            return Err(Refusal::InvalidTime);
        }
    }
    let mut elapsed_ms = incoming.elapsed_ms;
    if let Some(row) = previous {
        if row.game != incoming.game || row.started_ms != incoming.started_ms {
            return Err(Refusal::IdentityConflict);
        }
        elapsed_ms = elapsed_ms.max(row.elapsed_ms);
    }
    Ok(Record {
        elapsed_ms,
        ..incoming.clone()
    })
}

impl Kernel for Session {
    type Principal = ();
    type Snapshot = Option<Record>;
    type Command = Command;
    type WriteSet = Record;
    type Reply = ();
    type Error = Refusal;

    fn tenant(_: &Self::Principal) -> TenantId {
        TenantId(0)
    }

    fn transition(
        _: &Self::Principal,
        previous: &Option<Record>,
        cmd: &Command,
    ) -> Result<(Record, ()), Refusal> {
        match transition(previous, cmd) {
            Ok(record) => Ok((record, ())),
            Err(reason) => Err(reason),
        }
    }

    fn apply(_: &Option<Record>, record: &Record) -> Option<Record> {
        Some(record.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reordered_heartbeats_cannot_shorten_or_change_a_session() {
        let record = Record {
            game: b"game".to_vec(),
            started_ms: 100,
            elapsed_ms: 25,
        };
        let mut cmd = Command {
            record,
            now_ms: Some(150),
        };
        let saved = transition(&None, &cmd).unwrap();
        let state = Some(saved.clone());
        cmd.record.elapsed_ms = 16;
        assert_eq!(transition(&state, &cmd), Ok(saved));
        cmd.record.started_ms = 101;
        assert_eq!(transition(&state, &cmd), Err(Refusal::IdentityConflict));
        cmd.record.started_ms = 100;
        cmd.record.game = b"other".to_vec();
        assert_eq!(transition(&state, &cmd), Err(Refusal::IdentityConflict));
    }

    #[test]
    fn live_time_bounds_accept_the_endpoints_without_overflow() {
        let record = Record {
            game: b"game".to_vec(),
            started_ms: i64::MAX,
            elapsed_ms: 60_000,
        };
        let mut cmd = Command {
            record,
            now_ms: Some(i64::MAX - 60_000),
        };
        assert!(transition(&None, &cmd).is_ok());
        cmd.record.elapsed_ms = 60_001;
        assert_eq!(transition(&None, &cmd), Err(Refusal::InvalidTime));
        cmd.record.elapsed_ms = 60_000;
        cmd.now_ms = Some(i64::MIN);
        assert_eq!(transition(&None, &cmd), Err(Refusal::InvalidTime));
        cmd.now_ms = Some(i64::MAX);
        cmd.record.elapsed_ms = -1;
        assert_eq!(transition(&None, &cmd), Err(Refusal::InvalidTime));
    }
}
