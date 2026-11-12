#!/usr/bin/env bash
#
# Provision a Linux machine that builds Xcode projects without Xcode, for
# the sweetpad CLI. Run it on the Mac and it creates (or reuses) an OrbStack
# machine and provisions it; run it on a Linux host and it provisions that host.
# Every step is skipped when its result already exists, so a rerun is cheap.
#
#   Swift      the release toolchain, PGP-checked against swift.org's keys
#   xtool      its Darwin Swift SDK: the iOS and macOS SDKs plus an ld64.lld
#              that links for Apple platforms, extracted from an Xcode.app
#   swbuild    Swift Build's command-line driver, which the toolchain doesn't
#              ship, built from the separately pinned swiftlang/swift-build tag,
#              with swift-build-linux.patch, and linked into ~/bin
#   sweetpad-macros
#              the plugin for Apple's macros that Linux has no working build
#              of (SwiftData's, #Predicate, #Preview), built from
#              sweetpad-cli/macros into ~/bin
#   Rust       rustup, for building sweetpad itself
#
# The SDK comes from your own Xcode (XCODE_APP, default /Applications/Xcode.app,
# visible inside OrbStack at the same path). Apple's SDK license ties the SDK to
# Apple-branded hardware; this script never downloads or redistributes it.
#
# Knobs (env): SWEETPAD_VM (OrbStack machine name, default sweetpad-linux),
# XCODE_APP, SWIFT_VERSION, SWIFT_BUILD_VERSION, XTOOL_VERSION, XTOOL_SHA256,
# SWEETPAD_REFRESH_SDK=1 (replace a stale slim SDK, with rollback on error).
# SWEETPAD_SKIP_RUST=1 skips Rust when using the prebuilt CLI package.
set -euo pipefail

XTOOL_SHA256_OVERRIDE="${XTOOL_SHA256:-}"
VM="${SWEETPAD_VM:-sweetpad-linux}"
XCODE_APP="${XCODE_APP:-/Applications/Xcode.app}"
requirements="$(python3 - "$(dirname "$0")/linux-toolchain.json" "$(uname -m)" <<'PY'
import json, sys
with open(sys.argv[1]) as source:
    pins = json.load(source)
arch = sys.argv[2]
# A Mac only forwards the pins; its OrbStack guest selects its own host tools.
arch = {"arm64": "aarch64"}.get(arch, arch)
if arch not in pins["xtoolSha256"]:
    raise SystemExit("setup supports ARM64 and x86_64 hosts")
print(*(pins[key] for key in ("swift", "swiftBuild", "xtool")), pins["xtoolSha256"][arch])
PY
)"
read -r pinned_swift pinned_swb pinned_xtool pinned_xtool_sha <<< "$requirements"
SWIFT_VERSION="${SWIFT_VERSION:-$pinned_swift}"
SWIFT_BUILD_VERSION="${SWIFT_BUILD_VERSION:-$pinned_swb}"
XTOOL_VERSION="${XTOOL_VERSION:-$pinned_xtool}"
XTOOL_SHA256="${XTOOL_SHA256:-$pinned_xtool_sha}"

if [ "$(uname -s)" = Darwin ]; then
  command -v orb >/dev/null || { echo "error: OrbStack's 'orb' is not installed" >&2; exit 1; }
  orb status >/dev/null 2>&1 || orb start
  orb list -q | grep -qx "$VM" || orb create ubuntu:noble "$VM"
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  phrase_export="${XDG_CACHE_HOME:-$HOME/.cache}/sweetpad/sdk-resources/appintents-phrases.json"
  python3 "$(dirname "$self")/linux-appintents-resources.py" --xcode "$XCODE_APP" --out "$phrase_export"
  exec orb -m "$VM" env XCODE_APP="$XCODE_APP" SWIFT_VERSION="$SWIFT_VERSION" \
    SWEETPAD_APPINTENTS_RESOURCE_EXPORT="$phrase_export" \
    SWIFT_BUILD_VERSION="$SWIFT_BUILD_VERSION" \
    XTOOL_VERSION="$XTOOL_VERSION" XTOOL_SHA256="$XTOOL_SHA256_OVERRIDE" \
    SWEETPAD_SKIP_RUST="${SWEETPAD_SKIP_RUST:-0}" \
    SWEETPAD_REFRESH_SDK="${SWEETPAD_REFRESH_SDK:-0}" bash "$self"
fi

