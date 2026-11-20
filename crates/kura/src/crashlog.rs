//! The crash log: what a run did lately, kept where the page reads it straight from the linear
//! memory (web/src/crashlog.ts), so a crash can be diagnosed afterwards even when the wasm
//! instance is gone (a trap, an abort) or the tab gets killed (the page saves it every few
//! seconds).
//!
//! - `EV`: events. Every guest exception (access violation, illegal instruction, int3, divide
//!   error, ...) as it leaves the x86 core, with the registers, the code and stack there, the
//!   frame chain and return-address candidates resolved to `module!export+off`, the thread's
//!   last syscalls / API calls and the JIT's recent region jumps; process starts and ends;
//!   panics.
//! - `OUT`: everything written to stdout/stderr (sake's messages, its "Unhandled
//!   exception" report).
//! - `SNAP`: every guest thread's state now ([`vnhost_crashlog_snapshot`]), rewritten each time.
//!
//! Everything another thread reads here is an atomic (review 2026-11-15: no `unsafe impl Sync`
//! left): the rings' bytes, each thread's trap ring, and the registers a thread shows. A writer
//! reserves its bytes with one `fetch_add`, so lines from different threads do not interleave
//! while the ring is not lapped; a writer that stops while others go once round the ring leaves
//! mixed text, never a data race. Another thread's `Cpu` is never read: a guest thread shows its
//! registers ([`shown`]) at every slice and every exit from guest code, and the snapshot (and
//! MASU_TRACE_EIP) reads those. Nor is another thread's guest memory or JIT state (review
//! 2026-11-16: an atomic read here against sake's plain writes on another thread is still a
//! data race): guest memory is read only through an [`Own`], which only a registered guest
//! thread gets, for itself; a thread shows its return addresses with its registers, and the
//! modules and their exports are copied when sake maps them, before their code runs. Nothing on these paths allocates (the page may ask for a
//! snapshot while another thread died holding the allocator's lock), except the sake API-name
//! lookup in a guest thread's own exception record.
//!
//! The cost while nothing goes wrong: four stores per syscall (the thread's trap ring), ~24 per
//! exit from guest code and per slice (one guest read more: its return addresses are shown
//! again only when ESP, EBP or [ESP] changed), and a copy of each stderr write.

use core::fmt::{self, Write};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::Mutex;

use kome::locks::Lock as _;
use koji::Cpu;

const EV_SIZE: usize = 1 << 19;
const OUT_SIZE: usize = 1 << 19;
const SNAP_SIZE: usize = 1 << 19;
/// Syscalls / API calls kept per thread.
const TRAPS: usize = 128;

/// The address the page finds `x` at (wasm32: an address is a u32)
fn addr_of<T: ?Sized>(x: &T) -> u32 {
    kome::num::lo32(std::ptr::from_ref(x).cast::<u8>().expose_provenance())
}

/// Bytes the page reads as text: the same layout as `[u8; N]`, every byte an atomic.
struct Bytes<const N: usize>([AtomicU8; N]);

impl<const N: usize> Bytes<N> {
    const fn new() -> Self {
        Bytes([const { AtomicU8::new(0) }; N])
    }
    /// Byte `i` (wrapping at N)
    #[inline]
    fn put(&self, i: usize, b: u8) {
        if let Some(x) = self.0.get(i % N) {
            x.store(b, Relaxed);
        }
    }
    fn get(&self, i: usize) -> u8 {
        self.0.get(i % N).map_or(0, |x| x.load(Relaxed))
    }
    /// The first `n` bytes, copied
    fn text(&self, n: usize) -> String {
        let v: Vec<u8> = (0..n.min(N)).map(|i| self.get(i)).collect();
        String::from_utf8_lossy(&v).into_owned()
    }
}

/// A ring of text: `head` counts every byte ever put (the page takes it modulo N).
struct Ring<const N: usize> {
    buf: Bytes<N>,
    head: AtomicU32,
}

impl<const N: usize> Ring<N> {
    // (N divides 2^32: the byte index wraps with the u32 head)
    const OK: () = assert!(N.is_power_of_two() && N <= 1 << 31);

    const fn new() -> Self {
        let () = Self::OK;
        Ring { buf: Bytes::new(), head: AtomicU32::new(0) }
    }

    fn put(&self, b: &[u8]) {
        let b = b.get(..b.len().min(N / 4)).unwrap_or(b);
        let at = kome::num::us(self.head.fetch_add(kome::num::u32_of(b.len()), Relaxed));
        for (i, &c) in b.iter().enumerate() {
            self.buf.put(at.wrapping_add(i), c);
        }
    }

    fn addr(&self) -> u32 {
        addr_of(&self.buf)
    }
}

static EV: Ring<EV_SIZE> = Ring::new();
static OUT: Ring<OUT_SIZE> = Ring::new();

static SNAP: Bytes<SNAP_SIZE> = Bytes::new();
static SNAP_LEN: AtomicU32 = AtomicU32::new(0);
static SNAP_BUSY: AtomicBool = AtomicBool::new(false);

/// Crashes seen (the page polls it) and the latest one's reason.
static CRASHES: AtomicU32 = AtomicU32::new(0);
const REASON_SIZE: usize = 256;
static REASON: Bytes<REASON_SIZE> = Bytes::new();

static INFO: [AtomicU32; 16] = [const { AtomicU32::new(0) }; 16];

/// Where the page finds everything: magic "VNCL", version, then (address, size, address of
/// the head) for EV and OUT, (address, size, address of the length) for SNAP, the address of
/// the crash counter, of the reason (NUL-terminated) and its size.
#[unsafe(no_mangle)]
pub extern "C" fn vnhost_crashlog_info() -> u32 {
    let size = kome::num::u32_of;
    let v = [
        0x4c43_4e56,
        1,
        EV.addr(),
        size(EV_SIZE),
        addr_of(&EV.head),
        OUT.addr(),
        size(OUT_SIZE),
        addr_of(&OUT.head),
        addr_of(&SNAP),
        size(SNAP_SIZE),
        addr_of(&SNAP_LEN),
        addr_of(&CRASHES),
        addr_of(&REASON),
        size(REASON_SIZE),
        0,
        0,
    ];
    for (i, x) in INFO.iter().zip(v) {
        i.store(x, Relaxed);
    }
    addr_of(&INFO)
}

// ---------------------------------------------------------------------------------------------
// text without allocating

/// A line built on the stack (what does not fit is cut).
pub struct Line<const N: usize> {
    buf: [u8; N],
    n: usize,
}

impl<const N: usize> Line<N> {
    pub fn new() -> Self {
        Line { buf: [0; N], n: 0 }
    }
    pub fn bytes(&self) -> &[u8] {
        self.buf.get(..self.n).unwrap_or(&self.buf)
    }
    pub fn push(&mut self, b: &[u8]) {
        for (d, &s) in self.buf.iter_mut().skip(self.n).zip(b) {
            *d = s;
            self.n = self.n.saturating_add(1);
        }
    }
}

