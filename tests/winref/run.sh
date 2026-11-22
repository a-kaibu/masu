#!/bin/bash
# Runs every exe in <exe dir> and keeps <out dir>/<name>/stdout.txt and exit code. On Windows
# (Git Bash on the runner) the exes run natively: that output is the reference. With SAKE=path
# to sake.js (its node build) they run on sake; MASU_JIT=0 for the interpreter. FONTS=dir: sake's
# C:\windows\Fonts (tools/ci/sake-fonts.py) for the probes marked "needs: fonts" (they measure text).
#
#   tests/winref/run.sh <exe dir> <out dir> [name-filter]
set -uo pipefail
EXE=$(cd "$1" && pwd)
mkdir -p "$2"
OUT=$(cd "$2" && pwd)
FILTER=${3:-}
names=()
for exe in "$EXE"/*.exe; do
  [ -e "$exe" ] || continue
  n=$(basename "$exe" .exe)
  [[ -n "$FILTER" && "$n" != *$FILTER* ]] && continue
  # (GPU probes run in the browser on sake's side: run-browser.mjs)
  [[ -n "${SAKE:-}" && -f "$EXE/browser.txt" ]] && grep -qx "$n" "$EXE/browser.txt" && continue
  names+=("$n")
done
# (the plan first: compare.py fails a planned probe that leaves no result, and a run that
# planned nothing)
runner=${SAKE:+node}
runner=${runner:-windows}${FILTER:+-${FILTER//[^A-Za-z0-9_.-]/_}}
# (plan.py's format, written here: Windows' Git Bash has no python of its own)
mkdir -p "$OUT/.plan"
{ echo "# run.sh ${SAKE:+(sake) }exe=$EXE filter=${FILTER:-none}"; for n in "${names[@]}"; do echo "$n"; done; } > "$OUT/.plan/$runner"
for n in "${names[@]}"; do
  exe=$EXE/$n.exe
  d=$OUT/$n
  rm -rf "$d"; mkdir -p "$d/cwd"
  start=$(date +%s)
  if [ -n "${SAKE:-}" ]; then
    # JIT and interpreter passes run together. Each probe needs its own registry, temp
    # files and services; sharing /tmp/sake-c lets the other pass delete its state.
    prefix=${SAKE_C:-$d/drive_c}
    mkdir -p "$prefix/windows/system32" "$prefix/windows/temp"
    # (sake writes the console to stderr, with its own "sake:" lines)
    fonts=
    src=$(dirname "$0")/probes/${n%-[36][24]}.c
    [[ -n "${FONTS:-}" && -f "$src" ]] && grep -q 'needs: fonts' "$src" && fonts=$FONTS
    (cd "$d/cwd" && timeout 300 env SAKE_C="$prefix" MASU_SCREEN=1366x768 ${fonts:+SAKE_FONTS="$fonts"} node "$SAKE" "$exe" > ../stdout.raw 2>&1; echo $? > ../exit)
    grep -Ev '^(sake|kura):' "$d/stdout.raw" > "$d/stdout.txt"; grep -E '^(sake|kura):' "$d/stdout.raw" > "$d/stderr.txt"; rm "$d/stdout.raw"
  else
    (cd "$d/cwd" && timeout 300 "$exe" > ../stdout.txt 2> ../stderr.txt; echo $? > ../exit)
  fi
  echo "$n exit $(cat "$d/exit") $(( $(date +%s) - start ))s $(wc -l < "$d/stdout.txt") lines"
done
