#!/bin/bash
# Fetches the engine of the 吉里吉里2 benchmark (the KiriKiri game of testdata/kirikiri, packed by
# tests/bench/pack-kirikiri.mjs) into DIR/krkr2, once: 吉里吉里2 2.32r2 (krkr.eXe 2.32.2.426,
# built 2010-10-26, and plugin/) as krkren's own repository on GitHub imported it, the first
# commit of github.com/krkren/krkr2sdk (the release zip kr2_232r2.zip as it was, before the
# translations that came after it), fetched by its hash: git checks what it gets against it.
# (Vector, kr2_232r2.zip's other home, answers CI with a refusal page, 2026-11-24.) (GPL 2 / its
# own license: redistribution allowed; license.txt goes with the game.)
#
#   tests/bench/fetch-krkr2.sh DIR      (DIR/krkr2/kirikiri2/krkr.eXe)
set -euo pipefail
DIR=$(mkdir -p "$1" && cd "$1" && pwd)
REPO=https://github.com/krkren/krkr2sdk.git
COMMIT=7c97ecbd0c0bb9c6eedea7d0ca3b7ec646a6b06b
if [ ! -f "$DIR/krkr2/kirikiri2/krkr.eXe" ]; then
  rm -rf "$DIR/x" && git init -q "$DIR/x"
  for i in 1 2 3 4; do git -C "$DIR/x" fetch -q --depth 1 "$REPO" "$COMMIT" && break; sleep $((i * 2)); done
  [ "$(git -C "$DIR/x" rev-parse FETCH_HEAD)" = "$COMMIT" ]
  git -C "$DIR/x" -c core.autocrlf=false checkout -q FETCH_HEAD
  rm -rf "$DIR/x/.git" "$DIR/krkr2" && mv "$DIR/x" "$DIR/krkr2"
fi
