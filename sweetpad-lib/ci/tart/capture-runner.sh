#!/usr/bin/env bash
#
# capture-runner.sh — runs INSIDE the Tart VM, from the sweetpad-lib root.
#
# Invoked by capture.sh over SSH (or by hand on any canonical capture host).
# It enforces the byte-identity invariant, makes the image's bundled Xcode
# discoverable to the capture scripts WITHOUT re-downloading it, provisions
# the disposable simulator runtimes the version needs, then runs the §10.4
# corpus capture and (optionally) validates.
#
# Usage (cwd = .../sweetpad-lib):
#   ci/tart/capture-runner.sh <version> [--no-test]
#
# Set SWEETPAD_ALLOW_NONCANONICAL=1 to bypass the identity guard (NOT for a
# capture whose output you intend to commit — it reintroduces host path drift).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_ROOT="$(cd "$HERE/../.." && pwd)"
IMAGES_JSON="$HERE/images.json"
cd "$LIB_ROOT"

RUN_TEST=1
VERSION=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-test) RUN_TEST=0; shift ;;
    -*)        echo "unknown option: $1" >&2; exit 2 ;;
    *)         VERSION="$1"; shift ;;
  esac
done
[[ -n "$VERSION" ]] || { echo "missing <version>" >&2; exit 2; }

log() { echo "[capture-runner] $*" >&2; }

# --- read the pinned config -----------------------------------------------
read_cfg() {
  python3 - "$IMAGES_JSON" "$VERSION" <<'PY'
import json, sys, shlex
cfg = json.load(open(sys.argv[1])); ver = sys.argv[2]
v = cfg["versions"][ver]
print(f'CANONICAL_HOME={shlex.quote(cfg["canonical_home"])}')
print(f'CHECKOUT={shlex.quote(cfg["checkout_path"])}')
print(f'SUBSET={shlex.quote(",".join(v["subset"]))}')
print(f'RUNTIMES={shlex.quote(" ".join(v.get("runtimes", [])))}')
PY
}
eval "$(read_cfg)"

# --- identity guard: the whole point of capturing in the VM ----------------
# A capture run anywhere but the canonical home/path reintroduces the host
# fingerprint (the ~95k `/Users/<you>` strings, a different DerivedData hash)
# that the VM exists to eliminate. Refuse unless explicitly overridden.
if [[ "${SWEETPAD_ALLOW_NONCANONICAL:-0}" != "1" ]]; then
  [[ "$HOME" == "$CANONICAL_HOME" ]] || {
    log "ABORT: \$HOME='$HOME' != canonical '$CANONICAL_HOME'."
    log "Capture only on the canonical host, or set SWEETPAD_ALLOW_NONCANONICAL=1 (drift!)."
    exit 1
  }
  [[ "$LIB_ROOT" == "$CHECKOUT/sweetpad-lib" ]] || {
    log "ABORT: checkout '$LIB_ROOT' != canonical '$CHECKOUT/sweetpad-lib'."
    log "The DerivedData hash is path-derived; a different path is not byte-stable."
    exit 1
  }
fi

# --- make the bundled Xcode discoverable as Xcode-<ver>.app ----------------
# discover_installed_xcodes() (scripts/common.py) only matches
# /Applications/Xcode-X.Y(.Z).app. Cirrus images ship the Xcode as the
# selected default (often /Applications/Xcode.app), so symlink the versioned
# name to it. Then `13_capture_version.py --versions <ver>` finds it already
# installed and REUSES it (no `xcodes install`, no download).
XCODE_DEV="$(xcode-select -p)"
XCODE_APP="$(cd "$XCODE_DEV/../.." && pwd)"     # .../Xcode*.app
VERSIONED="/Applications/Xcode-$VERSION.app"
if [[ ! -e "$VERSIONED" ]]; then
  log "symlinking $VERSIONED -> $XCODE_APP"
  ln -s "$XCODE_APP" "$VERSIONED"
fi
export DEVELOPER_DIR="$VERSIONED/Contents/Developer"
log "Xcode: $(xcodebuild -version | tr '\n' ' ')"

# --- host tooling the capture scripts expect -------------------------------
# Cirrus xcode images already carry brew + Xcode; add the corpus-build tools.
for f in tuist xcodegen xclogparser; do
  command -v "$f" >/dev/null 2>&1 || brew install "$f" || brew install cirruslabs/cli/"$f" || true
done
rustup show >/dev/null 2>&1 || true            # install the pinned toolchain if rustup is present

# --- provision disposable simulator runtimes (DOCS.md §10.3) ---------------
# Build settings do NOT depend on runtime version (§5.1), so any runtime per
# platform satisfies that platform's captures. We pre-provision here and pass
# --no-runtime to the orchestrator so it doesn't manage them.
for plat in $RUNTIMES; do
  log "downloading $plat runtime"
  xcodebuild -downloadPlatform "$plat" || log "WARN: -downloadPlatform $plat failed (continuing)"
done
if [[ -n "$RUNTIMES" ]]; then
  # Warm CoreSimulator so -showdestinations doesn't race (§10.3 / Gotchas).
  FIRST_SIM="$(xcrun simctl list devices available | awk -F'[()]' '/\(.*\)/{print $2; exit}')" || true
  [[ -n "${FIRST_SIM:-}" ]] && xcrun simctl boot "$FIRST_SIM" 2>/dev/null || true
fi

# --- capture ---------------------------------------------------------------
log "capturing version=$VERSION subset=$SUBSET"
python3 scripts/13_capture_version.py \
  --versions "$VERSION" \
  --subset "$SUBSET" \
  --no-runtime --keep --force --min-disk-gb 5

# Regenerate the capture-completeness / feature-probe report against the fresh
# corpus (its corpus-tree probes need the clones, which exist here).
python3 scripts/05_validate.py || log "WARN: 05_validate failed"
python3 scripts/06_audit_coverage.py || log "WARN: 06_audit_coverage failed"

if [[ "$RUN_TEST" -eq 1 ]]; then
  log "validating: cargo test"
  cargo test || log "WARN: cargo test reported failures — triage per DOCS.md §10.5"
fi

log "done."