impl<const N: usize> Write for Line<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes());
        Ok(())
    }
}

/// "hh:mm:ss.mmm" (UTC) and the thread: the prefix of every record.
fn stamp(w: &mut impl Write) {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| kome::num::lo64(d.as_millis()));
    let s = ms / 1000;
    let _ = write!(
        w,
        "{:02}:{:02}:{:02}.{:03} t{} ",
        (s / 3600) % 24,
        (s / 60) % 60,
        s % 60,
        ms % 1000,
        crate::cpu::tid()
    );
}

/// An event line (or lines) in EV.
pub fn event(args: fmt::Arguments) {
    let mut l = Line::<1024>::new();
    stamp(&mut l);
    let _ = l.write_fmt(args);
    let _ = l.write_str("\n");
    EV.put(l.bytes());
}

/// A crash the page should save a report for at once.
pub fn crash(reason: &str) {
    let n = reason.len().min(REASON_SIZE.saturating_sub(1));
    for (i, &c) in reason.as_bytes().iter().take(n).enumerate() {
        REASON.put(i, c);
    }
    REASON.put(n, 0);
    event(format_args!("CRASH: {reason}"));
    CRASHES.fetch_add(1, SeqCst);
}

/// A write to stdout/stderr. A report of an unhandled exception is a crash.
pub fn output(fd: i32, b: &[u8]) {
    if b.is_empty() {
        return;
    }
    let mut l = Line::<40>::new();
    stamp(&mut l);
    if fd == 1 {
        let _ = l.write_str("[out] ");
    }
    OUT.put(l.bytes());
    OUT.put(b);
    if b.last() != Some(&b'\n') {
        OUT.put(b"\n");
    }
    // "Unhandled exception: ..."
    let find = |pat: &[u8]| b.windows(pat.len()).any(|w| w == pat);
    if find(b"Unhandled exception: ") {
        // the thread writing this is the one whose exception nobody handled: its stack still
        // has the code that raised it
        thread_state("the thread reporting the unhandled exception");
        let s = String::from_utf8_lossy(b.get(..b.len().min(250)).unwrap_or(b));
        crash(s.trim_end());
    }
}

/// The calling guest thread's state (as it showed it last: at its current syscall), deep, in EV.
fn thread_state(why: &str) {
    let Some(t) = CUR.with(Cell::get) else { return };
    let Some(o) = Own::here() else { return };
    let r = Regs::of(t);
    let mut l = Line::<16384>::new();
    stamp(&mut l);
    let mods = &mut [Module { base: 0, size: 0, name: [0; 40], len: 0 }; MAX_MODULES];
    let n = modules(mods);
    let mods = mods.get(..n).unwrap_or(&[]);
    let _ = write!(l, "{why}: at its syscall from {:08x}", rd32(o, r.sp()).unwrap_or(0));
    sym(&mut l, mods, rd32(o, r.sp()).unwrap_or(0));
    regs(&mut l, &r);
    detail(&mut l, &r, None, Some(o), Some(t), mods, true, DEEP);
    module_list(&mut l, mods, false);
    let _ = l.write_str("\n");
    EV.put(l.bytes());
}

// ---------------------------------------------------------------------------------------------
// guest threads

/// Where in the run loop a thread is: a thread that stands still outside guest code and outside
/// an API shows what it waits in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Phase {
    Nowhere,
    Park,
    JitRun,
    JitMaybeCompile,
    Interpreter,
    JitTakeBuilt,
    JitSharedFn,
    JitSavedFn,
    JitShared,
    JitSaved,
    JitCompile,
}

impl Phase {
    const ALL: [Phase; 11] = [
        Phase::Nowhere,
        Phase::Park,
        Phase::JitRun,
        Phase::JitMaybeCompile,
        Phase::Interpreter,
        Phase::JitTakeBuilt,
        Phase::JitSharedFn,
        Phase::JitSavedFn,
        Phase::JitShared,
        Phase::JitSaved,
        Phase::JitCompile,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Phase::Nowhere => "",
            Phase::Park => "park",
            Phase::JitRun => "jit::run",
            Phase::JitMaybeCompile => "jit::maybe_compile",
            Phase::Interpreter => "interpreter",
            Phase::JitTakeBuilt => "jit: take_built",
            Phase::JitSharedFn => "jit: shared fn",
            Phase::JitSavedFn => "jit: saved fn",
            Phase::JitShared => "jit: shared",
            Phase::JitSaved => "jit: saved",
            Phase::JitCompile => "jit: compile",
        }
    }
    fn of(v: u8) -> Phase {
        Phase::ALL.iter().copied().find(|p| *p as u8 == v).unwrap_or(Phase::Nowhere)
    }
}

/// The registers a guest thread showed last (its own Cpu is read by it alone).
struct Shown {
    gpr: [AtomicU64; 16],
    eip: AtomicU32,
    rip_hi: AtomicU32,
    flags: AtomicU32,
    fs_base: AtomicU32,
    gs_base: AtomicU32,
    long: AtomicBool,
    jit_ring: AtomicU32,
    /// [ESP] and the EBP chain's return addresses (32-bit code), `frames_n` of them, as the
    /// thread read them itself; (esp, ebp, [esp]) they were read at, so as not to walk again
    frames: [AtomicU32; FRAMES],
    frames_n: AtomicU32,
    frames_at: [AtomicU32; 3],
}

/// Return addresses a thread shows (the snapshot's frames of another thread)
const FRAMES: usize = 16;

/// The calling thread is the guest thread whose guest memory and JIT state is read: only it
/// reads them, at its own points (its exception, its syscall, its slice), never another thread
/// (the page's snapshot, MASU_TRACE_EIP's), so no read here meets a write of sake's on another
/// thread (review 2026-11-16). Not Send: it stays on the thread that took it.
#[derive(Clone, Copy)]
struct Own(std::marker::PhantomData<*const ()>);

impl Own {
    /// A registered guest thread (crashlog::register) is the caller: None on any other.
    fn here() -> Option<Own> {
        CUR.with(Cell::get).map(|_| Own(std::marker::PhantomData))
    }
    /// The image sake has just mapped and not run (add_module, on the thread loading it): no
    /// other code knows of it yet to write it.
    fn mapping() -> Own {
        Own(std::marker::PhantomData)
    }
}

/// A copy of what a thread showed (or of its own Cpu)
#[derive(Clone, Copy, Default)]
struct Regs {
    gpr: [u64; 16],
    eip: u32,
    rip_hi: u32,
    flags: u32,
    fs_base: u32,
    gs_base: u32,
    long: bool,
    jit_ring: u32,
}

