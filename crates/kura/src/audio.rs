//! The sound card behind sake's audio APIs (waveOut, DirectSound, XAudio2).
//!
//! Each render stream queues its frames here. The wall clock plays them: on
//! every call the elapsed time is mixed (44.1 kHz stereo f32) into `out`, which the
//! page reads from the ring (`vnhost_audio_ring`) into WebAudio. Under node nobody drains it and
//! `out` just stays capped, so the timing games wait on is the same in both.

use kome::num::*;
#[cfg(test)]
use kome::num::to_f32;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

/// the driver's mix format too: most VN audio is 44.1 kHz, which dsound then mixes without
/// its FIR resampler (costly in x87 code); the browser converts to the device's rate
pub const RATE: u32 = 44100;
/// frames kept for the page at most (older ones are dropped)
const OUT_CAP: usize = RATE as usize / 4;
/// frames a stream queues at most (4 s): a writer running ahead of real time (whole buffers at
/// once, a decoder pre-filling) has the rest refused instead of growing the queue for ever
pub const QUEUE_CAP: usize = RATE as usize * 4;
/// frames mixed at most per catch-up (half the ring): after a stall the worklet would otherwise
/// get more than the ring holds at once, overwriting itself and lapping its read position
const CATCH_UP: usize = RING_FRAMES / 2;

struct Stream {
    /// stereo frames at RATE
    q: VecDeque<[f32; 2]>,
}

struct Card {
    streams: HashMap<i32, Stream>,
    next_id: i32,
    /// time up to which `out` has been mixed
    clock: Instant,
    out: VecDeque<[f32; 2]>,
}

static CARD: Mutex<Option<Card>> = Mutex::new(None);

fn with<T>(f: impl FnOnce(&mut Card) -> T) -> T {
    let mut g = CARD.lock().unwrap_or_else(|e| e.into_inner());
    let c = g.get_or_insert_with(|| Card { streams: HashMap::new(), next_id: 1, clock: Instant::now(), out: VecDeque::new() });
    c.advance();
    f(c)
}

impl Card {
    /// Mixes the time elapsed since `clock`.
    fn advance(&mut self) {
        let now = Instant::now();
        let n = (now.duration_since(self.clock).as_secs_f64() * f64::from(RATE)) as u64;
        if n == 0 {
            return;
        }
        self.clock += std::time::Duration::from_nanos(n * 1_000_000_000 / u64::from(RATE));
        if self.streams.values().all(|s| s.q.is_empty()) {
            return;
        }
        // a long stall (a hung page, a debugger, a worker descheduled) doesn't replay seconds at
        // once, nor more than the ring takes
        let n = usize::try_from(n).unwrap_or(usize::MAX).min(CATCH_UP);
        let start = self.out.len();
        self.out.resize(start + n, [0.0; 2]);
        for s in self.streams.values_mut() {
            let k = s.q.len().min(n);
            for (o, f) in self.out.range_mut(start..).zip(s.q.drain(..k)) {
                o[0] += f[0];
                o[1] += f[1];
            }
        }
        if RING_ON.load(Ordering::Relaxed) {
            ring_put(&mut self.out);
        }
        if self.out.len() > OUT_CAP {
            let extra = self.out.len() - OUT_CAP;
            self.out.drain(..extra);
        }
    }
}

/// A new stream of stereo f32 frames at RATE.
pub fn open() -> i32 {
    // (always the first: in every run's log, whether the game opened any sound at all)
    static FIRST: std::sync::Once = std::sync::Once::new();
    FIRST.call_once(|| eprintln!("sake: audio: the first sound stream opened (the page plays the ring: {})", RING_ON.load(Ordering::Relaxed)));
    if stats() {
        eprintln!("vnvm audio: open");
    }
    with(|c| {
        let id = c.next_id;
        c.next_id += 1;
        c.streams.insert(id, Stream { q: VecDeque::new() });
        id
    })
}

pub fn close(id: i32) {
    with(|c| c.streams.remove(&id));
}

/// Frames of stream `id` not played yet.
pub fn queued(id: i32) -> u32 {
    with(|c| c.streams.get(&id).map_or(0, |s| lo32(s.q.len())))
}

