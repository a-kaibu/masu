//! The commands the page's WebGPU executor (web/src/d3d9.ts) runs, in batches.
//!
//! A batch is a list of u32 words (the commands) and a data block the commands point into
//! (vertices, indices, shader constants: the page writes it to one GPU buffer before it runs the
//! batch's draws; texture pixels and shader text). Guest threads append to the open batch; a
//! Present closes it. The page takes closed batches with `vnhost_d3d9_poll` every frame.
//!
//! Without a page (node) nothing ever polls, and batches are dropped at each Present.

#[cfg(test)]
use kome::locks::Lock;
use kome::num::*;
use crate::guest::gc;
use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Milliseconds by Date.now(): the clock all threads share (performance.now() differs per worker).
fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

pub const CREATE_TEXTURE: u32 = 1; // id w h levels format usage
pub const DESTROY_TEXTURE: u32 = 2; // id
pub const WRITE_TEXTURE: u32 = 3; // id level x y w h bytes_per_row blob_offset
pub const CREATE_SHADER: u32 = 4; // id blob_offset len (WGSL, UTF-8)
pub const CREATE_PIPELINE: u32 = 5; // see Pipeline::encode
pub const SET_TARGET: u32 = 6; // color_tex color_level depth_tex (0: none)
pub const CLEAR: u32 = 7; // flags color(rgba f32 x4) z stencil x y w h (w=0: whole target)
pub const DRAW: u32 = 8; // see Device::draw (its streams as n words of (buffer, offset) pairs, buffer DATA the data block, INDEX_PAIR | buffer the indices'; ends with: hash of what it reads, x0 y0 x1 y1 it draws to or u32::MAX)
pub const PRESENT: u32 = 9; // tex l t r b (screen rect the frame goes to) src_w src_h, or the part shown: l|t<<16 r|b<<16 (render/d3d.rs present_src) hwnd
pub const COPY: u32 = 10; // src_tex src_level sx sy sw sh dst_tex dst_level dx dy dw dh filter
pub const READBACK: u32 = 11; // request tex level x y w h dst_ptr pitch
pub const FILL: u32 = 12; // tex level x y w h color(bgra u32)
pub const WRITE_TEXTURE_MEM: u32 = 13; // id level x y w h pitch ptr: rows straight from guest memory
// Direct3D 11 (crates/kura/src/d3d11)
pub const CREATE_BUFFER: u32 = 20; // id size
pub const DESTROY_BUFFER: u32 = 21; // id
pub const WRITE_BUFFER: u32 = 22; // id offset blob_offset len
/// In WRITE_BUFFER's offset: the bytes are ones no draw before reads (written without ending the pass).
pub const WRITE_UNUSED: u32 = 0x8000_0000;
pub const CREATE_PIPELINE2: u32 = 23; // id len key[len] (see d3d11::draw)
pub const SET_TARGETS2: u32 = 24; // n (tex level layer)*n dtex dlevel dlayer
pub const DRAW2: u32 = 25; // see d3d11::draw (ends with the hash of what it reads from its buffers, 0: unknown)
pub const CLEAR_VIEW: u32 = 26; // tex level layer flags c0 c1 c2 c3 depth stencil
pub const GEN_MIPS: u32 = 27; // tex
pub const CREATE_TEXTURE2: u32 = 28; // id w h layers_or_depth mips format dim (0 2d, 1 3d, 2 1d; | 0x100 UAV, | 0x200 multisampled)
pub const WRITE_TEXTURE2: u32 = 29; // id level layer_or_z x y w h bytes_per_row blob_offset
pub const PRESENT_FLIP: u32 = 30; // as PRESENT, the texture bottom row first (OpenGL)
pub const DISPATCH: u32 = 31; // shader x y z n entries[n*8] (see d3d11::dispatch)
pub const RESOLVE: u32 = 32; // dst dst_level dst_layer src src_layer: ResolveSubresource (src multisampled, its level 0)
/// The page forgets a shader module (CREATE_SHADER's) and the compute pipelines made of it;
/// the render pipelines made of it go first (DESTROY_PIPELINE).
pub const DESTROY_SHADER: u32 = 33; // id
pub const DESTROY_PIPELINE: u32 = 34; // id (CREATE_PIPELINE's or CREATE_PIPELINE2's)
/// The display's gamma ramp (a full-screen Direct3D 9 swap chain's SetGammaRamp, DXGI
/// SetGammaControl): 3 x 256 u16 (red, green, blue) at blob_offset, or blob_offset u32::MAX for
/// none (the identity). The screen is drawn through it.
pub const GAMMA: u32 = 35; // blob_offset
/// Direct3D's queries (render/d3d/query.rs): occlusion query `ticket` counts the samples the
/// draws after QUERY_BEGIN let through; QUERY_END ends `ticket` (a `QueryEnd` word) there. The
/// page hands each ticket's value back (`vnhost_d3d9_query_done`).
pub const QUERY_BEGIN: u32 = 36; // ticket
pub const QUERY_END: u32 = 37; // ticket kind
/// Direct3D 11's buffers the GPU writes (stream output, the expanded draws' stores: d3d11/expand.rs):
/// READ_BUFFER copies `size` bytes from byte `offset` of page buffer `id` to guest memory `dst`
/// and says readback `request` is done (as READBACK; offset and size multiples of 4).
pub const READ_BUFFER: u32 = 38; // request id offset size dst
/// copyBufferToBuffer between page buffers (offsets and size multiples of 4), in command order.
pub const COPY_BUFFER: u32 = 39; // src src_offset dst dst_offset size
/// A counter query (d3d11/query.rs): ticket's value is what the u32 at byte `offset` of page
/// buffer `id` (counters the GPU adds to) grew by between COUNT_BEGIN and COUNT_END (mod 2^32).
pub const COUNT_BEGIN: u32 = 40; // ticket id offset
pub const COUNT_END: u32 = 41; // ticket
/// GEN_MIPS with point filtering: each texel of a level the texel at (2x + 1, 2y + 1) of the one
/// above (Direct3D 9's D3DTEXF_POINT AutoGenFilterType: winref d3d9tex's 1x1 level of the four
/// quadrants is the bottom right one's).
pub const GEN_MIPS_POINT: u32 = 42; // tex
/// Direct3D 11.1 logic ops on a multisampled target under a pixel-rate shader (d3d11/logic.rs):
/// each sample of `tex` that `mark` (r8uint, multisampled) marks gets the op (its place in
/// dxwgsl::logic::Op::ALL) over its bits in `stash` (the shader's source) and `copy` (the
/// target before the primitive), masked to the channels' bits, under write mask `mask`
/// (textures of the same size, 4 samples; the stash and mark UINT; sign 1: signed channels); a
/// sample is marked where bit `bit` of channel `ch` of its mark texel is 1 (dxwgsl::logic::Mark);
/// `buffered` 1: `stash` a page buffer of the sources by pixel (dxwgsl::logic::LOGIC_SRC).
pub const LOGIC_RESOLVE: u32 = 43; // tex copy stash mark mask op bits0 bits1 bits2 bits3 sign ch bit buffered

