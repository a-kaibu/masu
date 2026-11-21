//! ID3D11Query / ID3D11Predicate (and Direct3D 10's) measured on the page's GPU, as d3d9's
//! (d3d9/query.rs; render/d3d/query.rs measures, d3d9/gpu.rs hands values back by ticket).
//! What Windows does (D3D11 documentation of ID3D11DeviceContext::Begin/End/GetData and the
//! D3D11_QUERY_DATA_* structures; Wine's d3d11 tests test_occlusion_query,
//! test_timestamp_query; winref d3d11query):
//!
//! * GetData of a query never ended, or begun again and not ended: DXGI_ERROR_INVALID_CALL;
//! * a size other than 0 or GetDataSize, or data NULL with a size: E_INVALIDARG;
//! * not done yet: S_FALSE (unless D3D11_ASYNC_GETDATA_DONOTFLUSH, what was recorded is sent);
//! * done: S_OK, the whole structure: an occlusion query's u64 samples, an occlusion
//!   predicate's BOOL (any sample), an event's TRUE, a timestamp's u64 GPU time,
//!   TIMESTAMP_DISJOINT's { u64 Frequency (1 GHz), BOOL Disjoint (the device was lost and made
//!   again in between) }.
//!
//! Begin of an event or a timestamp does nothing; End of an occlusion query without a Begin
//! counts nothing.

use crate::d3d9::gpu::{self, Answer, QueryEnd};
use crate::d3d9::query::{ns_now, FREQ};
use kome::num::*;

const S_FALSE: u32 = 1;
const E_INVALIDARG: u32 = 0x8007_0057;
const DXGI_ERROR_INVALID_CALL: u32 = 0x887a_0001;
const DONOTFLUSH: u32 = 1;

/// D3D11_QUERY
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Event,
    Occlusion,
    Timestamp,
    TimestampDisjoint,
    PipelineStatistics,
    OcclusionPredicate,
    /// SO_STATISTICS (stream: None for all, Some(0..=3))
    SoStatistics(Option<u32>),
    /// SO_OVERFLOW_PREDICATE
    SoOverflowPredicate(Option<u32>),
}

impl Kind {
    pub fn of_type(t: u32) -> Option<Kind> {
        Some(match t {
            0 => Kind::Event,
            1 => Kind::Occlusion,
            2 => Kind::Timestamp,
            3 => Kind::TimestampDisjoint,
            4 => Kind::PipelineStatistics,
            5 => Kind::OcclusionPredicate,
            6 => Kind::SoStatistics(None),
            7 => Kind::SoOverflowPredicate(None),
            8..=15 if t.is_multiple_of(2) => Kind::SoStatistics(Some(t.saturating_sub(8) >> 1)),
            8..=15 => Kind::SoOverflowPredicate(Some(t.saturating_sub(9) >> 1)),
            _ => return None,
        })
    }

    pub fn type_id(self) -> u32 {
        match self {
            Kind::Event => 0,
            Kind::Occlusion => 1,
            Kind::Timestamp => 2,
            Kind::TimestampDisjoint => 3,
            Kind::PipelineStatistics => 4,
            Kind::OcclusionPredicate => 5,
            Kind::SoStatistics(None) => 6,
            Kind::SoOverflowPredicate(None) => 7,
            Kind::SoStatistics(Some(s)) => s.saturating_mul(2).saturating_add(8),
            Kind::SoOverflowPredicate(Some(s)) => s.saturating_mul(2).saturating_add(9),
        }
    }

    /// GetDataSize
    pub fn data_size(self) -> u32 {
        match self {
            Kind::Event | Kind::OcclusionPredicate | Kind::SoOverflowPredicate(_) => 4,
            Kind::Occlusion | Kind::Timestamp => 8,
            Kind::TimestampDisjoint | Kind::SoStatistics(_) => 16,
            Kind::PipelineStatistics => 88,
        }
    }

    /// Whether Begin starts it (the others only end).
    fn begins(self) -> bool {
        match self {
            Kind::Event | Kind::Timestamp => false,
            Kind::Occlusion | Kind::TimestampDisjoint | Kind::PipelineStatistics | Kind::OcclusionPredicate | Kind::SoStatistics(_) | Kind::SoOverflowPredicate(_) => true,
        }
    }

