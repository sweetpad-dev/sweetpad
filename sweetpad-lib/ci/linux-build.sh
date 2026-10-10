#!/usr/bin/env bash
#
# Build an unmodified Xcode project on Linux: sweetpad translates the
# .xcodeproj into Swift Build's PIF (`sweetpad pbxproj pif`), and Apple's
# open-source Swift Build (`swbuild`) builds that against xtool's Darwin SDK.
# No Xcode and no xcodebuild are involved. Exits non-zero unless the build
# produces an app whose executable is a Mach-O for the requested arch.
#
#   ci/linux-build.sh [xcodeproj] [target] [configuration]
#
# Defaults to ci/fixture-linux/LinuxApp.xcodeproj, target LinuxApp, Debug.
# Run it on the Mac and it reruns itself inside the OrbStack machine, where
# /Users is mounted at the same path; run it on Linux and it builds in place.
# The machine comes from ci/linux-setup.sh.
#
# The overrides passed to Swift Build are the ones a Linux host needs, not
# project changes:
#   CODE_SIGNING_ALLOWED=NO      swbuild can't answer Swift Build's provisioning
#                                request; signing happens after the build
#   SWIFT_SDK_TOOLSETS           xtool's toolset, whose ld64.lld links for iOS
#                                (the Swift toolchain's own refuses to)
#   LM_SKIP_METADATA_EXTRACTION  App Intents metadata needs an Xcode-only tool
#   DONT_RUN_SWIFT_STDLIB_TOOL   its Mach-O reader doesn't run on Linux; only an
#                                app that needs back-deployed Swift libraries
#                                misses it
#
# Knobs (env): SWEETPAD_VM, SDK (iphoneos), ARCH (arm64),
# EXTRA_SETTINGS ("NAME=value …", more overrides).
set -euo pipefail

if [ "$(uname -s)" = Darwin ]; then
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  args=()
  for arg in "$@"; do
    if [ -e "$arg" ]; then arg="$(cd "$(dirname "$arg")" && pwd)/$(basename "$arg")"; fi
    args+=("$arg")
  done
  vars=()
  for v in SDK ARCH EXTRA_SETTINGS; do
    [ -n "${!v:-}" ] && vars+=("$v=${!v}")
  done
  exec orb -m "${SWEETPAD_VM:-sweetpad-linux}" env ${vars[@]+"${vars[@]}"} bash "$self" ${args[@]+"${args[@]}"}
fi

CI_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$CI_DIR/../.." && pwd)"
PROJECT="$(realpath "${1:-$CI_DIR/fixture-linux/LinuxApp.xcodeproj}")"
TARGET="${2:-LinuxApp}"
CONFIGURATION="${3:-Debug}"
SDK="${SDK:-iphoneos}"
ARCH="${ARCH:-arm64}"
SDK_BUNDLE="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle"
SWIFT_DIR="$(ls -d "$HOME"/swift/swift-*-RELEASE 2>/dev/null | sort | tail -1)"
SWBUILD="$HOME/src/swift-build/.build/release/swbuild"
DERIVED="$HOME/sweetpad-linux-build/$(basename "$PROJECT" .xcodeproj)"

for p in "$SDK_BUNDLE/toolset-swb.json" "$SWIFT_DIR/usr/bin/swift" "$SWBUILD" "$HOME/.cargo/bin/cargo"; do
  [ -e "$p" ] || { echo "error: missing $p; run ci/linux-setup.sh first" >&2; exit 1; }
done

section() { echo; echo "==== $* ===="; }

section "sweetpad (cargo build)"
# A target dir of its own keeps Linux artifacts out of the Mac's target/.
export CARGO_TARGET_DIR="$HOME/sweetpad-target"
"$HOME/.cargo/bin/cargo" build --quiet --manifest-path "$REPO/Cargo.toml" -p sweetpad-cli --bin sweetpad
SWEETPAD="$CARGO_TARGET_DIR/debug/sweetpad"

section "PIF for $(basename "$PROJECT")"
mkdir -p "$DERIVED"
PIF="$DERIVED/project.pif.json"
"$SWEETPAD" pbxproj pif --project "$PROJECT" --file "$PIF"