/// What a QUERY_END ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryEnd {
    /// an occlusion query (QUERY_BEGIN's): its samples
    Occlusion,
    /// an event: done (1) once the GPU did what came before
    Event,
    /// the GPU's time (ns) when it gets here
    Timestamp,
}

impl QueryEnd {
    pub fn word(self) -> u32 {
        match self {
            QueryEnd::Occlusion => 0,
            QueryEnd::Event => 1,
            QueryEnd::Timestamp => 2,
        }
    }
    pub fn of_word(w: u32) -> Option<QueryEnd> {
        [QueryEnd::Occlusion, QueryEnd::Event, QueryEnd::Timestamp].into_iter().find(|k| k.word() == w)
    }
}

/// What the page said of a query ticket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    /// on its way
    Pending,
    Done(u64),
    /// nothing runs it: no page draws (node), or the ticket is not this run's (a state saved
    /// with it on its way)
    Unrun,
}

/// Texture formats on the page.
pub const FMT_BGRA8: u32 = 0;
pub const FMT_DEPTH24S8: u32 = 1;
pub const FMT_RGBA16F: u32 = 2;
pub const FMT_RGBA32F: u32 = 22; // (the page format list's rgba32float: halves without float32-filterable)

#[derive(Default)]
pub struct Batch {
    pub cmds: Vec<u32>,
    /// what goes into the page's GPU buffer: vertices, indices, uniforms
    pub data: Vec<u8>,
    /// the rest (texture pixels, shader text)
    pub blob: Vec<u8>,
    presents: u32,
    generation: u64,
    /// where its WRITE_TEXTURE_MEM commands are in `cmds`
    mems: Vec<usize>,
}

impl Batch {
    pub fn cmd(&mut self, words: &[u32]) {
        if matches!(words.first(), Some(&PRESENT) | Some(&PRESENT_FLIP)) {
            crate::display::presented();
        }
        self.cmds.extend_from_slice(words);
    }

    /// WRITE_TEXTURE_MEM id level x y w h pitch ptr (see `detach_mem`).
    pub fn cmd_mem(&mut self, words: &[u32; 9]) {
        self.mems.push(self.cmds.len());
        self.cmds.extend_from_slice(words);
    }

