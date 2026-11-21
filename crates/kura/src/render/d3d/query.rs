//! Direct3D's queries on the page's GPU (`QUERY_BEGIN` / `QUERY_END`, d3d9 and d3d11 alike):
//!
//! * an occlusion query counts the samples its draws let through (WebGPU's occlusion queries:
//!   every render pass that draws while one or more are open begins one query of the set,
//!   whose count goes to each of them; a pass never holds draws of two different sets of open
//!   queries, so a begin or end of a query ends the pass);
//! * a timestamp is the GPU's time in nanoseconds when it gets there (WebGPU's timestamp
//!   queries, written by an empty pass), or, on a device without "timestamp-query", the time
//!   the page heard the GPU finished the work before it;
//! * an event is done when the GPU finished the work before it (onSubmittedWorkDone).
//!
//! Each submit resolves the queries its passes wrote into a buffer the page maps; the counts
//! are added up per ticket, and a ticket that ended and has no counts on their way any more is
//! done: `vnhost_d3d9_query_done` hands its value to the guest's side (d3d9/gpu.rs).

use super::super::gpu::{self, usage, Id, QueryKind};
use super::D3d;
use crate::d3d9::gpu::{query_done, QueryEnd};
use kome::num::*;
use std::collections::HashMap;

/// Queries per set (WebGPU's limit): a submit with more passes that count is cut in two.
pub(super) const SET_SIZE: u32 = 4096;

/// A begun occlusion query: its count so far, the submits whose counts are still being read,
/// and whether it ended.
#[derive(Default)]
struct Sum {
    value: u64,
    reads: u32,
    in_submit: bool,
    ended: bool,
}

/// What an event-like ticket is done with once the GPU finished the work before it.
#[derive(Clone, Copy)]
pub(super) enum Then {
    /// a fixed value (an event: TRUE)
    Value(u64),
    /// the time then (a timestamp on a device without timestamp queries)
    Now,
}

/// A submit's query results being read: the buffer, its bytes, and per query whose they are.
pub(super) struct Read {
    buf: Id,
    data: Vec<u8>,
    occlusion: Vec<Vec<u32>>,
    stamps: Vec<u32>,
    stamps_at: usize,
}

#[derive(Default)]
pub(super) struct Queries {
    occlusion_set: Id,
    stamp_set: Id,
    /// queries of the sets written since the last submit, and whose they are
    occlusion: Vec<Vec<u32>>,
    stamps: Vec<u32>,
    /// the occlusion tickets open now, in the order they began
    active: Vec<u32>,
    sums: HashMap<u32, Sum>,
    /// event-like tickets ended since the last submit
    events: Vec<(u32, Then)>,
    reads: HashMap<u32, Read>,
    /// on_done tokens and the tickets done with them
    waits: HashMap<u32, Vec<(u32, Then)>>,
    /// the open pass began an occlusion query
    pub(super) counting: bool,
    /// a pass begun now must not count (a partial clear's quad: clears are not draws)
    pub(super) hold: bool,
    /// whether the device has timestamp queries (asked once per device)
    has_stamps: Option<bool>,
    /// counter tickets begun (COUNT_BEGIN): the 8-byte buffer the counter's word goes to at
    /// the begin (bytes 0..4) and at the end (4..8), the page buffer and offset of the word
    counts: HashMap<u32, (Id, u32, u32)>,
    /// counter tickets ended since the last submit
    counts_ended: Vec<(u32, Id)>,
    /// their reads: token -> (ticket, buffer, bytes)
    count_reads: HashMap<u32, (u32, Id, Vec<u8>)>,
}

/// performance.now() in nanoseconds.
fn now_ns() -> u64 {
    let ms = super::now();
    if ms.is_finite() && ms > 0.0 { trunc_u64(ms * 1e6) } else { 0 }
}

fn done(t: u32, v: u64) {
    query_done(t, lo32(v), lo32(v >> 32));
}

impl Queries {
    fn stamps_ok(&mut self) -> bool {
        *self.has_stamps.get_or_insert_with(|| gpu::has_feature("timestamp-query"))
    }

    /// Whether the next draw pass is to count.
    pub(super) fn wants_pass(&self) -> bool {
        !self.active.is_empty() && !self.hold
    }

    /// Whether a pass that counts can begin without a submit first (the set has room).
    pub(super) fn full(&self) -> bool {
        self.wants_pass() && self.occlusion.len() >= us(SET_SIZE)
    }

