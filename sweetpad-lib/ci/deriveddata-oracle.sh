#!/usr/bin/env bash
#
# Differential oracle for Xcode's DerivedData path hashing (issue #285 class —
# https://github.com/sweetpad-dev/sweetpad/issues/285).
#
# For each container shape it captures BUILD_DIR two ways and asserts they are
# byte-for-byte identical:
#
#   ORACLE   — real `xcodebuild … -showBuildSettings -json` (ground truth)
#   RESOLVER — the in-process resolver via `sweetpad settings show … --json`
#
# BUILD_DIR is the DerivedData-anchored path
# `~/Library/Developer/Xcode/DerivedData/<Name>-<28hash>/Build/Products`, so an
# exact match proves the resolver reproduces BOTH halves of the folder name (the
# `<Name>` stem AND the MD5/base-26 `<hash>` of the container path) that Xcode
# actually wrote.
#
# Scenarios (A–C are hard gates; D is an informational probe — see its note):
#   A  bare project              -project  DDProbe.xcodeproj
#   B  the embedded stub (#285)  -workspace DDProbe.xcodeproj/project.xcworkspace
#   C  a real sibling workspace  -workspace App.xcworkspace  (refs DDProbe.xcodeproj)
#   D  a non-ASCII (NFD) path    bare project under a precomposed "Café/" dir
#
# Requires: SWEETPAD_BIN pointing at the built binary; the fixture project
# already generated with `xcodegen generate` under fixture-deriveddata.
set -euo pipefail

BIN="${SWEETPAD_BIN:?set SWEETPAD_BIN to the sweetpad binary}"
ROOT="$(cd "$(dirname "$0")/fixture-deriveddata" && pwd -P)"
PROJ="$ROOT/DDProbe.xcodeproj"
SCHEME="DDProbe"
DEST="platform=macOS"

# Read one build-setting key. argv[1] selects the settings dict from the parsed
# document `d`; argv[2] is the key. Programmed via `-c` so stdin stays free for
# the piped JSON; tolerate any preamble by slicing from the first {/[.
get_key() {
  python3 -c '
import json, sys
raw = sys.stdin.read()
starts = [i for i in (raw.find("{"), raw.find("[")) if i != -1]
d = json.loads(raw[min(starts):]) if starts else json.loads(raw)
print(eval(sys.argv[1], {"d": d}).get(sys.argv[2], ""))
' "$1" "$2"
}
xcb_build_dir() { get_key 'd[0]["buildSettings"]'    BUILD_DIR; }
res_build_dir() { get_key 'd["targets"][0]["settings"]' BUILD_DIR; }

# The `<Name>-<hash>` DerivedData folder segment out of a BUILD_DIR.
dd_name() { sed -E 's#.*/DerivedData/([^/]+)/Build/.*#\1#' <<<"$1"; }

fail=0

# compare <label> <oracle_build_dir> <resolver_build_dir> <fatal:1|0>
compare() {
  local label="$1" o="$2" r="$3" fatal="$4"
  echo "---- $label ----"
  echo "  ORACLE   BUILD_DIR: ${o:-<empty>}   (folder: $(dd_name "$o"))"
  echo "  RESOLVER BUILD_DIR: ${r:-<empty>}   (folder: $(dd_name "$r"))"
  if [ -z "$o" ] || [ -z "$r" ]; then
    echo "  ERROR: one side did not resolve a BUILD_DIR — setup fault." >&2
    [ "$fatal" = 1 ] && fail=1
    echo; return
  fi
  if [ "$o" = "$r" ]; then
    echo "  PASS: paths are identical."
  else
    echo "  MISMATCH: resolver path differs from xcodebuild." >&2
    if [ "$fatal" = 1 ]; then
      fail=1
    else
      echo "  (non-fatal probe — see scenario note)"
    fi
  fi
  echo
}

echo "=================================================================="
echo " DerivedData path-hash differential oracle"
echo " project: $PROJ"
echo " HOME=$HOME"
xcodebuild -version | head -1
echo "=================================================================="
echo

# ---- A: bare project -------------------------------------------------------
A_ORACLE="$(xcodebuild -project "$PROJ" -scheme "$SCHEME" -configuration Debug \
  -destination "$DEST" -showBuildSettings -json | xcb_build_dir)"
A_RES="$("$BIN" settings show --project "$PROJ" --scheme "$SCHEME" \
  --destination "$DEST" --json | res_build_dir)"
compare "A · bare project (-project DDProbe.xcodeproj)" "$A_ORACLE" "$A_RES" 1

