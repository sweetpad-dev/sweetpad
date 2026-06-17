#!/usr/bin/env bash
#
# capture.sh — drive an oracle recapture inside an isolated Tart macOS VM.
#
# Runs on an Apple Silicon Mac with Tart installed. It clones the pinned,
# Xcode-bundled image for a version, pushes THIS working tree into the VM at
# the canonical checkout path, runs the §10.4 capture there, and pulls the
# regenerated `fixtures/` + `xcspec-cache/` back out. Because the VM user
# (`/Users/admin`) and the checkout path are fixed (ci/tart/images.json), the
# recapture is byte-reproducible — no Xcode ever lands on the host Mac, it
# lives in the image.
#
# Usage:
#   ci/tart/capture.sh <version> [options]
#
#   <version>          A key under "versions" in images.json (e.g. 26.5.0).
#
# Options:
#   --keep             Leave the VM running after capture (for debugging).
#   --no-test          Skip the in-VM `cargo test` validation pass.
#   --image <ref>      Override the image ref from images.json.
#   --name <vm>        VM clone name (default: sweetpad-cap-<version>).
#
# Prereqs on the host: tart, sshpass, rsync, python3.  Install with:
#   brew install cirruslabs/cli/tart sshpass rsync
#
# This drives capture only. Reviewing the diff, recalibrating per-version
# floors (DOCS.md §10.6) and committing stay human steps.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_ROOT="$(cd "$HERE/../.." && pwd)"          # .../sweetpad-lib
IMAGES_JSON="$HERE/images.json"

KEEP=0
RUN_TEST=1
IMAGE_OVERRIDE=""
VM_NAME=""
VERSION=""

die() { echo "capture.sh: $*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep)      KEEP=1; shift ;;
    --no-test)   RUN_TEST=0; shift ;;
    --image)     IMAGE_OVERRIDE="$2"; shift 2 ;;
    --name)      VM_NAME="$2"; shift 2 ;;
    -h|--help)   sed -n '2,40p' "$0"; exit 0 ;;
    -*)          die "unknown option: $1" ;;
    *)           [[ -z "$VERSION" ]] && VERSION="$1" && shift || die "unexpected arg: $1" ;;
  esac
done

[[ -n "$VERSION" ]] || die "missing <version> (a key under images.json 'versions')"
for tool in tart sshpass rsync python3; do
  command -v "$tool" >/dev/null 2>&1 || die "missing host prerequisite: $tool"
done

# --- read the pinned config for this version -------------------------------
# A tiny python reader keeps jq out of the prereqs. Emits shell `KEY=value`.
read_cfg() {
  python3 - "$IMAGES_JSON" "$VERSION" <<'PY'
import json, sys, shlex
cfg = json.load(open(sys.argv[1]))
ver = sys.argv[2]
versions = cfg["versions"]
if ver not in versions:
    sys.exit(f"version {ver!r} not in images.json (have: {', '.join(sorted(versions))})")
v = versions[ver]
d = cfg.get("defaults", {})
def emit(k, val):
    print(f"{k}={shlex.quote(str(val))}")
emit("IMAGE", v["image"])
emit("CANONICAL_HOME", cfg["canonical_home"])
emit("CHECKOUT", cfg["checkout_path"])
emit("CPU", v.get("cpu", d.get("cpu", 4)))
emit("MEMORY", v.get("memory", d.get("memory", 8192)))
emit("DISK", v.get("disk", d.get("disk", 100)))
PY
}
eval "$(read_cfg)" || die "could not read config for $VERSION"
[[ -n "$IMAGE_OVERRIDE" ]] && IMAGE="$IMAGE_OVERRIDE"
[[ -n "$VM_NAME" ]] || VM_NAME="sweetpad-cap-$VERSION"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
SSH_PASS="${TART_VM_PASSWORD:-admin}"           # Cirrus base images: admin/admin
SSH_USER="${TART_VM_USER:-admin}"

