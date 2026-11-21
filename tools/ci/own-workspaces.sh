#!/bin/bash
# Every Cargo workspace of its own (fuzz harnesses, the JIT differential tools, test crates, the
# lint fixtures): `cargo check --all-targets` on each (--locked with its lock file), found by its `[workspace]` table,
# so a new one is checked without a list to add it to. The main workspace's build does not
# compile them: a renamed field, a dependency the code moved to or a std call changed by a sweep
# broke three of them at once and was found only by the slow jobs that run them (review e18bf44:
# jit64test's Vec::from_fn, crt-fuzz's Mode, masu-fuzz's lazy_regex).
set -euo pipefail
cd "$(dirname "$0")/../.."
found=0
while IFS= read -r -d '' toml; do
  dir=$(dirname "$toml")
  [ "$dir" = . ] && continue
  case "$dir" in vendor/*) continue ;; esac
  grep -qx '\[workspace\]' "$toml" || continue
  found=$((found + 1))
  echo "::group::$dir"
  # (--locked where the lock file is tracked: a test crate without one resolves afresh)
  locked=()
  git ls-files --error-unmatch "$dir/Cargo.lock" >/dev/null 2>&1 && locked=(--locked)
  (cd "$dir" && cargo check --all-targets "${locked[@]}" --quiet)
  echo "::endgroup::"
done < <(git ls-files -z -- '*Cargo.toml')
if [ "$found" -eq 0 ]; then
  echo "own-workspaces: no workspace of its own found (git ls-files failed?)" >&2
  exit 1
fi
echo "own-workspaces: $found checked"
