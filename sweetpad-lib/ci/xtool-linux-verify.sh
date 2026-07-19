#!/usr/bin/env bash
#
# Local end-to-end check of the xtool backend's *Linux* build path — no CI.
#
# The Mac builds the Darwin Swift SDK from its own installed Xcode; a Linux Swift
# container then cross-compiles a sweetpad-generated package against that SDK,
# bind-mounted straight in (not round-tripped through CI artifacts). The result is
# a Mach-O arm64 iOS binary produced on Linux.
#
#   1. (Mac)  xtool sdk build <Xcode.app> <cache>      -> darwin.artifactbundle   [cached]
#   2. (Mac)  xcodegen generate ci/fixture-xtool;  sweetpad build generate --backend xtool
#   3. (cont) swift sdk install <bundle> (+ clang headers);  xtool dev build
#
# `xtool sdk build` takes the installed Xcode.app directly (no Xcode.xip needed),
# and emits a bundle whose linker toolset (ld64.lld) is a *Linux* binary for the
# target host arch — so a Mac-built bundle is exactly what the container consumes.
#
# Requires on the Mac: a running Docker, xcodegen, and xtool
#   (`brew install xtool-org/tap/xtool`). The sweetpad binary comes from
#   SWEETPAD_BIN, else it is built with `cargo build --bin sweetpad`.
# The container's Swift version is pinned to the Mac's Xcode toolchain so the
# Darwin SDK's binary swiftmodules load cleanly.
#
# Knobs (env): SWEETPAD_BIN, XTOOL_VERIFY_CACHE, REBUILD_SDK=1, XTOOL_VERIFY_UBUNTU.
#
# Known limitation: apps using `#Preview` cannot cross-build — the SwiftUI
# previews macro (PreviewsMacros) is a macOS-only host plugin with no Linux build.
# The fixture omits it; see docs/dev/build-backends.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
LIB_ROOT="$(cd "$ROOT/.." && pwd)"
FIXTURE="$ROOT/fixture-xtool"
CACHE="${XTOOL_VERIFY_CACHE:-$HOME/Library/Caches/sweetpad-xtool}"
UBUNTU="${XTOOL_VERIFY_UBUNTU:-jammy}"   # jammy (22.04) matches xtool's own image

section() { echo; echo "==== $* ===="; }
fail() { echo "  ✗ $*" >&2; exit 1; }

# The app name — and thus the .xcodeproj and .app basenames — is the fixture's
# XcodeGen `name:`, the single source of truth.
APP_NAME="$(sed -n 's/^name:[[:space:]]*//p' "$FIXTURE/project.yml")"
[ -n "$APP_NAME" ] || fail "could not read 'name:' from $FIXTURE/project.yml"
PROJECT="$FIXTURE/$APP_NAME.xcodeproj"

# --- preflight ---------------------------------------------------------------
section "preflight"
command -v docker   >/dev/null || fail "docker not found"
docker info >/dev/null 2>&1     || fail "docker daemon not running (start Docker Desktop)"
command -v xcodegen >/dev/null  || fail "xcodegen not found (brew install xcodegen)"
command -v xtool    >/dev/null  || fail "xtool not found (brew install xtool-org/tap/xtool)"

XCODE_APP="$(cd "$(xcode-select -p)/../.." && pwd)"
[ -d "$XCODE_APP/Contents/Developer" ] || fail "could not locate Xcode.app from xcode-select -p"

SWIFT_VER="$(swift --version 2>/dev/null | sed -n 's/.*Swift version \([0-9][0-9.]*\).*/\1/p')"
[ -n "$SWIFT_VER" ] || fail "could not parse the Xcode Swift version"
IMAGE="swift:${SWIFT_VER}-${UBUNTU}"

case "$(uname -m)" in
  arm64|aarch64) SDK_ARCH=arm64;  APPIMAGE_ARCH=aarch64 ;;
  x86_64)        SDK_ARCH=x86_64; APPIMAGE_ARCH=x86_64  ;;
  *) fail "unsupported host arch $(uname -m)" ;;
esac

SWEETPAD_BIN="${SWEETPAD_BIN:-}"
if [ -z "$SWEETPAD_BIN" ]; then
  echo "  building the sweetpad binary (set SWEETPAD_BIN to skip)…"
  ( cd "$LIB_ROOT" && cargo build --bin sweetpad >/dev/null )
  SWEETPAD_BIN="$LIB_ROOT/target/debug/sweetpad"
fi
[ -x "$SWEETPAD_BIN" ] || fail "sweetpad binary not executable: $SWEETPAD_BIN"

