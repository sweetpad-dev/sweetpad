#!/usr/bin/env bash
# Keep Flutter's iOS snapshot tool inside Linux. OrbStack's binfmt bridge can
# otherwise run the Mach-O compiler silently. The SDK installation stays intact.
set -euo pipefail

backend="$FLUTTER_ROOT/packages/flutter_tools/bin/xcode_backend.sh"
if [[ "$(uname -s)" != Linux ]]; then
  exec /bin/sh "$backend" "$@"
fi
mode="${FLUTTER_BUILD_MODE:-${CONFIGURATION:-}}"
if [[ "${mode,,}" != release || "${PLATFORM_NAME:-}" != iphoneos || "${ARCHS:-}" != arm64 ]]; then
  echo "This fixture's Linux AOT compiler supports Release iOS ARM64 device builds only." >&2
  exit 1
fi

compiler="${SWEETPAD_FLUTTER_IOS_GEN_SNAPSHOT:?Set an explicit Linux-hosted iOS gen_snapshot}"
python3 - "$compiler" <<'PY'
import sys
from pathlib import Path
with Path(sys.argv[1]).open('rb') as compiler:
    header = compiler.read(20)
if header[:6] != b'\x7fELF\x02\x01' or int.from_bytes(header[18:20], 'little') != 183:
    raise SystemExit('The iOS snapshot compiler must be an ARM64 Linux ELF executable')
PY

exec bwrap --bind / / --dev-bind /dev /dev --proc /proc \
  --ro-bind "$compiler" "$FLUTTER_ROOT/bin/cache/artifacts/engine/ios-release/gen_snapshot_arm64" \
  -- /bin/sh "$backend" "$@"
