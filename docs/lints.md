# Lints that keep the rewrites from coming undone

The type-safety and architecture work of 2026-11-14..17 made many mistakes compile errors or
removed the code that allowed them. What a type already forbids needs no lint; what is a rule
about the repository (where a primitive may be called, what may not come back, which guard must
stay) is checked by crates/masu-lint (Rust; `cargo run -q --release -p masu-lint -- arch | ratchet`)
and the crates' own tests, all in CI's `test` job:

| tool | what it sees | time |
| --- | --- | --- |
| `masu-lint arch` (src/arch.rs) | rules about the repository: Rust parsed with syn (items, impls, paths, macro bodies' tokens; #[cfg(test)] modules left out), other files as text | seconds, before the build |
| `masu-lint ratchet` (src/ratchet.rs) | clippy's lints and the masu-* lints per file and per unsafe site, only ever down; clippy runs with `--force-warn`, so an `#[allow]`, `#![allow(clippy::all)]` or a Cargo.toml `[lints]` allow does not hide a counted lint, and each `allow`/`expect` naming one is counted itself (read as tokens: over several lines, in `cfg_attr`, in macro bodies) | one clippy run |
| tests (`outptr_audit`, compile_fail doctests, trybuild-style goldens) | what needs the crate's own tables | `cargo test` |

`cargo test -p masu-lint` (part of CI's `cargo test --workspace`) proves the lints themselves:
every arch rule must find its regression in a fixture and in the real tree with the regression
put into a real file, and must let the right spelling through (arch_tests.rs: a rule without
fixtures does not compile, the table is an exhaustive match); every counted ratchet lint must
fire on its marked line in tools/lint-fixtures/ratchet (ratchet_tests.rs). A lint that silently stops firing (a clippy upgrade renames it, a
clippy.toml path stops resolving, a regex stops matching) fails there instead of counting 0.

## The invariants and what checks each

| invariant (where it was set) | check |
| --- | --- |
| no `as`, no `_` on enums, no unwrap/expect (2026-11-14 type-safety policy) | ratchet LINTS |
| unsafe kinds only where they already are, site by site (review 2026-11-15) | ratchet PINNED + lint-ratchet-sites.tsv |
| set_len / from_raw_parts / transmute / zeroed only where listed | clippy.toml disallowed-methods -> masu-* |
| each clippy.toml disallowed path's reason starts with its masu-* name (what the ratchet counts it as) | arch `disallowed-named` |
| the server reads an object's bytes only at its known sites (no relay, 2026-11-16) | masu-relay (`masu::storage::Storage::get`) |
| one wire-format parser for "vnvm-tree" | masu-wire_tree |
| no build output in git | arch `build-output` |
| HRESULTs live in sake::hresult only | arch `hresult-copy` |
| conversions, le readers, lock traits live in kome only | arch `kome-copy` |
| one golden switch, MASU_UPDATE_GOLDEN, read by kome::golden only | arch `golden-env` |
| sake mem's safe borrows and api's untyped out-parameter paths stay removed | arch `dead-api` |
| Ga is a value (no r32/w32 methods), Args has no Deref | arch `untyped-way-back` |
| view_unchecked, guest atomics, locked_anyway only in their homes | arch `home-only` |
| every repr(C) type has a compile-time size assert | arch `repr-c-layout` |
| release.yml builds and releases only | arch `release-build-only` |
| MASU_OUTS_STRICT, outptr_audit, the compile_fail doctests, these CI steps stay | arch `required` |
| an exported extern fn taking a raw pointer is `unsafe extern` (review 2026-11-17, vnhost_jit_late) | arch `safe-raw-pointer-abi` |
| the unchecked JIT (koji::jit::NOCHECK, MASU_JIT_NOCHECK) starts off, is set only in kura's jit.rs, and nothing in CI, scripts or the page turns it on (review 2026-11-17) | arch `unchecked-jit-mode` |
| an `#[allow]` (several lines, cfg_attr, crate-wide, Cargo.toml) does not hide a counted lint (review 2026-11-17) | ratchet `--force-warn` + allow-attr |
| a LockWord's / Object's `key()` is no guest address: not the address of mem's readers and writers, ga(), Ga::from (review 2026-11-17, 06dc4bf2) | arch `out-escape` |
| koji's Memory methods keep raw host pointers inside the implementation | arch `memory-pointer` |
| paged reads convert file byte counts with checked arithmetic; page indexing uses the bounded page location | arch `paged-read-cast` |
| GLSL array specifiers propagate layout errors to the shader compiler | arch `shader-layout-panic` |
| physical bucket deletion belongs to the storage GC | arch `store-deletion`, `home-only` |
| no comment or log line excuses code from Windows' behavior: no to-do markers, "for now", simplifications, "not implemented", stubs that answer anything, approximations, best effort (English or Japanese); "not supported" / an approximation / ignored flags only with Windows' evidence in the same comment (user rule 2026-11-20) | arch `escape-comment` (src/escape.rs, escape_tests.rs: each word alone) |
| a sake handler that ignores its arguments and returns a constant says in its doc which Windows answer that is (winref, a golden, MEASURED, Wine's tests); otherwise it is an empty success to implement (user rule 2026-11-20) | arch `constant-handler` |
| no Arc::try_unwrap: two last holders letting go at once both get Err and neither does the last-one-out step; Arc::into_inner, or a Drop that does it (Arc audit 2026-11-20, mf::source::Data) | arch `arc-try-unwrap` |
| a decision on Arc::strong_count / weak_count is a known site saying which lock keeps clones and drops from racing it (Arc audit 2026-11-20) | arch `arc-count` |
| koji's registers are named, not numbered: `Cpu::regs`/`hi` are `Gprs` indexed by `Gpr`, `sreg`/`sbase` `Segs` by `Seg`, `Cpu::gpr` takes a `Reg64`; no `Gpr::ALL[3]`/`values()[3]`, and no index, From or public array that takes a number (2026-11-20) | the types (a number doesn't compile) + arch `typed-registers` |
| save metadata is published by cacheSave or acknowledgeSave; production Wasm acquisition goes through runtimeWasm | web `pnpm run check` and `pnpm run lint` (TypeScript symbol references, including imports under another name) |
| no catch_unwind in a crate sake.wasm links: it is built with panic=abort and the catch never happens (82cdd0d0, review 2026-11-14 P1-7) | arch `catch-unwind-abort` (the crates: sake and its [dependencies] by path) |
| a workflow's env or a script does not set one of bash's own variables (GROUPS, RANDOM, SECONDS..; bench.yml passed its input as GROUPS) | arch `shell-special-env` |
| the depth limits put on recursion over game and file input stay declared and compared against (review b00/b04/b07, b87128be) | arch `depth-guard` (regress.rs DEPTH_GUARDS: a new limit is added there) |
| Box::leak / String::leak / Vec::leak only at their known sites (61153f30: GetProcAddress leaked its name per call; review 2026-11-16: LSan's 6000 leaks) | clippy.toml disallowed-methods -> masu-leak (pinned per site) |
| every pointer-sized out-parameter is taken as typed Args | `sake::outptr_audit` (test) |
| Pool Lease / brand, Ga, Out misuse does not compile | compile_fail doctests |

## When a rule is in the way

- A known site that stays: add a line to `tools/ci/arch-lint-sites.tsv` (rule, file, the line's
  code, why). `masu-lint arch --update` only removes lines whose site is gone.
- A new home for a contract primitive: add the file to `HOMES` in src/arch.rs with the reason.
- A new rule: a `Rule` variant in src/arch.rs; it does not compile until arch_tests.rs's
  fixture table has its regression, its right spelling and its mutation of a real file.
- A new ratchet lint: add it to the list and a marked line (`//~ lint`) to
  `tools/lint-fixtures/ratchet`; the self-test refuses a lint without one.