    /// Guest memory [lo, hi) is about to change: the WRITE_TEXTURE_MEM commands that read it get
    /// a copy of their rows in the blob (WRITE_TEXTURE with the same pitch) instead.
    fn detach(&mut self, lo: u32, hi: u32) {
        let mut i = 0;
        while i < self.mems.len() {
            let at = self.mems[i];
            let c = &self.cmds[at..at + 9];
            let (h, pitch, ptr) = (c[6], c[7], c[8]);
            let len = us(pitch) * us(h);
            if u64::from(ptr) < u64::from(hi) && u64::from(ptr) + len as u64 > u64::from(lo) {
                let off = self.blob(unsafe { std::slice::from_raw_parts(gc::<u8>(ptr), len) });
                self.cmds[at] = WRITE_TEXTURE;
                self.cmds[at + 8] = off;
                self.mems.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn reads(&self, lo: u32, hi: u32) -> bool {
        self.mems.iter().any(|&at| {
            let c = &self.cmds[at..at + 9];
            u64::from(c[8]) < u64::from(hi) && u64::from(c[8]) + u64::from(c[7]) * u64::from(c[6]) > u64::from(lo)
        })
    }

    /// Appends data aligned to `align` bytes; returns its offset.
    pub fn data(&mut self, bytes: &[u8], align: usize) -> u32 {
        let at = self.data.len().next_multiple_of(align);
        self.data.resize(at, 0);
        self.data.extend_from_slice(bytes);
        lo32(at)
    }

    /// Appends to the blob (4-byte aligned); returns the offset.
    pub fn blob(&mut self, bytes: &[u8]) -> u32 {
        let at = self.blob.len().next_multiple_of(4);
        self.blob.resize(at, 0);
        self.blob.extend_from_slice(bytes);
        lo32(at)
    }

    /// Room for `len` bytes aligned to `align`; returns the offset.
    pub fn reserve(&mut self, len: usize, align: usize) -> u32 {
        let at = self.data.len().next_multiple_of(align);
        self.data.resize(at + len, 0);
        lo32(at)
    }
}

struct Queue {
    open: Batch,
    closed: VecDeque<Batch>,
    /// the batch the page is reading (kept until its next poll)
    reading: Option<Batch>,
    /// a batch the page is done with, emptied, its Vecs' room kept: the next open batch
    /// (`retire`, `fresh`)
    spare: Option<Batch>,
    presented: u64,
    shown: u64,
    last_poll: Option<f64>,
    readbacks_done: Vec<u32>,
    /// readbacks the page is copying into guest memory right now
    readbacks_writing: Vec<u32>,
    /// readbacks whose waiter gave up: the page must not write their pixels any more (the
    /// guest may have freed that memory already)
    readbacks_dropped: Vec<u32>,
    generation: u64,
    /// batches up to this one are done (the page ran them, or nobody will)
    done: u64,
    /// query tickets given out and not forgotten: None while on their way
    tickets: HashMap<u32, Option<u64>>,
    next_ticket: u32,
}

static QUEUE: Mutex<Option<Queue>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn with<R>(f: impl FnOnce(&mut Queue) -> R) -> R {
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    f(queue(&mut g))
}

/// The queue, made at its first use (a Present or a wait before any command was recorded:
/// a back buffer that could not be made, a game's first call a Present).
fn queue(g: &mut Option<Queue>) -> &mut Queue {
    g.get_or_insert_with(|| Queue {
        open: Batch::default(),
        closed: VecDeque::new(),
        reading: None,
        spare: None,
        presented: 0,
        shown: 0,
        last_poll: None,
        readbacks_done: Vec::new(),
        readbacks_writing: Vec::new(),
        readbacks_dropped: Vec::new(),
        generation: 0,
        done: 0,
        tickets: HashMap::new(),
        next_ticket: 0,
    })
}

/// A readback request's id as the page answers it (vnhost_d3d9_readback_done). Issued only by
/// [`Readback::new`]: every API (D3D9 devices, D3D11, GL) sends its readbacks through this one
/// queue and its one set of finished ids, so the queue numbers them; a number per device (D3D9's
/// `readbacks`) or per API (D3D11's 0x4000_0000 | n beside GL's page ids from 0x4000_0000) made
/// two outstanding requests share one, and one's answer let the other go on before its pixels
/// came (review of 6852be1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Readback(u32);

impl Readback {
    /// The next id (never 0, which the page's commands never carry).
    pub fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
        loop {
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n != 0 {
                return Readback(n);
            }
        }
    }
    /// The word the READBACK / READ_BUFFER command carries.
    pub fn word(self) -> u32 {
        self.0
    }
    /// Tests: the id a command written by hand carries (each test keeps its own range).
    #[cfg(test)]
    pub fn of_word(w: u32) -> Self {
        Readback(w)
    }
}

/// Tests: whether readback `id` was said to be done (and not taken by its waiter yet).
#[cfg(test)]
pub fn readback_is_done(id: Readback) -> bool {
    with(|q| q.readbacks_done.contains(&id.0))
}

// Tests: readbacks on this thread are given up (wait_readback answers false), as when the page
// stalls or goes: what their callers keep must survive it.
#[cfg(test)]
thread_local! {
    pub static GIVE_UP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Tests: ids from many threads at once are all different (the one numbering every API's
/// readbacks share).
#[cfg(test)]
#[test]
fn readback_ids_are_never_shared() {
    let ids: Vec<Vec<Readback>> = std::thread::scope(|s| (0..8).map(|_| s.spawn(|| (0..20_000).map(|_| Readback::new()).collect::<Vec<_>>())).collect::<Vec<_>>().into_iter().map(|h| h.join().unwrap_or_default()).collect());
    let all: std::collections::HashSet<u32> = ids.iter().flatten().map(|r| r.word()).collect();
    assert_eq!(all.len(), 8 * 20_000);
    assert!(!all.contains(&0));
}

// Tests: the page's executor in the same thread, run where the guest would wait for it.
#[cfg(test)]
thread_local! {
    static PUMP: std::cell::RefCell<Option<Box<dyn FnMut()>>> = std::cell::RefCell::new(None);
}

/// (tests) Runs `f` as the page whenever the guest waits for it; None: no page. Starts from an
/// empty queue (what other tests left is dropped).
#[cfg(test)]
pub fn set_pump(f: Option<Box<dyn FnMut()>>) {
    *QUEUE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // (a page from the start: the guest then waits for it where it must)
    if f.is_some() {
        with(|q| q.last_poll = Some(now()));
    }
    PUMP.with(|p| *p.borrow_mut() = f);
}

/// (tests) Lets the page run what was recorded so far.
#[cfg(test)]
pub fn pump() {
    flush_now();
    pump_page();
}

// (tests) The page is away: it neither polls nor runs anything (`set_away`).
#[cfg(test)]
thread_local! {
    static AWAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// (tests) The page goes away (its last poll long ago, nothing run) or comes back.
#[cfg(test)]
pub fn set_away(away: bool) {
    AWAY.with(|a| a.set(away));
    with(|q| q.last_poll = Some(if away { now() - 10_000.0 } else { now() }));
}

/// (tests) The page takes what is closed.
#[cfg(test)]
fn pump_page() {
    if AWAY.with(|a| a.get()) {
        return;
    }
    let f = PUMP.with(|p| p.borrow_mut().take());
    if let Some(mut f) = f {
        f();
        PUMP.with(|p| *p.borrow_mut() = Some(f));
    }
}

/// (tests) The page: takes the next closed batch and runs `f` on it (as vnhost_d3d9_poll and
/// vnhost_d3d9_done, with host pointers that need not fit 32 bits). False: there was none.
#[cfg(test)]
pub fn run_next(f: impl FnOnce(&[u32], &[u8], &[u8])) -> bool {
    let Some(b) = with(|q| {
        q.last_poll = Some(now());
        let b = q.closed.pop_front()?;
        q.shown += b.presents as u64;
        Some(b)
    }) else {
        return false;
    };
    f(&b.cmds, &b.data, &b.blob);
    with(|q| {
        q.done = q.done.max(b.generation + 1);
        retire(q, b);
    });
    WAKE.notify_all();
    true
}

/// A batch whose blob grew past this is closed after the command that grew it: a game loading
/// its images before the first Present put them all in one batch (Artemis: a 1 GB blob, out of
/// memory).
const BLOB_MAX: usize = 64 << 20;

/// The most bytes of closed batches that wait for the page before the guest waits for it too.
/// Only a Present waited: a game loading its textures for seconds without one (a loading
/// screen, with the tab hidden or the page slow) queued 64 MiB batches without end, and what
/// the BLOB_MAX above kept out of one batch ran the 2 GB heap out through the queue.
pub(crate) static QUEUE_MAX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(256 << 20);

/// The bytes of the closed batches.
fn queued(q: &Queue) -> usize {
    q.closed.iter().map(|b| b.blob.len() + b.data.len() + b.cmds.len() * 4).sum()
}

/// Waits while more than `QUEUE_MAX` bytes of closed batches wait for a page that has ever
/// polled (as `present` waits for a page two frames behind). Not while a state is taken.
fn wait_room() {
    let max = QUEUE_MAX.load(std::sync::atomic::Ordering::Relaxed);
    loop {
        #[cfg(test)]
        if with(|q| queued(q) > max && q.last_poll.is_some()) {
            pump_page();
        }
        let g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(q) = g.as_ref() else { return };
        if queued(q) <= max || q.last_poll.is_none() || crate::snap::pausing() {
            return;
        }
        drop(WAKE.wait_timeout(g, Duration::from_millis(20)).unwrap_or_else(|e| e.into_inner()));
    }
}

/// The room a spare batch keeps per Vec: what a frame's batch usually holds, not the blob of a
/// game loading its images (BLOB_MAX and past it)
const SPARE_ROOM: usize = 8 << 20;

/// A batch nobody reads any more: emptied and kept as the next open one (one spare), so the
/// next batch does not grow its commands, data and blob again from nothing. Measured
/// 2026-11-21 (callgrind, draw_tests' d3d9_draws_read_what_direct3d_reads): growing a new
/// batch's Vecs was 60% of `draw` (malloc consolidating, realloc copying) when every batch
/// started empty; with the spare, `draw` took 9.1M instructions instead of 19.7M.
fn retire(q: &mut Queue, mut b: Batch) {
    if q.spare.is_some() {
        return;
    }
    b.cmds.clear();
    b.data.clear();
    b.blob.clear();
    b.mems.clear();
    b.cmds.shrink_to(SPARE_ROOM / 4);
    b.data.shrink_to(SPARE_ROOM);
    b.blob.shrink_to(SPARE_ROOM);
    b.presents = 0;
    b.generation = 0;
    q.spare = Some(b);
}

/// The open batch closed: it is returned, a spare (or a new batch) open in its place.
fn take_open(q: &mut Queue) -> Batch {
    let next = q.spare.take().unwrap_or_default();
    std::mem::replace(&mut q.open, next)
}

/// Whether it closed the open batch.
fn close_if_big(q: &mut Queue) -> bool {
    if q.open.blob.len() < BLOB_MAX || q.open.cmds.is_empty() {
        return false;
    }
    let mut b = take_open(q);
    b.generation = q.generation;
    q.generation += 1;
    if q.last_poll.is_none() {
        // node: nobody draws
        q.done = q.generation;
        retire(q, b);
    } else {
        q.closed.push_back(b);
    }
    true
}

/// Records into the open batch.
pub fn record<R>(f: impl FnOnce(&mut Batch) -> R) -> R {
    let (r, closed) = with(|q| {
        let r = f(&mut q.open);
        (r, close_if_big(q))
    });
    if closed {
        wait_room();
    }
    r
}

/// Records into the open batch; also returns its generation (see `wait_done`).
pub fn record_gen<R>(f: impl FnOnce(&mut Batch) -> R) -> (R, u64) {
    let (r, closed) = with(|q| {
        let r = (f(&mut q.open), q.generation);
        (r, close_if_big(q))
    });
    if closed {
        wait_room();
    }
    r
}

fn page_attached(q: &Queue) -> bool {
    q.last_poll.is_some_and(|t| now() - t < 3000.0)
}

/// Frames and bytes the page may be behind before Present stops letting the game go on after
/// 250 ms. A page that polls but draws slower than 4 frames a second (a weak phone's GPU two
/// frames behind, a page whose frames take seconds) otherwise fell further behind with every
/// Present: the queue grew without end, as a hidden tab's did (2026-11-14 金恋 OOM).
const BACKLOG_FRAMES: u64 = 8;
const BACKLOG_BYTES: usize = 256 << 20;

/// Whether a Present `behind` frames ahead of the page (with `bytes` of batches queued) returns:
/// two frames behind at most, as a vsync; past its 250 ms when the page polls and is not
/// `BACKLOG_*` behind; never while the page is away.
fn may_go(behind: u64, bytes: usize, past_deadline: bool, attached: bool) -> bool {
    behind <= 2 || (past_deadline && attached && behind <= BACKLOG_FRAMES && bytes <= BACKLOG_BYTES)
}

/// The open batch's generation and how many command words it has (where the next command goes)
pub fn batch_end() -> (u64, usize) {
    with(|q| (q.generation, q.open.cmds.len()))
}

/// Changes whenever a new batch opens (what the page knows of draw state is per batch).
pub fn batch_gen() -> u64 {
    with(|q| q.generation)
}

/// Closes the open batch (if it has anything), and waits for the page when too much is queued.
pub fn flush() {
    flush_now();
    wait_room();
}

fn flush_now() {
    with(|q| {
        if !q.open.cmds.is_empty() {
            let mut b = take_open(q);
            b.generation = q.generation;
            q.closed.push_back(b);
            q.generation += 1;
        }
    });
}

/// After a Present was recorded: closes the batch, then waits while the page is more than two
/// frames behind (as a vsync would; without a page, drops what nobody will draw). A page that
/// polls but is slow holds a frame at most 250 ms while it is less than `BACKLOG_FRAMES` /
/// `BACKLOG_BYTES` behind, then until it caught up to that; a page that stopped polling (its
/// tab hidden: no animation frames) holds it until it polls again. Returning then, as before, let a game
/// in a background tab spin at a whole core and pile up frames nobody drew: 金恋 ran out of
/// memory after 6 minutes, and showed white for minutes after while the page caught up.
/// (`hwnd`: the window presented to; each window is paced on its own)
pub fn present(hwnd: u32) {
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    {
        let q = queue(&mut g);
        q.open.presents += 1;
        q.presented += 1;
        let mut b = take_open(q);
        b.generation = q.generation;
        q.generation += 1;
        q.closed.push_back(b);
        if q.last_poll.is_none() {
            // node: nobody draws
            while let Some(b) = q.closed.pop_front() {
                retire(q, b);
            }
            q.shown = q.presented;
            q.done = q.generation;
            return;
        }
    }
    // a display's rate at most (games that present in a busy loop would spend the CPU on
    // frames nobody sees)
    drop(g);
    pace(hwnd);
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = now() + 250.0;
    loop {
        let q = queue(&mut g);
        // (a state being taken: the game must reach its pause, not wait here)
        if may_go(q.presented.saturating_sub(q.shown), queued(q), now() >= deadline, page_attached(q)) || crate::snap::pausing() {
            return;
        }
        g = WAKE.wait_timeout(g, Duration::from_millis(20)).unwrap_or_else(|e| e.into_inner()).0;
    }
}

/// Waits until the batch recorded in generation `gen` (see `batch_gen`) is done: guest memory
/// its commands read (WRITE_TEXTURE_MEM) may change after that.
pub fn wait_done(generation: u64) {
    #[cfg(test)]
    pump();
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = now() + 1000.0;
    loop {
        let q = queue(&mut g);
        if q.done > generation || q.last_poll.is_none() || !page_attached(q) || now() >= deadline {
            if q.last_poll.is_none() {
                q.done = q.done.max(generation + 1);
            }
            return;
        }
        if q.generation == generation && !q.open.cmds.is_empty() {
            // still open: close it
            let mut b = take_open(q);
            b.generation = q.generation;
            q.closed.push_back(b);
            q.generation += 1;
        }
        g = WAKE.wait_timeout(g, Duration::from_millis(5)).unwrap_or_else(|e| e.into_inner()).0;
    }
}

/// Guest memory [lo, hi) is about to change: the batches not taken by the page yet copy what
/// their WRITE_TEXTURE_MEM commands read of it (a copy instead of waiting up to a frame for the
/// page). Returns the generation to wait for when the page is running such a batch right now.
pub fn detach_mem(lo: u32, hi: u32) -> Option<u64> {
    with(|q| {
        for b in q.closed.iter_mut() {
            b.detach(lo, hi);
        }
        q.open.detach(lo, hi);
        let done = q.done;
        q.reading.as_ref().filter(|b| b.generation >= done && b.reads(lo, hi)).map(|b| b.generation)
    })
}

/// Whether the batch of generation `gen` is done (see `wait_done`).
pub fn is_done(generation: u64) -> bool {
    with(|q| q.done > generation || q.last_poll.is_none())
}

/// The page ran the batch it took last.
#[unsafe(no_mangle)]
pub extern "C" fn vnhost_d3d9_done() {
    with(|q| {
        if let Some(b) = &q.reading {
            q.done = q.done.max(b.generation + 1);
        }
    });
    WAKE.notify_all();
}

/// Waits until the page has done readback `id` (or gave up).
pub fn wait_readback(id: Readback) -> bool {
    let id = id.0;
    flush();
    #[cfg(test)]
    pump();
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    // (tests: a page that gives up on this readback, as a stalled or gone one does)
    #[cfg(test)]
    if GIVE_UP.with(std::cell::Cell::get) {
        queue(&mut g).readbacks_dropped.push(id);
        return false;
    }
    // (10 s without the page finishing a batch: a slow GPU with frames queued before the
    // readback, or shaders compiling, is still making progress)
    let mut deadline = now() + 10000.0;
    let mut done = g.as_ref().map_or(0, |q| q.done);
    loop {
        let q = queue(&mut g);
        if q.done != done {
            done = q.done;
            deadline = now() + 10000.0;
        }
        if let Some(i) = q.readbacks_done.iter().position(|&r| r == id) {
            q.readbacks_done.swap_remove(i);
            return true;
        }
        // giving up (the page is gone or stalled): the page must not write the pixels later,
        // into memory the guest may have freed and reused by then
        if (now() >= deadline || !page_attached(q)) && !q.readbacks_writing.contains(&id) {
            eprintln!("kura: readback {id:#x} given up ({})", if page_attached(q) { "timed out" } else { "page not polling" });
            q.readbacks_dropped.push(id);
            return false;
        }
        g = WAKE.wait_timeout(g, Duration::from_millis(5)).unwrap_or_else(|e| e.into_inner()).0;
    }
}

/// The page: takes the next closed batch. `out` gets [cmds ptr, cmd words, data ptr, data bytes,
/// blob ptr, blob bytes];
/// returns 0 when there is none. The batch stays valid until the next call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vnhost_d3d9_poll(out: *mut [u32; 6]) -> u32 {
    let out = &mut *out;
    let r = with(|q| {
        q.last_poll = Some(now());
        // (the batch read before: the page is done with it)
        let next = q.closed.pop_front();
        if let Some(old) = std::mem::replace(&mut q.reading, next) {
            retire(q, old);
        }
        let Some(b) = &q.reading else { return 0 };
        super::stats::batch(b.data.len(), b.blob.len());
        q.shown += u64::from(b.presents);
        out[0] = lo32(b.cmds.as_ptr().expose_provenance());
        out[1] = lo32(b.cmds.len());
        out[2] = lo32(b.data.as_ptr().expose_provenance());
        out[3] = lo32(b.data.len());
        out[4] = lo32(b.blob.as_ptr().expose_provenance());
        out[5] = lo32(b.blob.len());
        1
    });
    WAKE.notify_all();
    r
}

/// A new query ticket (on its way until the page answers it, `query_done`).
pub fn query_ticket() -> u32 {
    with(|q| {
        loop {
            q.next_ticket = q.next_ticket.wrapping_add(1);
            if q.next_ticket != 0 && !q.tickets.contains_key(&q.next_ticket) {
                break;
            }
        }
        q.tickets.insert(q.next_ticket, None);
        q.next_ticket
    })
}

/// (tests) The tickets given out and not forgotten.
#[cfg(test)]
pub fn tickets_live() -> usize {
    with(|q| q.tickets.len())
}

/// Ticket `t` is not asked about any more (its query issued again, or released).
pub fn query_forget(t: u32) {
    with(|q| q.tickets.remove(&t));
}

/// What the page said of ticket `t`.
pub fn query_answer(t: u32) -> Answer {
    with(|q| match q.tickets.get(&t) {
        _ if q.last_poll.is_none() => Answer::Unrun,
        None => Answer::Unrun,
        Some(None) => Answer::Pending,
        Some(Some(v)) => Answer::Done(*v),
    })
}

/// Waits for the page's answer of ticket `t` (Pending: the page gave up or stalled, 10 s
/// without finishing a batch, as `wait_readback`).
pub fn query_wait(t: u32) -> Answer {
    flush();
    #[cfg(test)]
    pump();
    let mut g = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    let mut deadline = now() + 10000.0;
    let mut done = g.as_ref().map_or(0, |q| q.done);
    loop {
        let q = queue(&mut g);
        match q.tickets.get(&t) {
            _ if q.last_poll.is_none() => return Answer::Unrun,
            None => return Answer::Unrun,
            Some(Some(v)) => return Answer::Done(*v),
            Some(None) => {}
        }
        if q.done != done {
            done = q.done;
            deadline = now() + 10000.0;
        }
        if now() >= deadline || !page_attached(q) {
            return Answer::Pending;
        }
        g = WAKE.wait_timeout(g, Duration::from_millis(5)).unwrap_or_else(|e| e.into_inner()).0;
    }
}

/// The page: ticket `t`'s value is `lo | hi << 32` (render/d3d/query.rs).
pub fn query_done(t: u32, lo: u32, hi: u32) {
    with(|q| {
        if let Some(v) = q.tickets.get_mut(&t) {
            *v = Some(u64::from(lo) | u64::from(hi) << 32);
        }
    });
    WAKE.notify_all();
}

/// The page wrote the pixels of readback `id`.
#[unsafe(no_mangle)]
pub extern "C" fn vnhost_d3d9_readback_done(id: u32) {
    with(|q| {
        q.readbacks_writing.retain(|&r| r != id);
        if let Some(i) = q.readbacks_dropped.iter().position(|&r| r == id) {
            q.readbacks_dropped.swap_remove(i);
        } else {
            q.readbacks_done.push(id);
        }
    });
    WAKE.notify_all();
}

/// The page: whether it may now write readback `id`'s pixels into guest memory (0: its waiter
/// gave up, skip the copy). Until readback_done, the waiter keeps waiting.
#[unsafe(no_mangle)]
pub extern "C" fn vnhost_d3d9_readback_begin(id: u32) -> u32 {
    with(|q| {
        if q.readbacks_dropped.contains(&id) {
            0
        } else {
            q.readbacks_writing.push(id);
            1
        }
    })
}

fn pace(hwnd: u32) {
    if crate::nopace() {
        return; // (benchmarks: the page's rate only, tests/bench)
    }
    // (per window: a game with two windows, or two devices, presents each at 60 a second;
    // one clock for all gave them 30 each)
    static NEXT: Mutex<Option<HashMap<u32, f64>>> = Mutex::new(None);
    const FRAME: f64 = 1000.0 / 60.0;
    let t = now();
    let wait = pace_window(NEXT.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new), hwnd, t, FRAME);
    if wait > 0.0 {
        std::thread::sleep(Duration::from_secs_f64(wait / 1000.0));
    }
}

/// How long a Present at `t` waits for the frame after `next` (and `next` moved on): late
/// frames don't make the next ones early, and no wait is longer than a frame. The clock is
/// the wall clock: stepped back (NTP, a phone waking) by a minute, the next Present slept a
/// minute.
/// `pace_wait` for window `hwnd` of `next`'s; windows not presented to for a second are
/// forgotten (a game making and closing windows).
fn pace_window(next: &mut HashMap<u32, f64>, hwnd: u32, t: f64, frame: f64) -> f64 {
    if next.len() >= 16 {
        next.retain(|_, n| *n + 1000.0 > t);
    }
    pace_wait(next.entry(hwnd).or_insert(0.0), t, frame)
}

fn pace_wait(next: &mut f64, t: f64, frame: f64) -> f64 {
    // (late: `next` is behind `t`, and max takes `t`; a `next` more than a frame ahead is a clock
    // stepped back)
    let at = if *next > t + frame { t } else { next.max(t) };
    *next = at + frame;
    at - t
}

#[cfg(test)]
mod pace_tests {
    use kome::num::*;
    use proptest::prelude::*;
    proptest! {
        /// Any clock, stepped back or forward: every wait is between 0 and a frame, and with a
        /// steady clock presents a frame apart do not wait at all. (The wait is slept: the clock
        /// moves on by it, as the caller's does. Without that, two presents at one time left
        /// `next` a frame ahead of a clock that never caught up, and every later present
        /// "waited".)
        #[test]
        fn a_wait_is_a_frame_at_most(steps in proptest::collection::vec(-100_000.0f64..100_000.0, 1..200)) {
            let (mut next, mut t) = (0.0, 1e12);
            for d in steps {
                t += d;
                let w = super::pace_wait(&mut next, t, 16.0);
                prop_assert!((0.0..=16.0).contains(&w), "{}", w);
                t += w;
            }
            for _ in 0..10 {
                t += 16.0;
                prop_assert_eq!(super::pace_wait(&mut next, t, 16.0), 0.0);
            }
        }

        /// A game whose frames take any time up to a frame (a burst of presents at once among
        /// them), sleeping each wait: it presents about once a frame, never at half the rate
        /// (a pacer a frame ahead of the clock makes every present wait a whole frame), never
        /// faster than one a frame.
        #[test]
        fn a_game_presents_once_a_frame(work in proptest::collection::vec(prop_oneof![Just(0.0f64), 0.0f64..16.0], 50..300)) {
            let (mut next, start) = (0.0, 1e9);
            let mut t = start;
            for &w in &work {
                t += w;
                t += super::pace_wait(&mut next, t, 16.0);
            }
            let frames = (t - start) / 16.0;
            let n = f64::from(u32::try_from(work.len()).unwrap());
            prop_assert!(frames <= n + 1.0, "{} presents took {} frames", n, frames);
            prop_assert!(frames >= n - 1.0, "{} presents in {} frames: faster than the frame", n, frames);
        }

        /// Windows are paced apart: `n` windows each presenting once a frame, each at its own
        /// time in the frame, never wait (one clock for all made each wait for the others); a
        /// window presenting twice in a frame waits; the table stays small however many
        /// windows come and go.
        #[test]
        fn windows_are_paced_apart(n in 1usize..6, frames in 2usize..40, offsets in proptest::collection::vec(0.0f64..16.0, 6), others in proptest::collection::vec(1000u32..100_000, 0..200)) {
            let mut next = std::collections::HashMap::new();
            let mut t = 1e9;
            for f in 0..frames {
                for w in 1..=lo32(n) {
                    let at = t + offsets[us(w) - 1];
                    let wait = super::pace_window(&mut next, w, at, 16.0);
                    prop_assert!(f == 0 || wait == 0.0, "frame {} window {} waited {}", f, w, wait);
                }
                t += 16.0;
            }
            // (window 1 again at once: a second present in the frame waits)
            prop_assert!(super::pace_window(&mut next, 1, t - 16.0 + offsets[0] + 1.0, 16.0) > 0.0);
            for (k, w) in others.into_iter().enumerate() {
                super::pace_window(&mut next, w, t + k as f64 * 100.0, 16.0);
                prop_assert!(next.len() <= 16 + 1, "{} windows kept", next.len());
            }
        }

        /// The table forgets only windows not presented to for a second: a window presented
        /// just now keeps its pace while many others come (its second present in the frame
        /// still waits), however long ago the others were.
        #[test]
        fn recent_windows_are_kept(others in proptest::collection::vec((1000u32..100_000, 0.0f64..5000.0), 16..60), gap in 0.0f64..15.0) {
            let mut next = std::collections::HashMap::new();
            let t = 1e9;
            for &(w, ago) in &others {
                super::pace_window(&mut next, w, t - ago, 16.0);
            }
            super::pace_window(&mut next, 1, t, 16.0);
            for (k, &(w, _)) in others.iter().enumerate() {
                super::pace_window(&mut next, w + 100_000 + u32::try_from(k).unwrap(), t, 16.0);
            }
            prop_assert!(super::pace_window(&mut next, 1, t + gap, 16.0) > 0.0, "window 1 forgotten");
            prop_assert!(next.values().all(|&n| n + 1000.0 > t - 16.0), "a window idle for a second kept");
        }
    }
}

#[cfg(test)]
mod hidden_tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering::SeqCst};
    use std::sync::Arc;

    fn queued() -> (usize, usize) {
        with(|q| (q.closed.len(), q.closed.iter().map(|b| b.blob.len()).sum()))
    }

    /// A Present, a wait for a batch and a wait for a readback before anything was ever
    /// recorded (no queue yet) go on as without a page; they used to abort the VM.
    #[test]
    fn waits_before_the_first_command_go_on() {
        let _g = super::super::trace_tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        for first in 0..3 {
            set_pump(None);
            assert!(QUEUE.locked().is_none());
            match first {
                0 => present(1),
                1 => wait_done(3),
                _ => assert!(!wait_readback(Readback(7))),
            }
            present(1);
            wait_done(0);
            assert!(is_done(0));
            assert_eq!(with(|q| q.presented), 1 + (first == 0) as u64);
        }
    }

    /// A game presenting 1 MiB frames into a page that stopped polling (a hidden tab) waits
    /// in Present with at most 3 frames queued, however long the page is away, and goes on
    /// when it polls again; a page that polls but is slow holds a frame 250 ms at most.
    #[test]
    fn a_page_away_holds_the_game_not_its_frames() {
        let _g = super::super::trace_tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_pump(None);
        with(|q| q.last_poll = Some(now() - 10_000.0));
        let done = Arc::new(AtomicU32::new(0));
        let d = done.clone();
        let game = std::thread::spawn(move || {
            for _ in 0..40 {
                record(|b| {
                    b.cmd(&[0]);
                    b.blob.extend(std::iter::repeat_n(7u8, 1 << 20));
                });
                present(1);
                d.fetch_add(1, SeqCst);
            }
        });
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(100));
            let (n, bytes) = queued();
            assert!(n <= 3 && bytes <= 3 << 20, "{n} frames, {bytes} bytes queued for a page away");
        }
        assert!(done.load(SeqCst) <= 3, "{} presents went through", done.load(SeqCst));
        // (a state taken meanwhile: the game is let go to reach its pause)
        let before = done.load(SeqCst);
        crate::snap::VNHOST_SNAP.pause.store(1, SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        crate::snap::VNHOST_SNAP.pause.store(0, SeqCst);
        assert!(done.load(SeqCst) > before, "a pause did not let the game out of Present");
        // (the page back: it takes the frames, the game goes on to the end)
        while !game.is_finished() {
            while run_next(|_, _, _| {}) {}
            std::thread::sleep(Duration::from_millis(2));
        }
        game.join().unwrap();
        assert_eq!(done.load(SeqCst), 40);
        // (a page polling but not keeping up: Present goes on after 250 ms)
        let t = now();
        for _ in 0..4 {
            with(|q| q.last_poll = Some(now()));
            record(|b| b.cmd(&[0]));
            present(1);
        }
        assert!(now() - t < 2000.0, "a slow page held the game {} ms", now() - t);
        set_pump(None);
    }

