#!/usr/bin/env bash
#
# End-to-end coverage for the standalone `sweetpad` CLI, exercising every command
# against a real Xcode app (ci/fixture-app) and a real Swift package
# (ci/fixture-spm) on a macOS runner with Xcode. Run by .github/workflows/cli-smoke.yaml.
#
# Requires: SWEETPAD_BIN pointing at the built binary; the app fixture already
# generated with `xcodegen generate`.
set -euo pipefail

BIN="${SWEETPAD_BIN:?set SWEETPAD_BIN to the sweetpad binary}"
ROOT="$(cd "$(dirname "$0")" && pwd)"
APP_DIR="$ROOT/fixture-app"
SPM_DIR="$ROOT/fixture-spm"
APP="$APP_DIR/SweetpadCIApp.xcodeproj"

CHECKS=0
section() { echo; echo "==== $* ===="; }
ok() {
  CHECKS=$((CHECKS + 1))
  echo "  ✓ $*"
}
fail() {
  echo "  ✗ $*" >&2
  exit 1
}
contains() { case "$1" in *"$2"*) ;; *) fail "expected output to contain '$2', got: $1" ;; esac }

# Expect a command to exit with a specific code.
expect_code() {
  local want="$1"
  shift
  local rc=0
  "$@" >/dev/null 2>&1 || rc=$?
  [ "$rc" -eq "$want" ] || fail "expected exit $want, got $rc: $*"
}

