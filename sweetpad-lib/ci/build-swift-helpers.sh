#!/usr/bin/env bash
#
# Build the Swift helpers the Linux package ships beside the sweetpad binary:
#
#   swbuild, SWBBuildServiceBundle and SwiftBuild_*.bundle
#              Swift Build's driver, its build service and the spec resources
#              the service reads, from the pinned swiftlang/swift-build tag
#              with the patches beside this script
#   sweetpad-macros
#              the plugin for Apple's macros that Linux has no working build
#              of (SwiftData's, #Predicate, #Preview), from sweetpad-cli/macros
#
# The Swift runtime and Foundation libraries they load are copied from the
# pinned toolchain beside them, and their only RUNPATH is $ORIGIN, so they
# run with any Swift toolchain, or none, on PATH. Release CI runs this on
# Debian 12, the oldest glibc the package supports.
#
#   build-swift-helpers.sh OUT_DIR
#
# OUT_DIR receives libexec/sweetpad/ and licenses/swift-build/. Sources and
# build products are cached in SWEETPAD_HELPERS_CACHE (default
# ~/.cache/sweetpad/swift-helpers), keyed by the pins and patches.
set -euo pipefail

[ $# -eq 1 ] || { echo "usage: $0 OUT_DIR" >&2; exit 2; }
OUT="$(mkdir -p "$1" && cd "$1" && pwd)"
CI="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$CI/../.." && pwd)"
JOBS="${SWEETPAD_SETUP_JOBS:-$(nproc)}"
read -r SWIFT_VERSION SWIFT_BUILD_VERSION < <(python3 -c 'import json,sys; pins = json.load(open(sys.argv[1])); print(pins["swift"], pins["swiftBuild"])' "$CI/linux-toolchain.json")
SWB_TAG="swift-$SWIFT_BUILD_VERSION-RELEASE"
PATCH="$CI/swift-build-linux.patch"
GENERIC_PATCH="$CI/swift-build-generic-debug-bridges.patch"
LLBUILD_PATCH="$CI/swift-llbuild-directory-race.patch"
LLBUILD_SPAWN_PATCH="$CI/swift-llbuild-spawn-descriptors.patch"
LOCKFILE="$CI/swift-build-Package.resolved"
# What the helpers were built from, which 'sweetpad doctor' compares with the
# patches the running sweetpad carries. The same digest names the cache.
DIGEST="$(cat "$PATCH" "$GENERIC_PATCH" "$LLBUILD_PATCH" "$LLBUILD_SPAWN_PATCH" "$LOCKFILE" | sha256sum | cut -d' ' -f1)"
CACHE="${SWEETPAD_HELPERS_CACHE:-$HOME/.cache/sweetpad/swift-helpers}"
SWB="$CACHE/swift-build-$SWIFT_BUILD_VERSION-${DIGEST:0:12}"
# Leave out the toolchain's runtime folder, which SwiftPM puts before
# $ORIGIN, so the copies beside the helpers are the ones loaded.
FLAGS=(-Xswiftc -no-toolchain-stdlib-rpath)

section() { echo; echo "==== $* ===="; }
section "Swift Build $SWB_TAG"
swift --version 2>&1 | grep -q "Swift version $SWIFT_VERSION " || {
  echo "error: the bundled runtime must come from Swift $SWIFT_VERSION; 'swift' on PATH is $(swift --version 2>&1 | head -1)" >&2
  exit 1
}
RUNTIME="$(swiftc -print-target-info | python3 -c 'import json,sys; print(json.load(sys.stdin)["paths"]["runtimeLibraryPaths"][0])')"
if [ ! -d "$SWB/.git" ]; then
  rm -rf "$SWB.partial"
  git clone -q --depth 1 --branch "$SWB_TAG" https://github.com/swiftlang/swift-build.git "$SWB.partial"
  # swift-build-linux.patch ports host-specific build tasks: Launch Services
  # registration, signing through SWEETPAD_PROVISIONING, derq on PATH, Mach-O
  # structures and text encodings without the Darwin SDK, LLVM libtool
  # versions, an MD5 without CryptoKit, and stale loose asset catalog outputs.
  # The generic patch corrects two self-nesting Optional Objective-C bridge
  # helpers in generated C++ headers.
  git -C "$SWB.partial" apply "$PATCH" "$GENERIC_PATCH"
  cp "$LOCKFILE" "$SWB.partial/Package.resolved"
  (cd "$SWB.partial" && swift package --force-resolved-versions resolve)
  # llbuild's directory iterator must tolerate entries a script removes, and
  # Linux has no POSIX_SPAWN_CLOEXEC_DEFAULT, so without the spawn patch every
  # task inherits the build service's descriptors and a process a script
  # leaves running keeps swbuild from exiting.
  git -C "$SWB.partial/.build/checkouts/swift-llbuild" apply "$LLBUILD_PATCH" "$LLBUILD_SPAWN_PATCH"
  mv "$SWB.partial" "$SWB"
