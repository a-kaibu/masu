#!/bin/sh
# A/B of koji at commit BASE (default origin/main) against the working tree:
#   tools/jitbench/ab.sh [rounds] [programs] [knobs]
set -e
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
base=${BASE:-origin/main}
wt=${WT:-$here/target/base}
if [ ! -d "$wt" ]; then git -C "$root" worktree add -q --detach "$wt" "$base"; fi
# (the bench is the tree's, copied in after: what a base already has of it goes first)
rm -rf "$wt/tools/jitbench"
git -C "$wt" checkout -q -f --detach "$base"
rm -rf "$wt/tools/jitbench"
mkdir -p "$wt/tools/jitbench"
cp -r "$here/src" "$here/Cargo.toml" "$here/build.sh" "$wt/tools/jitbench/"
[ -f "$here/Cargo.lock" ] && cp "$here/Cargo.lock" "$wt/tools/jitbench/"
OUT="$here/target/a.wasm" CARGO_TARGET_DIR="$here/target/base-target" "$wt/tools/jitbench/build.sh"
OUT="$here/target/b.wasm" "$here/build.sh"
node "$here/ab.mjs" "$here/target/a.wasm" "$here/target/b.wasm" "${1:-5}" "${2:-}" "${3:-}"
