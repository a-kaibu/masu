# shellcheck shell=bash
# Sourced: Emscripten (the release the project builds with, 4.0.15) in $EMSDK4 (default
# ~/emsdk4), fetched the first time from the release ci-bundles (tools/ci/fetch.sh). Used by tools/ci/build.sh and tests/bench/js.sh.
export EMSDK4=${EMSDK4:-$HOME/emsdk4}
EMSCRIPTEN_HASH=b412b6307e541b93dd93f01b61181e15c17302ec

if [ ! -f "$EMSDK4/emsdk_env.sh" ]; then
  mkdir -p "$EMSDK4"
  # (the release ci-bundles' copy of Emscripten's binaries: tools/ci/mirror.txt)
  "$(dirname "${BASH_SOURCE[0]}")/fetch.sh" "emsdk-$EMSCRIPTEN_HASH.tar.xz" "$EMSDK4/wasm-binaries.tar.xz"
  tar xJf "$EMSDK4/wasm-binaries.tar.xz" -C "$EMSDK4"
  rm "$EMSDK4/wasm-binaries.tar.xz"
  ln -sfn install "$EMSDK4/upstream"
  printf 'export EMSDK=%s\nexport EM_CONFIG=%s/.emscripten\nexport PATH=%s/upstream/emscripten:%s/upstream/bin:$PATH\n' "$EMSDK4" "$EMSDK4" "$EMSDK4" "$EMSDK4" > "$EMSDK4/emsdk_env.sh"
fi
# (written every time: the cache is shared across workflows, and a job's node (setup-node puts it
# under a versioned path) or home is not the one the cache was saved with)
printf "LLVM_ROOT = '%s/upstream/bin'\nBINARYEN_ROOT = '%s/upstream'\nNODE_JS = '%s'\n" "$EMSDK4" "$EMSDK4" "$(command -v node)" > "$EMSDK4/.emscripten"
