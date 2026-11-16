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
#   Rust       rustup, for building sweetpad itself
#
# Swift Build and the macro plugin ship in the sweetpad package. A sweetpad
# built from a checkout finds them in libexec/sweetpad beside its binary;
# build-swift-helpers.sh puts them there, for example
#   build-swift-helpers.sh ~/sweetpad-target/debug
#
# The SDK comes from your own Xcode (XCODE_APP, default /Applications/Xcode.app,
# visible inside OrbStack at the same path). Apple's SDK license ties the SDK to
# Apple-branded hardware; this script never downloads or redistributes it.
#
# Knobs (env): SWEETPAD_VM (OrbStack machine name, default sweetpad-linux),
# XCODE_APP, SWIFT_VERSION, XTOOL_VERSION, XTOOL_SHA256,
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
print(*(pins[key] for key in ("swift", "xtool")), pins["xtoolSha256"][arch])
PY
)"
read -r pinned_swift pinned_xtool pinned_xtool_sha <<< "$requirements"
SWIFT_VERSION="${SWIFT_VERSION:-$pinned_swift}"
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
    SWEETPAD_BUILD_MACHINE_OS_BUILD="$(sw_vers -buildVersion)" \
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
mkdir -p "$DOWNLOADS" "$HOME/swift" "$HOME/bin"

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
# BuildMachineOSBuild: the Mac's build, else Xcode's own build machine's.
python3 - "$BUNDLE/build-machine.plist" "$XCODE_APP/Contents/Info.plist" "${SWEETPAD_BUILD_MACHINE_OS_BUILD:-}" <<'PY'
import os, plistlib, sys
target, info, build = sys.argv[1:]
if not build and os.path.isfile(info):
    with open(info, "rb") as source:
        build = plistlib.load(source).get("BuildMachineOSBuild", "")
if build:
    with open(target, "wb") as out:
        plistlib.dump({"BuildMachineOSBuild": build}, out)
PY
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

if [ "${SWEETPAD_SKIP_RUST:-0}" != 1 ]; then
  section "Rust"
  if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  fi
  "$HOME/.cargo/bin/cargo" --version
fi

echo
echo "ready: use sweetpad build with your project, scheme, and destination"
