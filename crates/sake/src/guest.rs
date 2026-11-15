//! C library functions as guest x86 code (guest/crt.c, built by guest/gen-guest.sh): qsort and
//! bsearch call the game's comparison function directly (no trap per call), and the small
//! string/memory functions skip the trap entirely. Placed at a fixed address before anything
//! else is allocated. x64 programs get the x86-64 build, which also has the x64 forms of
//! fast.rs's stubs (critical sections, last error, TLS, small heap blocks, errno).

#[path = "guest_crt.rs"]
mod code;
#[path = "guest_crt64.rs"]
mod code64;

use crate::mem;
use std::sync::atomic::{AtomicBool, Ordering};

static PLACED: AtomicBool = AtomicBool::new(false);

/// Maps the code (read+execute: the JIT translates no writable code). VirtualQuery calls it
/// MEM_IMAGE: the system DLLs' functions it stands for (GetLastError, HeapAlloc, ...) are image
/// code on Windows, and programs that check their imports look (tests/apiimage.c).
pub fn init() {
    if crate::env_on!("SAKE_NO_GUEST_CRT") {
        return;
    }
    let (want, bytes) = if mem::x64() { (code64::AT, code64::CODE) } else { (code::AT, code::CODE) };
    let size = (bytes.len() as u32 + 0xfff) & !0xfff;
    let Some(at) = mem::reserve(want, size, 0x20, 0x100_0000, false) else {
        eprintln!("sake: no room for the guest C library at {want:#x}");
        return;
    };
    mem::commit(at, 0x20);
    mem::write(at, bytes);
    PLACED.store(true, Ordering::Relaxed);
}

/// Points the code's host slots at the traps (once the C library's Rust side is registered).
pub fn bind_host() {
    if !PLACED.load(Ordering::Relaxed) {
        return;
    }
    if mem::x64() {
        // (x64: the fast paths are in the guest code too, the traps behind them)
        for (slot, dll, name) in [
            ("host_memmove", "msvcrt", "memmove"),
            ("host_memset", "msvcrt", "memset"),
            ("host_realloc", "msvcrt", "realloc"),
            ("host_malloc", "msvcrt", "malloc"),
            ("host_free", "msvcrt", "free"),
            ("host_enter_cs", "kernel32", "EnterCriticalSection"),
            ("host_tls_get", "kernel32", "TlsGetValue"),
            ("host_heap_alloc", "kernel32", "HeapAlloc"),
            ("host_heap_free", "kernel32", "HeapFree"),
        ] {
            mem::w64(sym(slot).unwrap(), crate::api::export(dll, name) as u64);
        }
        mem::w64(sym("host_cs_wake").unwrap(), crate::api::host_fn("cs_wake64", cs_wake, 0) as u64);
        mem::w64(sym("slab_table").unwrap(), mem::slab_table() as u64);
        return;
    }
    // big copies and fills, realloc's other cases: the host's traps; malloc and free: the x86
    // stubs (small blocks from the free lists)
    for (slot, name, stub) in [
        ("host_memmove", "memmove", false),
        ("host_memset", "memset", false),
        ("host_realloc", "realloc", false),
        ("host_malloc", "malloc", true),
        ("host_free", "free", true),
    ] {
        let trap = crate::api::export("msvcrt", name);
        mem::w32(sym(slot).unwrap(), if stub { crate::fast::bind("msvcrt", name, trap) } else { trap });
    }
}

fn cs_wake(c: &mut crate::api::Ctx) -> u32 {
    crate::k32::cs_wake(c.a(0));
    0
}

fn sym(name: &str) -> Option<u32> {
    let syms = if mem::x64() { code64::SYMS } else { code::SYMS };
    syms.iter().find(|(n, _)| *n == name).map(|&(_, a)| a)
}

/// x64: the kernel32 / ntdll exports the guest code has fast paths of (and their Rtl names).
fn x64_fast(key: &str, name: &str) -> Option<&'static str> {
    if !matches!(key, "kernel32" | "kernelbase" | "ntdll") {
        return None;
    }
    Some(match name {
        "EnterCriticalSection" | "RtlEnterCriticalSection" => "EnterCriticalSection",
        "LeaveCriticalSection" | "RtlLeaveCriticalSection" => "LeaveCriticalSection",
        "GetLastError" | "RtlGetLastWin32Error" => "GetLastError",
        "SetLastError" | "RtlSetLastWin32Error" => "SetLastError",
        "TlsGetValue" => "TlsGetValue",
        "HeapAlloc" | "RtlAllocateHeap" => "HeapAlloc",
        "HeapFree" | "RtlFreeHeap" => "HeapFree",
        _ => return None,
    })
}