    /// What its QUERY_END measures on the page; None: nothing the page measures.
    fn end(self) -> Option<QueryEnd> {
        match self {
            Kind::Occlusion | Kind::OcclusionPredicate => Some(QueryEnd::Occlusion),
            Kind::Timestamp => Some(QueryEnd::Timestamp),
            Kind::Event | Kind::TimestampDisjoint => Some(QueryEnd::Event),
            Kind::PipelineStatistics | Kind::SoStatistics(_) | Kind::SoOverflowPredicate(_) => None,
        }
    }
}

/// (Direct3D 9's: begun with an occlusion query's ticket, 0 none; the device losses then)
pub use crate::d3d9::query::State;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Query {
    pub kind: Kind,
    pub state: State,
    /// PIPELINE_STATISTICS: the device's counts (`Stats`) at Begin, then what came between
    pub stats: Stats,
    /// the counter tickets of `counter_words` (COUNT_BEGIN / COUNT_END), 0: none
    pub counts: [u32; 8],
}

/// The words of the page's counters (dxwgsl::expand::counter) a query reads: PIPELINE_STATISTICS
/// GSPrimitives, CInvocations, CPrimitives; SO_STATISTICS and SO_OVERFLOW_PREDICATE a stream's
/// primitives written and needed (2 s, 2 s + 1), all four streams' without one.
pub fn counter_words(kind: Kind) -> &'static [u32] {
    use dxwgsl::expand::counter as c;
    const STATS: [u32; 3] = [c::GS_PRIMITIVES, c::C_INVOCATIONS, c::C_PRIMITIVES];
    const SO: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    match kind {
        Kind::PipelineStatistics => &STATS,
        Kind::SoStatistics(Some(s)) | Kind::SoOverflowPredicate(Some(s)) => SO.get(us(s.min(3).saturating_mul(2))..us(s.min(3).saturating_mul(2).saturating_add(2))).unwrap_or(&[]),
        Kind::SoStatistics(None) | Kind::SoOverflowPredicate(None) => &SO,
        Kind::Event | Kind::Occlusion | Kind::Timestamp | Kind::TimestampDisjoint | Kind::OcclusionPredicate => &[],
    }
}

/// SO_STATISTICS of the counters' values (by `counter_words`): written, needed, the streams' summed.
fn so_counts(v: &[u64]) -> (u64, u64) {
    v.chunks(2).fold((0, 0), |(w, n), c| (w.wrapping_add(c.first().copied().unwrap_or(0)), n.wrapping_add(c.get(1).copied().unwrap_or(0))))
}

/// SO_OVERFLOW_PREDICATE of the counters' values: a stream needed more than it wrote.
fn overflowed(v: &[u64]) -> bool {
    v.chunks(2).any(|c| c.get(1).copied().unwrap_or(0) > c.first().copied().unwrap_or(0))
}

/// D3D11_QUERY_DATA_PIPELINE_STATISTICS's counts as the device keeps them from what it sends
/// the page (`count_draw`, `count_dispatch`): IAVertices, IAPrimitives, VSInvocations,
/// GSInvocations, GSPrimitives, CInvocations, CPrimitives, PSInvocations (the samples the page
/// measures: an occlusion ticket's), HSInvocations, DSInvocations, CSInvocations.
pub type Stats = [u64; 11];

/// The primitives `n` vertices of D3D11_PRIMITIVE_TOPOLOGY `topology` make.
pub fn primitives(topology: u32, n: u32) -> u64 {
    let n = u64::from(n);
    match topology {
        1 => n,
        2 => n / 2,
        3 => n.saturating_sub(1),
        4 => n / 3,
        5 => n.saturating_sub(2),
        10 => n / 4,
        11 => n.saturating_sub(3),
        12 => n / 6,
        13 => n.saturating_sub(4) / 2,
        33..=64 => n.checked_div(u64::from(topology.saturating_sub(32))).unwrap_or(0),
        _ => 0,
    }
}