ARCH="$(uname -m)"
. "$(dirname "$0")/linux-host-tools.sh"
sweetpad_linux_host
if [ -n "${SWEETPAD_SDK_ARCHIVE:-}" ]; then
  python3 "$(dirname "$0")/linux-sdk.py" inspect --archive "$SWEETPAD_SDK_ARCHIVE"
fi
TAG="swift-$SWIFT_VERSION-RELEASE"
SWIFT_DIR="$HOME/swift/$TAG"
DOWNLOADS="$HOME/downloads"
mkdir -p "$DOWNLOADS" "$HOME/swift" "$HOME/src" "$HOME/bin"

section() { echo; echo "==== $* ===="; }

section "Host packages and Swift $SWIFT_VERSION"
sweetpad_install_host_tools

section "xtool $XTOOL_VERSION"
sweetpad_install_xtool

section "Darwin SDK from $XCODE_APP"
BUNDLE="${SWEETPAD_DARWIN_SDK:-$HOME/.swiftpm/swift-sdks/darwin.artifactbundle}"
if [ -n "${SWEETPAD_SDK_ARCHIVE:-}" ]; then
  import_args=(import --archive "$SWEETPAD_SDK_ARCHIVE" --destination "$BUNDLE" --xtool "$HOME/bin/xtool")
  [ -z "${SWEETPAD_AEA_KEY_FILE:-}" ] || import_args+=(--aea-key-file "$SWEETPAD_AEA_KEY_FILE")
  [ -z "${SWEETPAD_AEA_PUBLIC_KEY_FILE:-}" ] || import_args+=(--aea-public-key-file "$SWEETPAD_AEA_PUBLIC_KEY_FILE")
  [ "${SWEETPAD_REFRESH_SDK:-0}" != 1 ] || import_args+=(--replace)
  python3 "$(dirname "$0")/linux-sdk.py" "${import_args[@]}"
else
if [ "${SWEETPAD_REFRESH_SDK:-0}" = 1 ] && [ -d "$BUNDLE" ]; then
  backup="$(mktemp -d "$DOWNLOADS/sweetpad-sdk-backup.XXXXXX")"
  mv "$BUNDLE" "$backup/bundle"
  if xtool sdk install --slim "$XCODE_APP"; then
    rm -rf "$backup"
  else
    rm -rf "$BUNDLE"
    mv "$backup/bundle" "$BUNDLE"
    rmdir "$backup"
    echo "error: SDK refresh failed; restored the previous SDK" >&2
    exit 1
  fi
elif [ ! -f "$BUNDLE/toolset-swb.json" ]; then
  [ -d "$XCODE_APP" ] || { echo "error: $XCODE_APP is not visible here" >&2; exit 1; }
  # --slim copies just the files the SDK needs; without it an unpacked
  # Xcode.app is copied whole and then hard-linked.
  xtool sdk install --slim "$XCODE_APP"
fi
# An Xcode update does not update an existing slim SDK. Compare the SDKs
# actually installed, rather than the Xcode.app directory's name.
python3 - "$XCODE_APP/Contents" "$BUNDLE" <<'PY'
from pathlib import Path
import plistlib
import sys

def versions(root):
    found = {}
    for path in Path(root).glob("Developer/Platforms/*.platform/Developer/SDKs/*.sdk/SDKSettings.plist"):
        with path.open("rb") as source:
            name = plistlib.load(source).get("CanonicalName")
        found.setdefault(path.parents[3].name, set()).add(name)
    return found

source, installed = map(versions, sys.argv[1:])
for platform, sdk in installed.items():
    if source.get(platform) != sdk:
        raise SystemExit(f"error: stale extracted SDK for {platform}: {sorted(sdk)}; Xcode has {sorted(source.get(platform, set()))}. Rerun with SWEETPAD_REFRESH_SDK=1.")
PY
# xtool's slim SDK includes iOS and macOS. Add the other platforms present
# in this Xcode so embedded watch products and their SDKs resolve as well.
python3 "$(dirname "$0")/linux-sdk-platforms.py" "$XCODE_APP" "$BUNDLE"
if [ -n "${SWEETPAD_APPINTENTS_RESOURCE_EXPORT:-}" ]; then
  cp "$SWEETPAD_APPINTENTS_RESOURCE_EXPORT" "$BUNDLE/appintents-phrases.json"
fi
# xtool leaves out the version files, which an app's Info.plist takes
# DTXcode, DTXcodeBuild and DTPlatformBuild from. Xcode's own goes beside the
# bundle's Developer folder, where Xcode.app keeps it, for sweetpad to read;
# each platform's goes in the platform, where Swift Build reads it.
if [ -f "$XCODE_APP/Contents/version.plist" ]; then
  cat "$XCODE_APP/Contents/version.plist" > "$BUNDLE/version.plist"
