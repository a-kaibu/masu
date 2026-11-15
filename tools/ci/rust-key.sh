#!/bin/bash
# The part of a cargo cache key that says which compiler built it (actions/cache keys: a target
# directory from another rustc is rebuilt from scratch, so restoring it is only wasted time), into
# $GITHUB_ENV as RUST_KEY, and that compiler's sysroot as RUST_SYSROOT (the rust-src and target
# std caches' paths). Also turns incremental builds off (CI builds once; their state is big).
# Without an argument it first installs the toolchain rust-toolchain.toml names (the runner
# image's stable is whatever release the image was built with).
#   tools/ci/rust-key.sh [toolchain]   (e.g. nightly; default: rust-toolchain.toml's)
set -euo pipefail
cd "$(dirname "$0")/../.."
[ -n "${1:-}" ] || rustup toolchain install --no-self-update
v=$(rustc ${1:++$1} -vV | sed -n 's/^release: //p; s/^commit-hash: \(.\{9\}\).*/\1/p' | tr '\n' '-')
{
  echo "RUST_KEY=rust-${v%-}"
  echo "RUST_SYSROOT=$(rustc ${1:++$1} --print sysroot)"
  echo "CARGO_INCREMENTAL=0"
} >> "${GITHUB_ENV:-/dev/stdout}"
