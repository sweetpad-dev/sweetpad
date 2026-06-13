#!/usr/bin/env bash
#
# Differential oracle for https://github.com/sweetpad-dev/sweetpad/issues/265.
#
# Captures BUILD_DIR two ways for the SAME workspace, then compares which
# DerivedData container each keys the build under:
#
#   ORACLE   — real `xcodebuild -workspace Apps.xcworkspace -scheme XCodeTarget
#              -showBuildSettings -json` (ground truth)
#   RESOLVER — the in-process resolver via
#              `sweetpad settings show --workspace Apps.xcworkspace --target XCodeTarget`
#
# Xcode keys DerivedData by whichever *container* it opened — the workspace —
# so the oracle's BUILD_DIR lives under `.../DerivedData/Apps-<hash>/...`. The
# member project (`XCodeTarget.xcodeproj`) sits two directories below the
# workspace (`Modules/App/`), so the resolver's container inference can't find
# `Apps.xcworkspace` and falls back to hashing the project itself, yielding
# `.../DerivedData/XCodeTarget-<hash>/...` — a tree xcodebuild never populates.
# That is exactly the symptom reported in #265 (app installed under the `Apps-`
# tree, SweetPad looking under the `XCodeTarget-` tree).
#
# This script exits non-zero when the resolver keys the wrong container. It was
# the original proof of the bug; with the fix in place (the resolver threads the
# workspace through as the DerivedData container) it passes, and stays in CI as
# the regression gate.
#
# Requires: SWEETPAD_BIN pointing at the built binary; the fixture project
# already generated with `xcodegen generate` under fixture-workspace/Modules/App.
set -euo pipefail

BIN="${SWEETPAD_BIN:?set SWEETPAD_BIN to the sweetpad binary}"
# `pwd -P` resolves symlinks so the absolute path we hand to BOTH tools is the
# physical one — xcodebuild and the resolver then hash identical bytes, so the
# 28-char path hash matches too (not just the `<Name>-` prefix).
ROOT="$(cd "$(dirname "$0")/fixture-workspace" && pwd -P)"
WS="$ROOT/Apps.xcworkspace"
SCHEME="XCodeTarget"
TARGET="XCodeTarget"

# Pull `<Name>-<28hash>` out of a BUILD_DIR like
#   /…/DerivedData/<Name>-<hash>/Build/Products
container_name() { sed -n 's#.*/DerivedData/\([^/]*\)/Build/.*#\1#p' <<<"$1"; }
# Strip the trailing `-<28 lowercase letters>` to get just the container name.
container_prefix() { sed 's/-[a-z]\{28\}$//' <<<"$1"; }

# Robust JSON field read: reads the document on stdin and tolerates any
# non-JSON xcodebuild preamble by slicing from the first `{`/`[`. The field
# <expr> (argv[1]) is evaluated over `d`. The program is passed via `-c` so
# stdin stays free for the piped JSON.
json_get() {
  python3 -c '
import json, sys
raw = sys.stdin.read()
starts = [i for i in (raw.find("{"), raw.find("[")) if i != -1]
d = json.loads(raw[min(starts):]) if starts else json.loads(raw)
print(eval(sys.argv[1], {"d": d}))
' "$1"
}

echo "==== issue #265 workspace DerivedData differential ===="
echo "workspace: $WS"
echo "scheme:    $SCHEME (oracle) / target $TARGET (resolver)"
echo

# --- ORACLE: real xcodebuild -------------------------------------------------
oracle_build_dir="$(
  xcodebuild -workspace "$WS" -scheme "$SCHEME" \
    -configuration Debug -sdk iphonesimulator -showBuildSettings -json \
    | json_get 'd[0]["buildSettings"]["BUILD_DIR"]'
)"

# --- RESOLVER: sweetpad-lib, fully in-process --------------------------------
# `--target` bypasses scheme resolution (no `xcodebuild -list`), so this BUILD_DIR
# is computed purely by the resolver — a clean differential against the oracle.
resolver_build_dir="$(
  "$BIN" settings show --workspace "$WS" --target "$TARGET" --key BUILD_DIR --json \
    | json_get 'd["targets"][0]["settings"]["BUILD_DIR"]'
)"

oracle_name="$(container_name "$oracle_build_dir")"
resolver_name="$(container_name "$resolver_build_dir")"
oracle_prefix="$(container_prefix "$oracle_name")"
resolver_prefix="$(container_prefix "$resolver_name")"

echo "ORACLE   BUILD_DIR: $oracle_build_dir"
echo "RESOLVER BUILD_DIR: $resolver_build_dir"
echo
echo "ORACLE   DerivedData container: $oracle_name   (prefix: $oracle_prefix)"
echo "RESOLVER DerivedData container: $resolver_name   (prefix: $resolver_prefix)"
echo

if [ -z "$oracle_prefix" ]; then
  echo "ERROR: could not parse a DerivedData container from the oracle BUILD_DIR" >&2
  echo "       (got: '$oracle_build_dir') — setup fault, not a resolver verdict." >&2
  exit 2
fi

# The crux: compare the container NAME (independent of the path hash, so the
# verdict can't be muddied by absolute-path/symlink drift in the 28 chars).
if [ "$oracle_prefix" != "$resolver_prefix" ]; then
  echo "FAIL (issue #265 reproduced):" >&2
  echo "  Xcode keys DerivedData under '$oracle_prefix' (the workspace it opened)," >&2
  echo "  but the resolver keyed it under '$resolver_prefix' (the member project)." >&2
  echo "  The resolver is hashing $resolver_prefix.xcodeproj instead of the" >&2
  echo "  $oracle_prefix.xcworkspace container — so it points at a Build/Products" >&2
  echo "  tree xcodebuild never writes, and the built app can't be found." >&2
  exit 1
fi

# Container name agrees. With physical paths the full hash must match too — if
# not, surface it (informational; the name verdict above is the gate).
if [ "$oracle_build_dir" != "$resolver_build_dir" ]; then
  echo "WARN: container name matches but full BUILD_DIR differs (path-hash drift):" >&2
  echo "      oracle  =$oracle_build_dir" >&2
  echo "      resolver=$resolver_build_dir" >&2
fi

echo "PASS: resolver BUILD_DIR is keyed under the workspace Xcode opened ($oracle_name)."