impl Regs {
    fn from_cpu(c: &Cpu) -> Regs {
        Regs {
            gpr: koji::Reg64::ALL.map(|r| c.gpr(r)),
            eip: c.eip,
            rip_hi: c.rip_hi,
            flags: c.flags(),
            fs_base: c.fs_base,
            gs_base: c.gs_base,
            long: c.mode64 != 0,
            jit_ring: c.jit_ring,
        }
    }
    fn of(t: &ThreadLog) -> Regs {
        let s = &t.shown;
        Regs {
            gpr: std::array::from_fn(|i| s.gpr.get(i).map_or(0, |x| x.load(Relaxed))),
            eip: s.eip.load(Relaxed),
            rip_hi: s.rip_hi.load(Relaxed),
            flags: s.flags.load(Relaxed),
            fs_base: s.fs_base.load(Relaxed),
            gs_base: s.gs_base.load(Relaxed),
            long: s.long.load(Relaxed),
            jit_ring: s.jit_ring.load(Relaxed),
        }
    }
    /// Register `i`'s low 32 bits
    fn r(&self, i: usize) -> u32 {
        self.gpr.get(i).map_or(0, |&v| kome::num::lo32(v))
    }
    fn sp(&self) -> u32 {
        self.r(4)
    }
}

impl Shown {
    const fn new() -> Self {
        Shown {
            gpr: [const { AtomicU64::new(0) }; 16],
            eip: AtomicU32::new(0),
            rip_hi: AtomicU32::new(0),
            flags: AtomicU32::new(0),
            fs_base: AtomicU32::new(0),
            gs_base: AtomicU32::new(0),
            long: AtomicBool::new(false),
            jit_ring: AtomicU32::new(0),
            frames: [const { AtomicU32::new(0) }; FRAMES],
            frames_n: AtomicU32::new(0),
            frames_at: [const { AtomicU32::new(0) }; 3],
        }
    }
    /// The calling thread's return addresses at `r` (its own registers), read by itself; not
    /// again while ESP, EBP and [ESP] are what they were.
    fn set_frames(&self, o: Own, r: &Regs) {
        let (esp, ebp) = (r.sp(), r.r(5));
        let top = rd32(o, esp).unwrap_or(0);
        let at = [esp, ebp, top];
        if self.frames_n.load(Relaxed) != 0 && self.frames_at.iter().zip(at).all(|(x, v)| x.load(Relaxed) == v) {
            return;
        }
        let mut out = [0u32; FRAMES];
        let mut n = 0;
        if let Some(x) = out.first_mut() {
            *x = top;
            n = 1;
        }
        if !r.long {
            let mut ebp = ebp;
            while n < FRAMES {
                let (Some(ret), Some(next)) = (rd32(o, ebp.wrapping_add(4)), rd32(o, ebp)) else { break };
                if let Some(x) = out.get_mut(n) {
                    *x = ret;
                }
                n += 1;
                if next <= ebp {
                    break;
                }
                ebp = next;
            }
        }
        for (d, v) in self.frames.iter().zip(out) {
            d.store(v, Relaxed);
        }
        self.frames_n.store(kome::num::u32_of(n), Relaxed);
        for (d, v) in self.frames_at.iter().zip(at) {
            d.store(v, Relaxed);
        }
    }
    /// What set_frames showed last: [ESP] first, then the frames' return addresses
    fn frames(&self) -> ([u32; FRAMES], usize) {
        let n = kome::num::us(self.frames_n.load(Relaxed)).min(FRAMES);
        (std::array::from_fn(|i| self.frames.get(i).map_or(0, |x| x.load(Relaxed))), n)
    }
    fn set(&self, r: &Regs) {
        for (d, &v) in self.gpr.iter().zip(&r.gpr) {
            d.store(v, Relaxed);
        }
        self.eip.store(r.eip, Relaxed);
        self.rip_hi.store(r.rip_hi, Relaxed);
        self.flags.store(r.flags, Relaxed);
        self.fs_base.store(r.fs_base, Relaxed);
        self.gs_base.store(r.gs_base, Relaxed);
        self.long.store(r.long, Relaxed);
        self.jit_ring.store(r.jit_ring, Relaxed);
    }
}

struct ThreadLog {
    tid: AtomicI32,
    /// its Cpu's address: which thread this is, never read through
    cpu: AtomicUsize,
    /// 1 while it runs guest code, 0 in sake (a syscall, an API call)
    guest: AtomicU32,
    /// (trap address, EAX, [ESP], [ESP+4]) of its last syscalls / API calls
    traps: [[AtomicU32; 4]; TRAPS],
    n: AtomicU32,
    exceptions: AtomicU32,
    phase: AtomicU8,
    shown: Shown,
}

impl ThreadLog {
    fn new() -> Self {
        ThreadLog {
            tid: AtomicI32::new(0),
            cpu: AtomicUsize::new(0),
            guest: AtomicU32::new(0),
            traps: [const { [const { AtomicU32::new(0) }; 4] }; TRAPS],
            n: AtomicU32::new(0),
            exceptions: AtomicU32::new(0),
            phase: AtomicU8::new(0),
            shown: Shown::new(),
        }
    }
    fn trap(&self, k: u32) -> [u32; 4] {
        self.traps.get(kome::num::us(k) % TRAPS).map_or([0; 4], |e| std::array::from_fn(|i| e.get(i).map_or(0, |x| x.load(Relaxed))))
    }
}

/// The running threads' records (each lent by RETIRED)
static THREADS: Mutex<Vec<crate::pool::Lease<&'static ThreadLog, Retired>>> = Mutex::new(Vec::new());

thread_local! {
    static CUR: Cell<Option<&'static ThreadLog>> = const { Cell::new(None) };
}

fn id(cpu: *const Cpu) -> usize {
    cpu.expose_provenance()
}

/// The calling thread got `cpu` (cpu::create).
pub fn register(cpu: &Cpu) {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let mut l = Line::<512>::new();
            let _ = write!(l, "panic: {info}");
            crash(std::str::from_utf8(l.bytes()).unwrap_or("panic"));
            prev(info);
        }));
    });
    let me = id(cpu);
    let mut g = THREADS.locked_anyway();
    // (a record of a thread that ended is the next owner's: only that thread writes it)
    let t = RETIRED.locked_anyway().take(|| Box::leak(Box::new(ThreadLog::new()))).lease();
    t.tid.store(crate::cpu::tid(), Relaxed);
    t.cpu.store(me, Relaxed);
    t.guest.store(0, Relaxed);
    t.n.store(0, Relaxed);
    t.exceptions.store(0, Relaxed);
    t.phase.store(Phase::Nowhere as u8, Relaxed);
    t.shown.set(&Regs::from_cpu(cpu));
    t.shown.frames_n.store(0, Relaxed);
    CUR.with(|c| c.set(Some(*t)));
    let mut r = RETIRED.locked_anyway();
    for old in g.extract_if(.., |x| x.cpu.load(Relaxed) == me) {
        r.give(old);
    }
    drop(r);
    g.push(t);
}