    /// The descriptor's `,"occlusionQuerySet":n` for a pass that counts ("" otherwise).
    pub(super) fn pass_json(&mut self) -> String {
        if !self.wants_pass() {
            return String::new();
        }
        if self.occlusion_set == 0 {
            self.occlusion_set = gpu::query_set(QueryKind::Occlusion, SET_SIZE);
        }
        format!(r#","occlusionQuerySet":{}"#, self.occlusion_set)
    }

    /// After beginRenderPass with `pass_json`'s descriptor: its query begins.
    pub(super) fn pass_begun(&mut self) {
        if !self.wants_pass() {
            return;
        }
        gpu::begin_occlusion(lo32(self.occlusion.len()));
        self.occlusion.push(self.active.clone());
        for t in &self.active {
            if let Some(s) = self.sums.get_mut(t) {
                s.in_submit = true;
            }
        }
        self.counting = true;
    }

    /// Before endRenderPass.
    pub(super) fn pass_ending(&mut self) {
        if self.counting {
            gpu::end_occlusion();
            self.counting = false;
        }
    }

    /// Whether the encoder holds query writes to resolve.
    pub(super) fn to_resolve(&self) -> bool {
        !self.occlusion.is_empty() || !self.stamps.is_empty()
    }

    /// Before the submit (no pass open): resolves what the sets got into a buffer to map.
    /// Returns it with the bytes to read.
    pub(super) fn resolve(&mut self) -> Option<(Id, u32, Read)> {
        if !self.to_resolve() {
            return None;
        }
        let n = lo32(self.occlusion.len());
        let m = lo32(self.stamps.len());
        let at = n.saturating_mul(8).next_multiple_of(256);
        let size = at.saturating_add(m.saturating_mul(8)).max(8);
        let src = gpu::buffer(size, usage::QUERY_RESOLVE | usage::COPY_SRC);
        let dst = gpu::buffer(size, usage::MAP_READ | usage::COPY_DST);
        if n > 0 {
            gpu::resolve_queries(self.occlusion_set, 0, n, src, 0);
        }
        if m > 0 {
            gpu::resolve_queries(self.stamp_set, 0, m, src, at);
        }
        gpu::copy_buffer(src, dst, size);
        for ts in self.occlusion.iter().flatten() {
            if let Some(s) = self.sums.get_mut(ts) && s.in_submit {
                s.in_submit = false;
                s.reads = s.reads.saturating_add(1);
            }
        }
        let read = Read { buf: dst, data: vec![0; us(size)], occlusion: std::mem::take(&mut self.occlusion), stamps: std::mem::take(&mut self.stamps), stamps_at: us(at) };
        Some((src, size, read))
    }

    /// The submit's reads asked for (`token`): the resolved queries' bytes come with `mapped`.
    pub(super) fn reading(&mut self, token: u32, size: u32, mut r: Read) {
        gpu::read_buffer(r.buf, size, r.data.as_mut_ptr(), token);
        self.reads.insert(token, r);
    }

    /// The events ended since the last submit: done with on_done(`token`).
    pub(super) fn take_events(&mut self) -> Vec<(u32, Then)> {
        std::mem::take(&mut self.events)
    }

    pub(super) fn wait(&mut self, token: u32, events: Vec<(u32, Then)>) {
        self.waits.insert(token, events);
    }

    /// on_done(`token`) came: whether it was one of the events' (and they are done now).
    pub(super) fn gpu_done(&mut self, token: u32) -> bool {
        let Some(es) = self.waits.remove(&token) else { return false };
        let t = now_ns();
        for (q, then) in es {
            done(q, match then {
                Then::Value(v) => v,
                Then::Now => t,
            });
        }
        true
    }

    /// The bytes of read `token` came (or not: `ok` false, a lost device's): whether it was a
    /// query read. The tickets it finished are done.
    pub(super) fn mapped(&mut self, token: u32, ok: bool) -> bool {
        if let Some((t, buf, data)) = self.count_reads.remove(&token) {
            gpu::destroy(buf);
            let word = |k: usize| data.get(k..k.saturating_add(4)).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap_or([0; 4])));
            done(t, if ok { u64::from(word(4).wrapping_sub(word(0))) } else { 0 });
            return true;
        }
        let Some(r) = self.reads.remove(&token) else { return false };
        gpu::destroy(r.buf);
        let value = |k: usize| if ok { r.data.get(k..k.saturating_add(8)).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))) } else { 0 };
        let mut touched: Vec<u32> = Vec::new();
        for (i, owners) in r.occlusion.iter().enumerate() {
            let v = value(i.saturating_mul(8));
            for t in owners {
                if let Some(s) = self.sums.get_mut(t) {
                    s.value = s.value.saturating_add(v);
                }
                if !touched.contains(t) {
                    touched.push(*t);
                }
            }
        }
        for t in touched {
            let finished = match self.sums.get_mut(&t) {
                Some(s) => {
                    s.reads = s.reads.saturating_sub(1);
                    s.ended && s.reads == 0 && !s.in_submit
                }
                None => false,
            };
            if finished && let Some(s) = self.sums.remove(&t) {
                done(t, s.value);
            }
        }
        for (i, &t) in r.stamps.iter().enumerate() {
            done(t, value(r.stamps_at.saturating_add(i.saturating_mul(8))));
        }
        true
    }

    /// The device is lost: what was on its way never comes. Every ticket not done yet is done
    /// now, with 0 (the guest's side sees the loss: a disjoint timestamp query).
    pub(super) fn lose(&mut self) {
        let mut ts: Vec<u32> = self.sums.drain().map(|(t, _)| t).collect();
        let mut reads: Vec<u32> = self.reads.keys().copied().collect();
        reads.sort_unstable();
        for k in reads {
            if let Some(r) = self.reads.remove(&k) {
                ts.extend(r.stamps);
            }
        }
        let mut waits: Vec<u32> = self.waits.keys().copied().collect();
        waits.sort_unstable();
        for k in waits {
            ts.extend(self.waits.remove(&k).into_iter().flatten().map(|e| e.0));
        }
        ts.extend(self.events.drain(..).map(|e| e.0));
        ts.extend(self.counts.drain().map(|e| e.0));
        ts.extend(self.counts_ended.drain(..).map(|e| e.0));
        ts.extend(self.count_reads.drain().map(|e| e.1.0));
        ts.append(&mut self.stamps);
        ts.sort_unstable();
        ts.dedup();
        for t in ts {
            done(t, 0);
        }
        *self = Queries::default();
    }
}

