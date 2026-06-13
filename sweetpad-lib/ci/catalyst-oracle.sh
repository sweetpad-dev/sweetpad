#!/usr/bin/env bash
#
# Differential oracle for PR #264 / native-macOS Catalyst misdetection
# (https://github.com/sweetpad-dev/sweetpad/pull/264).
#
# Captures the Catalyst-defining build settings two ways for the SAME target,
# built for macOS:
#
#   ORACLE   — real `xcodebuild -project … -target … -sdk macosx
#              -showBuildSettings -json` (ground truth)
#   RESOLVER — the in-process resolver via
#              `sweetpad settings show --project … --target … --destination platform=macOS`
#
# The fixture target has a project-wide iOS-family Base SDK (`SDKROOT=iphoneos`)
# but supports native macOS with `SUPPORTS_MACCATALYST=NO`. Xcode builds it as a
# native macOS app — IS_MACCATALYST=NO, EFFECTIVE_PLATFORM_NAME="", products
# under `…/Debug/`. The resolver currently keys Catalyst off the iOS-family
# SDKROOT alone, so it reports IS_MACCATALYST=YES and `…/Debug-maccatalyst/`,
# pointing at a build dir Xcode never writes (the symptom in #264).
#
# This script exits non-zero on that divergence: a RED run captures the oracle
# and proves the bug. With PR #264's detect_catalyst reorder (explicit
# SUPPORTS_MACCATALYST=NO wins) it goes green.
#
# Requires: SWEETPAD_BIN pointing at the built binary; the fixture project
# already generated with `xcodegen generate` under fixture-catalyst.
set -euo pipefail

BIN="${SWEETPAD_BIN:?set SWEETPAD_BIN to the sweetpad binary}"
ROOT="$(cd "$(dirname "$0")/fixture-catalyst" && pwd -P)"
PROJ="$ROOT/CatalystProbe.xcodeproj"
TARGET="CatalystProbe"

# Read one build-setting key. <expr> (argv[1]) selects the settings dict from
# the parsed document `d`; argv[2] is the key. Program via `-c` so stdin stays
# free for the piped JSON; tolerate any preamble by slicing from the first {/[.
get_key() {
  python3 -c '
import json, sys
raw = sys.stdin.read()
starts = [i for i in (raw.find("{"), raw.find("[")) if i != -1]
d = json.loads(raw[min(starts):]) if starts else json.loads(raw)
print(eval(sys.argv[1], {"d": d}).get(sys.argv[2], ""))
' "$1" "$2"
}
oracle_key()   { get_key 'd[0]["buildSettings"]' "$1" <<<"$oracle_json"; }
resolver_key() { get_key 'd["targets"][0]["settings"]' "$1" <<<"$resolver_json"; }

echo "==== PR #264 native-macOS Catalyst differential ===="
echo "project: $PROJ"
echo "target:  $TARGET   destination: macOS"
echo

oracle_json="$(
  xcodebuild -project "$PROJ" -target "$TARGET" \
    -configuration Debug -sdk macosx -showBuildSettings -json
)"
resolver_json="$(
  "$BIN" settings show --project "$PROJ" --target "$TARGET" \
    --destination "platform=macOS" --json
)"

printf '  %-24s %-14s %s\n' "setting" "ORACLE" "RESOLVER"
for key in PLATFORM_NAME IS_MACCATALYST EFFECTIVE_PLATFORM_NAME TARGET_BUILD_DIR; do
  o="$(oracle_key "$key")"
  r="$(resolver_key "$key")"
  printf '  %-24s %-14s %s\n' "$key" "${o:-<empty>}" "${r:-<empty>}"
done
echo

# Sanity guard: both sides must actually have resolved for macOS, or the
# IS_MACCATALYST comparison below is meaningless (a false green). If the
# resolver didn't bind macosx, that's a setup fault, not a verdict.
o_platform="$(oracle_key PLATFORM_NAME)"
r_platform="$(resolver_key PLATFORM_NAME)"
if [ "$o_platform" != "macosx" ] || [ "$r_platform" != "macosx" ]; then
  echo "ERROR: expected both sides to resolve for macOS (PLATFORM_NAME=macosx);" >&2
  echo "       got oracle='$o_platform' resolver='$r_platform' — setup fault." >&2
  exit 2
fi

oracle_is="$(oracle_key IS_MACCATALYST)"
resolver_is="$(resolver_key IS_MACCATALYST)"
if [ -z "$oracle_is" ]; then
  echo "ERROR: oracle did not report IS_MACCATALYST — setup fault." >&2
  exit 2
fi

if [ "$oracle_is" != "$resolver_is" ]; then
  echo "FAIL (PR #264 reproduced):" >&2
  echo "  xcodebuild builds this native macOS target with IS_MACCATALYST=$oracle_is," >&2
  echo "  but the resolver reports IS_MACCATALYST=$resolver_is. It is keying Catalyst" >&2
  echo "  off the iOS-family SDKROOT and ignoring the explicit SUPPORTS_MACCATALYST=NO," >&2
  echo "  so it points at a '-maccatalyst' build dir Xcode never writes." >&2
  exit 1
fi

echo "PASS: resolver matches xcodebuild — IS_MACCATALYST=$oracle_is (native macOS, not Catalyst)."
