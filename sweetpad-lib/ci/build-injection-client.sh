#!/usr/bin/env bash
#
# Milestone 5 — build the InjectionNext client dylib *from source* against the
# ACTIVE Xcode, so the injection client matches the toolchain (no prebuilt-binary
# version skew, no InjectionNext.app dependency). This is what lets hot reload
# work across Xcode versions.
#
# The recipe is upstream's own (App/Makefile): clone with submodules, then
# `xcodebuild` the InjectionNext app — its build produces the per-platform
# injection dylibs (`lib<sdk>Injection.dylib`). We extract the simulator one.
#
# Usage: build-injection-client.sh <workdir> [sdk]   (sdk default: iphonesimulator)
# Echoes the built dylib path on stdout; all progress goes to stderr.
set -euo pipefail

WORK="${1:?usage: build-injection-client.sh <workdir> [sdk]}"
SDK="${2:-iphonesimulator}"
DYLIB_NAME="lib${SDK}Injection.dylib"
TAG="${INJECTIONNEXT_TAG:-$(gh release view --repo johnno1962/InjectionNext --json tagName -q .tagName)}"

mkdir -p "$WORK"
SRC="$WORK/InjectionNext-$TAG"

if [ ! -d "$SRC" ]; then
  echo "==> cloning InjectionNext @ $TAG (with submodules)" >&2
  git clone --recurse-submodules --depth 1 --shallow-submodules \
    --branch "$TAG" https://github.com/johnno1962/InjectionNext "$SRC" >&2
fi

echo "==> building InjectionNext with $(xcodebuild -version | head -1) (this is slow; cached per Xcode)" >&2
(
  cd "$SRC/App"
  xcodebuild \
    -project InjectionNext.xcodeproj \
    -scheme InjectionNext \
    -configuration Debug \
    -destination 'platform=macOS' \
    -derivedDataPath build \
    CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO \
    build -quiet >&2
)

DYLIB="$(find "$SRC/App/build/Build/Products" -name "$DYLIB_NAME" -type f 2>/dev/null | head -1)"
if [ -z "$DYLIB" ]; then
  echo "ERROR: $DYLIB_NAME not found after build. Injection dylibs produced:" >&2
  find "$SRC/App/build/Build/Products" -name '*Injection*.dylib' >&2 || true
  exit 1
fi
echo "==> built $DYLIB" >&2
echo "$DYLIB"
