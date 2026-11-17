# Lints that keep the rewrites from coming undone

The type-safety and architecture work of 2026-11-14..17 made many mistakes compile errors or
removed the code that allowed them. What a type already forbids needs no lint; what is a rule
about the repository (where a primitive may be called, what may not come back, which guard must
stay) is checked by three tools, all in CI's `test` job:

| tool | what it sees | time |
| --- | --- | --- |
| `tools/ci/arch-lint.py` | rules about the repository, by text (comments and strings taken out with the ratchet's lexer) | seconds, before the build |
| `tools/ci/lint-ratchet.py` | clippy's lints and the masu-* lints per file and per unsafe site, only ever down | one clippy run |
| tests (`outptr_audit`, compile_fail doctests, trybuild-style goldens) | what needs the crate's own tables | `cargo test` |

Each tool has a `--self-test` that CI runs too: every rule must find its regression in a fixture
(and, for arch-lint, in the real tree with the regression applied to a real file), and must let
the right spelling through. A lint that silently stops firing (a clippy upgrade renames it, a
clippy.toml path stops resolving, a regex stops matching) fails there instead of counting 0.

## The invariants and what checks each

| invariant (where it was set) | check |
| --- | --- |
| no `as`, no `_` on enums, no unwrap/expect (2026-11-14 type-safety policy) | ratchet LINTS |
| unsafe kinds only where they already are, site by site (review 2026-11-15) | ratchet PINNED + lint-ratchet-sites.tsv |
| set_len / from_raw_parts / transmute / zeroed only where listed | clippy.toml disallowed-methods -> masu-* |
| clippy.toml and the ratchet name the same disallowed paths | arch-lint `disallowed-in-step` |
| the server reads an object's bytes only at its known sites (no relay, 2026-11-16) | masu-relay (`masu::storage::Storage::get`) |
| one wire-format parser for "vnvm-tree" | masu-wire_tree |
| no build output in git | arch-lint `build-output` |
| HRESULTs live in sake::hresult only | arch-lint `hresult-copy` |
| conversions, le readers, lock traits live in kome only | arch-lint `kome-copy` |
| one golden switch, MASU_UPDATE_GOLDEN, read by kome::golden only | arch-lint `golden-env` |
| sake mem's safe borrows and api's untyped out-parameter paths stay removed | arch-lint `dead-api` |
| Ga is a value (no r32/w32 methods), Args has no Deref | arch-lint `untyped-way-back` |
| view_unchecked, guest atomics, locked_anyway only in their homes | arch-lint `home-only` |
| every repr(C) type has a compile-time size assert | arch-lint `repr-c-layout` |
| release.yml builds and releases only | arch-lint `release-build-only` |
| MASU_OUTS_STRICT, outptr_audit, the compile_fail doctests, these CI steps stay | arch-lint `required` |
| every pointer-sized out-parameter is taken as typed Args | `sake::outptr_audit` (test) |
| Pool Lease / brand, Ga, Out misuse does not compile | compile_fail doctests |

## When a rule is in the way

- A known site that stays: add a line to `tools/ci/arch-lint-sites.tsv` (rule, file, the line's
  code, why). `--update` only removes lines whose site is gone.
- A new home for a contract primitive: add the file to `HOMES` in arch-lint.py with the reason.
- A new rule: a function under `@rule(bad=..., good=..., mutate=...)` in arch-lint.py; the
  self-test refuses one without fixtures that prove it fires.
- A new ratchet lint: add it to the list and a marked line (`//~ lint`) to
  `tools/lint-fixtures/ratchet`; the self-test refuses a lint without one.