/// What a draw of `n` vertices, `instances` times, adds on the device's side: the input
/// assembler's vertices and primitives, a vertex shader run per vertex, a geometry shader's
/// (`gs`: its GS instances) per primitive and instance; with a hull and domain shader (`tess`),
/// a hull shader run per patch. GSPrimitives, CInvocations and CPrimitives are the page's
/// counters (expand.rs: the expanded draws a PIPELINE_STATISTICS query makes).
pub fn count_draw(s: &mut Stats, topology: u32, n: u32, instances: u32, gs: Option<u32>, tess: bool) {
    let v = u64::from(n).saturating_mul(u64::from(instances));
    let p = primitives(topology, n).saturating_mul(u64::from(instances));
    let add = |s: &mut Stats, i: usize, x: u64| {
        if let Some(c) = s.get_mut(i) {
            *c = c.saturating_add(x);
        }
    };
    add(s, 0, v);
    add(s, 1, p);
    add(s, 2, v);
    if let Some(k) = gs {
        add(s, 3, p.saturating_mul(u64::from(k)));
    }
    if tess {
        add(s, 8, p);
    }
}

/// A dispatch of `groups` thread groups of `threads` each.
pub fn count_dispatch(s: &mut Stats, groups: u64, threads: u64) {
    s[10] = s[10].saturating_add(groups.saturating_mul(threads));
}

fn losses() -> u32 {
    crate::render::lost_gen()
}

impl Query {
    pub fn new(kind: Kind) -> Query {
        Query { kind, state: State::Fresh, stats: [0; 11], counts: [0; 8] }
    }

    pub fn forget(&self) {
        self.state.forget();
        for &t in self.counts.iter().filter(|&&t| t != 0) {
            gpu::query_forget(t);
        }
    }

    /// Whether it reads the page's counters (the draws while it is open are expanded).
    pub fn counts_on_page(&self) -> bool {
        !counter_words(self.kind).is_empty()
    }

    /// ID3D11DeviceContext::Begin (`now`: the device's counts; `counters`: the page buffer of
    /// the counters, expand.rs)
    pub fn begin(&mut self, now: &Stats, counters: u32) {
        if !self.kind.begins() {
            return;
        }
        self.forget();
        self.counts = [0; 8];
        for (t, &w) in self.counts.iter_mut().zip(counter_words(self.kind)) {
            *t = gpu::query_ticket();
            let c = [gpu::COUNT_BEGIN, *t, counters, w.saturating_mul(4)];
            gpu::record(|b| b.cmd(&c));
        }
        let ticket = match self.kind.end() {
            _ if self.kind == Kind::PipelineStatistics => {
                self.stats = *now;
                let t = gpu::query_ticket();
                gpu::record(|b| b.cmd(&[gpu::QUERY_BEGIN, t]));
                t
            }
            Some(QueryEnd::Occlusion) => {
                let t = gpu::query_ticket();
                gpu::record(|b| b.cmd(&[gpu::QUERY_BEGIN, t]));
                t
            }
            Some(QueryEnd::Event | QueryEnd::Timestamp) | None => 0,
        };
        self.state = State::Building { ticket, lost: losses() };
    }

    /// ID3D11DeviceContext::End (`now`: the device's counts)
    pub fn end(&mut self, now: &Stats) {
        let (begun, lost) = match self.state {
            State::Building { ticket, lost } => (ticket, lost),
            State::Fresh | State::Issued { .. } => (0, losses()),
        };
        if begun == 0 {
            self.forget();
            // (ended without a begin: tickets the page answers 0)
            self.counts = [0; 8];
            for (t, _) in self.counts.iter_mut().zip(counter_words(self.kind)) {
                *t = gpu::query_ticket();
            }
        }
        for &t in self.counts.iter().filter(|&&t| t != 0) {
            gpu::record(|b| b.cmd(&[gpu::COUNT_END, t]));
        }
        let t = if begun != 0 { begun } else { gpu::query_ticket() };
        if self.kind == Kind::PipelineStatistics {
            let from = if begun != 0 { self.stats } else { *now };
            let mut d = *now;
            for (x, f) in d.iter_mut().zip(from) {
                *x = x.saturating_sub(f);
            }
            self.stats = d;
        }
        let e = match self.kind {
            Kind::PipelineStatistics => QueryEnd::Occlusion,
            Kind::Event | Kind::Occlusion | Kind::Timestamp | Kind::TimestampDisjoint | Kind::OcclusionPredicate | Kind::SoStatistics(_) | Kind::SoOverflowPredicate(_) => self.kind.end().unwrap_or(QueryEnd::Event),
        };
        gpu::record(|b| b.cmd(&[gpu::QUERY_END, t, e.word()]));
        self.state = State::Issued { ticket: t, lost };
    }

