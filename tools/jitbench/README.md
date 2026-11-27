# jitbench: koji's JIT measured in V8

koji (the translator itself, as kura has it) built to wasm32 with shared memory, and the
modules it makes loaded the way kura's vnjit.js loads them: one instance per translation, its
functions in a shared table, the dispatcher's module from `jb_dispatch_module`. Fixed x86
programs (src/progs.rs) run from their entry to their end, translated when a block has run
50 times (`jb_run`, as kura's ring).

| program | the shape it stands for |
|---|---|
| crc32 | a table-driven byte loop |
| bigadd | an ADC carry chain over memory |
| calls | small calls in a loop (a callee that writes the flags before it reads them) |
| vm | a bytecode VM: byte opcode, jump table, a value stack in memory |
| sse_int | integer SSE2 (PADDW/PMULLW/PSHUFD...) |
| x87_dot | an x87 dot product |
| sort | insertion sort (loads, stores and compares in a nested loop) |
| flags_mem | memory operands between a flag writer and its reader (exits that must keep the flags) |
| avx2_int | 64-bit integer AVX2 over both halves of YMM registers (add, multiply, logic, shift and subtract) |

## Running

    tools/jitbench/build.sh                        # target/jitbench.wasm (OUT= elsewhere)
    node tools/jitbench/bench.mjs target/jitbench.wasm [programs] [knob=v,...]
    node tools/jitbench/browser.mjs target/jitbench.wasm [programs] [knob=v,...]
    tools/jitbench/ab.sh [rounds] [programs] [knobs]  # origin/main (A) against the tree (B)
    JB_BROWSER=1 node tools/jitbench/ab.mjs A.wasm B.wasm [rounds]

build.sh needs a nightly-capable toolchain for `-Zbuild-std` (RUSTC_BOOTSTRAP=1 is set: the
shared memory needs std built with atomics). ab.sh builds A in a worktree at target/base
(BASE=<commit> for another one) and alternates the rounds, each a new process (or a new
Chromium with JB_BROWSER).

browser.mjs runs a round in Chromium as an Android phone: V8's explicit bounds checks
(--wasm-enforce-bounds-checks, as Android's V8 compiles kura's memory), Pixel 7's screen and user agent,
the CPU slowed 4 times by CDP (JB_THROTTLE), the page cross-origin isolated on a localhost
origin served by Playwright.

Knobs: `fn` (function mode, 1 by default as kura), `hoist`, `memcount` (MASU_JIT_MEMCOUNT),
`mxexact`, `nojit` (the interpreter alone), `ssa` (koji::jit::SSA, MASU_JIT_SSA). Environment: JB_HOT (hot runs, 7), JB_SCALE,
JB_DUMP=dir (each translated module as dir/NNN.wasm), JB_SWAP=dir (dir/NNN.wasm loaded in a
module's place: experiments on what V8 is given), JB_KNOBS_A / JB_KNOBS_B (ab.mjs: one side's
knobs, e.g. one build with `JB_KNOBS_A=ssa=0 JB_KNOBS_B=ssa=1`).

Columns of ab.mjs: hot ms (each round's median of the hot runs, then the median over rounds),
cold ms (the first run: the interpreter's warm-up, translation and V8's compile) and its p95
over the rounds (the tail a scene change sees), translate ms (koji's own time), V8 load ms
(WebAssembly.Module + Instance of the translations), wasm B (their bytes), interp steps (the
instructions the interpreter ran in the cold run), step calls and exits per hot run (the ways
back to the interpreter and to the dispatcher), the checksums (A and B must agree), the peak
RSS (node) or JS heap (Chromium). The per-round JSON also reports the dynamic memory
access counts (per run, classified by the emitter as Cpu or other addresses) when `memcount=1`.