echo "  Xcode:     $XCODE_APP"
echo "  Swift:     $SWIFT_VER  →  container image $IMAGE  ($SDK_ARCH)"
echo "  sweetpad:  $SWEETPAD_BIN"

# --- 1. Darwin Swift SDK from the local Xcode (cached) -----------------------
section "Darwin SDK (built on the Mac from Xcode.app)"
BUNDLE="$CACHE/sdk-out/darwin.artifactbundle"
if [ -d "$BUNDLE" ] && [ -z "${REBUILD_SDK:-}" ]; then
  echo "  reusing cached bundle (REBUILD_SDK=1 to rebuild): $BUNDLE"
else
  rm -rf "$CACHE/sdk-out"
  xtool sdk build "$XCODE_APP" "$CACHE/sdk-out" --arch "$SDK_ARCH"
fi
[ -f "$BUNDLE/info.json" ] || fail "SDK bundle looks incomplete: $BUNDLE"

# --- xtool AppImage for the container (cached) -------------------------------
APPIMG="$CACHE/dl/xtool-${APPIMAGE_ARCH}.AppImage"
if [ ! -f "$APPIMG" ]; then
  echo "  downloading xtool AppImage ($APPIMAGE_ARCH)…"
  url="$(curl -fsSL https://api.github.com/repos/xtool-org/xtool/releases/latest \
         | grep -oE "https://[^\"]+xtool-${APPIMAGE_ARCH}\.AppImage" | head -1)"
  [ -n "$url" ] || fail "could not resolve the xtool AppImage download URL"
  mkdir -p "$CACHE/dl"; curl -fsSL -o "$APPIMG" "$url"; chmod +x "$APPIMG"
fi

# --- 2. generate the package on the Mac --------------------------------------
section "generate xtool config from the Xcode project"
( cd "$FIXTURE" && xcodegen generate >/dev/null )
"$SWEETPAD_BIN" --project "$PROJECT" build generate --backend xtool
[ -f "$FIXTURE/Package.swift" ] && [ -f "$FIXTURE/xtool.yml" ] || fail "generator did not write Package.swift + xtool.yml"

# --- 3. cross-build in the Linux container -----------------------------------
# The recipe runs as a `bash -s` heredoc (lintable, no quote-escaping); the
# values that must cross into the container are passed explicitly with `-e`.
section "cross-build on Linux ($IMAGE)"
docker run --rm -i \
  -e APPIMAGE_EXTRACT_AND_RUN=1 \
  -e APP_NAME="$APP_NAME" \
  -v "$BUNDLE:/sdk/darwin.artifactbundle:ro" \
  -v "$APPIMG:/dl/xtool.AppImage:ro" \
  -v "$FIXTURE:/app" \
  "$IMAGE" bash -s <<'INNER'
set -euo pipefail

# xtool: extract the AppImage once (Docker has no FUSE) and expose it.
cd /opt && /dl/xtool.AppImage --appimage-extract >/dev/null
ln -sf /opt/squashfs-root/usr/bin/xtool /usr/local/bin/xtool
xtool --version

# Install the Mac-built Darwin SDK where SwiftPM looks for Swift SDKs, then
# backfill the clang resource headers (xtool does this at install time; some
# xtool versions strip Xcode's copy from the bundle). arm64-apple-ios is the iOS
# *device* target (always arm64, independent of the Linux host arch); the
# swiftResourcesPath is identical across the targets the bundle defines.
swift sdk install /sdk/darwin.artifactbundle
RES="$(swift sdk configure darwin arm64-apple-ios --show-configuration | sed -n 's/^swiftResourcesPath: //p')"
[ -e "$RES/clang/include" ] || cp -a "$(clang -print-resource-dir)/include" "$RES/clang/include"
swift sdk list

cd /app && rm -rf .build xtool
xtool dev build

# Assert xtool emitted a 64-bit Mach-O (iOS) binary: magic MH_MAGIC_64
# (0xFEEDFACF) is the little-endian on-disk bytes "cffaedfe". Reading the magic
# directly avoids installing the `file` package (keeps the container offline).
bin="xtool/$APP_NAME.app/$APP_NAME"
[ -f "$bin" ] || { echo "  ✗ no app binary produced at $bin" >&2; exit 1; }
magic="$(head -c4 "$bin" | od -An -tx1 | tr -d ' \n')"
[ "$magic" = "cffaedfe" ] || { echo "  ✗ not a Mach-O 64-bit binary at $bin (magic: $magic)" >&2; exit 1; }
echo "  ✓ produced a Mach-O iOS binary on Linux: $bin"
INNER

section "PASS"
echo "  the xtool backend cross-built a sweetpad-generated package on Linux."
