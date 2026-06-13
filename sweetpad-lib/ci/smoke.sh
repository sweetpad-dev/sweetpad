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

# doctor: on a runner with Xcode this must report zero *problems*. Warnings
# (e.g. a missing SwiftLint, which is optional) are tolerated and exit 0.
"$BIN" doctor
out=$("$BIN" doctor --json)
assert_json "$out" "d['summary']['problems']" "0"
ok "doctor (no problems)"

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

# Operations on the booted sim (screenshot/appearance leave it booted, so the
# app-lifecycle section below can reuse it; shutdown/erase run at teardown).
SHOT="$(mktemp -u)-sweetpad.png"
"$BIN" simulator screenshot "$UDID" --output "$SHOT"
test -f "$SHOT" || fail "screenshot file not written: $SHOT"
ok "simulator screenshot"
out=$("$BIN" simulator screenshot "$UDID" --output "$SHOT" --json)
assert_json "$out" "d['udid']" "$UDID"
ok "simulator screenshot --json"
"$BIN" simulator appearance dark "$UDID"
"$BIN" simulator appearance light "$UDID"
ok "simulator appearance dark/light"
"$BIN" simulator open || echo "  (Simulator.app GUI skipped on headless runner)"
ok "simulator open attempted"

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
section "project new (scaffold a fresh app and build it)"
# Generate a project from scratch, then prove the hand-assembled pbxproj is one
# real xcodebuild actually accepts and compiles — the runtime counterpart to the
# parser round-trip unit tests.
GEN_DIR="$(mktemp -d)"
( cd "$GEN_DIR" && "$BIN" project new SmokeGen --bundle-id dev.sweetpad.ci.smokegen --no-git )
GEN_PROJ="$GEN_DIR/SmokeGen/SmokeGen.xcodeproj"
test -d "$GEN_PROJ" || fail "generated .xcodeproj missing: $GEN_PROJ"
ok "project new wrote $GEN_PROJ"
out=$("$BIN" project info --project "$GEN_PROJ" --json)
assert_json "$out" "d['targets']" "['SmokeGen']"
assert_json "$out" "d['schemes']" "['SmokeGen']"
ok "generated project resolves (target + shared scheme)"
"$BIN" build start --project "$GEN_PROJ" --scheme SmokeGen --destination "$DEST"
ok "build start on generated project (iOS simulator)"
# --current-dir variant: scaffold in place, name defaults to the directory.
GEN_HERE="$GEN_DIR/HereApp"
mkdir -p "$GEN_HERE"
out=$(cd "$GEN_HERE" && "$BIN" project new --current-dir --no-git --json)
assert_json "$out" "d['name']" "HereApp"
test -d "$GEN_HERE/HereApp.xcodeproj" || fail "--current-dir did not scaffold in place"
ok "project new --current-dir (in-place, name from directory)"

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
"$BIN" app open-url "https://example.com" --simulator "$UDID"
ok "app open-url"
out=$("$BIN" app open-url "https://example.com" --simulator "$UDID" --json)
assert_json "$out" "d['url']" "https://example.com"
ok "app open-url --json"
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
section "derived-data"
# Whole-store inspection (the runner has a DerivedData root from the builds above).
out=$("$BIN" derived-data path --all --json)
assert_json "$out" "'DerivedData' in d['root']" "True"
ok "derived-data path --all --json"
"$BIN" derived-data size --all >/dev/null
ok "derived-data size --all"
# Project-scoped: SweetpadCIApp built above, so it has a <Name>-<hash> folder.
out=$("$BIN" derived-data path --project "$APP" --json)
assert_json "$out" "len(d['paths'])>=1" "True"
ok "derived-data path --project (folder present)"
out=$("$BIN" derived-data size --project "$APP" --json)
assert_json "$out" "d['folders']>=1" "True"
ok "derived-data size --project"
# Purge just this project's folder(s), then confirm they're gone.
"$BIN" derived-data purge --project "$APP" --yes
out=$("$BIN" derived-data path --project "$APP" --json)
assert_json "$out" "len(d['paths'])" "0"
ok "derived-data purge --project (roundtrip)"

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
section "simulator teardown"
"$BIN" simulator shutdown "$UDID"
ok "simulator shutdown"
"$BIN" simulator shutdown "$UDID"   # idempotent: already shut down is success
ok "simulator shutdown (idempotent)"
"$BIN" simulator erase "$UDID"      # requires the device to be shut down first
ok "simulator erase"

# ---------------------------------------------------------------------------
section "error paths"
expect_code 2 "$BIN" bogus-command
ok "unknown command exits 2"
expect_code 1 "$BIN" build start --project "$APP" --scheme NoSuchScheme --destination "$DEST"
ok "unknown scheme exits 1"
# Project-scoped derived-data with no container in cwd (sweetpad-lib has no
# Xcode project/package) is a strict error, not a silent empty result.
expect_code 1 "$BIN" derived-data path
ok "derived-data --project with no container exits 1"
expect_code 1 "$BIN" simulator screenshot definitely-not-a-real-sim
ok "simulator op with unknown target exits 1"

# ---------------------------------------------------------------------------
echo
echo "==== all $CHECKS checks passed ===="
