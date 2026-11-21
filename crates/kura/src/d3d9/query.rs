//! IDirect3DQuery9 on the page's GPU (render/d3d/query.rs measures; gpu.rs hands the values
//! back by ticket). What Windows does, by Wine's d3d9 tests (device.c test_occlusion_query,
//! test_timestamp_query, test_query_support) and tests/winref/probes/d3d9query.c:
//!
//! * a query never issued: GetData is S_OK and fills the bytes asked for (up to its data size)
//!   with 0xdd;
//! * issued, not done yet: S_FALSE (D3DGETDATA_FLUSH sends what is recorded to the page);
//! * done: S_OK with min(size, the value's bytes) of the value: an occlusion query's u64 count
//!   (a DWORD's worth by its data size, all 8 bytes when asked for), an event's TRUE, a
//!   timestamp's u64 GPU time, the frequency of those (1 GHz: WebGPU's are nanoseconds), a
//!   disjoint query's BOOL (the device was lost and made again in between);
//! * an occlusion query begun and not ended (building): S_FALSE; ended without a begin: 0
//!   samples; Issue(BEGIN) again starts it over. Issue(BEGIN) of the queries that only end
//!   (event, timestamp, frequency) is S_OK and does nothing.

use super::gpu::{self, Answer, QueryEnd};
use kome::num::*;

pub const EVENT: u32 = 8;
pub const OCCLUSION: u32 = 9;
pub const TIMESTAMP: u32 = 10;
pub const TIMESTAMPDISJOINT: u32 = 11;
pub const TIMESTAMPFREQ: u32 = 12;

const ISSUE_END: u32 = 1;
const ISSUE_BEGIN: u32 = 2;
const GETDATA_FLUSH: u32 = 1;

/// The ticks a second of TIMESTAMP's values.
pub const FREQ: u64 = 1_000_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Event,
    Occlusion,
    Timestamp,
    TimestampDisjoint,
    TimestampFreq,
}

impl Kind {
    /// D3DQUERYTYPE's that CreateQuery makes (the others: D3DERR_NOTAVAILABLE).
    pub fn of_type(t: u32) -> Option<Kind> {
        match t {
            EVENT => Some(Kind::Event),
            OCCLUSION => Some(Kind::Occlusion),
            TIMESTAMP => Some(Kind::Timestamp),
            TIMESTAMPDISJOINT => Some(Kind::TimestampDisjoint),
            TIMESTAMPFREQ => Some(Kind::TimestampFreq),
            _ => None,
        }
    }

    pub fn type_id(self) -> u32 {
        match self {
            Kind::Event => EVENT,
            Kind::Occlusion => OCCLUSION,
            Kind::Timestamp => TIMESTAMP,
            Kind::TimestampDisjoint => TIMESTAMPDISJOINT,
            Kind::TimestampFreq => TIMESTAMPFREQ,
        }
    }

    /// GetDataSize
    pub fn data_size(self) -> u32 {
        match self {
            Kind::Event | Kind::Occlusion | Kind::TimestampDisjoint => 4,
            Kind::Timestamp | Kind::TimestampFreq => 8,
        }
    }

    /// The bytes of its value GetData writes at most.
    fn value_bytes(self) -> u32 {
        match self {
            Kind::Event | Kind::TimestampDisjoint => 4,
            Kind::Occlusion | Kind::Timestamp | Kind::TimestampFreq => 8,
        }
    }

