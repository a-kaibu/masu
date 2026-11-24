#!/bin/bash
# Translation validation of koji's 32-bit JIT (tools/jit32tv): the wasm koji generates for short
# integer sequences, run on terms, against x86's semantics written apart, decided by Z3 (Z3: the
# z3 on PATH, or $Z3). Fails on any difference, and when the cases left undecided are not
# exactly those of formal/jit32tv-undecided.txt (a new one, or one listed that is decided now).
set -euo pipefail
cd "$(dirname "$0")/../tools/jit32tv"
cargo test --release --locked --quiet
cargo build --release --locked --quiet
target/release/jit32tv --expect ../../formal/jit32tv-undecided.txt "$@"
