#!/usr/bin/env bash
#
# capture-runner.sh — the CAPTURE mode body, run INSIDE the Tart VM from the
# sweetpad-lib root. Shares the environment with `test`/`shell` via
# env-setup.sh; this script adds only the capture-specific steps (corpus build
# tools, disposable simulator runtimes, the §10.4 capture, report + validate).
#
# Invoked by env.sh (capture mode) over SSH, by Cirrus, or by hand on a
# canonical host. Usage (cwd = .../sweetpad-lib):
#   ci/tart/capture-runner.sh <version> [--no-test]

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

# Shared env prep: identity guard, Xcode symlink, Rust, persisted profile.
bash "$HERE/env-setup.sh" "$VERSION"
export DEVELOPER_DIR="/Applications/Xcode-$VERSION.app/Contents/Developer"
export PATH="$HOME/.cargo/bin:$PATH"

eval "$(python3 - "$IMAGES_JSON" "$VERSION" <<'PY'
import json, sys, shlex
cfg = json.load(open(sys.argv[1])); v = cfg["versions"][sys.argv[2]]
print(f'SUBSET={shlex.quote(",".join(v["subset"]))}')
print(f'RUNTIMES={shlex.quote(" ".join(v.get("runtimes", [])))}')
PY
)"

# --- capture-only tooling --------------------------------------------------
for f in tuist xcodegen xclogparser; do
  command -v "$f" >/dev/null 2>&1 || brew install "$f" || brew install cirruslabs/cli/"$f" || true
done

# --- disposable simulator runtimes (DOCS.md §10.3) -------------------------
for plat in $RUNTIMES; do
  log "downloading $plat runtime"
  xcodebuild -downloadPlatform "$plat" || log "WARN: -downloadPlatform $plat failed (continuing)"
done
if [[ -n "$RUNTIMES" ]]; then
  FIRST_SIM="$(xcrun simctl list devices available | awk -F'[()]' '/\(.*\)/{print $2; exit}')" || true
  [[ -n "${FIRST_SIM:-}" ]] && xcrun simctl boot "$FIRST_SIM" 2>/dev/null || true
fi

# --- capture ---------------------------------------------------------------
log "capturing version=$VERSION subset=$SUBSET"
python3 scripts/13_capture_version.py \
  --versions "$VERSION" --subset "$SUBSET" \
  --no-runtime --keep --force --min-disk-gb 5

python3 scripts/05_validate.py || log "WARN: 05_validate failed"
python3 scripts/06_audit_coverage.py || log "WARN: 06_audit_coverage failed"

if [[ "$RUN_TEST" -eq 1 ]]; then
  log "validating: cargo test"
  cargo test || log "WARN: cargo test reported failures — triage per DOCS.md §10.5"
fi
log "done."
