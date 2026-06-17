#!/usr/bin/env bash
#
# env.sh — ONE environment for tests, capture, and debugging.
#
# A single pinned Tart VM per Xcode version (ci/tart/images.json) is the shared
# substrate for all three activities, so a failing oracle reproduces under the
# exact paths, Xcode, and toolchain it was captured with. Runs on an Apple
# Silicon Mac with Tart installed.
#
#   ci/tart/env.sh <command> <version> [options] [-- args]
#
# Commands:
#   up <ver>            Ensure the VM exists, is booted, has this tree synced,
#                       and the canonical env prepared. Idempotent; persistent.
#   shell <ver>         Interactive debug shell in the VM at the canonical
#                       checkout (DEVELOPER_DIR + cargo already on PATH).
#   test <ver> [-- …]   Sync this tree in, run `cargo test [args]` in the VM.
#   run  <ver> -- <cmd> Sync this tree in, run an arbitrary command in the VM.
#   capture <ver>       Full oracle capture; pulls fixtures/ + xcspec-cache/ back.
#   sync <ver>          Re-push this working tree into a running VM (after edits).
#   down <ver>          Stop and delete the VM.
#
# Options:  --keep (don't auto-delete a VM this run created), --image <ref>,
#           --name <vm>, --no-test (capture: skip the in-VM cargo test).
#
# Lifecycle: `up`/`shell` create a PERSISTENT VM you keep reusing. `test`/`run`/
# `capture` REUSE that VM if present; if they had to create one they delete it
# afterward (override with --keep). So bring a box up once and test/capture/
# debug against it, or fire a one-shot that cleans up after itself.
#
# Prereqs:  brew install cirruslabs/cli/tart sshpass rsync

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LIB_ROOT="$(cd "$HERE/../.." && pwd)"
IMAGES_JSON="$HERE/images.json"
die() { echo "env.sh: $*" >&2; exit 1; }

COMMAND="${1:-}"; shift || true
[[ -n "$COMMAND" ]] || { sed -n '2,40p' "$0"; exit 0; }

KEEP=0; RUN_TEST=1; IMAGE_OVERRIDE=""; VM_NAME=""; VERSION=""; EXTRA=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --keep)    KEEP=1; shift ;;
    --no-test) RUN_TEST=0; shift ;;
    --image)   IMAGE_OVERRIDE="$2"; shift 2 ;;
    --name)    VM_NAME="$2"; shift 2 ;;
    --)        shift; EXTRA=("$@"); break ;;
    -*)        die "unknown option: $1" ;;
    *)         [[ -z "$VERSION" ]] && VERSION="$1" && shift || die "unexpected arg: $1" ;;
  esac
done
[[ -n "$VERSION" ]] || die "missing <version> (a key under images.json 'versions')"
for t in tart python3; do command -v "$t" >/dev/null 2>&1 || die "missing prerequisite: $t"; done

# --- pinned config ---------------------------------------------------------
eval "$(python3 - "$IMAGES_JSON" "$VERSION" <<'PY'
import json, sys, shlex
cfg = json.load(open(sys.argv[1])); ver = sys.argv[2]
v = cfg["versions"].get(ver)
if v is None: sys.exit(f"version {ver!r} not in images.json (have: {', '.join(sorted(cfg['versions']))})")
d = cfg.get("defaults", {})
q = shlex.quote
print(f'IMAGE={q(v["image"])}'); print(f'CHECKOUT={q(cfg["checkout_path"])}')
print(f'CANONICAL_HOME={q(cfg["canonical_home"])}')
print(f'CPU={v.get("cpu", d.get("cpu",4))}'); print(f'MEMORY={v.get("memory", d.get("memory",8192))}')
print(f'DISK={v.get("disk", d.get("disk",100))}')
PY
)" || die "config read failed"
[[ -n "$IMAGE_OVERRIDE" ]] && IMAGE="$IMAGE_OVERRIDE"
[[ -n "$VM_NAME" ]] || VM_NAME="sweetpad-$VERSION"

SSH_OPTS=(-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR)
SSH_PASS="${TART_VM_PASSWORD:-admin}"; SSH_USER="${TART_VM_USER:-admin}"

vm_exists()  { tart list --format json 2>/dev/null | python3 -c "import json,sys;print(any(v['Name']=='$VM_NAME' for v in json.load(sys.stdin)))" | grep -q True; }
vm_ip()      { tart ip "$VM_NAME" 2>/dev/null || true; }
ssh_vm()     { sshpass -p "$SSH_PASS" ssh "${SSH_OPTS[@]}" "$SSH_USER@$IP" "$@"; }
ssh_tty()    { sshpass -p "$SSH_PASS" ssh -t "${SSH_OPTS[@]}" "$SSH_USER@$IP" "$@"; }
in_vm()      { ssh_vm "bash -lc $(printf '%q' "$1")"; }   # login shell => sourced env
rsync_vm()   { command -v rsync >/dev/null || die "missing prerequisite: rsync"
               rsync -e "sshpass -p $SSH_PASS ssh ${SSH_OPTS[*]}" "$@"; }