    /// What its QUERY_END measures on the page.
    fn end(self) -> QueryEnd {
        match self {
            Kind::Occlusion => QueryEnd::Occlusion,
            Kind::Timestamp => QueryEnd::Timestamp,
            Kind::Event | Kind::TimestampDisjoint | Kind::TimestampFreq => QueryEnd::Event,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// never issued
    Fresh,
    /// begun, not ended: an occlusion query's ticket; a disjoint query's device losses then
    Building { ticket: u32, lost: u32 },
    /// ended: the page answers the ticket
    Issued { ticket: u32, lost: u32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Query {
    pub kind: Kind,
    pub state: State,
}

/// The device losses so far (render: `lost_gen`).
fn losses() -> u32 {
    crate::render::lost_gen()
}

/// The host clock in ns since 1970 (D3D9 and D3D11 timestamps).
pub(crate) fn ns_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

impl State {
    /// Its ticket is not asked about any more (D3D9 and D3D11 queries).
    pub fn forget(&self) {
        match *self {
            State::Fresh => {}
            State::Building { ticket, .. } | State::Issued { ticket, .. } => {
                if ticket != 0 {
                    gpu::query_forget(ticket);
                }
            }
        }
    }
}

impl Query {
    pub fn new(kind: Kind) -> Query {
        Query { kind, state: State::Fresh }
    }

    /// Its ticket is not asked about any more (released, or issued again).
    pub fn forget(&self) {
        self.state.forget();
    }

    /// IDirect3DQuery9::Issue
    pub fn issue(&mut self, flags: u32) -> u32 {
        let begin = flags & ISSUE_BEGIN != 0;
        let end = flags & ISSUE_END != 0;
        match self.kind {
            Kind::Occlusion => {
                if begin {
                    self.forget();
                    let t = gpu::query_ticket();
                    gpu::record(|b| b.cmd(&[gpu::QUERY_BEGIN, t]));
                    self.state = State::Building { ticket: t, lost: losses() };
                }
                if end {
                    let t = match self.state {
                        State::Building { ticket, .. } => ticket,
                        State::Fresh | State::Issued { .. } => {
                            self.forget();
                            gpu::query_ticket()
                        }
                    };
                    gpu::record(|b| b.cmd(&[gpu::QUERY_END, t, QueryEnd::Occlusion.word()]));
                    self.state = State::Issued { ticket: t, lost: losses() };
                }
            }
            Kind::TimestampDisjoint => {
                if begin {
                    self.forget();
                    self.state = State::Building { ticket: 0, lost: losses() };
                }
                if end {
                    let lost = match self.state {
                        State::Building { lost, .. } => lost,
                        State::Fresh | State::Issued { .. } => losses(),
                    };
                    self.forget();
                    let t = gpu::query_ticket();
                    gpu::record(|b| b.cmd(&[gpu::QUERY_END, t, QueryEnd::Event.word()]));
                    self.state = State::Issued { ticket: t, lost };
                }
            }
            Kind::Event | Kind::Timestamp | Kind::TimestampFreq => {
                if end {
                    self.forget();
                    let t = gpu::query_ticket();
                    gpu::record(|b| b.cmd(&[gpu::QUERY_END, t, self.kind.end().word()]));
                    self.state = State::Issued { ticket: t, lost: losses() };
                }
            }
        }
        0
    }

    /// IDirect3DQuery9::GetData into `out` (`size` bytes of it; 0 / NULL: only the answer).
    /// Returns the HRESULT and the bytes to write.
    pub fn get_data(&self, size: u32, flags: u32) -> (u32, Vec<u8>) {
        const S_FALSE: u32 = 1;
        let n = |cap: u32| us(size.min(cap));
        match self.state {
            State::Fresh => (0, vec![0xdd; n(self.kind.data_size())]),
            State::Building { .. } => (S_FALSE, Vec::new()),
            State::Issued { ticket, lost } => {
                let v = match gpu::query_answer(ticket) {
                    Answer::Pending => {
                        if flags & GETDATA_FLUSH != 0 {
                            gpu::flush();
                            // (tests: the page runs what was sent)
                            #[cfg(test)]
                            gpu::pump();
                        }
                        return (S_FALSE, Vec::new());
                    }
                    Answer::Done(v) => v,
                    // (nothing draws: no samples, the time now)
                    Answer::Unrun => match self.kind {
                        Kind::Occlusion => 0,
                        Kind::Timestamp => ns_now(),
                        Kind::Event | Kind::TimestampDisjoint | Kind::TimestampFreq => 0,
                    },
                };
                let v = match self.kind {
                    Kind::Occlusion | Kind::Timestamp => v,
                    Kind::Event => 1,
                    Kind::TimestampFreq => FREQ,
                    Kind::TimestampDisjoint => u64::from(losses() != lost),
                };
                (0, v.to_le_bytes().into_iter().take(n(self.kind.value_bytes())).collect())
            }
        }
    }
}

/// The value bytes `get_data` gave, as the little-endian number they are (tests).
#[cfg(test)]
pub fn le(b: &[u8]) -> u64 {
    b.iter().rev().fold(0, |a, &x| a << 8 | u64::from(x))
}

