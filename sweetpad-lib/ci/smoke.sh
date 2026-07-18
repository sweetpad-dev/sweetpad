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
# Every `--json` result is the standardized `{schema, ok, data}` success
# envelope (see output.rs::json_value), so `d` is bound to the `data` payload.
assert_json() {
  python3 - "$@" <<'PY'
import json, sys
data = json.loads(sys.argv[1])["data"]
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

DEST=$(python3 -c "import json,subprocess;d=json.loads(subprocess.check_output(['$BIN','destination','list','--json']))['data']['destinations'];print(next(x['destination'] for x in d if x['kind']=='simulator' and x['os']=='iOS'))")
UDID="${DEST##*id=}"
echo "  using $DEST"
xcrun simctl boot "$UDID" || true
xcrun simctl bootstatus "$UDID" -b || true
"$BIN" simulator boot "$UDID" >/dev/null 2>&1 || true   # already booted is fine
ok "simulator boot"

# Operations on the booted sim (screenshot/appearance leave it booted, so the
# app-lifecycle section below can reuse it; shutdown/erase run at teardown).
SHOT="$(mktemp -u)-sweetpad.png"
"$BIN" simulator screenshot "$UDID" --output-file "$SHOT"
test -f "$SHOT" || fail "screenshot file not written: $SHOT"
ok "simulator screenshot"
out=$("$BIN" simulator screenshot "$UDID" --output-file "$SHOT" --json)
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
# Same, for a generated macOS app.
( cd "$GEN_DIR" && "$BIN" project new MacGen --platform macos --bundle-id dev.sweetpad.ci.macgen --no-git )
MAC_PROJ="$GEN_DIR/MacGen/MacGen.xcodeproj"
out=$("$BIN" project info --project "$MAC_PROJ" --json)
assert_json "$out" "d['targets']" "['MacGen']"
"$BIN" build start --project "$MAC_PROJ" --scheme MacGen --destination "platform=macOS"
ok "project new --platform macos + build start (macOS)"
# --current-dir variant: scaffold in place, name defaults to the directory.
GEN_HERE="$GEN_DIR/HereApp"
mkdir -p "$GEN_HERE"
out=$(cd "$GEN_HERE" && "$BIN" project new --current-dir --no-git --json)
assert_json "$out" "d['name']" "HereApp"
test -d "$GEN_HERE/HereApp.xcodeproj" || fail "--current-dir did not scaffold in place"
ok "project new --current-dir (in-place, name from directory)"

# ---------------------------------------------------------------------------
section "pbxproj plumbing (stored settings, folders, membership; §9f/§9g)"
# Runtime truth for the mutation surface, on the generated macOS app: the
# synchronized folder really is the membership (a file appears by existing),
# the stored-settings edits land, the Info.plist auto-exception keeps a
# custom plist out of Copy Bundle Resources, and an excluded file really
# leaves the build — all proven by real xcodebuild builds.

# Implicit membership: a new file on disk builds with zero pbxproj edits.
cat > "$GEN_DIR/MacGen/MacGen/Extra.swift" <<'SWIFT'
func extraGreeting() -> String { "from a file the pbxproj never saw" }
SWIFT
"$BIN" build start --project "$MAC_PROJ" --scheme MacGen --destination "platform=macOS"
ok "synchronized folder picks up a new file (no pbxproj change)"

# Stored settings: set at target scope, read back the stored layer, rebuild.
"$BIN" pbxproj settings set ENABLE_HARDENED_RUNTIME=YES --target MacGen --project "$MAC_PROJ"
out=$("$BIN" pbxproj settings show --target MacGen --key ENABLE_HARDENED_RUNTIME --project "$MAC_PROJ")
contains "$out" "YES"
ok "pbxproj settings set + show (stored layer)"

# Custom Info.plist inside the folder: the auto-exception must keep it out of
# resources, or the build breaks (verified failure mode, CLI_DESIGN §9f).
mkdir -p "$GEN_DIR/MacGen/MacGen/Resources"
cat > "$GEN_DIR/MacGen/MacGen/Resources/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleExecutable</key><string>$(EXECUTABLE_NAME)</string>
	<key>CFBundleIdentifier</key><string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>$(PRODUCT_NAME)</string>
	<key>CFBundlePackageType</key><string>$(PRODUCT_BUNDLE_PACKAGE_TYPE)</string>
	<key>CFBundleShortVersionString</key><string>$(MARKETING_VERSION)</string>
	<key>CFBundleVersion</key><string>$(CURRENT_PROJECT_VERSION)</string>
</dict>
</plist>
PLIST
"$BIN" pbxproj settings set GENERATE_INFOPLIST_FILE=NO INFOPLIST_FILE=MacGen/Resources/Info.plist \
  --target MacGen --project "$MAC_PROJ"
out=$("$BIN" pbxproj folder list --project "$MAC_PROJ" --json)
assert_json "$out" "d['targets'][0]['folders'][0]['exceptions']" "['Resources/Info.plist']"
"$BIN" build start --project "$MAC_PROJ" --scheme MacGen --destination "platform=macOS"
ok "INFOPLIST_FILE auto-exception (custom plist builds clean)"

# Membership exclusion: a broken file in the folder builds only while excluded.
echo "this is not swift {{{" > "$GEN_DIR/MacGen/MacGen/Broken.swift"
"$BIN" pbxproj membership exclude MacGen/Broken.swift --project "$MAC_PROJ"
"$BIN" build start --project "$MAC_PROJ" --scheme MacGen --destination "platform=macOS"
ok "pbxproj membership exclude (broken file stays out of the build)"
rm "$GEN_DIR/MacGen/MacGen/Broken.swift"
"$BIN" pbxproj membership include MacGen/Broken.swift --project "$MAC_PROJ" >/dev/null
ok "pbxproj membership include (exception dropped)"

# Classic membership is visible read-only on the committed fixture app.
out=$("$BIN" pbxproj membership list --project "$APP" --target SweetpadCIApp --json)
assert_json "$out" "len(d['targets'][0]['explicit'])>=1" "True"
ok "pbxproj membership list (classic entries on the fixture)"

# Generated-project guard: a spec file next to the .xcodeproj makes every
# pbxproj mutation refuse without --force (regeneration would silently
# clobber the edit); reads stay unguarded.
touch "$GEN_DIR/MacGen/project.yml"
expect_code 1 "$BIN" pbxproj settings set SWEETPAD_GUARD_PROBE=1 --project "$MAC_PROJ"
"$BIN" pbxproj settings show --project "$MAC_PROJ" >/dev/null
"$BIN" pbxproj settings set SWEETPAD_GUARD_PROBE=1 --project "$MAC_PROJ" --force >/dev/null
"$BIN" pbxproj settings unset SWEETPAD_GUARD_PROBE --project "$MAC_PROJ" --force >/dev/null
rm "$GEN_DIR/MacGen/project.yml"
# The sweetpad.toml `generator` declaration guards the same way, spec file or not.
printf 'generator = "xcodegen"\n' > "$GEN_DIR/MacGen/sweetpad.toml"
expect_code 1 "$BIN" pbxproj folder add Extras --project "$MAC_PROJ"
rm "$GEN_DIR/MacGen/sweetpad.toml"
ok "generated-project guard (spec file + config key; --force overrides)"

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
section "app screenshot (§9h)"
# Simulator delegation: the app verb reaches the same simctl capture
# `simulator screenshot` uses (the booted sim from the lifecycle section).
"$BIN" app screenshot --project "$APP" --scheme SweetpadCIApp --destination "$DEST" \
  --output-file "$GEN_DIR/app-shot.png"
test -s "$GEN_DIR/app-shot.png"
ok "app screenshot (simulator delegation)"
# The macOS resolution ladder's hard errors are TCC-independent — the
# capture itself needs the runner's Screen Recording permission, so only the
# error paths run here (the capture is e2e-verified on a real session).
expect_code 1 "$BIN" app screenshot --pid 999999
ok "app screenshot --pid on a dead pid exits 1"
expect_code 1 "$BIN" app screenshot --project "$MAC_PROJ" --scheme MacGen
ok "app screenshot (macOS, app not running) exits 1"
expect_code 1 "$BIN" app stop --project "$MAC_PROJ" --scheme MacGen
ok "app stop (macOS, app not running) exits 1"

# ---------------------------------------------------------------------------
section "macOS app run (best-effort, GUI headless)"
# `open` returns even when the window can't draw on a headless runner.
"$BIN" app run --project "$APP" --scheme SweetpadCIMac --mac --no-logs || echo "  (macOS GUI launch skipped on headless runner)"
ok "app run --mac attempted"

# launch/stop are symmetric on macOS: the app runs in place, so `launch` starts
# it detached (own session, file-backed stdio) and `stop` terminates it.
"$BIN" app stop --project "$APP" --scheme SweetpadCIMac --mac >/dev/null 2>&1 || true
if "$BIN" app launch --project "$APP" --scheme SweetpadCIMac --mac >/dev/null 2>&1; then
  sleep 1
  # The CLI has exited; the app must still be running for `stop` to find.
  "$BIN" app stop --project "$APP" --scheme SweetpadCIMac --mac >/dev/null
  ok "app launch --mac then stop (detached, outlives the CLI)"
else
  echo "  (macOS detached launch skipped on headless runner)"
fi

# `--detach` builds, launches, and returns; the app survives the CLI exiting.
if "$BIN" app run --project "$APP" --scheme SweetpadCIMac --mac --detach >/dev/null 2>&1; then
  sleep 1
  "$BIN" app stop --project "$APP" --scheme SweetpadCIMac --mac >/dev/null
  ok "app run --mac --detach (returns, app keeps running)"
else
  echo "  (macOS --detach skipped on headless runner)"
fi

# install/uninstall stay simulator/device-only — a macOS app is built in place.
expect_code 1 "$BIN" app install --project "$APP" --scheme SweetpadCIMac --mac
ok "app install --mac still refused (built in place)"

# --detach and --hot are contradictory: hot reload must stay attached.
expect_code 1 "$BIN" app run --project "$APP" --scheme SweetpadCIMac --mac --detach --hot
ok "--detach with --hot rejected"

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
# Unknown scheme / simulator / missing container are all TargetResolution
# errors, which the CLI's exit-code taxonomy maps to 4 (Generic is 1,
# BuildFailure 3, TargetResolution 4, ToolMissing 5 — see ErrorKind::exit_code).
expect_code 4 "$BIN" build start --project "$APP" --scheme NoSuchScheme --destination "$DEST"
ok "unknown scheme exits 4 (target resolution)"
# Project-scoped derived-data with no container in cwd (sweetpad-lib has no
# Xcode project/package) is a strict error, not a silent empty result.
expect_code 4 "$BIN" derived-data path
ok "derived-data --project with no container exits 4 (target resolution)"
expect_code 4 "$BIN" simulator screenshot definitely-not-a-real-sim
ok "simulator op with unknown target exits 4 (target resolution)"

# ---------------------------------------------------------------------------
echo
echo "==== all $CHECKS checks passed ===="
