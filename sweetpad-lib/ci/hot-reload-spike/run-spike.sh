#!/usr/bin/env bash
# Milestone-1 hot-reload spike harness (CLI_DESIGN §9d). Runs on a macOS runner.
#
# Generates a minimal iOS app, builds it for the simulator (capturing the
# swift-frontend command lines), boots a sim, launches the app with the
# InjectionNext client dylib injected via DYLD_INSERT_LIBRARIES, and runs the
# Rust spike server which speaks the :8887 protocol, recompiles ContentView.swift
# into a dylib, sends `.load`, and exits 0 iff the client replies `.injected`.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE="$HERE/fixture"
SERVER="$HERE/server"
WORK="${SPIKE_WORK:-$HERE/.work}"
rm -rf "$WORK" && mkdir -p "$WORK"

BUNDLE_ID="dev.sweetpad.spike.app"
APP_SOURCE="$FIXTURE/Sources/ContentView.swift"

section() { echo; echo "==== $* ===="; }

section "Select newest Xcode (the prebuilt client dylib must match Xcode's XCTest ABI)"
echo "installed Xcodes:"; ls -d /Applications/Xcode*.app 2>/dev/null || true
NEWEST_XCODE="$(ls -d /Applications/Xcode*.app 2>/dev/null | sort -V | tail -1)"
if [[ -n "$NEWEST_XCODE" && -d "$NEWEST_XCODE/Contents/Developer" ]]; then
  echo "selecting $NEWEST_XCODE"
  sudo xcode-select -s "$NEWEST_XCODE/Contents/Developer"
fi
DEVELOPER_DIR="$(xcode-select -p)"
echo "DEVELOPER_DIR=$DEVELOPER_DIR"

section "Tooling"
xcodebuild -version
which xcodegen
which gh || true

section "Build the spike server"
( cd "$SERVER" && cargo build )
SERVER_BIN="$SERVER/target/debug/spike-server"
test -x "$SERVER_BIN"

section "Generate the fixture project"
( cd "$FIXTURE" && xcodegen generate )

section "Download the InjectionNext client dylib (vendoring-B artifact)"
INJ="$WORK/inj"
mkdir -p "$INJ"
gh release download --repo johnno1962/InjectionNext --pattern '*.zip' --dir "$INJ" --clobber
echo "downloaded:"; ls -la "$INJ"
( cd "$INJ" && for z in *.zip; do unzip -q -o "$z" -d extracted; done )
DYLIB="$(find "$INJ/extracted" -name 'libiphonesimulatorInjection.dylib' | head -1 || true)"
if [[ -z "$DYLIB" ]]; then
  echo "❌ could not find libiphonesimulatorInjection.dylib in the release"; find "$INJ/extracted" -name '*.dylib' | head; exit 1
fi
echo "client dylib: $DYLIB"

section "Pick + boot a simulator"
UDID="$(xcrun simctl list devices available -j | python3 -c '
import json,sys
d=json.load(sys.stdin)["devices"]
ids=[x["udid"] for r in d.values() for x in r if "iPhone" in x["name"]]
print(ids[0] if ids else "")')"
test -n "$UDID"
echo "simulator udid: $UDID"
xcrun simctl boot "$UDID" 2>/dev/null || true
xcrun simctl bootstatus "$UDID" -b || true

section "Build the app for the simulator (capturing frontend commands)"
DD="$WORK/dd"
BUILD_LOG="$WORK/build.log"
set +e
xcodebuild \
  -project "$FIXTURE/HotReloadSpike.xcodeproj" \
  -scheme HotReloadSpike \
  -configuration Debug \
  -sdk iphonesimulator \
  -destination "platform=iOS Simulator,id=$UDID" \
  -derivedDataPath "$DD" \
  build | tee "$BUILD_LOG"
BUILD_RC=${PIPESTATUS[0]}
set -e
test "$BUILD_RC" -eq 0
APP="$DD/Build/Products/Debug-iphonesimulator/HotReloadSpike.app"
test -d "$APP"
echo "frontend command lines captured: $(grep -c -- '-primary-file' "$BUILD_LOG" || true)"

section "Install the app"
xcrun simctl install "$UDID" "$APP"

# XCTest search paths the injection dylib's deps need (mirrors hot-reload.ts).
PLATDEV="$DEVELOPER_DIR/Platforms/iPhoneSimulator.platform/Developer"
FRAMEWORK_PATH="$PLATDEV/Library/Frameworks:$PLATDEV/Library/PrivateFrameworks"
LIBRARY_PATH="$PLATDEV/usr/lib"

section "Simulate an edit (bump the marker)"
sed -i '' 's/static let marker = 1/static let marker = 2/' "$APP_SOURCE"
grep marker "$APP_SOURCE"

section "Start the spike server"
LOG_STREAM="$WORK/app-log.txt"
xcrun simctl spawn "$UDID" log stream --level debug --style compact \
  --predicate 'processImagePath CONTAINS "HotReloadSpike"' >"$LOG_STREAM" 2>&1 &
LOGGER_PID=$!

SPIKE_BUILD_LOG="$BUILD_LOG" \
SPIKE_SOURCE="$APP_SOURCE" \
SPIKE_DEVELOPER_DIR="$DEVELOPER_DIR" \
SPIKE_OUT_DIR="$WORK/out" \
  "$SERVER_BIN" &
SERVER_PID=$!
sleep 1  # let it bind :8887 before the app's +load dials in

section "Launch the app (client dylib injected)"
# --console captures the app's own stdout/stderr (InjectionNext's printf and
# dyld diagnostics) which `log stream` does not. DYLD_PRINT_* reveals whether the
# inserted client dylib actually loaded. Backgrounded so the server can proceed.
APP_CONSOLE="$WORK/app-console.txt"
SIMCTL_CHILD_DYLD_INSERT_LIBRARIES="$DYLIB" \
SIMCTL_CHILD_DYLD_FRAMEWORK_PATH="$FRAMEWORK_PATH" \
SIMCTL_CHILD_DYLD_LIBRARY_PATH="$LIBRARY_PATH" \
SIMCTL_CHILD_DYLD_PRINT_LIBRARIES="1" \
SIMCTL_CHILD_DYLD_PRINT_WARNINGS="1" \
SIMCTL_CHILD_INJECTION_HOST="127.0.0.1" \
SIMCTL_CHILD_INJECTION_PROJECT_ROOT="$FIXTURE" \
SIMCTL_CHILD_INJECTION_NOSTANDALONE="1" \
  xcrun simctl launch --terminate-running-process --console "$UDID" "$BUNDLE_ID" \
  >"$APP_CONSOLE" 2>&1 &
APP_PID=$!

section "Await result"
set +e
wait "$SERVER_PID"; RESULT=$?
set -e
kill "$APP_PID" 2>/dev/null || true
kill "$LOGGER_PID" 2>/dev/null || true

echo; echo "---- app console tail (dyld + InjectionNext) ----"
tail -n 60 "$APP_CONSOLE" 2>/dev/null || true
echo; echo "---- did the client dylib load? ----"
grep -i 'libiphonesimulatorInjection\|Injection\|could not\|not loaded\|Library not' "$APP_CONSOLE" 2>/dev/null | head -20 || true
echo; echo "---- app os_log tail ----"; tail -n 20 "$LOG_STREAM" 2>/dev/null || true

if [[ "$RESULT" -eq 0 ]]; then
  echo "✅ SPIKE PASSED — socket protocol + recompile/load validated"
else
  echo "❌ SPIKE FAILED (server exit $RESULT)"
fi
exit "$RESULT"
