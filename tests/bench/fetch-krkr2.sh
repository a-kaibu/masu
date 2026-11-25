#!/bin/bash
# Fetches the engine of the 吉里吉里2 benchmark (the KiriKiri game of testdata/kirikiri, packed by
# tests/bench/pack-kirikiri.mjs) into DIR/krkr2, once: 吉里吉里2 2.32r2 (krkr.eXe 2.32.2.426,
# built 2010-10-26, and plugin/) as the official site krkren.github.io hands it out, the release
# ci-bundles' copy (tools/ci/mirror.txt, pinned by its sha256). Its krkr.eXe and plugin/ are
# byte for byte Vector's kr2_232r2.zip's (compared 2026-11-24; Vector answers CI with a refusal
# page); license.txt is the same text with LF. (GPL 2 / its own license: redistribution allowed;
# license.txt goes with the game.)
#
#   tests/bench/fetch-krkr2.sh DIR      (DIR/krkr2/kirikiri2/krkr.eXe)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
DIR=$(mkdir -p "$1" && cd "$1" && pwd)
if [ ! -f "$DIR/krkr2/kirikiri2/krkr.eXe" ]; then
  "$ROOT/tools/ci/fetch.sh" kr2_232r2.zip "$DIR/krkr2.zip"
  rm -rf "$DIR/x" && mkdir "$DIR/x"
  python3 -I -c "import zipfile,sys; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])" "$DIR/krkr2.zip" "$DIR/x"
  # (the site's zip carries its repository's .git)
  rm -rf "$DIR/x/kr2_232r2/.git" "$DIR/krkr2" && mv "$DIR/x/kr2_232r2" "$DIR/krkr2" && rm -rf "$DIR/x" "$DIR/krkr2.zip"
fi
