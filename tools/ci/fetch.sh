#!/bin/bash
# A file of tools/ci/mirror.txt from this repository's release ci-bundles, checked against the
# sha256 written there (2026-11-24: CI reads nothing from outside GitHub at run time).
#   tools/ci/fetch.sh NAME OUT
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 2 ] || { echo "usage: tools/ci/fetch.sh NAME OUT" >&2; exit 2; }
name=$1
out=$2
sum=$(awk -v n="$name" '$1 == n { print $2 }' "$here/mirror.txt")
[ -n "$sum" ] || { echo "fetch: $name is not a line of tools/ci/mirror.txt" >&2; exit 1; }
url=${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-a-kaibu/masu}/releases/download/ci-bundles/$name
curl -fsSL --retry 5 --retry-all-errors --connect-timeout 30 -o "$out.part" "$url" || { echo "fetch: $url could not be fetched: is it uploaded (the ci-bundles workflow)?" >&2; exit 1; }
echo "$sum  $out.part" | sha256sum -c --quiet || { rm -f "$out.part"; echo "fetch: $name is not the file mirror.txt names" >&2; exit 1; }
mv "$out.part" "$out"
