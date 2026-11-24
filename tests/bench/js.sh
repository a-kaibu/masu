#!/bin/bash
# The "js" benchmark group: cpu.c's computing cases and host/tests/bench.c compiled straight to
# wasm (emcc, SIMD on), run in V8 (node) and JavaScriptCore (WebKitGTK's build of its shell) by
# tests/bench/js.mjs, with dyn.js (wasm generated at run time, as koji's). Emscripten comes from
# tools/ci/emsdk.sh. The JavaScriptCore build is pinned (MASU_JSC_REV below and its line in
# tools/ci/mirror.txt: change them on purpose, the report's 計測条件 shows the one each run used).
#
#   tests/bench/js.sh OUT.json [WORK]      (JSC=path to the jsc shell skips installing it)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=$(realpath -m "$1")
mkdir -p "$(dirname "$OUT")"
WORK=${2:-$(mktemp -d)}
mkdir -p "$WORK"
. "$ROOT/tools/ci/emsdk.sh"
. "$EMSDK4/emsdk_env.sh" >/dev/null 2>&1
FLAGS="-O2 -msimd128 -msse2 -sENVIRONMENT=node,shell -sSINGLE_FILE=1 -sALLOW_MEMORY_GROWTH=1"
emcc $FLAGS "$ROOT/tests/bench/src/cpu.c" -o "$WORK/cpu.js"
emcc $FLAGS -I"$ROOT/host/tests/wasm" "$ROOT/host/tests/bench.c" -o "$WORK/bench.js"
cp "$ROOT/tests/bench/dyn.js" "$WORK/"
# The JavaScriptCore shell: WebKitGTK's nightly bundle of revision MASU_JSC_REV (the build jsvu
# would fetch), the release ci-bundles' copy (webkitgtk.org keeps only the last ~40 revisions),
# checked against its sha256 in tools/ci/mirror.txt and kept in MASU_JSC_CACHE (bench.yml caches
# it). Not JSC_*: the shell reads those as its options. To move the pin: pick a revision from
# webkitgtk.org's jsc-built-products/x86_64/release/ and add its zip as jsc-REV-main.zip to
# tools/ci/mirror.txt.
MASU_JSC_REV=${MASU_JSC_REV:-325822}
JSC_VERSION=unknown
if [ -z "${JSC:-}" ]; then
  MASU_JSC_CACHE=${MASU_JSC_CACHE:-$HOME/.cache/masu-jsc}
  dir=$MASU_JSC_CACHE/$MASU_JSC_REV zip=$MASU_JSC_CACHE/$MASU_JSC_REV@main.zip
  if [ ! -x "$dir/jsc" ]; then
    mkdir -p "$MASU_JSC_CACHE"
    [ -f "$zip" ] || "$ROOT/tools/ci/fetch.sh" "jsc-$MASU_JSC_REV-main.zip" "$zip"
    rm -rf "$dir" && mkdir -p "$dir" && unzip -q "$zip" -d "$dir"
  fi
  # (the bundle's own wrapper: it runs bin/jsc with the bundle's loader and libraries)
  JSC=$dir/jsc
  "$JSC" -e 'print("jsc ok")' >/dev/null
  JSC_VERSION=$MASU_JSC_REV@main
fi
EMCC_VERSION=$(emcc --version | head -1 | sed 's/.*) //; s/ .*//')
VERSIONS="emscripten=$EMCC_VERSION javascriptcore=$JSC_VERSION" \
  node "$ROOT/tests/bench/js.mjs" "$OUT" "$WORK" v8=node "jsc=$JSC"
