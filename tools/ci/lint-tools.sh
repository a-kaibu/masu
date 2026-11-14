#!/bin/bash
# The linters beside clippy, pinned release binaries in ~/.cache/masu-lint/bin (actions/cache,
# key: this file's hash): GitHub's releases are asked only on a miss. Prints the bin directory.
#   PATH=$(tools/ci/lint-tools.sh):$PATH
set -euo pipefail
B=$HOME/.cache/masu-lint/bin
ACTIONLINT=1.7.12 DENY=0.20.2 MACHETE=0.9.2
mkdir -p "$B"
get() {  # name version url [member path in the tarball]
  [ -x "$B/$1" ] && [ "$(cat "$B/$1.version" 2>/dev/null)" = "$2" ] && return
  local t
  t=$(mktemp -d)
  curl -sSfL --retry 5 -o "$t/a.tgz" "$3"
  tar -xzf "$t/a.tgz" -C "$t"
  install -m755 "$(find "$t" -type f -name "$1" | head -1)" "$B/$1"
  echo "$2" > "$B/$1.version"
  rm -rf "$t"
}
get actionlint $ACTIONLINT https://github.com/rhysd/actionlint/releases/download/v$ACTIONLINT/actionlint_${ACTIONLINT}_linux_amd64.tar.gz
get cargo-deny $DENY https://github.com/EmbarkStudios/cargo-deny/releases/download/$DENY/cargo-deny-$DENY-x86_64-unknown-linux-musl.tar.gz
get cargo-machete $MACHETE https://github.com/bnjbvr/cargo-machete/releases/download/v$MACHETE/cargo-machete-v$MACHETE-x86_64-unknown-linux-musl.tar.gz
echo "$B"