sync_tree() {
  echo "==> syncing working tree -> $CHECKOUT/sweetpad-lib" >&2
  ssh_vm "mkdir -p '$CHECKOUT/sweetpad-lib'"
  rsync_vm -a --delete --exclude '.git/' --exclude 'target/' --exclude 'corpus/' \
    --exclude '.xcodes/' --exclude 'node_modules/' --exclude 'DerivedData/' \
    "$LIB_ROOT/" "$SSH_USER@$IP:$CHECKOUT/sweetpad-lib/"
}

CREATED=0
ensure_up() {   # boot (cloning if needed) + capture IP; set CREATED + PREPARE
  local prepare="${1:-1}"
  if ! vm_exists; then
    echo "==> cloning image $IMAGE -> $VM_NAME" >&2
    tart clone "$IMAGE" "$VM_NAME"
    tart set "$VM_NAME" --cpu "$CPU" --memory "$MEMORY" --disk "$DISK"
    CREATED=1
  fi
  IP="$(vm_ip)"
  if [[ -z "$IP" ]]; then
    echo "==> booting $VM_NAME (headless)" >&2
    nohup tart run --no-graphics "$VM_NAME" >/tmp/tart-$VM_NAME.log 2>&1 &
    disown || true
    for _ in $(seq 1 90); do IP="$(vm_ip)"; [[ -n "$IP" ]] && break; sleep 2; done
  fi
  [[ -n "$IP" ]] || die "VM '$VM_NAME' never reported an IP (see /tmp/tart-$VM_NAME.log)"
  for _ in $(seq 1 60); do ssh_vm true 2>/dev/null && break; sleep 2; done
  ssh_vm true 2>/dev/null || die "SSH to '$VM_NAME' never came up"
  echo "==> $VM_NAME up at $IP" >&2
  if [[ "$prepare" == "1" && "$CREATED" == "1" ]]; then
    sync_tree
    in_vm "cd '$CHECKOUT/sweetpad-lib' && bash ci/tart/env-setup.sh '$VERSION'"
  fi
}

teardown_if_ephemeral() {   # one-shot created the VM and user didn't --keep
  [[ "$CREATED" == "1" && "$KEEP" == "0" ]] || return 0
  echo "==> tearing down ephemeral VM '$VM_NAME'" >&2
  tart stop "$VM_NAME" 2>/dev/null || true
  tart delete "$VM_NAME" 2>/dev/null || true
}

case "$COMMAND" in
  up)
    ensure_up
    echo "VM '$VM_NAME' ready. Reuse with: ci/tart/env.sh {shell,test,capture} $VERSION" ;;

  shell)
    ensure_up
    echo "==> entering $VM_NAME (exit leaves it running; 'env.sh down $VERSION' to delete)" >&2
    ssh_tty "cd '$CHECKOUT/sweetpad-lib'; exec \$SHELL -l" ;;

  test)
    ensure_up; sync_tree
    set +e; in_vm "cd '$CHECKOUT/sweetpad-lib' && cargo test ${EXTRA[*]}"; rc=$?; set -e
    teardown_if_ephemeral; exit $rc ;;

  run)
    [[ ${#EXTRA[@]} -gt 0 ]] || die "run needs: -- <command>"
    ensure_up; sync_tree
    set +e; in_vm "cd '$CHECKOUT/sweetpad-lib' && ${EXTRA[*]}"; rc=$?; set -e
    teardown_if_ephemeral; exit $rc ;;

  capture)
    ensure_up; sync_tree
    args=("$VERSION"); [[ "$RUN_TEST" == "0" ]] && args+=(--no-test)
    in_vm "cd '$CHECKOUT/sweetpad-lib' && bash ci/tart/capture-runner.sh ${args[*]}"
    echo "==> pulling fixtures/ + xcspec-cache/ back" >&2
    rsync_vm -a "$SSH_USER@$IP:$CHECKOUT/sweetpad-lib/fixtures/"     "$LIB_ROOT/fixtures/"
    rsync_vm -a "$SSH_USER@$IP:$CHECKOUT/sweetpad-lib/xcspec-cache/" "$LIB_ROOT/xcspec-cache/"
    teardown_if_ephemeral
    cat >&2 <<EOF

==> capture complete for $VERSION. Next (human, DOCS.md §10.5–10.8):
  git status   # expect only real deltas, no /Users churn
  ORACLE_ONLY_VERSION=$VERSION cargo test --test per_target_oracle -- --nocapture
EOF
    ;;

  sync)
    vm_exists || die "VM '$VM_NAME' does not exist (run: env.sh up $VERSION)"
    IP="$(vm_ip)"; [[ -n "$IP" ]] || die "VM '$VM_NAME' is not running"
    sync_tree ;;

  down)
    echo "==> stopping + deleting '$VM_NAME'" >&2
    tart stop "$VM_NAME" 2>/dev/null || true
    tart delete "$VM_NAME" 2>/dev/null || true ;;

  *)
    die "unknown command '$COMMAND' (up|shell|test|run|capture|sync|down)" ;;
esac