section "swbuild $TARGET ($CONFIGURATION, $SDK, $ARCH)"
# Swift Build derives the toolchain from the first `swift` on PATH and looks
# dsymutil up on PATH, so the SDK's toolset/bin goes first. Platforms come only
# from XCODE_EXTRA_PLATFORM_FOLDERS, since there is no Xcode to find them in.
export PATH="$SDK_BUNDLE/toolset/bin:$SWIFT_DIR/usr/bin:$PATH"
export XCODE_EXTRA_PLATFORM_FOLDERS="$SDK_BUNDLE/Developer/Platforms"
unset SDKROOT DEVELOPER_DIR
PARAMS="$DERIVED/build-parameters.json"
python3 -I - "$PARAMS" "$CONFIGURATION" "$SDK" "$ARCH" "$SDK_BUNDLE/toolset-swb.json" "${EXTRA_SETTINGS:-}" <<'PY'
import json, sys
out, configuration, sdk, arch, toolset, extra = sys.argv[1:7]
table = {
    "SDKROOT": sdk,
    "ARCHS": arch,
    "ONLY_ACTIVE_ARCH": "YES",
    "CODE_SIGNING_ALLOWED": "NO",
    "SWIFT_SDK_TOOLSETS": toolset,
    "LM_SKIP_METADATA_EXTRACTION": "YES",
    "DONT_RUN_SWIFT_STDLIB_TOOL": "YES",
}
for item in extra.split():
    name, _, value = item.partition("=")
    table[name] = value
with open(out, "w") as f:
    json.dump({
        "action": "build",
        "configurationName": configuration,
        "activeRunDestination": {
            "platform": sdk, "sdk": sdk, "sdkVariant": sdk,
            "targetArchitecture": arch, "supportedArchitectures": [arch],
            "disableOnlyActiveArch": False,
        },
        "overrides": {"commandLine": {"table": table}},
    }, f, indent=2)
PY

# swbuild streams JSON messages. Keep the raw stream in swbuild.log and print
# task names, diagnostics, and the output of any task that failed.
set +e
(cd "$DERIVED" && "$SWBUILD" build "$PIF" --target "$TARGET" --configuration "$CONFIGURATION" \
  --derivedDataPath "$DERIVED" --buildParametersFile "$PARAMS" 2>&1) \
  | tee "$DERIVED/swbuild.log" \
  | python3 -I -u -c '
import json, sys
outputs = {}
for line in sys.stdin:
    line = line.strip()
    try:
        m = json.loads(line) if line.startswith("{") else None
    except ValueError:
        m = None
    if m is None:
        if line and not line.isdigit():
            print(line)
        continue
    kind = m.get("kind")
    if kind == "taskStarted" and "executionDescription" in m:
        print("  " + m["executionDescription"])
    elif kind in ("error", "warning"):
        loc = m.get("location", {})
        where = loc.get("path", "") + (":%s" % loc["line"] if "line" in loc else "")
        print("%s: %s%s" % (kind, where + ": " if where else "", m.get("message", "")))
    elif kind == "taskOutput":
        outputs.setdefault(m.get("taskID"), []).append(m.get("data", ""))
    elif kind == "taskComplete" and m.get("result") == "failed":
        sys.stdout.write("".join(outputs.get(m.get("id"), [])))
'
status=${PIPESTATUS[0]}
set -e
[ "$status" -eq 0 ] || { echo "error: swbuild exited $status (raw log: $DERIVED/swbuild.log)" >&2; exit "$status"; }

# --derivedDataPath puts products at <dd>/Products, not <dd>/Build/Products.
APP="$(ls -d "$DERIVED/Products/$CONFIGURATION-$SDK/"*.app 2>/dev/null | head -1)"
[ -n "$APP" ] || { echo "error: no .app under $DERIVED/Products/$CONFIGURATION-$SDK" >&2; exit 1; }
EXECUTABLE="$APP/$(basename "$APP" .app)"
description="$(file -b "$EXECUTABLE")"
echo
echo "product: $APP"
echo "  $(basename "$EXECUTABLE"): $description"
case "$description" in
  *Mach-O*"$ARCH"*executable*) ;;
  *) echo "error: $EXECUTABLE is not a Mach-O $ARCH executable" >&2; exit 1 ;;
esac