    /// A game loading without a Present (1 MiB of texture, a flush, again and again) into a
    /// page that is away queues no more than the limit and one batch, waits, and when the page
    /// polls again every batch comes to it once and in order. With no page at all (node)
    /// nothing waits.
    #[test]
    fn loading_without_a_present_waits_for_the_page_too() {
        let _g = super::super::trace_tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let was = QUEUE_MAX.swap(8 << 20, SeqCst);
        let load = |n: u32, done: Arc<AtomicU32>| {
            for k in 0..n {
                record(|b| {
                    b.cmd(&[k]);
                    b.blob.extend(std::iter::repeat_n(k as u8, 1 << 20));
                });
                flush();
                done.fetch_add(1, SeqCst);
            }
        };
        // (no page: all at once)
        set_pump(None);
        let done = Arc::new(AtomicU32::new(0));
        let t = now();
        load(30, done.clone());
        assert!(now() - t < 5000.0 && done.load(SeqCst) == 30);
        // (a page that is away)
        set_pump(None);
        with(|q| q.last_poll = Some(now() - 10_000.0));
        let done = Arc::new(AtomicU32::new(0));
        let d = done.clone();
        let game = std::thread::spawn(move || load(60, d));
        for _ in 0..8 {
            std::thread::sleep(Duration::from_millis(50));
            let (n, bytes) = queued();
            assert!(bytes <= (8 << 20) + (1 << 20) + 4096, "{n} batches, {bytes} bytes queued for a page away");
        }
        let held = done.load(SeqCst);
        assert!((6..=9).contains(&held), "{held} flushes went through");
        // (the page back, slowly: the queue stays within the limit, the batches come in order)
        let mut got = Vec::new();
        while !game.is_finished() || queued().0 > 0 {
            let (_, bytes) = queued();
            assert!(bytes <= (8 << 20) + (1 << 20) + 4096, "{bytes} bytes queued");
            run_next(|c, _, blob| {
                assert!(blob.len() == 1 << 20 && blob.iter().all(|&x| x == c[0] as u8));
                got.push(c[0]);
            });
            std::thread::sleep(Duration::from_millis(1));
        }
        game.join().unwrap();
        assert_eq!(got, (0..60).collect::<Vec<u32>>());
        QUEUE_MAX.store(was, SeqCst);
        set_pump(None);
    }

