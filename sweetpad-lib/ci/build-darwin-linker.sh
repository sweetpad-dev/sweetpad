#!/usr/bin/env bash
#
# Build the linker and dSYM tool that Apple targets need on Linux, from the
# pinned upstream LLVM release:
#
#   llvm      one multicall binary with LLD's Mach-O linker and dsymutil;
#             it runs as the tool its argv[0] names, ld64.lld or dsymutil,
#             which is how sweetpad starts it
#
# Swift's own toolchain has an lld, but Swift's LLVM fork refuses to link for
# iOS, and no Swift toolchain ships dsymutil. Upstream LLVM 23.1.3 and later
# read the arm64e.x1 slices of SDK 27's stubs and need no patches. Building
# both tools into one binary shares LLVM's libraries between them. Release
# CI runs this on Debian 12, the oldest glibc the package supports.
#
#   build-darwin-linker.sh OUT_DIR
#
# OUT_DIR receives libexec/sweetpad/llvm, the record of what it was built
# from beside it, and licenses/llvm/. The source and build are cached in
# SWEETPAD_LINKER_CACHE (default ~/.cache/sweetpad/darwin-linker), keyed by
# the pinned version.
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 OUT_DIR" >&2; exit 2; }
OUT="$(mkdir -p "$1" && cd "$1" && pwd)"
CI="$(cd "$(dirname "$0")" && pwd)"
JOBS="${SWEETPAD_SETUP_JOBS:-$(nproc)}"
read -r LLVM_VERSION LLVM_SHA256 < <(python3 -c 'import json,sys; pins = json.load(open(sys.argv[1])); print(pins["llvm"], pins["llvmSourceSha256"])' "$CI/linux-toolchain.json")
CACHE="${SWEETPAD_LINKER_CACHE:-$HOME/.cache/sweetpad/darwin-linker}/llvm-$LLVM_VERSION"
SRC="$CACHE/llvm-project-$LLVM_VERSION.src"
BUILD="$CACHE/build"

for tool in cmake ninja c++ curl xz; do
  command -v "$tool" >/dev/null || {
    echo "error: $tool isn't installed; building LLVM takes cmake, ninja, a C++ compiler, curl and xz" >&2
    exit 1
  }
done
section() { echo; echo "==== $* ===="; }
section "LLVM $LLVM_VERSION source"
mkdir -p "$CACHE"
# Only what LLD and dsymutil build from; LLVM's CMake also reads libc's.
PARTS=(llvm lld cmake third-party libc libunwind/include)
if [ "$(cat "$SRC/.parts" 2>/dev/null)" != "${PARTS[*]}" ]; then
  tarball="$CACHE/llvm-project-$LLVM_VERSION.src.tar.xz"
  if [ ! -f "$tarball" ] || ! echo "$LLVM_SHA256  $tarball" | sha256sum -c --status; then
    curl -fsSL --retry 3 -o "$tarball.partial" \
      "https://github.com/llvm/llvm-project/releases/download/llvmorg-$LLVM_VERSION/llvm-project-$LLVM_VERSION.src.tar.xz"
    echo "$LLVM_SHA256  $tarball.partial" | sha256sum -c --status || {
      echo "error: llvm-project-$LLVM_VERSION.src.tar.xz doesn't match the pinned checksum" >&2
      exit 1
    }
    mv "$tarball.partial" "$tarball"
  fi
  rm -rf "$SRC" "$SRC.partial" "$BUILD"
  mkdir "$SRC.partial"
  tar -xJf "$tarball" -C "$SRC.partial" --strip-components=1 \
    "${PARTS[@]/#/llvm-project-$LLVM_VERSION.src/}"
  echo "${PARTS[*]}" > "$SRC.partial/.parts"
  mv "$SRC.partial" "$SRC"
fi

section "LLD and dsymutil"
# Only the two tools join the multicall binary, through the distribution
# components. Sections the link never reaches are dropped, and the
# libraries a Mach-O link has no use for (zlib, zstd, libxml2) stay out.
cmake -G Ninja -S "$SRC/llvm" -B "$BUILD" \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_SKIP_RPATH=ON \
  -DLLVM_ENABLE_PROJECTS=lld \
  -DLLVM_TARGETS_TO_BUILD="AArch64;X86" \
  -DLLVM_TOOL_LLVM_DRIVER_BUILD=ON \
  -DLLVM_DISTRIBUTION_COMPONENTS="llvm-driver;lld;dsymutil" \
  -DLLVM_INCLUDE_TESTS=OFF -DLLVM_INCLUDE_BENCHMARKS=OFF \
  -DLLVM_INCLUDE_EXAMPLES=OFF -DLLVM_INCLUDE_DOCS=OFF \
  -DLLVM_ENABLE_ZLIB=OFF -DLLVM_ENABLE_ZSTD=OFF -DLLVM_ENABLE_LIBXML2=OFF \
  -DLLVM_ENABLE_LIBEDIT=OFF -DLLVM_ENABLE_LIBPFM=OFF -DLLVM_ENABLE_CURL=OFF \
  -DLLVM_ENABLE_HTTPLIB=OFF -DLLVM_ENABLE_TERMINFO=OFF \
  -DCMAKE_C_FLAGS="-ffunction-sections -fdata-sections" \
  -DCMAKE_CXX_FLAGS="-ffunction-sections -fdata-sections" \
  -DCMAKE_EXE_LINKER_FLAGS="-Wl,--gc-sections" \
  > "$CACHE/cmake.log"
ninja -C "$BUILD" -j "$JOBS" llvm-driver

section "Staging"
DEST="$OUT/libexec/sweetpad"
mkdir -p "$DEST" "$OUT/licenses/llvm"
rm -f "$DEST/llvm"
cp "$BUILD/bin/llvm" "$DEST/llvm"
strip --strip-all "$DEST/llvm"
cp "$SRC/llvm/LICENSE.TXT" "$OUT/licenses/llvm/LICENSE.TXT"
cp "$SRC/lld/LICENSE.TXT" "$OUT/licenses/llvm/LLD-LICENSE.TXT"
printf '{"llvm": "%s", "sourceSha256": "%s"}\n' "$LLVM_VERSION" "$LLVM_SHA256" > "$DEST/sweetpad-llvm.json"
(exec -a ld64.lld "$DEST/llvm" --version)
(exec -a dsymutil "$DEST/llvm" --version) | head -2
ls -la "$DEST/llvm"
