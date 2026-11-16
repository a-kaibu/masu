//! Test-only guest memory and x86 argument frames; never linked into the runtime.
use crate::{api::{Ctx, Handler}, mem};
use std::sync::{Mutex, MutexGuard, OnceLock};
static SERIAL: Mutex<()> = Mutex::new(());
/// The top of the test guest memory: GUEST_TOP, or under AddressSanitizer the page below its
/// shadow memory, which starts at 0x7fff8000 on x86-64 Linux (inside sake's stub area: the
/// mapping failed with EEXIST and every guest test after it, review 2026-11-16)
pub const TOP: u32 = if cfg!(masu_sanitize) { 0x7fff_0000 } else { mem::GUEST_TOP };
pub fn enter() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    kura::outs::fail_reports();
    // (the outcome kept: a map that failed fails every test with its own reason, not with a
    // poisoned Once, review 2026-11-16)
    static MAP: OnceLock<Result<(), String>> = OnceLock::new();
    let mapped = MAP.get_or_init(|| {
        // (up to GUEST_TOP: the stub area above HI too; below TOP under a sanitizer)
        // SAFETY: a fixed anonymous mapping where nothing is (NOREPLACE: refused, not replaced,
        // where something is), never unmapped.
        let p = unsafe {
            libc::mmap(mem::LO as usize as *mut _, (TOP - mem::LO) as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE | libc::MAP_NORESERVE, -1, 0)
        };
        if p as usize != mem::LO as usize {
            return Err(format!("guest memory {:#x}..{TOP:#x}: {}", mem::LO, std::io::Error::last_os_error()));
        }
        mem::TEST_MAPPED.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    });
    if let Err(e) = mapped {
        panic!("{e}");
    }
    assert_eq!(mem::ps(), 4);
    guard
}
pub fn call(f: Handler, args: &[u32]) -> u32 {
    let sp = mem::heap_alloc(4 + 4 * args.len() as u32, true);
    for (i, &a) in args.iter().enumerate() { mem::w32(sp + 4 + 4 * i as u32, a); }
    let mut cpu = koji::Cpu::default();
    let result = f(&mut Ctx { cpu: &mut cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: 0 });
    mem::heap_free(sp);
    result
}
/// Makes the calling test thread a guest thread (a TEB: last_error, TLS), once per host thread.
pub fn guest_thread() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if !DONE.with(|d| d.replace(true)) {
        crate::enter(0, |_| 0).expect("TEB and stack");
    }
}
/// A behaviour snapshot: `out` against src/golden/<name>.txt, the first differing lines shown.
/// Pins what a module does before its code changes shape (MASU_UPDATE_GOLDEN=1 rewrites it).
pub fn snapshot(name: &str, out: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("golden").join(format!("{name}.txt"));
    kome::golden::check(&path, out);
}
/// Deterministic test inputs (xorshift64*), so a snapshot sweep is the same on every run.
pub use kome::golden::Rng;
/// FNV-1a of guest bytes (a snapshot line for a buffer).
pub fn hash(a: u32, n: u32) -> u64 {
    kome::golden::Fnv::of(&mem::bytes(a, n))
}
/// `call` with a guest stack under the CPU, for a handler that calls back into guest code
/// (Ctx::call; a host_fn trap stands in for the guest's function).
pub fn call_stacked(f: Handler, args: &[u32]) -> u32 {
    let stack = mem::heap_alloc(0x10000, true);
    let sp = mem::heap_alloc(4 + 4 * u32::try_from(args.len()).unwrap_or(0), true);
    for (i, &a) in (0u32..).zip(args) { mem::w32(sp + 4 + 4 * i, a); }
    // (kura's CPU: its run loop stops at sake's traps)
    let cpu = unsafe { &mut *kura::cpu::create(crate::api::TRAP_LO, crate::api::TRAP_HI) };
    cpu.regs[koji::ESP] = stack + 0xfff0;
    let result = f(&mut Ctx { cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: 0 });
    mem::heap_free(sp);
    mem::heap_free(stack);
    result
}
/// `call_stacked`, and the double the handler returns in st0 (None: it set none).
pub fn call_st0(f: Handler, args: &[u32]) -> (u32, Option<f64>) {
    let stack = mem::heap_alloc(0x10000, true);
    let sp = mem::heap_alloc(4 + 4 * u32::try_from(args.len()).unwrap_or(0), true);
    for (i, &a) in (0u32..).zip(args) { mem::w32(sp + 4 + 4 * i, a); }
    let cpu = unsafe { &mut *kura::cpu::create(crate::api::TRAP_LO, crate::api::TRAP_HI) };
    cpu.regs[koji::ESP] = stack + 0xfff0;
    let mut c = Ctx { cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None, outs: 0 };
    let result = f(&mut c);
    let st0 = c.st0;
    mem::heap_free(sp);
    mem::heap_free(stack);
    (result, st0)
}

/// Guest memory a test owns: reserved and committed while it lives, released (and so zeroed)
/// when it is dropped. Borrowing the guard of `enter` it cannot outlive the test's hold on the
/// guest. A test that wrote at a fixed address instead (ddraw's blit tests at 0x0100_0000)
/// left its bytes in free space, which the next reservation there showed as its contents
/// (#242: a new pagefile section read 0xdd513c44 in k32::mapping_names::model). From the top
/// of the address space, away from where the guest heap grows (snapshots show heap addresses).
pub struct Block<'g> {
    pub at: u32,
    _guest: std::marker::PhantomData<&'g MutexGuard<'static, ()>>,
}
impl<'g> Block<'g> {
    pub fn new(_guest: &'g MutexGuard<'static, ()>, size: u32) -> Self {
        let Some(at) = mem::reserve(0, size, 4, 0x2_0000, true) else { panic!("test block of {size:#x} bytes: no room") };
        mem::commit(at, 4);
        Block { at, _guest: std::marker::PhantomData }
    }
}
impl Drop for Block<'_> {
    fn drop(&mut self) {
        assert!(mem::release(self.at), "test block {:#x} was released by someone else", self.at);
    }
}
