#!/usr/bin/env bash
#
# Provision a Linux machine that builds Xcode projects without Xcode, for
# the sweetpad CLI. Run it on the Mac and it creates (or reuses) an OrbStack
# machine and provisions it; run it on a Linux host and it provisions that host.
# Every step is skipped when its result already exists, so a rerun is cheap.
#
#   Swift      the release toolchain, PGP-checked against swift.org's keys
#   Rust       rustup, for building sweetpad itself
#   SDK        the Darwin Swift SDK, which 'sweetpad sdk write-bundle' writes
#              from an Xcode.app or a private SDK archive (linux-sdk.py)
#
# Swift Build, the macro plugin and LLVM's Apple linker ship in the sweetpad
# package. A sweetpad built from a checkout finds them in libexec/sweetpad
# beside its binary; build-swift-helpers.sh and build-darwin-linker.sh put
# them there, for example
#   build-swift-helpers.sh ~/sweetpad-target/debug
#
# Writing the SDK takes a sweetpad binary: SWEETPAD_EXE, else one built from
# this checkout, else the one on PATH.
#
# The SDK comes from your own Xcode (XCODE_APP, default /Applications/Xcode.app,
# visible inside OrbStack at the same path). Apple's SDK license ties the SDK to
# Apple-branded hardware; this script never downloads or redistributes it.
#
# Knobs (env): SWEETPAD_VM (OrbStack machine name, default sweetpad-linux),
# XCODE_APP, SWIFT_VERSION, SWEETPAD_EXE,
# SWEETPAD_REFRESH_SDK=1 (replace the SDK, with rollback on error).
# SWEETPAD_SKIP_RUST=1 skips Rust when using the prebuilt CLI package.
set -euo pipefail

VM="${SWEETPAD_VM:-sweetpad-linux}"
XCODE_APP="${XCODE_APP:-/Applications/Xcode.app}"
requirements="$(python3 - "$(dirname "$0")/linux-toolchain.json" "$(uname -m)" <<'PY'
import json, sys
with open(sys.argv[1]) as source:
    pins = json.load(source)
arch = sys.argv[2]
# A Mac only forwards the pins; its OrbStack guest selects its own host tools.
arch = {"arm64": "aarch64"}.get(arch, arch)
if arch not in ("aarch64", "x86_64"):
    raise SystemExit("setup supports ARM64 and x86_64 hosts")
print(pins["swift"])
PY
)"
SWIFT_VERSION="${SWIFT_VERSION:-$requirements}"

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

if [ "${SWEETPAD_SKIP_RUST:-0}" != 1 ]; then
  section "Rust"
  if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  fi
  "$HOME/.cargo/bin/cargo" --version
fi

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
if [ -z "${SWEETPAD_EXE:-}" ] && [ -f "$REPO/Cargo.toml" ] && [ -x "$HOME/.cargo/bin/cargo" ]; then
  section "sweetpad (cargo build)"
  # The same target folder as linux-build.sh, apart from the Mac's target/.
  CARGO_TARGET_DIR="$HOME/sweetpad-target" "$HOME/.cargo/bin/cargo" build --quiet \
    --manifest-path "$REPO/Cargo.toml" -p sweetpad-cli --bin sweetpad
  export SWEETPAD_EXE="$HOME/sweetpad-target/debug/sweetpad"
fi

BUNDLE="${SWEETPAD_DARWIN_SDK:-$HOME/.swiftpm/swift-sdks/darwin.artifactbundle}"
if [ -n "${SWEETPAD_SDK_ARCHIVE:-}" ]; then
  section "Darwin SDK from $SWEETPAD_SDK_ARCHIVE"
  import_args=(import --archive "$SWEETPAD_SDK_ARCHIVE" --destination "$BUNDLE")
  [ -z "${SWEETPAD_AEA_KEY_FILE:-}" ] || import_args+=(--aea-key-file "$SWEETPAD_AEA_KEY_FILE")
  [ -z "${SWEETPAD_AEA_PUBLIC_KEY_FILE:-}" ] || import_args+=(--aea-public-key-file "$SWEETPAD_AEA_PUBLIC_KEY_FILE")
else
  section "Darwin SDK from $XCODE_APP"
  [ -d "$XCODE_APP" ] || { echo "error: $XCODE_APP is not visible here" >&2; exit 1; }
  # The importer reuses the SDK while Xcode's build is the same.
  import_args=(import --xcode "$XCODE_APP" --destination "$BUNDLE")
  [ -z "${SWEETPAD_APPINTENTS_RESOURCE_EXPORT:-}" ] ||
    import_args+=(--appintents-phrases "$SWEETPAD_APPINTENTS_RESOURCE_EXPORT")
  [ -z "${SWEETPAD_BUILD_MACHINE_OS_BUILD:-}" ] ||
    import_args+=(--build-machine-os-build "$SWEETPAD_BUILD_MACHINE_OS_BUILD")
fi
[ "${SWEETPAD_REFRESH_SDK:-0}" != 1 ] || import_args+=(--replace)
python3 "$(dirname "$0")/linux-sdk.py" "${import_args[@]}"

echo
echo "ready: use sweetpad build with your project, scheme, and destination"