// The records of threads that ended, for the next threads (they used to be leaked, one per
// thread: ~2 KB each; review P2-1). Kept, never freed: the snapshot reads entries it listed
// after letting go of THREADS.
crate::pool!(
    /// The records of threads that ended (RETIRED) and their brand
    mod retired: &'static ThreadLog as Retired
);
use retired::{Retired, POOL as RETIRED};

/// (tests: the records waiting for reuse, made, the most out at once)
pub fn counts() -> crate::pool::Counts {
    RETIRED.locked_anyway().counts()
}

/// `cpu` is going away (cpu::destroy).
pub fn unregister(cpu: *const Cpu) {
    let me = id(cpu);
    let mut g = THREADS.locked_anyway();
    let mut r = RETIRED.locked_anyway();
    for t in g.extract_if(.., |x| x.cpu.load(Relaxed) == me) {
        t.cpu.store(0, Relaxed);
        r.give(t);
    }
    drop(r);
    CUR.with(|c| {
        if c.get().is_some_and(|t| t.cpu.load(Relaxed) == 0) {
            c.set(None)
        }
    });
}

/// Where the calling thread is in the run loop now, shown in the snapshot.
#[inline]
pub fn phase(what: Phase) {
    CUR.with(|c| {
        if let Some(t) = c.get() {
            t.phase.store(what as u8, Relaxed);
        }
    });
}

/// The calling thread enters (true) or leaves guest code.
#[inline]
pub fn in_guest(on: bool) {
    CUR.with(|c| {
        if let Some(t) = c.get() {
            t.guest.store(u32::from(on), Relaxed);
        }
    });
}

/// The calling thread's registers now, for the snapshot and MASU_TRACE_EIP (cpu.rs: at every
/// slice and every exit from guest code). `cpu` is the calling thread's own.
#[inline]
pub fn shown(cpu: &Cpu) {
    CUR.with(|c| {
        if let Some(t) = c.get() {
            let r = Regs::from_cpu(cpu);
            t.shown.set(&r);
            // (CUR is set: this is a registered guest thread, reading its own stack)
            t.shown.set_frames(Own(std::marker::PhantomData), &r);
        }
    });
}

/// The guest got to a trap address (a syscall, a unix call, a sake API).
#[inline]
pub fn trap(cpu: &Cpu, addr: u32) {
    CUR.with(|c| {
        let Some(t) = c.get() else { return };
        // (CUR is set: a registered guest thread, its own stack)
        let o = Own(std::marker::PhantomData);
        let esp = cpu.regs[koji::ESP];
        let n = t.n.load(Relaxed);
        if let Some(e) = t.traps.get(kome::num::us(n) % TRAPS) {
            for (d, v) in e.iter().zip([addr, cpu.regs[koji::EAX], rd32(o, esp).unwrap_or(0), rd32(o, esp.wrapping_add(4)).unwrap_or(0)]) {
                d.store(v, Relaxed);
            }
        }
        t.n.store(n.wrapping_add(1), Relaxed);
    });
}

/// MASU_TRACE_EIP's line: each thread's EIP as it showed it last, `*` when it is in sake (a
/// syscall, an API call), then the return addresses ([esp], the EBP chain).
pub fn eip_line() -> String {
    let g = THREADS.locked_anyway();
    let line: Vec<String> = g
        .iter()
        .map(|t| {
            let r = Regs::of(t);
            let at = if t.guest.load(Relaxed) != 0 { "" } else { "*" };
            // (what the thread showed itself: another thread's stack is not read)
            let (frames, n) = t.shown.frames();
            let stack: Vec<String> = frames.iter().take(n.min(9)).map(|a| format!("{a:x}")).collect();
            format!("{}:{:08x}{at}<{}>", t.tid.load(Relaxed), r.eip, stack.join(","))
        })
        .collect();
    line.join(" ")
}

// ---------------------------------------------------------------------------------------------
// guest memory, modules, symbols

fn mem_end() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) as u64 * 65536
    }
    #[cfg(not(target_family = "wasm"))]
    {
        0
    }
}

fn readable(a: u32, n: u32) -> bool {
    a >= 0x10000 && u64::from(a).saturating_add(u64::from(n)) <= mem_end().min(u64::from(koji::jit::GUEST_HI))
}

/// `N` bytes of guest memory, read as atomics: the guest's threads write it meanwhile (as the
/// x86 they emulate allows), and a crash report reads whatever is there.
fn rd<const N: usize>(_: Own, a: u32) -> Option<[u8; N]> {
    if !readable(a, kome::num::u32_of(N)) {
        return None;
    }
    Some(std::array::from_fn(|i| {
        let p = std::ptr::with_exposed_provenance_mut::<u8>(kome::num::us(a).wrapping_add(i));
        // SAFETY: readable() above: guest memory, inside the wasm memory, valid for as long as
        // it is; any address is aligned for an AtomicU8. The Own says it is the calling guest
        // thread's own memory, read at its own point: no Rust code of another thread is
        // writing it (as for every API's read of what the program handed it).
        unsafe { AtomicU8::from_ptr(p) }.load(Relaxed)
    }))
}

fn rd32(o: Own, a: u32) -> Option<u32> {
    rd::<4>(o, a).map(u32::from_le_bytes)
}

fn rd16(o: Own, a: u32) -> Option<u16> {
    rd::<2>(o, a).map(u16::from_le_bytes)
}

fn rd8(o: Own, a: u32) -> Option<u8> {
    rd::<1>(o, a).map(|[b]| b)
}

#[derive(Clone, Copy)]
struct Module {
    base: u32,
    size: u32,
    name: [u8; 40],
    len: u8,
}

impl Module {
    fn name(&self) -> &str {
        std::str::from_utf8(&self.name[..self.len as usize]).unwrap_or("?")
    }
}

const MAX_MODULES: usize = 128;

/// Modules sake loaded (it keeps no loader list in the PEB).
static EXTRA: Mutex<Vec<Module>> = Mutex::new(Vec::new());

fn make_module(base: u32, size: u32, name: impl Iterator<Item = u8>) -> Module {
    let mut m = Module { base, size, name: [0; 40], len: 0 };
    for c in name.take(40) {
        m.name[m.len as usize] = c;
        m.len += 1;
    }
    m
}

/// The exports of the modules sake mapped, copied then: a record never reads an image (sake's
/// loader writes images and relocates them on other threads, review 2026-11-16)
struct Exports {
    base: u32,
    /// (rva, name), by rva
    list: Vec<(u32, Box<str>)>,
}

static EXPORTS: Mutex<Vec<Exports>> = Mutex::new(Vec::new());

