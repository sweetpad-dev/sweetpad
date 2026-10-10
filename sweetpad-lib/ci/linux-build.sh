#!/usr/bin/env bash
#
# Build an unmodified Xcode project on Linux with `sweetpad build`, which runs
# on Apple's open-source Swift Build there: sweetpad translates the project
# into Swift Build's PIF and builds it against a Darwin Swift SDK. No Xcode and
# no xcodebuild are involved. Exits non-zero unless the build produces an app
# whose executable is a Mach-O for the requested arch.
#
#   ci/linux-build.sh [xcodeproj] [scheme] [configuration]
#
# Defaults to ci/fixture-linux/LinuxApp.xcodeproj, scheme LinuxApp, Debug.
# Run it on the Mac and it reruns itself inside the OrbStack machine, where
# /Users is mounted at the same path; run it on Linux and it builds in place.
# The machine comes from ci/linux-setup.sh.
#
# Knobs (env): SWEETPAD_VM, DESTINATION (generic/platform=iOS), ARCH (the
# arch the product must be built for, arm64), EXTRA_SETTINGS ("NAME=value …",
# passed after '--').
set -euo pipefail

if [ "$(uname -s)" = Darwin ]; then
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  args=()
  for arg in "$@"; do
    if [ -e "$arg" ]; then arg="$(cd "$(dirname "$arg")" && pwd)/$(basename "$arg")"; fi
    args+=("$arg")
  done
  vars=()
  for v in DESTINATION ARCH EXTRA_SETTINGS; do
    [ -n "${!v:-}" ] && vars+=("$v=${!v}")
  done
  exec orb -m "${SWEETPAD_VM:-sweetpad-linux}" env ${vars[@]+"${vars[@]}"} bash "$self" ${args[@]+"${args[@]}"}
fi

CI_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$CI_DIR/../.." && pwd)"
PROJECT="$(realpath "${1:-$CI_DIR/fixture-linux/LinuxApp.xcodeproj}")"
SCHEME="${2:-LinuxApp}"
CONFIGURATION="${3:-Debug}"
DESTINATION="${DESTINATION:-generic/platform=iOS}"
ARCH="${ARCH:-arm64}"
SWIFT_DIR="$(ls -d "$HOME"/swift/swift-*-RELEASE 2>/dev/null | sort | tail -1)"

for p in "$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/toolset-swb.json" \
  "$SWIFT_DIR/usr/bin/swift" "$HOME/bin/swbuild" "$HOME/.cargo/bin/cargo"; do
  [ -e "$p" ] || { echo "error: missing $p; run ci/linux-setup.sh first" >&2; exit 1; }
done
# Swift Build derives the toolchain from the first `swift` on PATH.
export PATH="$HOME/bin:$SWIFT_DIR/usr/bin:$HOME/.cargo/bin:$PATH"

section() { echo; echo "==== $* ===="; }

section "sweetpad (cargo build)"
# A target dir of its own keeps Linux artifacts out of the Mac's target/.
export CARGO_TARGET_DIR="$HOME/sweetpad-target"
cargo build --quiet --manifest-path "$REPO/Cargo.toml" -p sweetpad-cli --bin sweetpad
SWEETPAD="$CARGO_TARGET_DIR/debug/sweetpad"

section "sweetpad build $SCHEME ($CONFIGURATION, $DESTINATION)"
build() {
  # shellcheck disable=SC2086 # EXTRA_SETTINGS is a word list on purpose
  "$SWEETPAD" build --project "$PROJECT" --scheme "$SCHEME" --configuration "$CONFIGURATION" \
    --destination "$DESTINATION" "$@" -- ${EXTRA_SETTINGS:-}
}
build --non-interactive
# The second run is an up-to-date build, there to read the product's path.
APP="$(build --json | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["data"]["productPath"] or "")')"
[ -n "$APP" ] || { echo "error: the build reported no product" >&2; exit 1; }

EXECUTABLE="$APP/$(basename "$APP" .app)"
description="$(file -b "$EXECUTABLE")"
echo
echo "product: $APP"
echo "  $(basename "$EXECUTABLE"): $description"
case "$description" in
  *Mach-O*"$ARCH"*executable*) ;;
  *) echo "error: $EXECUTABLE is not a Mach-O $ARCH executable" >&2; exit 1 ;;
esac
