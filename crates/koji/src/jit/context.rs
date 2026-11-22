//! What a translation is made under besides the guest's bytes (koji redesign, stage 2).
//!
//! The generators read the modes once, when a translation begins ([`TranslationContext::now`]),
//! and only from here after that: the host may turn a mode on while another thread translates
//! (the MXCSR flag checks, the page checks: settled once, never off again), and a translation
//! that read one mode at its start and another in its middle was some of each, kept as made
//! with the first. With the context given, a translation is a function of it and the guest's
//! bytes: two generators can be given the same input and compared, and what a saved or shared
//! translation is told apart by ([`TranslationContext::runs_now`]) is what it was made with.
//!
//! What stays outside: the host's address predicates (super::set_watch, set_fresh, set_deny),
//! set once before any translation, and the counters a translation bumps (EXIT_AGAIN...).

use core::sync::atomic::Ordering::Relaxed;

// The modes the host turns on (settled once, never off again), read by TranslationContext::now
// and by the host: none of the generators reads them

/// Whether translated code keeps the MXCSR exception flags as the hardware (jit::mxguard's
/// checks, which cost SSE float loops 2-3x). Off until the program first reads the flags
/// ([`mx_read`]: STMXCSR, FXSAVE, the CRT's status functions); flags raised before that by
/// translated code are not in MXCSR (the interpreter's are). Settled once: never off again.
pub static MX_EXACT: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
/// Set with [`MX_EXACT`]: the host drops every translation made without the checks
/// ([`mx_switched`]).
static MX_SWITCH: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Where the first read was (the instruction's address; 0: the CRT's status functions).
pub static MX_AT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// The program reads the MXCSR flags (at `at`): translated code keeps them from now on.
pub fn mx_read(at: u32) {
    use core::sync::atomic::Ordering::SeqCst;
    if !MX_EXACT.swap(true, SeqCst) {
        // (a swap, not a store: node 22's V8 baseline compiler (Liftoff) stores a local to a
        // constant address at or above 2 GB, where sake's statics are, at the local's value
        // as an address: Chromium 153 does not)
        MX_AT.swap(at, SeqCst);
        MX_SWITCH.store(true, SeqCst);
    }
}

/// Whether [`mx_read`] switched since the last call (the host then drops its translations).
pub fn mx_switched() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    MX_SWITCH.load(Relaxed) && MX_SWITCH.swap(false, core::sync::atomic::Ordering::SeqCst)
}

/// [`MX_EXACT`].
pub fn mx_exact() -> bool {
    MX_EXACT.load(core::sync::atomic::Ordering::Relaxed)
}

/// Whether translated code may run with this MXCSR: SSE exceptions masked and rounding to
/// nearest (the JIT's SSE is the plain wasm ops; other modes are the interpreter's, and a step
/// that changes MXCSR leaves the region: see the `step` import). With FTZ or DAZ only under
/// [`MX_EXACT`]: jit::mxguard then sends every operation with a denormal operand, or one whose
/// result may be denormal, to the interpreter, which flushes them (a program setting either
/// turns MX_EXACT on: [`mx_modes`]).
pub fn sse_ok(mxcsr: u32) -> bool {
    mxcsr & 0x7f80 == 0x1f80 && (mxcsr & 0x8040 == 0 || mx_exact())
}

/// MXCSR was loaded with `mxcsr` (LDMXCSR, FXRSTOR) at `at`: FTZ or DAZ turn [`MX_EXACT`] on.
pub fn mx_modes(mxcsr: u32, at: u32) {
    if mxcsr & 0x8040 != 0 && !mx_exact() {
        mx_read(at);
    }
}
/// Guest pages an access faults on (PAGE_NOACCESS, PAGE_GUARD): whether translated code checks
/// them. The host turns it on when the first such page appears (it drops what was translated
/// before); from then on every access checks its pages and leaves the block when one is marked,
/// so that the interpreter raises the fault. Accesses through ESP (the stack) are not checked.
/// Where the marks are is not written into the code: every module imports it as the global
/// "pages" ([`PAGES`]), the host address of a byte per page of [0, GUEST_HI), nonzero for such
/// a page (kura's fault map). A translation is then the same bytes in every run, whatever the
/// host's heap gave the map, and one made with the checks is kept across runs and used again
/// (review of 6852be1, §2: the address was an immediate, so saved code was never used once a
/// thread's stack guard page turned the checks on, that is in every run).
pub static PAGE_CHECK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Whether a translation checks the page marks: what it was made with ([`PAGE_CHECK`] then),
/// kept with it (saved::Entry, the host's caches), and what decides where it may run: one made
/// without the checks only while they are off.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Pages {
    Unchecked,
    Checked,
}

impl Pages {
    /// What translations are made with now.
    pub fn now() -> Pages {
        if page_check() { Pages::Checked } else { Pages::Unchecked }
    }
    /// Whether a translation made with `self` may run now: a checked one always (with no page
    /// marked it checks and finds none), an unchecked one only while the checks are off.
    pub fn runs_now(self) -> bool {
        match self {
            Pages::Checked => true,
            Pages::Unchecked => !page_check(),
        }
    }
}

/// Measuring only (MASU_JIT_NOCHECK=1): no range checks on memory accesses (unsafe).
pub static NOCHECK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The global every module imports first: the page marks' host address ([`PAGE_CHECK`]).
pub const PAGES: u32 = 0;

/// Whether accesses check the page marks ([`PAGE_CHECK`]).
pub fn page_check() -> bool {
    PAGE_CHECK.load(core::sync::atomic::Ordering::Relaxed)
}