    /// A batch the page is done with is the next open one, its room kept (retire): a new
    /// batch growing from nothing was 60% of draw (2026-11-21); a blob past SPARE_ROOM is not
    /// kept whole.
    #[test]
    fn a_done_batch_is_the_next_open_one() {
        let _g = super::super::trace_tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_pump(None);
        flush_now();
        with(|q| {
            q.closed.clear();
            q.spare = None;
            q.last_poll = Some(now());
        });
        record(|b| {
            b.cmd(&[0; 1000]);
            b.data(&[1; 4000], 4);
        });
        flush_now();
        assert!(run_next(|cmds, data, _| assert_eq!((cmds.len(), data.len()), (1000, 4000))));
        assert!(with(|q| q.spare.as_ref().is_some_and(|b| b.cmds.is_empty() && b.data.is_empty() && b.mems.is_empty() && b.presents == 0)));
        // (the next batch closed: the spare is open, with the room the done one had)
        record(|b| b.cmd(&[0]));
        flush_now();
        let (cmds, data) = with(|q| (q.open.cmds.capacity(), q.open.data.capacity()));
        assert!(cmds >= 1000 && data >= 4000, "the open batch is new: {cmds} {data}");
        assert!(run_next(|cmds, _, _| assert_eq!(cmds.len(), 1)));
        // (a game loading its images: the blob's room is cut to SPARE_ROOM)
        record(|b| {
            b.cmd(&[0]);
            b.blob(&vec![7; SPARE_ROOM * 2]);
        });
        flush_now();
        assert!(run_next(|_, _, blob| assert_eq!(blob.len(), SPARE_ROOM * 2)));
        assert!(with(|q| q.spare.as_ref().map_or(0, |b| b.blob.capacity())) <= SPARE_ROOM);
        with(|q| q.last_poll = None);
    }