/// Queues interleaved stereo frames; how many it took (up to [`QUEUE_CAP`] queued).
pub fn write(id: i32, frames: &[f32]) -> usize {
    with(|c| {
        let Some(s) = c.streams.get_mut(&id) else { return 0 };
        let before = s.q.len();
        let take = (frames.len() / 2).min(QUEUE_CAP.saturating_sub(before));
        s.q.extend(frames[..take * 2].chunks_exact(2).map(|f| [f[0], f[1]]));
        if stats() {
            let (sum, n) = s.q.range(before..).fold((0f64, 0usize), |(a, n), f| (a + f64::from(f[0] * f[0]), n + 1));
            static ACC: Mutex<(f64, usize)> = Mutex::new((0.0, 0));
            let mut acc = ACC.lock().unwrap_or_else(|e| e.into_inner());
            acc.0 += sum;
            acc.1 += n;
            if acc.1 >= us(RATE) * 5 {
                eprintln!("vnvm audio: rms {:.4} over {} frames", (acc.0 / acc.1 as f64).sqrt(), acc.1);
                *acc = (0.0, 0);
            }
        }
        take
    })
}

/// MASU_AUDIO_STATS=1: the level of what the streams write, every 5 s of frames
fn stats() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("MASU_AUDIO_STATS").is_some())
}

/// Frames in the ring the page's AudioWorklet reads (a power of two; 0.74 s).
const RING_FRAMES: usize = 1 << 15;

/// What the page plays: mixed frames (interleaved stereo f32, as their bits) at
/// `head % RING_FRAMES`, `head` counting every frame written. The AudioWorklet reads it straight
/// from the shared wasm memory with its own read position (web/src/audio.ts), so nothing goes
/// through the page's own thread, where a long frame would make the sound stutter.
///
/// The hand-over (review 2026-11-17: what kept the writer off the frames the reader was reading
/// was nowhere written down): the worklet takes `head` (Acquire, after the frames: Release), and
/// reads a block of at most [`BLOCK`] frames lying at most [`READER_LAG`] behind it (it skips
/// ahead otherwise); one mix writes at most [`CATCH_UP`] frames. So the frames a block reads are
/// not written again by the next mix: `CATCH_UP + READER_LAG + BLOCK <= RING_FRAMES` (asserted
/// below; `ring_tests` plays the protocol, with a mix between every head load and the read).
/// Only a worklet that stops inside one block for longer than the mixes' slack (~0.17 s of sound)
/// can read a frame written over: a click, never memory unsafety (the samples are atomics here
/// and plain numbers in JS).
#[repr(C)]
struct Ring {
    head: AtomicU32,
    _pad: u32,
    data: [AtomicU32; RING_FRAMES * 2],
}
// (the page's AudioWorklet reads the ring at these offsets: head, pad, then the samples; an
// AtomicU32 is a u32 in memory, as the Float32Array view over it expects)
const _: () = assert!(size_of::<Ring>() == 8 + RING_FRAMES * 2 * 4);
const _: () = assert!(size_of::<AtomicU32>() == size_of::<f32>() && align_of::<AtomicU32>() == align_of::<f32>());

/// The most frames the worklet reads per block (an AudioWorklet render quantum)
const BLOCK: usize = 128;
/// The most frames it lags behind `head` before it skips ahead (web/src/audio.ts MAX_QUEUE)
const READER_LAG: usize = RATE as usize / 5;
const _: () = assert!(CATCH_UP + READER_LAG + BLOCK <= RING_FRAMES);

static RING: Ring = Ring { head: AtomicU32::new(0), _pad: 0, data: [const { AtomicU32::new(0) }; RING_FRAMES * 2] };
static RING_ON: AtomicBool = AtomicBool::new(false);

fn ring_put(out: &mut VecDeque<[f32; 2]>) {
    put_frames(&RING, out.drain(..));
}

/// The frames into `ring` at its head, then the head past them (Release: the worklet reads up
/// to head)
fn put_frames(ring: &Ring, frames: impl ExactSizeIterator<Item = [f32; 2]>) {
    let head = ring.head.load(Ordering::Relaxed);
    let n = lo32(frames.len());
    for (i, f) in frames.enumerate() {
        let at = (us(head.wrapping_add(lo32(i))) & (RING_FRAMES - 1)) * 2;
        ring.data[at].store(f[0].to_bits(), Ordering::Relaxed);
        ring.data[at + 1].store(f[1].to_bits(), Ordering::Relaxed);
    }
    ring.head.store(head.wrapping_add(n), Ordering::Release);
}