impl D3d {
    /// COUNT_BEGIN: counter ticket `t` counts what the u32 at byte `off` of page buffer `id`
    /// grows by from here (begun again: from here).
    pub(super) fn count_begin(&mut self, t: u32, id: u32, off: u32) {
        self.end_pass();
        let buf = gpu::buffer(8, usage::COPY_DST | usage::MAP_READ);
        if let Some((old, _, _)) = self.queries.counts.insert(t, (buf, id, off)) {
            self.doomed.push(old);
        }
        self.count_copy(id, off, buf, 0);
    }

    /// COUNT_END: counter ticket `t` ends here (without a begin: 0).
    pub(super) fn count_end(&mut self, t: u32) {
        let Some((buf, id, off)) = self.queries.counts.remove(&t) else {
            self.queries.events.push((t, Then::Value(0)));
            return;
        };
        self.end_pass();
        self.count_copy(id, off, buf, 4);
        self.queries.counts_ended.push((t, buf));
    }

    /// The word at `off` of page buffer `id` into byte `at` of `buf` (none: it stays 0).
    fn count_copy(&mut self, id: u32, off: u32, buf: Id, at: u32) {
        let Some(&(src, size)) = self.buffers.get(&id) else { return };
        if off % 4 != 0 || u64::from(off) + 4 > u64::from(size) {
            return;
        }
        if !self.bused.contains(&id) {
            self.bused.push(id);
        }
        self.open = true;
        gpu::copy_buffer_at(src, off, buf, at, 4);
    }

    /// After a submit: the counter tickets it ended are read.
    pub(super) fn read_counts(&mut self) {
        for (t, buf) in std::mem::take(&mut self.queries.counts_ended) {
            let token = self.next_token;
            self.next_token = self.next_token.wrapping_add(1).max(1);
            let mut data = vec![0u8; 8];
            gpu::read_buffer(buf, 8, data.as_mut_ptr(), token);
            self.queries.count_reads.insert(token, (t, buf, data));
        }
    }

    /// QUERY_BEGIN: occlusion query `t` counts from here (begun again: from 0).
    pub(super) fn query_begin(&mut self, t: u32) {
        self.end_pass();
        self.queries.active.retain(|&a| a != t);
        self.queries.active.push(t);
        self.queries.sums.insert(t, Sum::default());
    }

    /// QUERY_END: ticket `t` of kind `kind` ends here (None: a word that is no kind; done with 0).
    pub(super) fn query_end(&mut self, t: u32, kind: Option<QueryEnd>) {
        match kind {
            Some(QueryEnd::Occlusion) => {
                self.end_pass();
                self.queries.active.retain(|&a| a != t);
                let s = self.queries.sums.entry(t).or_default();
                s.ended = true;
                // (nothing drawn while it was open, or ended without a begin: 0 samples, done
                // once the GPU did what came before)
                if s.reads == 0 && !s.in_submit {
                    let v = s.value;
                    self.queries.sums.remove(&t);
                    self.queries.events.push((t, Then::Value(v)));
                }
            }
            Some(QueryEnd::Timestamp) if self.queries.stamps_ok() => {
                self.end_pass();
                if self.queries.stamps.len() >= us(SET_SIZE) {
                    self.submit();
                }
                if self.queries.stamp_set == 0 {
                    self.queries.stamp_set = gpu::query_set(QueryKind::Timestamp, SET_SIZE);
                }
                self.open = true;
                gpu::timestamp(self.queries.stamp_set, lo32(self.queries.stamps.len()));
                self.queries.stamps.push(t);
            }
            Some(QueryEnd::Timestamp) => self.queries.events.push((t, Then::Now)),
            Some(QueryEnd::Event) => self.queries.events.push((t, Then::Value(1))),
            None => done(t, 0),
        }
    }
}
