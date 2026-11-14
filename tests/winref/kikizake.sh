#!/bin/bash
# kikizake (github.com/komodaru/kikizake): the Windows graphics conformance suites, with
# Windows' answers in its golden/. Checked out at the commit kikizake.rev names into
# $KIKIZAKE (default target/kikizake) unless it is already there; prints the directory.
# build.sh builds its executors (kikizake_<suite>-32/-64.exe), golden.py reads its answers.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
REV=$(cat "$HERE/kikizake.rev")
DIR=${KIKIZAKE:-$ROOT/target/kikizake}
if [ "$(git -C "$DIR" rev-parse HEAD 2>/dev/null)" != "$REV" ]; then
  mkdir -p "$DIR"
  git -C "$DIR" init -q
  git -C "$DIR" fetch -q --depth 1 https://github.com/komodaru/kikizake "$REV" >&2
  git -C "$DIR" checkout -q --force FETCH_HEAD
fi
echo "$DIR"
