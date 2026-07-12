#!/usr/bin/env bash
#
# End-to-end hot-reload / injection test for the real `sweetpad app run --hot`
# (CLI_DESIGN §9d), on a macOS runner with Xcode + a simulator. Drives the
# shipping code via the hidden `--hot-selfcheck` hook: it builds with the
# interposable / frontend-command flags, starts the :8887 injection server,
# launches the fixture app with the InjectionNext client dylib injected, edits a
# Swift file once, and asserts a `.injected` response — for *both* recompilers
# (resolver default + build-log), on *both* targets (iOS Simulator + native
# macOS). Run by .github/workflows/xcode-tests.yaml.
#
# Requires: SWEETPAD_BIN (the built binary); the fixture generated with xcodegen.
# Client resolution is the CLI's job: SWEETPAD_HOTRELOAD_DYLIB (override) if set,
# else the per-SDK client bundled into the binary (vendor/injection-client), else
# falls back. The override only fits one SDK, so the macOS runs always resolve
# from the binary/fallback (SWEETPAD_HOTRELOAD_DYLIB is unset for them).
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
echo "  client: ${SWEETPAD_HOTRELOAD_DYLIB:-<bundled into the CLI>}"

section "pick + boot a simulator"
DEST=$(python3 -c "import json,subprocess;d=json.loads(subprocess.check_output(['$BIN','destination','list','--json']))['data']['destinations'];print(next(x['destination'] for x in d if x['kind']=='simulator' and x['os']=='iOS'))")
echo "  $DEST"
UDID="${DEST##*id=}"
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b || true

# `app run --hot --hot-selfcheck` exits 0 only on a confirmed `.injected` whose
# new code demonstrably ran (nonce observed in the unified log).
run_selfcheck() {
  local mode="$1"
  section "hot reload self-check — iOS Simulator, $mode recompiler"
  "$BIN" app run --project "$APP" --scheme SweetpadCIApp --destination "$DEST" \
    --hot --hot-recompiler "$mode" --hot-selfcheck "$SRC" \
    || fail "simulator/$mode: injection self-check failed"
  echo "  ✓ simulator/$mode injected"
}

# Same round-trip against the fixture's macOS target: direct spawn with the raw
# insert env, the mac client dylib, host `log show` for the marker.
run_selfcheck_mac() {
  local mode="$1"
  section "hot reload self-check — macOS, $mode recompiler"
  env -u SWEETPAD_HOTRELOAD_DYLIB \
    "$BIN" app run --project "$APP" --scheme SweetpadCIMac --mac \
    --hot --hot-recompiler "$mode" --hot-selfcheck "$SRC" \
    || fail "macos/$mode: injection self-check failed"
  echo "  ✓ macos/$mode injected"
}

run_selfcheck resolver
run_selfcheck buildlog
run_selfcheck_mac resolver
run_selfcheck_mac buildlog

section "teardown"
xcrun simctl shutdown "$UDID" 2>/dev/null || true
echo
echo "==== hot reload e2e passed (simulator + macOS, resolver + buildlog) ===="