    /// A page that polls every 30 ms but takes a frame only every 600 ms (a weak GPU two frames
    /// behind): the game is let go past 250 ms only while the page is less than BACKLOG_FRAMES
    /// behind, so the queue stays bounded instead of growing by a frame or two a second.
    #[test]
    fn a_slow_page_holds_the_game_at_the_backlog() {
        let _g = super::super::trace_tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        set_pump(None);
        with(|q| {
            q.closed.clear();
            q.shown = q.presented;
            q.last_poll = Some(now());
        });
        let game = std::thread::spawn(|| {
            for _ in 0..60 {
                record(|b| {
                    b.cmd(&[0]);
                    b.blob.extend(std::iter::repeat_n(7u8, 1 << 16));
                });
                present(1);
            }
        });
        let mut most = 0;
        let mut ticks = 0;
        while !game.is_finished() {
            std::thread::sleep(Duration::from_millis(30));
            ticks += 1;
            if ticks % 20 == 0 {
                run_next(|_, _, _| {});
            } else {
                with(|q| q.last_poll = Some(now()));
            }
            most = most.max(queued().0);
            if ticks > 400 {
                // (the rest at full speed: the test's end)
                while !game.is_finished() {
                    while run_next(|_, _, _| {}) {}
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
        game.join().unwrap();
        assert!(most as u64 <= BACKLOG_FRAMES + 1, "{most} frames queued for a slow page");
        while run_next(|_, _, _| {}) {}
        set_pump(None);
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(128))]
        /// Present's rule over any page: a game presenting as fast as it may (each Present
        /// waiting out its 250 ms when it must) against a page that takes `take` batches every
        /// `every` ms, polls every `polls` ms (0: never, a hidden tab) and is away from `away`
        /// for `gone` ms. Whatever the rates, the batches queued stay within BACKLOG_FRAMES + 1
        /// frames and BACKLOG_BYTES plus one frame, and the game goes on whenever the page is
        /// at most two frames behind.
        #[test]
        fn present_bounds_the_queue_for_any_page(
            take in 0u64..4, every in 1u64..2000, polls in 0u64..4000,
            away in 0u64..20_000, gone in 0u64..60_000,
            sizes in proptest::collection::vec(0usize..(96 << 20), 1..8),
        ) {
            let (mut behind, mut bytes) = (0u64, std::collections::VecDeque::<usize>::new());
            let (mut last_poll, mut waiting_since) = (None::<u64>, None::<u64>);
            let mut frame = 0usize;
            for t in 0..120_000u64 {
                let present = !(t >= away && t < away + gone);
                if present && polls > 0 && t % polls == 0 {
                    last_poll = Some(t);
                }
                if present && t % every == 0 {
                    for _ in 0..take {
                        if bytes.pop_front().is_none() { break }
                        behind = behind.saturating_sub(1);
                        last_poll = Some(t);
                    }
                }
                let attached = last_poll.is_some_and(|p| t - p < 3000);
                // (the game: a Present recorded, then waiting until may_go)
                if waiting_since.is_none() {
                    behind += 1;
                    bytes.push_back(sizes[frame % sizes.len()]);
                    frame += 1;
                    waiting_since = Some(t);
                }
                let total: usize = bytes.iter().sum();
                let past = t - waiting_since.unwrap() >= 250;
                if may_go(behind, total, past, attached) {
                    waiting_since = None;
                }
                proptest::prop_assert!(behind <= BACKLOG_FRAMES + 1, "{} frames behind at {} ms", behind, t);
                proptest::prop_assert!(total <= BACKLOG_BYTES + (96 << 20), "{} bytes queued at {} ms", total, t);
                if behind <= 2 {
                    proptest::prop_assert!(may_go(behind, total, false, false));
                }
            }
        }
    }
}
