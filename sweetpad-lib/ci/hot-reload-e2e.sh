#!/usr/bin/env bash
#
# End-to-end hot-reload / injection test for the real `sweetpad app run --hot`
# (CLI_DESIGN §9d), on a macOS runner with Xcode + a simulator. Drives the
# shipping code via the hidden `--hot-selfcheck` hook: it builds with the
# interposable / frontend-command flags, starts the :8887 injection server,
# launches the fixture app with the InjectionNext client dylib injected, edits a
# Swift file once, and asserts a `.injected` response — for *both* recompilers
# (resolver default + build-log). Run by .github/workflows/xcode-tests.yaml.
#
# Requires: SWEETPAD_BIN (the built binary); the fixture generated with xcodegen.
# Uses SWEETPAD_HOTRELOAD_DYLIB if set, else downloads the InjectionNext client.
set -euo pipefail

BIN="${SWEETPAD_BIN:?set SWEETPAD_BIN to the sweetpad binary}"
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP_DIR="$ROOT/fixture-app"
APP="$APP_DIR/SweetpadCIApp.xcodeproj"
SRC="$APP_DIR/Sources/App/ContentView.swift"

section() { echo; echo "==== $* ===="; }
fail() { echo "  ✗ $*" >&2; exit 1; }

section "tooling"
xcodebuild -version
test -f "$SRC" || fail "fixture source missing: $SRC (run xcodegen generate first)"

section "injection client dylib"
# Build from source per-Xcode is the long-term plan (CLI_DESIGN §9d Milestone 5);
# until that's vendored, download the prebuilt client matching the active Xcode.
if [[ -z "${SWEETPAD_HOTRELOAD_DYLIB:-}" ]]; then
  INJ="$(mktemp -d)"
  gh release download --repo johnno1962/InjectionNext --pattern '*.zip' --dir "$INJ" --clobber
  ( cd "$INJ" && for z in *.zip; do unzip -q -o "$z" -d extracted; done )
  DYLIB="$(find "$INJ/extracted" -name 'libiphonesimulatorInjection.dylib' | head -1 || true)"
  [[ -n "$DYLIB" ]] || fail "could not find libiphonesimulatorInjection.dylib in the release"
  export SWEETPAD_HOTRELOAD_DYLIB="$DYLIB"
fi
echo "  client: $SWEETPAD_HOTRELOAD_DYLIB"

section "pick + boot a simulator"
DEST=$(python3 -c "import json,subprocess;d=json.loads(subprocess.check_output(['$BIN','destination','list','--json']))['destinations'];print(next(x['destination'] for x in d if x['kind']=='simulator' and x['os']=='iOS'))")
echo "  $DEST"
UDID="${DEST##*id=}"
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b || true

# `app run --hot --hot-selfcheck` exits 0 only on a confirmed `.injected`.
run_selfcheck() {
  local mode="$1"
  section "hot reload self-check — $mode recompiler"
  "$BIN" app run --project "$APP" --scheme SweetpadCIApp --destination "$DEST" \
    --hot --hot-recompiler "$mode" --hot-selfcheck "$SRC" \
    || fail "$mode recompiler: injection self-check failed"
  echo "  ✓ $mode recompiler injected"
}

run_selfcheck resolver
run_selfcheck buildlog

section "teardown"
xcrun simctl shutdown "$UDID" 2>/dev/null || true
echo
echo "==== hot reload e2e passed (resolver + buildlog) ===="