echo "==> version=$VERSION image=$IMAGE vm=$VM_NAME"
echo "==> canonical checkout=$CHECKOUT (home=$CANONICAL_HOME)"

# --- lifecycle -------------------------------------------------------------
VM_PID=""
cleanup() {
  [[ -n "$VM_PID" ]] && kill "$VM_PID" 2>/dev/null || true
  if [[ "$KEEP" -eq 1 ]]; then
    echo "==> --keep: leaving VM '$VM_NAME' (stop/delete it yourself: tart delete $VM_NAME)"
  else
    echo "==> tearing down VM '$VM_NAME'"
    tart stop "$VM_NAME" 2>/dev/null || true
    tart delete "$VM_NAME" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "==> pulling + cloning image"
tart clone "$IMAGE" "$VM_NAME"
tart set "$VM_NAME" --cpu "$CPU" --memory "$MEMORY" --disk "$DISK"

echo "==> booting VM (headless)"
# --no-graphics keeps it off the host display. If simulator booting fails in
# the runner, re-run with this flag removed so the guest gets a real display.
tart run --no-graphics "$VM_NAME" &
VM_PID=$!

echo "==> waiting for VM IP"
IP=""
for _ in $(seq 1 60); do
  IP="$(tart ip "$VM_NAME" 2>/dev/null || true)"
  [[ -n "$IP" ]] && break
  sleep 2
done
[[ -n "$IP" ]] || die "VM never reported an IP"
echo "==> VM IP: $IP"

ssh_vm()  { sshpass -p "$SSH_PASS" ssh "${SSH_OPTS[@]}" "$SSH_USER@$IP" "$@"; }
rsync_vm() { rsync -e "sshpass -p $SSH_PASS ssh ${SSH_OPTS[*]}" "$@"; }

echo "==> waiting for SSH"
for _ in $(seq 1 60); do
  ssh_vm true 2>/dev/null && break
  sleep 2
done
ssh_vm true 2>/dev/null || die "SSH never came up"

# --- push the working tree to the canonical path ---------------------------
# Exclude heavy/derived dirs the capture regenerates or ignores. The corpus
# clones (gitignored) are re-cloned inside the VM by 01_clone_corpus.py.
echo "==> syncing working tree -> $CHECKOUT/sweetpad-lib"
ssh_vm "mkdir -p '$CHECKOUT/sweetpad-lib'"
rsync_vm -a --delete \
  --exclude '.git/' \
  --exclude 'target/' \
  --exclude 'corpus/' \
  --exclude '.xcodes/' \
  --exclude 'node_modules/' \
  --exclude 'DerivedData/' \
  "$LIB_ROOT/" "$SSH_USER@$IP:$CHECKOUT/sweetpad-lib/"

# --- run the capture inside the VM -----------------------------------------
RUNNER_ARGS=("$VERSION")
[[ "$RUN_TEST" -eq 0 ]] && RUNNER_ARGS+=(--no-test)
echo "==> running capture in VM: ci/tart/capture-runner.sh ${RUNNER_ARGS[*]}"
ssh_vm "cd '$CHECKOUT/sweetpad-lib' && bash ci/tart/capture-runner.sh ${RUNNER_ARGS[*]}"

# --- pull the regenerated oracles back -------------------------------------
echo "==> pulling fixtures/ + xcspec-cache/ back to host"
rsync_vm -a \
  "$SSH_USER@$IP:$CHECKOUT/sweetpad-lib/fixtures/" "$LIB_ROOT/fixtures/"
rsync_vm -a \
  "$SSH_USER@$IP:$CHECKOUT/sweetpad-lib/xcspec-cache/" "$LIB_ROOT/xcspec-cache/"

cat <<EOF

==> capture complete for $VERSION.

Next (human, per DOCS.md §10.5–10.8):
  cd $LIB_ROOT
  git status                       # expect only real behaviour deltas, no /Users churn
  ORACLE_ONLY_VERSION=$VERSION cargo test --test per_target_oracle -- --nocapture
  # recalibrate per-version floors (§10.6), then commit.
EOF
