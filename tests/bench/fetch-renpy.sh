#!/bin/bash
# Fetches the engine of the Ren'Py benchmark's game (testdata/renpy, packed by
# tests/bench/pack-renpy.py) into DIR/renpy-sdk, once: Ren'Py 8.5.3's SDK (Windows x64 runtime,
# the engine's Python, and the Linux runtime that compiles the scripts), pinned by its hash
# (tools/ci/mirror.txt; fetched from the release ci-bundles).
# (Ren'Py is MIT with LGPL libraries: games ship it; LICENSE.txt goes with the game.)
#
#   tests/bench/fetch-renpy.sh DIR
set -euo pipefail
DIR=$(mkdir -p "$1" && cd "$1" && pwd)
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
if [ ! -f "$DIR/renpy-sdk/renpy.py" ]; then
  "$ROOT/tools/ci/fetch.sh" renpy-8.5.3-sdk.zip "$DIR/renpy.zip"
  rm -rf "$DIR/x" && mkdir "$DIR/x"
  python3 -I -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$DIR/renpy.zip" "$DIR/x"
  chmod +x "$DIR/x/renpy-8.5.3-sdk/renpy.sh" "$DIR"/x/renpy-8.5.3-sdk/lib/py3-linux-x86_64/*
  rm -rf "$DIR/renpy-sdk" && mv "$DIR/x/renpy-8.5.3-sdk" "$DIR/renpy-sdk" && rm -rf "$DIR/x" "$DIR/renpy.zip"
fi