/// The page: from now on mixed frames go to the ring; returns its address (head at +0, frames
/// from +8) and writes its size in frames to `frames`.
///
/// # Safety
/// `frames` is a writable u32.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vnhost_audio_ring(frames: *mut u32) -> *const u8 {
    *frames = lo32(RING_FRAMES);
    RING_ON.store(true, Ordering::Relaxed);
    std::ptr::from_ref(&RING).cast::<u8>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The worklet's limits as web/src/audio.ts has them, the ones the ring's assert counts on
    #[test]
    fn reader_limits_match_the_worklet() {
        let ts = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/audio.ts")).unwrap();
        assert!(ts.contains("const MAX_QUEUE = RATE / 5;"), "READER_LAG is web/src/audio.ts's MAX_QUEUE");
        assert!(ts.contains("const RATE = 44100;"));
        assert_eq!(READER_LAG, us(RATE) / 5);
    }

    /// sinkStep's choice of block (web/src/audio.ts): from `read`, skipping to `target` behind
    /// `head` when it lags more than `max`; the frames it reads
    fn block(read: &mut Option<u32>, head: u32, max: u32, target: u32) -> (u32, u32) {
        let mut r = read.unwrap_or(head.wrapping_sub(target));
        let mut avail = head.wrapping_sub(r);
        if avail > max {
            r = head.wrapping_sub(target);
            avail = target;
        }
        let n = avail.min(lo32(BLOCK));
        *read = Some(r.wrapping_add(n));
        (r, n)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        /// The hand-over played: mixes of any size up to CATCH_UP, the worklet taking head, a mix
        /// landing before it reads its block, then the block: every frame it reads is the one
        /// written at that position, none written over (each frame's samples carry its number).
        #[test]
        fn a_block_is_never_written_over_by_the_next_mix(steps in prop::collection::vec((0..=lo32(CATCH_UP), 0..=lo32(CATCH_UP), any::<bool>()), 1..60)) {
            let ring: &'static Ring = Box::leak(Box::new(Ring { head: AtomicU32::new(0), _pad: 0, data: [const { AtomicU32::new(0) }; RING_FRAMES * 2] }));
            let mut written = 0u32;
            let mut put = |n: u32| {
                put_frames(ring, (0..n).map(|i| { let k = written.wrapping_add(i); [f32::from_bits(k), f32::from_bits(!k)] }).collect::<Vec<_>>().into_iter());
                written = written.wrapping_add(n);
            };
            let mut read = None;
            for (before, between, skip) in steps {
                put(before);
                let head = ring.head.load(Ordering::Acquire);
                if skip {
                    continue;
                }
                let (r, n) = block(&mut read, head, lo32(READER_LAG), RATE / 20);
                // (the mix that lands between the worklet's head load and its reads)
                put(between);
                // (frames before the first written, where the worklet starts behind head 0: none to check)
                for k in (0..n).map(|i| r.wrapping_add(i)) {
                    if k < head {
                        let at = (us(k) & (RING_FRAMES - 1)) * 2;
                        prop_assert_eq!(ring.data[at].load(Ordering::Relaxed), k, "frame {} written over", k);
                        prop_assert_eq!(ring.data[at + 1].load(Ordering::Relaxed), !k);
                    }
                }
            }
        }
    }

    /// (the card is one static)
    static SERIAL: Mutex<()> = Mutex::new(());

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]
        /// Writes of any sizes: a stream never holds more than QUEUE_CAP frames, every write
        /// takes what fits, and what it took is queued in order.
        #[test]
        fn queue_is_bounded(sizes in proptest::collection::vec(0usize..200_000, 1..8)) {
            let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            let id = open();
            for (k, n) in sizes.into_iter().enumerate() {
                let before = us(queued(id));
                let frames = vec![to_f32(k); n * 2];
                let took = write(id, &frames);
                prop_assert!(took <= n);
                let after = us(queued(id));
                prop_assert!(after <= QUEUE_CAP);
                // (the wall clock plays some meanwhile)
                prop_assert!(after + us(RATE) / 2 >= (before + took).min(QUEUE_CAP));
                prop_assert!(took == n || before + took >= QUEUE_CAP - us(RATE) / 2);
            }
            close(id);
        }
    }

    /// After a stall the mix catches up with at most half the ring (it was up to a second, more
    /// than the ring's 0.74 s: the worklet's read position lapped).
    #[test]
    fn catch_up_fits_the_ring() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut card = Card {
            streams: [(1, Stream { q: VecDeque::from(vec![[0.5; 2]; CATCH_UP * 2]) })].into(),
            next_id: 2,
            clock: Instant::now() - std::time::Duration::from_secs(3),
            out: VecDeque::new(),
        };
        let head = RING.head.load(Ordering::Relaxed);
        RING_ON.store(true, Ordering::Relaxed);
        card.advance();
        RING_ON.store(false, Ordering::Relaxed);
        assert_eq!(us(RING.head.load(Ordering::Relaxed).wrapping_sub(head)), CATCH_UP);
    }
}
