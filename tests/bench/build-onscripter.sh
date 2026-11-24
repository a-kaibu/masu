#!/bin/bash
# Builds the engine of the ONScripter benchmark's game (testdata/onscripter) into
# DIR/onscripter.exe (32-bit, as NScripter games are) and DIR/onscripter64.exe (x64), once: ONScripter 20230825 (GPL 2; Debian's copy of the release) linked
# statically with SDL 1.2.15, SDL_image (BMP and PNG), SDL_mixer (WAV), SDL_ttf, FreeType, libpng,
# zlib and bzip2, every source pinned by its hash (tools/ci/mirror.txt). SDL 1.2 draws with GDI (windib) and plays through
# waveOut. One change (onscripter-benchlog.patch): a
# `benchlog "text"` command that prints the line, the harness's signals (NScripter has no way to).
#
#   tests/bench/build-onscripter.sh DIR       (needs i686- and x86_64-w64-mingw32-gcc/g++, make)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
DIR=$(mkdir -p "$1" && cd "$1" && pwd)
[ -f "$DIR/onscripter.exe" ] && [ -f "$DIR/onscripter64.exe" ] && exit 0
# (the release ci-bundles' copies, each pinned by its hash in tools/ci/mirror.txt)
SRC=(onscripter_20230825.orig.tar.gz SDL-1.2.15.tar.gz SDL_image-1.2.12.tar.gz SDL_mixer-1.2.12.tar.gz SDL_ttf-2.0.11.tar.gz
  freetype-2.13.3.tar.xz zlib-1.3.1.tar.gz libpng-1.6.44.tar.xz bzip2-1.0.8.tar.gz)
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
mkdir -p "$W/src"
for n in "${SRC[@]}"; do
  "$HERE/../../tools/ci/fetch.sh" "$n" "$W/$n"
  tar -C "$W/src" -xf "$W/$n"
done
patch -d "$W/src/onscripter-20230825" -p1 -s < "$HERE/onscripter-benchlog.patch"
for H in i686-w64-mingw32 x86_64-w64-mingw32; do
  EXE=onscripter.exe; [ $H = x86_64-w64-mingw32 ] && EXE=onscripter64.exe
  [ -f "$DIR/$EXE" ] && continue
  # (each architecture builds in its own copy of the sources, into its own prefix)
  S=$W/$H P=$W/$H/prefix
  rm -rf "$S" && cp -r "$W/src" "$S" && mkdir -p "$P/include" "$P/lib"
  export CC=$H-gcc CXX=$H-g++ AR=$H-ar RANLIB=$H-ranlib
  export CPPFLAGS="-I$P/include -I$P/include/freetype2" LDFLAGS="-L$P/lib" PKG_CONFIG_PATH="$P/lib/pkgconfig" PKG_CONFIG_LIBDIR="$P/lib/pkgconfig"
  J=-j$(nproc)
  conf() { ./configure --host=$H --prefix="$P" --disable-shared --enable-static "$@" >/dev/null; make $J >/dev/null; make install >/dev/null; }
  q() { "$@" >/dev/null 2>&1 || { echo "failed: $*" >&2; "$@"; }; }

  cd "$S/zlib-1.3.1" && q make -f win32/Makefile.gcc PREFIX=$H- libz.a && cp libz.a "$P/lib" && cp zlib.h zconf.h "$P/include"
  cd "$S/bzip2-1.0.8" && q make CC=$CC AR=$AR RANLIB=$RANLIB libbz2.a && cp libbz2.a "$P/lib" && cp bzlib.h "$P/include"
  cd "$S/libpng-1.6.44" && conf
  cd "$S/freetype-2.13.3" && conf --enable-freetype-config --without-harfbuzz --without-brotli --without-bzip2 --without-png --with-zlib=no
  cd "$S/SDL-1.2.15" && conf --disable-directx --disable-stdio-redirect
  # (SDL_image: BMP is built in; PNG linked in, not loaded at run time)
  cd "$S/SDL_image-1.2.12" && conf --with-sdl-prefix="$P" --disable-jpg --disable-tif --disable-webp --disable-png-shared
  cd "$S/SDL_mixer-1.2.12" && conf --with-sdl-prefix="$P" --disable-music-mod --disable-music-midi --disable-music-ogg --disable-music-flac --disable-music-mp3 --disable-smpegtest
  # (SDL_ttf 2.0.11's TTF_RenderGlyph_Shaded, which ONScripter draws text with, copies the rows of
  # the mono bitmap's count, 0 when only the pixmap was made: blank text. Distributions patch it.)
  cd "$S/SDL_ttf-2.0.11" && perl -0pi -e 's/row < glyph->bitmap\.rows; \+\+row \) \{\n(\s*)memcpy\( dst, src, glyph->pixmap\.width \)/row < glyph->pixmap.rows; ++row ) {\n$1memcpy( dst, src, glyph->pixmap.width )/' SDL_ttf.c && grep -q "row < glyph->pixmap.rows" SDL_ttf.c && conf --with-sdl-prefix="$P" --with-freetype-prefix="$P" --without-x

  cd "$S/onscripter-20230825"
  # (its __iob_func, for old MSVC builds of SDL, clashes with x64 mingw's declaration)
  [ $H = x86_64-w64-mingw32 ] && sed -i '/^FILE __iob_func\[3\]/d' ONScripter_file.cpp
  for f in *.cpp; do
    case $f in onscripter_main.cpp|DirectReader.cpp|SarReader.cpp|NsaReader.cpp|sjis2utf16.cpp|ScriptHandler.cpp|ScriptParser*.cpp|ONScripter*.cpp|AnimationInfo.cpp|FontInfo.cpp|DirtyRect.cpp|Encoding.cpp|resize_image.cpp) ;;
    *) continue ;;
    esac
    echo "$f"
  done | xargs -P "$(nproc)" -I{} sh -c "$CXX -O2 -DWIN32 -DUNICODE -D_UNICODE -DUTF8_CAPTION -I$P/include -I$P/include/SDL -c {} -o {}.o"
  $CXX -static -mwindows -o "$DIR/$EXE" ./*.o -L"$P/lib" -lmingw32 -lSDLmain -lSDL_ttf -lSDL_image -lSDL_mixer -lSDL \
    -lfreetype -lpng16 -lz -lbz2 -lwinmm -lgdi32 -ldxguid -luser32 -limm32 -lole32 -loleaut32 -lversion -liconv 2>/dev/null ||
  $CXX -static -mwindows -o "$DIR/$EXE" ./*.o -L"$P/lib" -lmingw32 -lSDLmain -lSDL_ttf -lSDL_image -lSDL_mixer -lSDL \
    -lfreetype -lpng16 -lz -lbz2 -lwinmm -lgdi32 -ldxguid -luser32 -limm32 -lole32 -loleaut32 -lversion
  cp COPYING "$DIR/COPYING.onscripter"
  echo "built $DIR/$EXE"
done