# Run a streaming command for N seconds, then stop it (SIGTERM is success).
run_briefly() {
  local secs="$1"
  shift
  "$@" &
  local pid=$!
  sleep "$secs"
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

# A JSON field assertion via python3: <json> <python-expr over `d`> <expected>.
assert_json() {
  python3 - "$@" <<'PY'
import json, sys
data = json.loads(sys.argv[1])
got = str(eval(sys.argv[2], {"d": data}))
want = sys.argv[3]
if got != want:
    sys.exit(f"json assertion failed: {sys.argv[2]} == {got!r}, expected {want!r}")
PY
}

# ---------------------------------------------------------------------------
section "tooling"
xcodebuild -version
"$BIN" --version >/dev/null 2>&1 || true   # tolerate until a --version lands
ok "xcodebuild present"

# ---------------------------------------------------------------------------
section "explorers (Xcode app)"
out=$("$BIN" scheme list --project "$APP")
contains "$out" "SweetpadCIApp"
ok "scheme list"

out=$("$BIN" scheme list --project "$APP" --json)
assert_json "$out" "any(s['name']=='SweetpadCIApp' for s in d['schemes'])" "True"
ok "scheme list --json"

out=$("$BIN" project info --project "$APP")
contains "$out" "SweetpadCIAppTests"
ok "project info"

out=$("$BIN" project info --project "$APP" --json)
assert_json "$out" "'SweetpadCIApp' in d['targets']" "True"
ok "project info --json"

out=$("$BIN" settings show --project "$APP" --scheme SweetpadCIApp --key PRODUCT_BUNDLE_IDENTIFIER)
contains "$out" "dev.sweetpad.ci.app"
ok "settings show --key"

out=$("$BIN" settings show --project "$APP" --target SweetpadCIApp --json)
assert_json "$out" "len(d['targets'])>=1" "True"
ok "settings show --target --json"

# ---------------------------------------------------------------------------
section "destinations & simulators"
out=$("$BIN" destination list --json)
assert_json "$out" "any(x['kind']=='macOS' for x in d['destinations'])" "True"
assert_json "$out" "any(x['kind']=='simulator' for x in d['destinations'])" "True"
ok "destination list --json (macOS + simulators)"

"$BIN" destination list >/dev/null
ok "destination list (human)"

"$BIN" simulator list >/dev/null
"$BIN" simulator list --json >/dev/null
ok "simulator list"

DEST=$(python3 -c "import json,subprocess;d=json.loads(subprocess.check_output(['$BIN','destination','list','--json']))['destinations'];print(next(x['destination'] for x in d if x['kind']=='simulator' and x['os']=='iOS'))")
UDID="${DEST##*id=}"
echo "  using $DEST"
xcrun simctl boot "$UDID" || true
xcrun simctl bootstatus "$UDID" -b || true
"$BIN" simulator boot "$UDID" >/dev/null 2>&1 || true   # already booted is fine
ok "simulator boot"

# ---------------------------------------------------------------------------
section "bsp & completions"
( cd "$APP_DIR" && "$BIN" bsp init --project SweetpadCIApp.xcodeproj && test -f buildServer.json )
ok "bsp init wrote buildServer.json"
for sh in zsh bash fish; do "$BIN" completions "$sh" >/dev/null; done
ok "completions zsh/bash/fish"

# ---------------------------------------------------------------------------
section "build (iOS + macOS)"
"$BIN" build start --project "$APP" --scheme SweetpadCIApp --destination "$DEST"
ok "build start (iOS simulator)"
"$BIN" build start --project "$APP" --scheme SweetpadCIApp --destination "$DEST" --clean
ok "build start --clean"
"$BIN" build start --project "$APP" --scheme SweetpadCIMac --destination "platform=macOS"
ok "build start (macOS)"

# ---------------------------------------------------------------------------
section "test (iOS)"
out=$("$BIN" test run --project "$APP" --scheme SweetpadCIApp --destination "$DEST" --json)
assert_json "$out" "d['passed']" "True"
assert_json "$out" "d['failedTests']" "0"
ok "test run --json (passed)"
"$BIN" test run --project "$APP" --scheme SweetpadCIApp --destination "$DEST" \
  --only-testing SweetpadCIAppTests/AppTests/testArithmetic >/dev/null
ok "test run --only-testing"

# ---------------------------------------------------------------------------
section "app lifecycle (simulator)"
"$BIN" app run --project "$APP" --scheme SweetpadCIApp --destination "$DEST" --no-logs
ok "app run --no-logs"
"$BIN" app install --project "$APP" --scheme SweetpadCIApp --destination "$DEST"
ok "app install"
"$BIN" app launch --project "$APP" --scheme SweetpadCIApp --destination "$DEST"
ok "app launch"
run_briefly 8 "$BIN" app logs --project "$APP" --scheme SweetpadCIApp --destination "$DEST"
ok "app logs (streamed briefly)"
run_briefly 12 "$BIN" app run --project "$APP" --scheme SweetpadCIApp --destination "$DEST"
ok "app run (logs follow, streamed briefly)"
"$BIN" app stop --project "$APP" --scheme SweetpadCIApp --destination "$DEST"
ok "app stop"

# ---------------------------------------------------------------------------
section "macOS app run (best-effort, GUI headless)"
# `open` returns even when the window can't draw on a headless runner.
"$BIN" app run --project "$APP" --scheme SweetpadCIMac --mac --no-logs || echo "  (macOS GUI launch skipped on headless runner)"
ok "app run --mac attempted"

# ---------------------------------------------------------------------------
section "devices"
"$BIN" device list >/dev/null         # no devices on CI — must still succeed
"$BIN" device list --json >/dev/null
ok "device list (no devices)"

# ---------------------------------------------------------------------------
section "format"
"$BIN" format run "$APP_DIR/Sources/App/ContentView.swift"
ok "format run (swift-format, in place)"

# ---------------------------------------------------------------------------
section "Swift package (SPM)"
out=$(cd "$SPM_DIR" && "$BIN" scheme list)
contains "$out" "SweetpadCITool"
ok "spm scheme list"
( cd "$SPM_DIR" && "$BIN" build start --scheme SweetpadCITool --destination "platform=macOS" )
ok "spm build start"
out=$(cd "$SPM_DIR" && "$BIN" test run --scheme SweetpadCITool --destination "platform=macOS" --json)
assert_json "$out" "d['passed']" "True"
ok "spm test run --json"
out=$(cd "$SPM_DIR" && "$BIN" app run --scheme SweetpadCITool)
contains "$out" "hello from sweetpad ci tool"
ok "spm app run (swift run)"

# ---------------------------------------------------------------------------
section "error paths"
expect_code 2 "$BIN" bogus-command
ok "unknown command exits 2"
expect_code 1 "$BIN" build start --project "$APP" --scheme NoSuchScheme --destination "$DEST"
ok "unknown scheme exits 1"

# ---------------------------------------------------------------------------
echo
echo "==== all $CHECKS checks passed ===="