/// sake: an image it mapped at `base` from `path` (before any of its code runs).
pub fn add_module(base: u32, path: &str) {
    let o = Own::mapping();
    let size = rd32(o, base.wrapping_add(rd32(o, base.wrapping_add(0x3c)).unwrap_or(0)).wrapping_add(0x50)).unwrap_or(0x1000);
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let m = make_module(base, size, name.bytes().filter(|c| c.is_ascii_graphic()));
    let list = read_exports(o, base, size).unwrap_or_default();
    let mut g = EXTRA.locked_anyway();
    g.retain(|x| x.base != base);
    if g.len() < MAX_MODULES {
        g.push(m);
        drop(g);
        let mut e = EXPORTS.locked_anyway();
        e.retain(|x| x.base != base);
        e.push(Exports { base, list });
    }
}

/// The modules sake mapped, as add_module copied them (sake keeps no loader list in the PEB;
/// a guest's own list would be written by its loader on other threads)
fn modules(out: &mut [Module; MAX_MODULES]) -> usize {
    let mut n = 0;
    // (try_lock: a thread may have died holding it)
    if let Ok(g) = EXTRA.try_lock() {
        for (d, m) in out.iter_mut().zip(g.iter()) {
            *d = *m;
            n += 1;
        }
    }
    n
}

/// The named exports of the image at `base` (`size` bytes): (rva, name) by rva.
fn read_exports(o: Own, base: u32, size: u32) -> Option<Vec<(u32, Box<str>)>> {
    // (all of it read from guest memory: wrapping, as the rest of the file; with overflow
    // checks a damaged header aborted the process from inside the crash logger)
    let pe = base.wrapping_add(rd32(o, base.wrapping_add(0x3c))?);
    if rd32(o, pe)? != 0x4550 {
        return None;
    }
    let oh = pe.wrapping_add(24);
    let dir = if rd16(o, oh)? == 0x20b { oh.wrapping_add(112) } else { oh.wrapping_add(96) };
    let exp = rd32(o, dir)?;
    if exp == 0 || exp >= size {
        return None;
    }
    let ed = base.wrapping_add(exp);
    let names = rd32(o, ed.wrapping_add(0x18))?.min(20000);
    let at = |off: u32| -> Option<u32> { Some(base.wrapping_add(rd32(o, ed.wrapping_add(off))?)) };
    let (funcs, name_tab, ords) = (at(0x1c)?, at(0x20)?, at(0x24)?);
    let mut list = Vec::new();
    for i in 0..names {
        let ord = u32::from(rd16(o, ords.wrapping_add(i.wrapping_mul(2)))?);
        let f = rd32(o, funcs.wrapping_add(ord.wrapping_mul(4)))?;
        let name = base.wrapping_add(rd32(o, name_tab.wrapping_add(i.wrapping_mul(4)))?);
        let text: String = (0..64u32).map_while(|k| rd8(o, name.wrapping_add(k)).filter(|c| (0x20..0x80).contains(c))).map(char::from).collect();
        list.push((f, text.into_boxed_str()));
    }
    list.sort_by_key(|e| e.0);
    Some(list)
}

/// `addr` as "module+rva (export+off)", or nothing outside the modules.
fn sym(w: &mut impl Write, mods: &[Module], addr: u32) {
    let Some(m) = mods.iter().find(|m| addr >= m.base && addr - m.base < m.size) else { return };
    let rva = addr.wrapping_sub(m.base);
    let _ = write!(w, " {}+{rva:x}", m.name());
    // (try_lock: a thread may have died holding it)
    let Ok(g) = EXPORTS.try_lock() else { return };
    let Some(e) = g.iter().find(|e| e.base == m.base) else { return };
    let k = e.list.partition_point(|x| x.0 <= rva);
    if let Some((f, name)) = k.checked_sub(1).and_then(|k| e.list.get(k)) {
        let _ = write!(w, " ({name}+{:x})", rva.wrapping_sub(*f));
    }
}

/// sake: the API at a trap address (crates/sake api::name_of).
static NAMER: std::sync::OnceLock<fn(u32) -> Option<String>> = std::sync::OnceLock::new();

pub fn set_trap_namer(f: fn(u32) -> Option<String>) {
    let _ = NAMER.set(f);
}

// ---------------------------------------------------------------------------------------------
// records

fn regs(w: &mut impl Write, cpu: &Regs) {
    let r: [u32; 8] = std::array::from_fn(|i| cpu.r(i));
    if cpu.long {
        const N: [&str; 16] = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
        for (i, n) in N.iter().enumerate() {
            let _ = write!(w, "{}{n} {:016x}", if i % 4 == 0 { "\n  " } else { " " }, cpu.gpr.get(i).copied().unwrap_or(0));
        }
        let _ = write!(w, "\n  rip {:08x}{:08x} rflags {:08x} gs {:08x} fs {:08x}", cpu.rip_hi, cpu.eip, cpu.flags, cpu.gs_base, cpu.fs_base);
    } else {
        let _ = write!(
            w,
            "\n  eax {:08x} ecx {:08x} edx {:08x} ebx {:08x} esp {:08x} ebp {:08x} esi {:08x} edi {:08x}\n  eip {:08x} eflags {:08x} fs {:08x}",
            r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7], cpu.eip, cpu.flags, cpu.fs_base
        );
    }
}

/// How far `detail` looks: stack bytes scanned for code addresses, how many it lists, frames.
#[derive(Clone, Copy)]
struct Depth {
    scan: u32,
    found: u32,
    frames: u32,
}

const NORMAL: Depth = Depth { scan: 0x800, found: 24, frames: 16 };
/// The thread that reports an unhandled exception: its stack holds the dispatcher's frames
/// (records, contexts) above the code that raised it.
const DEEP: Depth = Depth { scan: 0x6000, found: 64, frames: 40 };