fi
for platform in "$BUNDLE"/Developer/Platforms/*.platform; do
  source="$XCODE_APP/Contents/Developer/Platforms/$(basename "$platform")/version.plist"
  [ ! -f "$source" ] || cat "$source" > "$platform/version.plist"
done
# Slim SDK extraction omits the executable Swift Build copies into Messages
# and sticker extensions.
for platform in "$BUNDLE"/Developer/Platforms/*.platform; do
  source="$XCODE_APP/Contents/Developer/Platforms/$(basename "$platform")/Library/Application Support/MessagesApplicationExtensionStub"
  destination="$platform/Library/Application Support/MessagesApplicationExtensionStub"
  if [ -d "$source" ] && ! cmp -s "$source/MessagesApplicationExtensionStub" "$destination/MessagesApplicationExtensionStub"; then
    mkdir -p "$destination"
    # Stream the copy: optimized sparse copies from OrbStack's shared Mac
    # mount can produce a correctly sized file containing only zeroes.
    tar -C "$source" -cf - . | tar -C "$destination" -xf -
  fi
done
swift sdk list

# Cross-compilation uses the extracted toolchain's resource directory. The
# Swift/C++ interoperability headers are beside it, outside the platform SDK.
source="$XCODE_APP/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift"
destination="$BUNDLE/Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift"
if [ -d "$source" ]; then
  mkdir -p "$destination"
  tar -C "$source" -cf - . | tar -C "$destination" -xf -
fi
fi # SDK archive or visible Xcode.app

SWB_TAG="swift-$SWIFT_BUILD_VERSION-RELEASE"
section "swbuild ($SWB_TAG)"
PATCH="$(cd "$(dirname "$0")" && pwd)/swift-build-linux.patch"
GENERIC_PATCH="$(dirname "$PATCH")/swift-build-generic-debug-bridges.patch"
LLBUILD_PATCH="$(dirname "$PATCH")/swift-llbuild-directory-race.patch"
LOCKFILE="$(dirname "$PATCH")/swift-build-Package.resolved"
SWB="$HOME/src/swift-build"
# A new tag, lockfile, or base patch gets its own cache. Keep older checkouts
# and local edits intact instead of resetting a developer's source tree.
if [ -d "$SWB/.git" ]; then
  current="$(git -C "$SWB" rev-parse HEAD)"
  wanted="$(git -C "$SWB" rev-parse "$SWB_TAG^{commit}" 2>/dev/null || true)"
  if [ "$current" != "$wanted" ] || ! cmp -s "$LOCKFILE" "$SWB/Package.resolved" ||
      ! git -C "$SWB" apply --reverse --check "$PATCH" 2>/dev/null ||
      ! git -C "$SWB" apply --reverse --check "$GENERIC_PATCH" 2>/dev/null ||
      ! git -C "$SWB/.build/checkouts/swift-llbuild" apply --reverse --check "$LLBUILD_PATCH" 2>/dev/null; then
    cache_key="$(cat "$PATCH" "$GENERIC_PATCH" "$LLBUILD_PATCH" "$LOCKFILE" | sha256sum | cut -c1-12)"
    SWB="$HOME/src/swift-build-$SWIFT_BUILD_VERSION-$cache_key"
  fi
fi
[ -d "$SWB/.git" ] || git clone -q --depth 1 --branch "$SWB_TAG" https://github.com/swiftlang/swift-build.git "$SWB"
if [ "$(git -C "$SWB" rev-parse HEAD)" != "$(git -C "$SWB" rev-parse "$SWB_TAG^{commit}")" ]; then
  echo "error: $SWB must use $SWB_TAG; use a separate cache for another revision" >&2
  exit 1
fi
# swift-build-linux.patch ports host-specific build tasks. A macOS app's build registers
# it with Launch Services through /System/…/lsregister, and Swift Build fails
# the build when that can't start, where it only notes a run that fails; the
# patch notes both. Swift Build's console asks no one for a signing identity
# or provisioning profile, so it signs only ad hoc; the patch has it ask the
# tool SWEETPAD_PROVISIONING names, which sweetpad sets for a device build.
# It runs /usr/bin/derq by absolute path for a signed target's DER
# entitlements; the patch has it look derq up on the search path first.
# Its Mach-O reader takes the file's structures from the Darwin SDK and
# fails every read elsewhere; the patch defines them in a new file,
# SWBUtil/MachOStructures.swift, and names the architectures itself. And it
# finds a text encoding by name only through CoreFoundation, so a Mac app's
# .strings can't be written as UTF-16; the patch names the encodings
# Foundation converts on every platform and detects Unicode BOMs. LLVM
# libtool version discovery avoids treating its version as a cctools version.
# Incompatible cached checkouts remain intact; the cache selection above uses
# a separate directory for this revision. New files are marked intent-to-add, so `git diff` in the
# checkout writes the whole patch back; `git apply --intent-to-add` would
# drop every other file from the index.
if ! git -C "$SWB" apply --reverse --check "$PATCH" 2>/dev/null; then
  if ! git -C "$SWB" diff --quiet || [ -n "$(git -C "$SWB" ls-files --others --exclude-standard -- Sources)" ]; then
    echo "error: $SWB contains local edits; preserve it and select an empty setup cache" >&2
    exit 1
  fi
  git -C "$SWB" apply "$PATCH"
  git -C "$SWB" add --intent-to-add -- $(git -C "$SWB" ls-files --others --exclude-standard -- Sources)
  rm -f "$SWB/.build/release/swbuild"
fi
# Correct only the two self-nesting Optional Objective-C bridge helpers in
# generated C++ headers. Keep DEBUG and all other forced debugger thunks.
if ! git -C "$SWB" apply --reverse --check "$GENERIC_PATCH" 2>/dev/null; then
  git -C "$SWB" apply --check "$GENERIC_PATCH"
  git -C "$SWB" apply "$GENERIC_PATCH"
  rm -f "$SWB/.build/release/swbuild"
fi
# Resolve dependencies before patching the pinned llbuild checkout. Its directory
# iterator must tolerate entries removed by dependency replacement scripts.
cp "$LOCKFILE" "$SWB/Package.resolved"
(cd "$SWB" && swift package --force-resolved-versions resolve)
LLBUILD="$SWB/.build/checkouts/swift-llbuild"
if ! git -C "$LLBUILD" apply --reverse --check "$LLBUILD_PATCH" 2>/dev/null; then
  git -C "$LLBUILD" apply --check "$LLBUILD_PATCH"
  git -C "$LLBUILD" apply "$LLBUILD_PATCH"
  (cd "$SWB" && swift build --jobs "${SWEETPAD_SETUP_JOBS:-4}" -c release --product swbuild && swift build --jobs "${SWEETPAD_SETUP_JOBS:-4}" -c release --product SWBBuildServiceBundle)
fi
if [ ! -x "$SWB/.build/release/swbuild" ] || [ ! -x "$SWB/.build/release/SWBBuildServiceBundle" ]; then
  (cd "$SWB" && swift build --jobs "${SWEETPAD_SETUP_JOBS:-4}" -c release --product swbuild && swift build --jobs "${SWEETPAD_SETUP_JOBS:-4}" -c release --product SWBBuildServiceBundle)
fi
# sweetpad looks swbuild up on PATH; it finds its service bundle through the
# link.
ln -sf "$SWB/.build/release/swbuild" "$HOME/bin/swbuild"
ls "$SWB/.build/release/swbuild" "$SWB/.build/release/SWBBuildServiceBundle"

section "sweetpad-macros"
# Built outside the checkout, which the Mac shares, so the two builds keep
# apart.
MACROS="$(cd "$(dirname "$0")/../../sweetpad-cli/macros" && pwd)"
swift build --jobs "${SWEETPAD_SETUP_JOBS:-4}" -c release --package-path "$MACROS" --scratch-path "$HOME/src/sweetpad-macros" --product sweetpad-macros
ln -sf "$HOME/src/sweetpad-macros/release/sweetpad-macros" "$HOME/bin/sweetpad-macros.bin"
ln -sf "$(command -v swift)" "$HOME/bin/sweetpad-macro-swift"
# Remove the old executable symlink before installing the launcher.
rm -f "$HOME/bin/sweetpad-macros"
install -m 755 "$(dirname "$0")/linux-macro-launcher.sh" "$HOME/bin/sweetpad-macros"
ls "$HOME/bin/sweetpad-macros"

if [ "${SWEETPAD_SKIP_RUST:-0}" != 1 ]; then
  section "Rust"
  if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  fi
  "$HOME/.cargo/bin/cargo" --version
fi

echo
echo "ready: use sweetpad build with your project, scheme, and destination"