    /// A predicate's answer for SetPredication, waited for (None: not a predicate, never ended,
    /// or the page gave up).
    pub fn predicate_answer(&self) -> Option<bool> {
        let State::Issued { ticket, .. } = self.state else { return None };
        match self.kind {
            Kind::OcclusionPredicate => match gpu::query_wait(ticket) {
                Answer::Done(v) => Some(v != 0),
                Answer::Unrun | Answer::Pending => None,
            },
            Kind::SoOverflowPredicate(_) => {
                let mut v = Vec::new();
                for &t in self.counts.iter().take(counter_words(self.kind).len()) {
                    match gpu::query_wait(t) {
                        Answer::Done(x) => v.push(x),
                        Answer::Unrun => v.push(0),
                        Answer::Pending => return None,
                    }
                }
                Some(overflowed(&v))
            }
            Kind::Event | Kind::Occlusion | Kind::Timestamp | Kind::TimestampDisjoint | Kind::PipelineStatistics | Kind::SoStatistics(_) => None,
        }
    }

    /// ID3D11DeviceContext::GetData(size, flags): the HRESULT and the bytes to write.
    pub fn get_data(&self, has_data: bool, size: u32, flags: u32) -> (u32, Vec<u8>) {
        let State::Issued { ticket, lost } = self.state else { return (DXGI_ERROR_INVALID_CALL, Vec::new()) };
        if size != 0 && (size != self.kind.data_size() || !has_data) {
            return (E_INVALIDARG, Vec::new());
        }
        let pending = || {
            if flags & DONOTFLUSH == 0 {
                gpu::flush();
                #[cfg(test)]
                gpu::pump();
            }
            (S_FALSE, Vec::new())
        };
        // the counters' values (by counter_words)
        let mut counts = Vec::new();
        for &t in self.counts.iter().take(counter_words(self.kind).len()) {
            match gpu::query_answer(t) {
                Answer::Pending => return pending(),
                Answer::Done(x) => counts.push(x),
                Answer::Unrun => counts.push(0),
            }
        }
        let v = match gpu::query_answer(ticket) {
            Answer::Pending => return pending(),
            Answer::Done(v) => v,
            Answer::Unrun => match self.kind {
                Kind::Timestamp => ns_now(),
                Kind::Event | Kind::Occlusion | Kind::TimestampDisjoint | Kind::PipelineStatistics | Kind::OcclusionPredicate | Kind::SoStatistics(_) | Kind::SoOverflowPredicate(_) => 0,
            },
        };
        if size == 0 {
            return (0, Vec::new());
        }
        let mut out = vec![0u8; us(size)];
        match self.kind {
            Kind::Occlusion | Kind::Timestamp => out.copy_from_slice(&v.to_le_bytes()),
            Kind::Event => out.copy_from_slice(&1u32.to_le_bytes()),
            Kind::OcclusionPredicate => out.copy_from_slice(&u32::from(v != 0).to_le_bytes()),
            Kind::TimestampDisjoint => {
                let bytes = FREQ.to_le_bytes().into_iter().chain(u32::from(losses() != lost).to_le_bytes());
                for (o, b) in out.iter_mut().zip(bytes) {
                    *o = b;
                }
            }
            Kind::PipelineStatistics => {
                let mut st = self.stats;
                st[7] = v;
                for (i, c) in [4usize, 5, 6].into_iter().zip(&counts) {
                    if let Some(s) = st.get_mut(i) {
                        *s = *c;
                    }
                }
                for (o, b) in out.iter_mut().zip(st.iter().flat_map(|x| x.to_le_bytes())) {
                    *o = b;
                }
            }
            Kind::SoStatistics(_) => {
                let (w, n) = so_counts(&counts);
                for (o, b) in out.iter_mut().zip(w.to_le_bytes().into_iter().chain(n.to_le_bytes())) {
                    *o = b;
                }
            }
            Kind::SoOverflowPredicate(_) => out.copy_from_slice(&u32::from(overflowed(&counts)).to_le_bytes()),
        }
        (0, out)
    }
}
