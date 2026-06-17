#!/usr/bin/env bash
#
# env-setup.sh — prepare the canonical capture/test/debug environment INSIDE
# the Tart VM. Shared by every mode (test, capture, shell) so all three run
# against an identical, byte-stable environment. Idempotent.
#
# Usage (cwd = .../sweetpad-lib):  ci/tart/env-setup.sh <version>
#
# It (1) enforces the identity invariant, (2) exposes the image's bundled
# Xcode as /Applications/Xcode-<ver>.app so the capture scripts discover it
# without a download, (3) ensures the pinned Rust toolchain, and (4) persists
# DEVELOPER_DIR + PATH + the canonical cwd into the login profiles so every
# later shell — interactive (`shell`) or programmatic (`test`/`capture`) —
# inherits the same env. Set SWEETPAD_ALLOW_NONCANONICAL=1 to bypass the guard
# (throwaway experiments only — it reintroduces host path drift).

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_ROOT="$(cd "$HERE/../.." && pwd)"
IMAGES_JSON="$HERE/images.json"
cd "$LIB_ROOT"

VERSION="${1:?usage: env-setup.sh <version>}"
log() { echo "[env-setup] $*" >&2; }

eval "$(python3 - "$IMAGES_JSON" "$VERSION" <<'PY'
import json, sys, shlex
cfg = json.load(open(sys.argv[1])); ver = sys.argv[2]
if ver not in cfg["versions"]:
    sys.exit(f"version {ver!r} not in images.json")
print(f'CANONICAL_HOME={shlex.quote(cfg["canonical_home"])}')
print(f'CHECKOUT={shlex.quote(cfg["checkout_path"])}')
PY
)"

# --- identity guard --------------------------------------------------------
if [[ "${SWEETPAD_ALLOW_NONCANONICAL:-0}" != "1" ]]; then
  [[ "$HOME" == "$CANONICAL_HOME" ]] || {
    log "ABORT: \$HOME='$HOME' != canonical '$CANONICAL_HOME'"; exit 1; }
  [[ "$LIB_ROOT" == "$CHECKOUT/sweetpad-lib" ]] || {
    log "ABORT: checkout '$LIB_ROOT' != canonical '$CHECKOUT/sweetpad-lib'"; exit 1; }
fi

# --- expose the bundled Xcode as Xcode-<ver>.app (no download) --------------
# discover_installed_xcodes() (common.py) only matches /Applications/
# Xcode-X.Y(.Z).app; Cirrus images ship it as the selected default, so
# symlink the versioned name. The orchestrator then reuses it in place.
XCODE_APP="$(cd "$(xcode-select -p)/../.." && pwd)"
VERSIONED="/Applications/Xcode-$VERSION.app"
[[ -e "$VERSIONED" ]] || { log "symlink $VERSIONED -> $XCODE_APP"; ln -s "$XCODE_APP" "$VERSIONED"; }
DEVELOPER_DIR="$VERSIONED/Contents/Developer"

# --- pinned Rust toolchain (needed by `test` and capture validation) -------
if ! command -v rustup >/dev/null 2>&1; then
  log "installing rustup"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain none --no-modify-path
fi
PATH="$HOME/.cargo/bin:$PATH" rustup show >/dev/null 2>&1 || true   # honors rust-toolchain.toml

# --- persist the env into login profiles -----------------------------------
# So `shell` (interactive zsh) and `test`/`capture` (bash -lc) share one env.
BLOCK_BEGIN="# >>> sweetpad-tart env >>>"
BLOCK_END="# <<< sweetpad-tart env <<<"
read -r -d '' BLOCK <<EOF || true
$BLOCK_BEGIN
export DEVELOPER_DIR="$DEVELOPER_DIR"
export PATH="\$HOME/.cargo/bin:\$PATH"
cd "$CHECKOUT/sweetpad-lib" 2>/dev/null || true
$BLOCK_END
EOF
for profile in "$HOME/.zprofile" "$HOME/.bash_profile"; do
  if [[ ! -f "$profile" ]] || ! grep -qF "$BLOCK_BEGIN" "$profile"; then
    printf '\n%s\n' "$BLOCK" >> "$profile"
    log "wrote env block -> $profile"
  fi
done

log "ready: Xcode=$(DEVELOPER_DIR="$DEVELOPER_DIR" xcodebuild -version | tr '\n' ' ')"