/// `own`: the calling thread's own Cpu (its FPU, SSE and JIT state too), None for what a
/// thread showed (`t`: its record, for its last calls; `name`: with the API names). `mem`: the
/// calling thread is the one `cpu` is of, and reads its code, stack and JIT ring itself; None
/// (another thread's record) shows only the return addresses that thread showed.
fn detail(w: &mut impl Write, cpu: &Regs, own: Option<&Cpu>, mem: Option<Own>, t: Option<&ThreadLog>, mods: &[Module], name: bool, depth: Depth) {
    let long = cpu.long;
    if let Some(c) = own {
        let f = &c.fpu;
        let _ = write!(w, "\n  fpu cw {:04x} sw {:04x} top {} empty {:02x} mxcsr {:08x}", f.cw, f.sw, f.top, f.empty, c.mxcsr);
        for (i, x) in c.xmm.iter().enumerate() {
            if *x != 0 {
                let _ = write!(w, " xmm{i} {x:032x}");
            }
        }
        if long {
            for (i, x) in c.xmm8.iter().enumerate() {
                if *x != 0 {
                    let _ = write!(w, " xmm{} {x:032x}", i.saturating_add(8));
                }
            }
        }
    }
    let Some(o) = mem else {
        // (another thread's: what it showed itself, its memory is not read here)
        if let Some(t) = t {
            let (frames, n) = t.shown.frames();
            let _ = w.write_str("\n  return addresses ([esp], then the ebp chain, as the thread showed them):");
            for &a in frames.iter().take(n) {
                let _ = write!(w, "\n    {a:08x}");
                sym(w, mods, a);
            }
            traps(w, t, mods, name, 24);
        }
        jit_totals(w, own);
        return;
    };
    let _ = write!(w, "\n  code @{:08x}:", cpu.eip);
    for i in 0..16 {
        match rd8(o, cpu.eip.wrapping_add(i)) {
            Some(b) => {
                let _ = write!(w, " {b:02x}");
            }
            None => {
                let _ = w.write_str(" ??");
            }
        }
    }
    let esp = cpu.sp();
    let _ = write!(w, "\n  stack @{esp:08x}:");
    for i in 0..32u32 {
        if i % 8 == 0 {
            let _ = w.write_str("\n   ");
        }
        match rd32(o, esp.wrapping_add(4 * i)) {
            Some(v) => {
                let _ = write!(w, " {v:08x}");
            }
            None => {
                let _ = w.write_str(" ????????");
            }
        }
    }
    // the frame chain (32-bit code mostly keeps EBP frames)
    if !long {
        let _ = w.write_str("\n  frames (ebp chain):");
        let mut ebp = cpu.r(5);
        for _ in 0..depth.frames {
            let (Some(ret), Some(next)) = (rd32(o, ebp.wrapping_add(4)), rd32(o, ebp)) else { break };
            let _ = write!(w, "\n    {ret:08x}");
            sym(w, mods, ret);
            if next <= ebp {
                break;
            }
            ebp = next;
        }
    }
    // stack words that point into a module: the return addresses (and some function pointers)
    let _ = w.write_str("\n  code addresses on the stack:");
    let mut found = 0;
    let step = if long { 8 } else { 4 };
    let mut a = esp;
    while a < esp.wrapping_add(depth.scan) && found < depth.found {
        let Some(v) = rd32(o, a) else { break };
        if long && rd32(o, a + 4).unwrap_or(1) > 0xffff {
            a += step;
            continue;
        }
        if mods.iter().any(|m| v >= m.base && v - m.base < m.size && v - m.base >= 0x1000) {
            let _ = write!(w, "\n    [{:x}] {v:08x}", a - esp);
            sym(w, mods, v);
            found += 1;
        }
        a += step;
    }
    // the last syscalls / API calls
    if let Some(t) = t {
        traps(w, t, mods, name, 24);
    }
    // the JIT: its recent jumps between regions (from, to), newest first
    if cpu.jit_ring != 0 {
        let ring = cpu.jit_ring;
        if let Some(n) = rd32(o, ring) {
            let _ = w.write_str("\n  jit region jumps (newest first):");
            let size = koji::jit::RING_SIZE;
            for k in 0..n.min(size).min(16) {
                let i = (n.wrapping_sub(1 + k)) & (size - 1);
                let (a, b) = (rd32(o, ring + 4 + 8 * i).unwrap_or(0), rd32(o, ring + 8 + 8 * i).unwrap_or(0));
                let _ = write!(w, " {a:x}>{b:x}");
            }
        }
    }
    jit_totals(w, own);
}

fn jit_totals(w: &mut impl Write, own: Option<&Cpu>) {
    let c = crate::jit::compiled_totals();
    let _ = write!(w, "\n  jit: on {} (x64 {}), regions built {}, functions {}, rebuilt {}, call depth {:?}", crate::jit::enabled(), crate::jit::enabled64(), c[0], c[1], c[2], own.map(|c| c.jit_depth));
    let m = crate::jit::jit_modules();
    let _ = write!(w, "\n  jit modules: loaded {} ({} KiB, {} ms), freed {} ({} KiB), table {}", m[0], m[1] >> 10, m[5] / 1000, m[3], m[4] >> 10, m[2]);
}

fn traps(w: &mut impl Write, t: &ThreadLog, mods: &[Module], name: bool, max: u32) {
    let n = t.n.load(Relaxed);
    let _ = write!(w, "\n  last syscalls/API calls (newest first, {n} in all): trap eax [esp] [esp+4]");
    // (a call repeated, or two alternating — a loop's — is shown once with a count)
    let mut shown: [[u32; 4]; 2] = [[0; 4]; 2];
    let mut repeats = 0;
    let mut lines = 0;
    for k in 0..n.min(TRAPS as u32) {
        let e = t.trap(n.wrapping_sub(k.wrapping_add(1)));
        if lines > 0 && (e == shown[0] || e == shown[1]) {
            repeats += 1;
            continue;
        }
        if repeats > 0 {
            let _ = write!(w, "\n    (repeated {repeats} more times)");
            repeats = 0;
        }
        if lines == max {
            break;
        }
        shown = [e, shown[0]];
        lines += 1;
        let [addr, eax, r0, r1] = e;
        let _ = write!(w, "\n    {addr:08x} {eax:08x}");
        if name && let Some(s) = NAMER.get().and_then(|f| f(addr)) {
            let _ = write!(w, " {s}");
        }
        let _ = write!(w, " | {r0:08x}");
        sym(w, mods, r0);
        let _ = write!(w, " | {r1:08x}");
        sym(w, mods, r1);
    }
    if repeats > 0 {
        let _ = write!(w, "\n    (repeated {repeats} more times)");
    }
}

/// A hash of the modules last listed: a record lists them again only when they changed.
static LISTED: Mutex<u32> = Mutex::new(0);

fn module_list(w: &mut impl Write, mods: &[Module], always: bool) {
    let h = mods.iter().fold(0x811c9dc5u32, |h, m| (h ^ m.base ^ m.size).wrapping_mul(0x0100_0193));
    if !always {
        let mut g = LISTED.locked_anyway();
        if *g == h {
            let _ = write!(w, "\n  modules: as listed before ({})", mods.len());
            return;
        }
        *g = h;
    }
    let _ = write!(w, "\n  modules ({}):", mods.len());
    for (i, m) in mods.iter().enumerate() {
        let _ = write!(w, "{}{:08x}-{:08x} {}", if i % 3 == 0 { "\n   " } else { "  " }, m.base, m.base.wrapping_add(m.size), m.name());
    }
}

