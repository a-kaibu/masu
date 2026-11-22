#!/bin/bash
# Builds ReactOS's user-mode API tests (modules/rostests/apitests) as Windows programs, the way
# ReactOS builds them (its own CMake build, headers, import libraries and wine/test.h, with the
# distribution's mingw-w64), from a pinned ReactOS commit fetched at build time. Nothing of
# ReactOS is kept in this repository (the tests are GPL-2.0-or-later, LGPL-2.1-or-later and MIT);
# only the list of tests, Windows' results and the results on sake are.
#
#   tests/rostests/build.sh [test ...]   (default: every test program in tests/rostests/dlls.txt)
#   ROSTESTS=dir (default target/rostests): bin/<arch>/<test>_apitest.exe beside the files the
#     tests read (ReactOS's bin directory: test DLLs, manifests, testdata/), plus src/ and build-<arch>/
#   ARCHS="i386 x86_64" (default both)
#
# Needs: mingw-w64, cmake, ninja, flex, bison, git, libgmp-dev and libmpc-dev (ReactOS's x64 SEH
# plugin for GCC). ~5 min per architecture on 4 cores.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
COMMIT=303d27d62f801f1d27a056b9f296b9a5baead3a3
R=${ROSTESTS:-$HERE/../../target/rostests}
ARCHS=${ARCHS:-i386 x86_64}
mkdir -p "$R"
R=$(cd "$R" && pwd)
# (a line's only:<arch> limits it to that architecture: ReactOS builds those tests for it alone)
list_for() { grep -v '^#' "$HERE/dlls.txt" | awk -v a="$1" 'NF { for (i = 2; i <= NF; i++) if ($i ~ /^only:./ && $i != "only:" a) next; print $1 }'; }

S=$R/src
if [ "$(git -C "$S" rev-parse HEAD 2>/dev/null || true)" != "$COMMIT" ]; then
  rm -rf "$S"
  git init -q "$S"
  git -C "$S" fetch -q --depth 1 https://github.com/reactos/reactos "$COMMIT"
  git -C "$S" checkout -q FETCH_HEAD
fi
# (the commit is the pin: a fetch that gave anything else stops here)
test "$(git -C "$S" rev-parse HEAD)" = "$COMMIT"

status=0
for a in $ARCHS; do
  case $a in i386) ra=i386 ;; x86_64) ra=amd64 ;; *) echo "unknown arch $a"; exit 1 ;; esac
  B=$R/build-$a
  if [ ! -f "$B/build.ninja" ]; then
    # (Vista's exports, as ReactOS's own CI builds one of its configurations: the toolchain's
    # libgcc calls the condition variable functions. A null pointer read stays a read, as the
    # Windows compilers keep it: the tests read through NULL inside _SEH2_TRY on purpose, and
    # GCC 13 otherwise drops such a block with its handler labels (shlwapi IsQSForward, x64))
    cmake -S "$S" -B "$B" -G Ninja -DCMAKE_TOOLCHAIN_FILE:FILEPATH=toolchain-gcc.cmake -DARCH:STRING=$ra \
      -DCMAKE_BUILD_TYPE=Release -DENABLE_ROSTESTS=1 -DDLL_EXPORT_VERSION=0x600 \
      -DCMAKE_C_FLAGS=-fno-delete-null-pointer-checks -DCMAKE_CXX_FLAGS=-fno-delete-null-pointer-checks > "$R/configure-$a.log" 2>&1 \
      || { tail -20 "$R/configure-$a.log"; exit 1; }
  fi
  # the files ReactOS puts beside its tests (reactos/bin on its CD): the test programs and the
  # DLLs, programs, manifests and data they load. Not the kernel-mode tests and drivers, nor
  # the other suites' programs (Wine's tests run from tests/winetest).
  progs=() ; files=()
  for t in ${*:-$(list_for "$a")}; do progs+=("${t}_apitest.exe"); done
  while IFS='=' read -r dst src; do
    rel=${dst#reactos/bin/}
    case $rel in
      *_apitest.exe|*_winetest.exe|*_unittest.exe|*_drvtest.exe|*.sys|*kmtest*|suppl/*|regtest.cmd) continue ;;
    esac
    files+=("$rel=$src")
  done < <(grep '^reactos/bin/' "$B/boot/bootcd.Release.lst" | sort -u)
  targets=()
  for p in "${progs[@]}"; do
    f=$(grep "^reactos/bin/$p=" "$B/boot/bootcd.Release.lst" | head -1 | cut -d= -f2)
    [ -n "$f" ] || { echo "not a ReactOS test program: $p"; status=1; continue; }
    files+=("$p=$f")
  done
  for e in "${files[@]}"; do
    src=${e#*=}
    case $src in "$B"/*) targets+=("${src#"$B"/}") ;; esac
  done
  ninja -C "$B" -k0 "${targets[@]}" > "$R/make-$a.log" 2>&1 || grep -E '^FAILED|: error:' "$R/make-$a.log" | head -20
  O=$R/bin/$a
  rm -rf "$O"
  mkdir -p "$O"
  case $a in i386) s=i686-w64-mingw32-strip ;; *) s=$a-w64-mingw32-strip ;; esac
  for e in "${files[@]}"; do
    rel=${e%%=*} src=${e#*=}
    if [ ! -f "$src" ]; then echo "not built: $a $rel"; status=1; continue; fi
    mkdir -p "$(dirname "$O/$rel")"
    # (the programs and DLLs built here lose their debug sections; the source tree's files,
    # testdata among them, are copied as they are)
    case $src in
      "$B"/*.exe|"$B"/*.dll) $s -o "$O/$rel" "$src" ;;
      *) cp "$src" "$O/$rel" ;;
    esac
  done
done
echo "built $(ls "$R"/bin/*/*_apitest.exe | wc -l) test programs in $R/bin"
exit $status