/// The guest code for C library export `name` of DLL `key` (dll_key), if there is one.
pub fn lookup(key: &str, name: &str) -> Option<u32> {
    if !PLACED.load(Ordering::Relaxed) || name.starts_with("host_") || name == "slab_table" {
        return None;
    }
    if mem::x64() {
        if let Some(n) = x64_fast(key, name) {
            return sym(n);
        }
    } else if matches!(name, "EnterCriticalSection" | "LeaveCriticalSection" | "GetLastError" | "SetLastError" | "TlsGetValue" | "HeapAlloc" | "HeapFree" | "malloc" | "free" | "_errno") {
        return None;
    }
    let crt = matches!(key, "msvcrt" | "ucrtbase" | "crtdll") || (key.starts_with("msvcr") && key[5..].bytes().all(|b| b.is_ascii_digit()));
    if !crt {
        return None;
    }
    sym(name)
}

/// The guest code's function `pc` is in, [start, the next one's) (guest/crt.c has no unwind
/// table; the JIT builds whole functions from where they start: kura jit::set_fn_start)
pub fn function_start(pc: u32) -> Option<std::ops::Range<u32>> {
    if !PLACED.load(Ordering::Relaxed) {
        return None;
    }
    let (at, bytes, funcs) = if mem::x64() { (code64::AT, code64::CODE, code64::FUNCS) } else { (code::AT, code::CODE, code::FUNCS) };
    start_in(at, bytes, funcs, pc)
}

/// function_start in code `bytes` at `at` with starts `funcs`
fn start_in(at: u32, bytes: &[u8], funcs: &[u32], pc: u32) -> Option<std::ops::Range<u32>> {
    let end = at.checked_add(u32::try_from(bytes.len()).ok()?)?;
    if !(at..end).contains(&pc) {
        return None;
    }
    let n = funcs.partition_point(|&f| f <= pc);
    let start = *funcs.get(n.checked_sub(1)?)?;
    Some(start..funcs.get(n).copied().unwrap_or(end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn builds() -> [(u32, &'static [u8], &'static [u32], &'static [(&'static str, u32)]); 2] {
        [(code::AT, code::CODE, code::FUNCS, code::SYMS), (code64::AT, code64::CODE, code64::FUNCS, code64::SYMS)]
    }

    #[test]
    fn every_function_starts_where_its_symbol_is() {
        for (at, bytes, funcs, syms) in builds() {
            assert!(funcs.windows(2).all(|w| matches!(w, [a, b] if a < b)), "ascending");
            for name in ["qsort", "bsearch", "memmove", "strlen", "strstr"] {
                let (_, f) = syms.iter().copied().find(|(n, _)| *n == name).unwrap();
                for pc in [f, f + 1, f + 7] {
                    let r = start_in(at, bytes, funcs, pc).unwrap();
                    assert_eq!(r.start, f, "{name} {pc:#x}");
                    assert!(r.contains(&pc) && !funcs.iter().any(|g| r.contains(g) && *g != f), "{name}: {r:x?}");
                }
            }
            assert_eq!(start_in(at, bytes, funcs, at.wrapping_sub(1)), None);
            let end = at + u32::try_from(bytes.len()).unwrap();
            assert_eq!(start_in(at, bytes, funcs, end), None);
        }
    }

    proptest! {
        /// the start is the last one at or before pc: no function starts between it and pc
        #[test]
        fn the_start_is_the_nearest_before(off in 0u32..0x2000, x64 in any::<bool>()) {
            let (at, bytes, funcs, _) = builds()[usize::from(x64)];
            let pc = at.wrapping_add(off);
            match start_in(at, bytes, funcs, pc) {
                Some(r) => {
                    let s = r.start;
                    prop_assert!(r.contains(&pc) && funcs.contains(&s));
                    prop_assert!(!funcs.iter().any(|&f| f > s && f < r.end));
                    prop_assert!(r.end == at + u32::try_from(bytes.len()).unwrap() || funcs.contains(&r.end));
                    prop_assert!(off < u32::try_from(bytes.len()).unwrap());
                }
                None => prop_assert!(off >= u32::try_from(bytes.len()).unwrap() || funcs.first().is_none_or(|&f| f > pc)),
            }
        }
    }
}
