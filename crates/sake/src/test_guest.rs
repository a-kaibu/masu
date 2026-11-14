//! Test-only guest memory and x86 argument frames; never linked into the runtime.
use crate::{api::{Ctx, Handler}, mem};
use std::sync::{Mutex, MutexGuard, Once};
static SERIAL: Mutex<()> = Mutex::new(());
pub fn enter() -> MutexGuard<'static, ()> {
    let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    static MAP: Once = Once::new();
    MAP.call_once(|| unsafe {
        let p = libc::mmap(mem::LO as usize as *mut _, (mem::HI - mem::LO) as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_FIXED_NOREPLACE | libc::MAP_NORESERVE, -1, 0);
        assert_eq!(p as usize, mem::LO as usize, "guest memory: {}", std::io::Error::last_os_error());
        mem::TEST_MAPPED.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    assert_eq!(mem::ps(), 4);
    guard
}
pub fn call(f: Handler, args: &[u32]) -> u32 {
    let sp = mem::heap_alloc(4 + 4 * args.len() as u32, true);
    for (i, &a) in args.iter().enumerate() { mem::w32(sp + 4 + 4 * i as u32, a); }
    let mut cpu = koji::Cpu::default();
    let result = f(&mut Ctx { cpu: &mut cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None });
    mem::heap_free(sp);
    result
}
/// Makes the calling test thread a guest thread (a TEB: last_error, TLS), once per host thread.
pub fn guest_thread() {
    thread_local!(static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });
    if !DONE.with(|d| d.replace(true)) {
        crate::enter(0).expect("TEB and stack");
    }
}
/// A behaviour snapshot: `out` against src/golden/<name>.txt, the first differing lines shown.
/// Pins what a module does before its code changes shape (MASU_UPDATE_GOLDEN=1 rewrites it).
pub fn snapshot(name: &str, out: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("golden").join(format!("{name}.txt"));
    if std::env::var_os("MASU_UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, out).unwrap();
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    if want != out {
        let d: Vec<String> = want.lines().zip(out.lines()).enumerate().filter(|(_, (a, b))| a != b).take(20)
            .map(|(i, (a, b))| format!("{}:\n- {a}\n+ {b}", i + 1)).collect();
        panic!("golden/{name}.txt differs ({} lines vs {}; MASU_UPDATE_GOLDEN=1 rewrites it):\n{}", want.lines().count(), out.lines().count(), d.join("\n"));
    }
}
/// Deterministic test inputs (xorshift64*), so a snapshot sweep is the same on every run.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[usize::try_from(self.below(u64::try_from(v.len()).unwrap_or(u64::MAX))).unwrap_or(0)]
    }
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next().to_le_bytes()[0]).collect()
    }
}
/// FNV-1a of guest bytes (a snapshot line for a buffer).
pub fn hash(a: u32, n: u32) -> u64 {
    mem::slice(a, n).iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
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
    let result = f(&mut Ctx { cpu, esp: sp, edx: None, jumped: false, st0: None, trap: 0, this: None });
    mem::heap_free(sp);
    mem::heap_free(stack);
    result
}

/// Guest memory a test owns: reserved and committed while it lives, released (and so zeroed)
/// when it is dropped. Borrowing the guard of `enter` it cannot outlive the test's hold on the
/// guest. A test that wrote at a fixed address instead (ddraw's blit tests at 0x0100_0000)
/// left its bytes in free space, which the next reservation there showed as its contents
/// (#242: a new pagefile section read 0xdd513c44 in k32::mapping_names::model).
pub struct Block<'g> {
    pub at: u32,
    _guest: std::marker::PhantomData<&'g MutexGuard<'static, ()>>,
}
impl<'g> Block<'g> {
    pub fn new(_guest: &'g MutexGuard<'static, ()>, size: u32) -> Self {
        let Some(at) = mem::reserve(0, size, 4, 0x2_0000, false) else { panic!("test block of {size:#x} bytes: no room") };
        mem::commit(at, 4);
        Block { at, _guest: std::marker::PhantomData }
    }
}
impl Drop for Block<'_> {
    fn drop(&mut self) {
        assert!(mem::release(self.at), "test block {:#x} was released by someone else", self.at);
    }
}
