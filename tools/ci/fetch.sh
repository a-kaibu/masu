#!/bin/bash
# A file of tools/ci/mirror.txt from this repository's release ci-bundles (2026-11-24: CI reads
# nothing from outside GitHub at run time), checked against the sha256 written there, or for a
# git tree or docker image (made on the release, not byte for byte reproducible) against the
# .sha256 beside it.
#   tools/ci/fetch.sh NAME OUT
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 2 ] || { echo "usage: tools/ci/fetch.sh NAME OUT" >&2; exit 2; }
name=$1
out=$2
sum=$(awk -v n="$name" '$1 == n { print $2 }' "$here/mirror.txt")
[ -n "$sum" ] || { echo "fetch: $name is not a line of tools/ci/mirror.txt" >&2; exit 1; }
url=${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-a-kaibu/masu}/releases/download/ci-bundles/$name
get() {  # <url> <file>
  curl -fsSL --retry 5 --retry-all-errors --connect-timeout 30 -o "$2" "$1" || { echo "fetch: $1 could not be fetched: is it uploaded (the ci-bundles workflow)?" >&2; exit 1; }
}
get "$url" "$out.part"
case $sum in
  git:*|docker)
    get "$url.sha256" "$out.sha256"
    sum=$(cut -d' ' -f1 "$out.sha256")
    rm "$out.sha256"
    ;;
esac
echo "$sum  $out.part" | sha256sum -c --quiet || { rm -f "$out.part"; echo "fetch: $name is not the file it should be" >&2; exit 1; }
mv "$out.part" "$out"
