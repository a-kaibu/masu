#!/bin/sh
# Builds jitbench.wasm (with the koji at ../../crates/koji) into OUT (default
# target/jitbench.wasm). For another commit's koji: copy this directory into that checkout's
# tools/ and run its copy (ab.sh does). The module imports a shared memory (as kura's) of which
# the guest has [0, 2 GiB): this crate's data, stack and heap go above (--global-base).
set -e
# (OUT relative to where this is run from, not to this directory)
out=$(realpath -m "${OUT:-$(dirname "$0")/target/jitbench.wasm}")
cd "$(dirname "$0")"
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+simd128 -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294901760 -C link-arg=--global-base=2148007936 -C link-arg=-zstack-size=8388608" \
  cargo build -q --release --target wasm32-unknown-unknown -Zbuild-std=std,panic_abort
mkdir -p "$(dirname "$out")"
cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/jitbench.wasm" "$out"