# ---- B: the embedded project.xcworkspace stub (issue #285) -----------------
# Xcode auto-generates this inside every .xcodeproj. A user can point
# xcodeWorkspacePath straight at it; Xcode keys DerivedData on the OUTER
# .xcodeproj (DDProbe-<hash>), never the stub (project-<hash>). The hard gate
# below is what empirically confirms that — if reality used `project-<hash>`,
# the resolver (which now collapses the stub) would mismatch and fail here.
STUB="$PROJ/project.xcworkspace"
if [ ! -e "$STUB/contents.xcworkspacedata" ]; then
  # Defensive: synthesize the self-referencing stub if XcodeGen didn't emit one.
  mkdir -p "$STUB"
  cat >"$STUB/contents.xcworkspacedata" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<Workspace version = "1.0">
   <FileRef location = "self:"></FileRef>
</Workspace>
XML
fi
B_ORACLE="$(xcodebuild -workspace "$STUB" -scheme "$SCHEME" -configuration Debug \
  -destination "$DEST" -showBuildSettings -json | xcb_build_dir)"
B_RES="$("$BIN" settings show --workspace "$STUB" --scheme "$SCHEME" \
  --destination "$DEST" --json | res_build_dir)"
compare "B · embedded stub (-workspace DDProbe.xcodeproj/project.xcworkspace) [#285]" \
  "$B_ORACLE" "$B_RES" 1

# Cross-check: the stub must resolve to the SAME folder as the bare project,
# and the folder must be DDProbe-*, not project-*.
echo "---- B cross-checks ----"
if [ "$B_ORACLE" = "$A_ORACLE" ]; then
  echo "  PASS: xcodebuild keys the stub on the outer .xcodeproj (same as A)."
else
  echo "  NOTE: xcodebuild's stub folder ($(dd_name "$B_ORACLE")) differs from the" >&2
  echo "        bare project ($(dd_name "$A_ORACLE")) — the resolver matches the" >&2
  echo "        oracle either way (gate B), but the #285 assumption needs review." >&2
fi
case "$(dd_name "$B_ORACLE")" in
  DDProbe-*) echo "  PASS: folder prefix is DDProbe-, not project-." ;;
  project-*) echo "  FAIL: xcodebuild used a project-<hash> folder for the stub." >&2; fail=1 ;;
  *)         echo "  NOTE: unexpected folder $(dd_name "$B_ORACLE")." >&2 ;;
esac
echo

# ---- C: a real sibling workspace ------------------------------------------
WS="$ROOT/App.xcworkspace"
mkdir -p "$WS"
cat >"$WS/contents.xcworkspacedata" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<Workspace version = "1.0">
   <FileRef location = "group:DDProbe.xcodeproj"></FileRef>
</Workspace>
XML
C_ORACLE="$(xcodebuild -workspace "$WS" -scheme "$SCHEME" -configuration Debug \
  -destination "$DEST" -showBuildSettings -json | xcb_build_dir)"
C_RES="$("$BIN" settings show --workspace "$WS" --scheme "$SCHEME" \
  --destination "$DEST" --json | res_build_dir)"
compare "C · real sibling workspace (-workspace App.xcworkspace)" "$C_ORACLE" "$C_RES" 1
case "$(dd_name "$C_ORACLE")" in
  App-*) echo "  (folder is App-<hash> — keyed on the workspace, not the member project)"; echo ;;
esac

# ---- D: non-ASCII (NFD) path — INFORMATIONAL PROBE -------------------------
# We MD5 the NFD form of the path. Whether that matches xcodebuild depends on
# the host filesystem: HFS+ stores/returns decomposed names (so NFD is right),
# but the GitHub runner is APFS, which is normalization-INSENSITIVE and
# preserves the bytes as created — so xcodebuild may hash the precomposed (NFC)
# spelling here. This probe is therefore NON-FATAL: it records what real macOS
# does so we can decide whether the NFD step is correct, without gating CI on a
# genuinely filesystem-dependent answer.
ACCENT="$(python3 -c 'import sys; sys.stdout.write("Café")')" # precomposed é = U+00E9
WORK="$ROOT/_work/$ACCENT"
rm -rf "$ROOT/_work"; mkdir -p "$WORK"
cp -R "$PROJ" "$WORK/"
NPROJ="$WORK/DDProbe.xcodeproj"
D_ORACLE="$(xcodebuild -project "$NPROJ" -scheme "$SCHEME" -configuration Debug \
  -destination "$DEST" -showBuildSettings -json | xcb_build_dir)"
D_RES="$("$BIN" settings show --project "$NPROJ" --scheme "$SCHEME" \
  --destination "$DEST" --json | res_build_dir)"
compare "D · non-ASCII path under precomposed 'Café/' (NFD probe, non-fatal)" \
  "$D_ORACLE" "$D_RES" 0

echo "=================================================================="
if [ "$fail" = 0 ]; then
  echo "RESULT: PASS — resolver BUILD_DIR matches xcodebuild on every hard gate."
else
  echo "RESULT: FAIL — at least one hard gate diverged (see above)." >&2
fi
echo "=================================================================="
exit "$fail"