/// The instruction set and its address model
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Arch {
    /// 16-bit code at linear addresses, its code segment's base (super::translate_region16)
    Real16 { cs_base: u32 },
    /// 32-bit flat code (super::translate_region, translate_function)
    Flat32,
    /// long mode (super::long)
    Long64,
}

/// How guest calls translate (super::FN, long::FN64)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Calls {
    /// regions stop at calls: the dispatcher takes them
    Regions,
    /// a guest function becomes a wasm function, a CALL a wasm call (super::T_FN)
    Functions,
}

/// The SSE float exceptions (super::MX_EXACT)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Float {
    /// MXCSR's flags not kept (the program never read them)
    Fast,
    /// kept as the hardware (jit::mxguard's checks)
    Exact,
}

/// Range checks on memory accesses (super::NOCHECK: measuring only)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Ranges {
    Checked,
    /// none (unsafe: an access outside the guest range is not caught)
    Unchecked,
}

/// What the code measures (debugging: none of it in a saved translation)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub struct Measure {
    /// super::COUNT: counters of control transfers
    pub count: bool,
    /// super::PROFILE: block entry counts
    pub profile: bool,
    /// super::LAYOUT: Region::layout filled
    pub layout: bool,
    /// wasm::MEMCOUNT: per memory access site counts
    pub mem_sites: bool,
    /// super::PROBES: instructions left to the interpreter (0: none)
    pub probes: [u32; 8],
}

impl Measure {
    /// Whether `ip` is a probe (an instruction the interpreter runs, printing the state)
    pub fn probed(&self, ip: u32) -> bool {
        ip != 0 && self.probes.contains(&ip)
    }
}

/// What the optimizations may take
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Budget {
    /// slots a loop keeps (super::SLOTS_MAX)
    pub slots: usize,
    /// range checks hoisted out of loops (super::HOIST)
    pub hoist: bool,
    /// functions entered in the middle too (super::FN_ENTRIES)
    pub fn_entries: bool,
}

/// Debugging switches of the long-mode generator (long::STRUCTURED, DEBUG_*)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub struct Debug64 {
    /// regions laid out by `structure` too (long::STRUCTURED)
    pub structured: bool,
    /// every register moved at entries and exits (long::DEBUG_ALL_REGS)
    pub all_regs: bool,
    /// no side entries (long::DEBUG_NO_SIDE)
    pub no_side: bool,
    /// lazy conditions checked against the materialized flags (long::DEBUG_CHECK)
    pub check: bool,
}

/// Everything a translation is made under (see the module's doc)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct TranslationContext {
    pub arch: Arch,
    pub calls: Calls,
    pub float: Float,
    pub pages: Pages,
    pub ranges: Ranges,
    pub measure: Measure,
    pub budget: Budget,
    pub debug64: Debug64,
}

impl TranslationContext {
    /// The modes now, for a translation of `arch` beginning
    pub fn now(arch: Arch) -> TranslationContext {
        let calls = match arch {
            Arch::Real16 { .. } => Calls::Regions,
            Arch::Flat32 => on(super::FN.load(Relaxed)),
            Arch::Long64 => on(super::long::FN64.load(Relaxed)),
        };
        let mut probes = [0; 8];
        for (p, a) in probes.iter_mut().zip(&super::PROBES) {
            *p = a.load(Relaxed);
        }
        TranslationContext {
            arch,
            calls,
            float: if mx_exact() { Float::Exact } else { Float::Fast },
            pages: Pages::now(),
            ranges: if NOCHECK.load(Relaxed) { Ranges::Unchecked } else { Ranges::Checked },
            measure: Measure {
                count: super::COUNT.load(Relaxed),
                profile: super::PROFILE.load(Relaxed),
                layout: super::LAYOUT.load(Relaxed),
                mem_sites: super::wasm::MEMCOUNT.load(Relaxed),
                probes,
            },
            budget: Budget {
                slots: kome::num::us(super::SLOTS_MAX.load(Relaxed)),
                hoist: super::HOIST.load(Relaxed),
                fn_entries: super::FN_ENTRIES.load(Relaxed),
            },
            debug64: Debug64 {
                structured: super::long::STRUCTURED.load(Relaxed),
                all_regs: super::long::DEBUG_ALL_REGS.load(Relaxed),
                no_side: super::long::DEBUG_NO_SIDE.load(Relaxed),
                check: super::long::DEBUG_CHECK.load(Relaxed),
            },
        }
    }

    /// The code segment's base of 16-bit code
    pub fn seg16(&self) -> Option<u32> {
        match self.arch {
            Arch::Real16 { cs_base } => Some(cs_base),
            Arch::Flat32 | Arch::Long64 => None,
        }
    }

    pub fn functions(&self) -> bool {
        self.calls == Calls::Functions
    }

    pub fn exact(&self) -> bool {
        self.float == Float::Exact
    }

    pub fn page_check(&self) -> bool {
        self.pages == Pages::Checked
    }

    pub fn nocheck(&self) -> bool {
        self.ranges == Ranges::Unchecked
    }

    /// Whether a translation made under `self` may be saved for another run: without the
    /// MXCSR flag checks (saved code is loaded only without them) and without counters (their
    /// addresses are this run's)
    pub fn saveable(&self) -> bool {
        self.float == Float::Fast && !self.measure.count && !self.measure.profile && !self.measure.mem_sites && self.ranges == Ranges::Checked
    }

    /// Whether a translation made under `self` may run now: with the MXCSR flag checks as they
    /// are now (made without them, it goes once they come on; they never go off), the page
    /// checks as [`Pages::runs_now`]
    pub fn runs_now(&self) -> bool {
        self.exact() == mx_exact() && self.pages.runs_now()
    }
}

fn on(functions: bool) -> Calls {
    if functions { Calls::Functions } else { Calls::Regions }
}