/// A guest exception as the x86 core stops on it (before sake's SEH sees it).
/// `what` names it, `access` is (address, write) for access violations.
pub fn exception(cpu: &Cpu, what: &str, access: Option<(u32, bool)>, from_jit: bool) {
    let t = CUR.with(Cell::get);
    let k = t.map_or(0, |t| t.exceptions.fetch_add(1, Relaxed));
    // all of the first ones of a thread, then one in 100 (programs that use exceptions for
    // control flow); the others a line without the modules and symbols, decided first: walking
    // the loader list and the exports was ~10^4-10^5 reads before the guest's SEH started
    if !(k < 16 || k % 100 == 0) {
        let mut l = Line::<256>::new();
        stamp(&mut l);
        let _ = write!(l, "exception #{} {what} at {:08x} [{}]", k.saturating_add(1), cpu.eip, if from_jit { "jit" } else { "interpreter" });
        if let Some((a, wr)) = access {
            let _ = write!(l, " {} {a:08x}", if wr { "writing" } else { "reading" });
        }
        let _ = l.write_str("\n");
        EV.put(l.bytes());
        return;
    }
    let mut l = Line::<12288>::new();
    stamp(&mut l);
    let mods = &mut [Module { base: 0, size: 0, name: [0; 40], len: 0 }; MAX_MODULES];
    let r = Regs::from_cpu(cpu);
    let n = modules(mods);
    let mods = mods.get(..n).unwrap_or(&[]);
    let _ = write!(l, "exception #{} {what} at {:08x}", k.saturating_add(1), cpu.eip);
    sym(&mut l, mods, cpu.eip);
    let _ = write!(l, " [{}]", if from_jit { "jit" } else { "interpreter" });
    if let Some((a, wr)) = access {
        let _ = write!(l, " {} {a:08x}", if wr { "writing" } else { "reading" });
    }
    regs(&mut l, &r);
    // (the faulting thread's own: it reads its code and stack; a thread sake did not register
    // shows its registers only)
    detail(&mut l, &r, Some(cpu), Own::here(), t, mods, true, NORMAL);
    module_list(&mut l, mods, false);
    let _ = l.write_str("\n");
    EV.put(l.bytes());
}