fi
for product in swbuild SWBBuildServiceBundle; do
  (cd "$SWB" && swift build --jobs "$JOBS" -c release "${FLAGS[@]}" --product "$product")
done

section "sweetpad-macros"
# Built outside the checkout, which a Mac may share with this host.
swift build --jobs "$JOBS" -c release "${FLAGS[@]}" --package-path "$REPO/sweetpad-cli/macros" \
  --scratch-path "$CACHE/sweetpad-macros" --product sweetpad-macros

section "Staging into $OUT"
LIBEXEC="$OUT/libexec/sweetpad"
LICENSES="$OUT/licenses/swift-build"
rm -rf "$LIBEXEC" "$LICENSES" "$OUT/licenses/sweetpad-macros" "$OUT/licenses/swift" "$OUT/licenses/ncurses"
mkdir -p "$LIBEXEC" "$LICENSES/patches"
install -m 755 "$SWB/.build/release/swbuild" "$SWB/.build/release/SWBBuildServiceBundle" \
  "$CACHE/sweetpad-macros/release/sweetpad-macros" "$LIBEXEC/"
for bundle in "$SWB"/.build/release/SwiftBuild_*.resources "$SWB"/.build/release/SwiftBuild_*.bundle; do
  [ ! -d "$bundle" ] || cp -R "$bundle" "$LIBEXEC/"
done
# The runtime libraries the helpers need, and the ones those need in turn.
# ncurses goes along too: Arch has only the wide-character libncursesw.so.6
# and no separate libtinfo.so.6.
needed() { readelf -d "$1" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'; }
host_library() { ldconfig -p | awk -v name="$1" '$1 == name { print $NF; exit }'; }
queue=("$LIBEXEC/swbuild" "$LIBEXEC/SWBBuildServiceBundle" "$LIBEXEC/sweetpad-macros")
while [ ${#queue[@]} -gt 0 ]; do
  for library in $(needed "${queue[0]}"); do
    source=
    if [ -f "$RUNTIME/$library" ]; then
      source="$RUNTIME/$library"
    elif [ "$library" = libncurses.so.6 ] || [ "$library" = libtinfo.so.6 ]; then
      source="$(host_library "$library")"
    fi
    if [ -n "$source" ] && [ ! -f "$LIBEXEC/$library" ]; then
      install -m 644 "$(readlink -f "$source")" "$LIBEXEC/$library"
      queue+=("$LIBEXEC/$library")
    fi
  done
  queue=("${queue[@]:1}")
done
strip "$LIBEXEC/swbuild" "$LIBEXEC/SWBBuildServiceBundle" "$LIBEXEC/sweetpad-macros" "$LIBEXEC"/*.so*
# A library copied from the host has the host's RUNPATH, usually none.
for library in "$LIBEXEC"/libncurses.so.6 "$LIBEXEC"/libtinfo.so.6; do
  [ ! -f "$library" ] || patchelf --set-rpath '$ORIGIN' "$library"
done
printf '{"swiftBuild":"%s","patches":"%s","swift":"%s"}\n' "$SWIFT_BUILD_VERSION" "$DIGEST" "$SWIFT_VERSION" > "$LIBEXEC/sweetpad-swift-build.json"
cp "$SWB/LICENSE.txt" "$LICENSES/LICENSE.txt"
for checkout in "$SWB"/.build/checkouts/*; do
  for file in "$checkout"/LICENSE* "$checkout"/NOTICE*; do
    [ ! -f "$file" ] || { mkdir -p "$LICENSES/dependencies/$(basename "$checkout")"; cp "$file" "$LICENSES/dependencies/$(basename "$checkout")/"; }
  done
done
for file in "$CACHE"/sweetpad-macros/checkouts/swift-syntax/LICENSE*; do
  [ ! -f "$file" ] || { mkdir -p "$OUT/licenses/sweetpad-macros/swift-syntax"; cp "$file" "$OUT/licenses/sweetpad-macros/swift-syntax/"; }
done
cp "$PATCH" "$GENERIC_PATCH" "$LLBUILD_PATCH" "$LLBUILD_SPAWN_PATCH" "$LICENSES/patches/"
cp "$CI/swift-helpers-NOTICE.md" "$LICENSES/README.md"
for package in libncurses6 libtinfo6; do
  if [ -f "/usr/share/doc/$package/copyright" ]; then
    mkdir -p "$OUT/licenses/ncurses"
    cp "/usr/share/doc/$package/copyright" "$OUT/licenses/ncurses/$package-copyright"
  fi
done
mkdir -p "$OUT/licenses/swift"
cp "$(dirname "$(dirname "$(dirname "$RUNTIME")")")/share/swift/LICENSE.txt" "$OUT/licenses/swift/LICENSE.txt"
find "$OUT/libexec" "$OUT/licenses" -type d -exec chmod 755 {} +
find "$OUT/libexec" "$OUT/licenses" -type f ! -perm -u+x -exec chmod 644 {} +
du -sh "$LIBEXEC"
