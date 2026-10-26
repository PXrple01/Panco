#!/usr/bin/env bash
# Vorschau-Build für Windows aus Linux (MinGW). Für echte Releases unter Windows mit MSVC bauen
# (siehe pc-app/README.md) – das ist schneller, nutzt optimierte CPU-Befehle und braucht keine Tricks.
#
# Voraussetzungen (Ubuntu): mingw-w64 cmake clang, rustup target add x86_64-pc-windows-gnu
# Ergebnis: pc-app/target/x86_64-pc-windows-gnu/release/{panco.exe,WebView2Loader.dll}
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
TARGET=x86_64-pc-windows-gnu
# Ältere MinGW-Header kennen einige Typen nicht, die ggml (whisper.cpp) verwendet.
export CFLAGS_x86_64_pc_windows_gnu="-include $ROOT/tools/cross/mingw-compat.h"
export CXXFLAGS_x86_64_pc_windows_gnu="$CFLAGS_x86_64_pc_windows_gnu"
# Beim Cross-Build erkennt ggml die CPU nicht selbst: AVX2/FMA einschalten (jeder Gaming-PC ab ~2013).
export GGML_NATIVE=OFF GGML_AVX=ON GGML_AVX2=ON GGML_FMA=ON GGML_F16C=ON GGML_BMI2=ON
# C++-Laufzeit statisch einbinden, damit die .exe keine MinGW-DLLs braucht: whisper-rs-sys verlangt
# "-lstdc++" – ein Shim-Ordner lässt das auf die statische Bibliothek zeigen.
SHIM="$ROOT/pc-app/target/mingw-static-shim"
mkdir -p "$SHIM"
cp -f "$(x86_64-w64-mingw32-g++ -print-file-name=libstdc++.a)" "$SHIM/libstdc++.dll.a"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L native=$SHIM -C link-arg=-static-libgcc \
  -C link-arg=-Wl,--start-group -C link-arg=-lmingw32 -C link-arg=-lmingwex -C link-arg=-lmsvcrt \
  -C link-arg=-lkernel32 -C link-arg=-lgcc -C link-arg=-l:libgcc_eh.a -C link-arg=-Wl,--end-group"

npm run -s -w pc-app ui:build --prefix "$ROOT"
cd "$ROOT/pc-app"
build() { cargo build -p panco --release --target $TARGET --features tauri/custom-protocol; }
if ! build; then
  # MinGW-CMake benennt die ggml-Bibliotheken "ggml.a" statt "libggml.a" – Kopien anlegen, nochmal.
  for dir in target/$TARGET/release/build/whisper-rs-sys-*/out/lib; do
    for f in "$dir"/ggml*.a; do cp -f "$f" "$dir/lib$(basename "$f")"; done
    cp -f "$dir"/libggml*.a "$dir/.." 2>/dev/null || true
  done
  build
fi
ls -la target/$TARGET/release/panco.exe target/$TARGET/release/WebView2Loader.dll