/// Writes every guest thread's state into SNAP; returns its length (0 if busy).
#[unsafe(no_mangle)]
pub extern "C" fn vnhost_crashlog_snapshot() -> u32 {
    if SNAP_BUSY.swap(true, Acquire) {
        return 0;
    }
    let mut w = SnapWriter { n: 0 };
    stamp(&mut w);
    // (a fixed list: another thread may have died holding the allocator's lock)
    let mut list: [Option<&'static ThreadLog>; 256] = [None; 256];
    let mut count = 0;
    match THREADS.try_lock() {
        Ok(g) => {
            // (the newest first: the list keeps every thread ever made, so after enough of them
            // the first 256 are long dead and the live, crashing ones would not be in it)
            for (s, t) in list.iter_mut().zip(g.iter().rev()) {
                *s = Some(**t);
                count += 1;
            }
        }
        Err(_) => {
            let _ = w.write_str("(thread list busy) ");
        }
    }
    let _ = write!(w, "threads now: {count}");
    let mut listed = false;
    for &t in list.iter().flatten() {
        if t.cpu.load(Relaxed) == 0 {
            continue;
        }
        let cpu = Regs::of(t);
        let mods = &mut [Module { base: 0, size: 0, name: [0; 40], len: 0 }; MAX_MODULES];
        let n = modules(mods);
        let mods = mods.get(..n).unwrap_or(&[]);
        let _ = write!(
            w,
            "\n\nthread t{} {} at {:08x}",
            t.tid.load(Relaxed),
            if t.guest.load(Relaxed) != 0 { "in guest code" } else { "in a syscall/API (host side)" },
            cpu.eip
        );
        let phase = Phase::of(t.phase.load(Relaxed));
        if phase != Phase::Nowhere {
            let _ = write!(w, " [run loop: {}]", phase.name());
        }
        sym(&mut w, mods, cpu.eip);
        let _ = write!(w, ", exceptions {}", t.exceptions.load(Relaxed));
        regs(&mut w, &cpu);
        // (not the page's thread's: what each thread showed, no memory of theirs)
        detail(&mut w, &cpu, None, None, Some(t), mods, false, NORMAL);
        if !listed {
            listed = true;
            module_list(&mut w, mods, true);
        }
    }
    let _ = w.write_str("\n");
    let n = kome::num::u32_of(w.n);
    SNAP_LEN.store(n, Release);
    SNAP_BUSY.store(false, Release);
    n
}

/// Every guest thread's state as text (SAKE_THREADS_STACKS in node, where no page reads SNAP).
pub fn snapshot_text() -> String {
    SNAP.text(kome::num::us(vnhost_crashlog_snapshot()))
}

struct SnapWriter {
    n: usize,
}

impl Write for SnapWriter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for &c in s.as_bytes().iter().take(SNAP_SIZE.saturating_sub(self.n)) {
            SNAP.put(self.n, c);
            self.n = self.n.saturating_add(1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod snapshot_tests {
    #[test]
    fn snapshot_text_lists_the_threads() {
        let t = super::snapshot_text();
        assert!(t.contains("threads now: "), "{t}");
        // (again: the snapshot is rewritten, not appended to)
        let u = super::snapshot_text();
        assert_eq!(u.matches("threads now: ").count(), 1, "{u}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// (the rings, the thread list and SNAP are statics)
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn ev_text() -> String {
        EV.buf.text(EV.head.load(Relaxed) as usize)
    }

    /// The first 16 exceptions of a thread and every 100th have the registers and modules; the
    /// others are one line, decided before any of that work.
    #[test]
    fn exceptions_past_the_first_are_one_line() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // (the test's own Cpu, freed once unregistered: LeakSanitizer reports one left, #242)
        let mut owned = Box::new(Cpu { eip: 0x0040_1234, ..Cpu::default() });
        let cpu: *mut Cpu = &mut *owned;
        register(&owned);
        let before = EV.head.load(Relaxed);
        for _ in 0..120 {
            exception(&owned, "access violation", Some((0x10, false)), true);
        }
        let all = ev_text();
        let ours = &all[before as usize..];
        let records: Vec<&str> = ours.split("exception #").skip(1).collect();
        assert_eq!(records.len(), 120);
        for (k, r) in records.iter().enumerate() {
            let full = k < 16 || k % 100 == 0;
            assert_eq!(r.contains("eax "), full, "#{}: {r}", k + 1);
            if !full {
                // (its one line, then the next one's time stamp)
                assert!(r.lines().count() <= 2, "#{}: {r}", k + 1);
                assert!(r.lines().next().unwrap().contains("at 00401234 [jit] reading 00000010"), "{r}");
            }
        }
        unregister(cpu);
        drop(owned);
    }

    /// Review 2026-11-16: only a registered guest thread gets an Own (reads guest memory), and
    /// for itself; a snapshot taken on another thread shows what each thread showed, never its
    /// code or stack read from there.
    #[test]
    fn other_threads_memory_is_not_read() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        assert!(std::thread::spawn(|| Own::here().is_none()).join().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let h = std::thread::spawn(move || {
            let owned = Box::new(Cpu { eip: 0x0061_2345, ..Cpu::default() });
            register(&owned);
            shown(&owned);
            tx.send(Own::here().is_some()).unwrap();
            done_rx.recv().unwrap();
            unregister(std::ptr::from_ref(&*owned));
        });
        assert!(rx.recv().unwrap());
        let want = format!("at {:08x}", 0x0061_2345);
        let snap = (0..10_000).map(|_| snapshot_text()).find(|t| t.contains(&want)).expect("the thread in a snapshot");
        let ours = snap.split("\n\nthread ").find(|t| t.contains(&want)).unwrap();
        assert!(ours.contains("return addresses"), "{ours}");
        assert!(!ours.contains("code @") && !ours.contains("stack @"), "{ours}");
        done_tx.send(()).unwrap();
        h.join().unwrap();
    }

    /// With more threads made than the snapshot takes (the list keeps every one), the newest
    /// are in it.
    #[test]
    fn snapshot_has_the_newest_threads() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut cpus: Vec<Box<Cpu>> = (0..300).map(|i| Box::new(Cpu { eip: 0x0050_0000 + i, ..Cpu::default() })).collect();
        for c in &mut cpus {
            register(c);
        }
        // (the snapshot only try_locks the thread list, as a crashing thread may hold it, and
        // other tests' Cpus register meanwhile: then it says "(thread list busy)" and a later
        // one is taken; SNAP_BUSY likewise, #242)
        let newest = format!("at {:08x}", 0x0050_0000 + 299);
        let seen = (0..10_000).any(|_| {
            let ok = snapshot_text().contains(&newest);
            if !ok {
                std::thread::yield_now();
            }
            ok
        });
        assert!(seen, "the newest thread is not in the snapshot");
        for c in &mut cpus {
            unregister(&**c);
        }
        // (freed once unregistered, #242)
        drop(cpus);
    }

    /// (no `unsafe impl Sync` makes these shareable: their parts are atomics)
    const _: fn() = || {
        fn sync<T: Sync>() {}
        sync::<ThreadLog>();
        sync::<Ring<16>>();
        sync::<Bytes<16>>();
    };

    /// Threads that write the rings, show registers and trap while others take snapshots and
    /// trace lines (review 2026-11-15: the rings and the trap ring were plain bytes written by
    /// several threads, and the snapshot read other threads' Cpus as they ran). Run it under
    /// ThreadSanitizer (tools/ci sanitize job) to see it race free; here: every line written
    /// whole while the ring is not lapped, and nothing panics.
    #[test]
    fn writers_and_snapshots_at_once() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let stop = AtomicBool::new(false);
        // (lines each writer wrote: the snapshots go on until every one has written some; CI
        // 2026-11-16 took its 200 before a writer ran, and the ring had no writer line)
        let wrote = [const { std::sync::atomic::AtomicU32::new(0) }; 4];
        std::thread::scope(|s| {
            for k in 0..4u32 {
                let stop = &stop;
                let wrote = &wrote;
                s.spawn(move || {
                    let mut cpu = Box::new(Cpu { eip: 0x0060_0000 + k, ..Cpu::default() });
                    register(&cpu);
                    let mut i = 0u32;
                    while !stop.load(Relaxed) {
                        cpu.regs[koji::EAX] = i;
                        cpu.eip = 0x0060_0000 + k + (i << 8);
                        shown(&cpu);
                        trap(&cpu, 0xfe00_0000 + i);
                        phase(Phase::ALL[i as usize % Phase::ALL.len()]);
                        event(format_args!("writer {k} line {i}"));
                        output(2, format!("writer {k} out {i}\n").as_bytes());
                        i = i.wrapping_add(1);
                        if let Some(w) = wrote.get(kome::num::us(k)) {
                            w.store(i, Relaxed);
                        }
                    }
                    unregister(&*cpu);
                });
            }
            let mut n = 0u32;
            while n < 200 || wrote.iter().any(|w| w.load(Relaxed) < 16) {
                let t = snapshot_text();
                assert!(t.is_empty() || t.contains("threads now: "), "{t}");
                let _ = eip_line();
                n = n.saturating_add(1);
            }
            stop.store(true, Relaxed);
        });
        // (the ring holds the last EV_SIZE bytes; a line not cut by the ring's start reads whole)
        let text = EV.buf.text(EV_SIZE);
        let lines: Vec<&str> = text.lines().filter(|l| l.contains(" writer ")).collect();
        assert!(!lines.is_empty());
        let whole = lines.iter().filter(|l| l.split(" writer ").nth(1).is_some_and(|r| {
            let w: Vec<&str> = r.split(' ').collect();
            w.len() == 3 && w[1] == "line" && w[0].parse::<u32>().is_ok() && w[2].parse::<u32>().is_ok()
        })).count();
        // (only the line the ring's start cuts, and lines of other tests' threads, may differ)
        assert!(whole + 2 >= lines.len(), "{whole} of {} whole", lines.len());
    }

    proptest! {
        /// Writes that do not lap the ring read back as written, in order, whatever their
        /// lengths and wherever the head starts (also past 2^32 bytes: the head wraps)
        #[test]
        fn ring_keeps_what_fits(start: u32, msgs in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..20), 0..10)) {
            let r = Ring::<256>::new();
            r.head.store(start, Relaxed);
            for m in &msgs {
                r.put(m);
            }
            let all: Vec<u8> = msgs.concat();
            prop_assume!(all.len() <= 256);
            let got: Vec<u8> = (0..all.len()).map(|i| r.buf.get((start as usize).wrapping_add(i))).collect();
            prop_assert_eq!(got, all);
            prop_assert_eq!(r.head.load(Relaxed), start.wrapping_add(msgs.iter().map(|m| m.len() as u32).sum::<u32>()));
        }

        /// A write longer than a quarter of the ring is cut to that
        #[test]
        fn ring_cuts_long_writes(n in 0usize..1000) {
            let r = Ring::<256>::new();
            r.put(&vec![7; n]);
            prop_assert_eq!(r.head.load(Relaxed) as usize, n.min(64));
        }

        /// Any phase stored and read back is the same one
        #[test]
        fn phase_round_trips(i in 0usize..Phase::ALL.len()) {
            let p = Phase::ALL[i];
            prop_assert_eq!(Phase::of(p as u8), p);
        }

        /// Any module and address (a damaged PE header in guest memory, a module at the top
        /// of the address space): no overflow panic (tests check overflow).
        #[test]
        fn export_near_any_module(base: u32, size: u32, addr: u32) {
            let m = make_module(base, size, b"x.dll".iter().copied());
            let _ = read_exports(Own::mapping(), base, size);
            let mut l = Line::<256>::new();
            sym(&mut l, &[m], addr);
        }
    }
}
