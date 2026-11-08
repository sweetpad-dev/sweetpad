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

# Use Flutter's public local-engine interface so execution audits can inspect
# the actual ELF path without depending on a private mount namespace.
engine="${PROJECT_TEMP_DIR:?Set PROJECT_TEMP_DIR}/SweetpadFlutterEngine"
if [[ ! -f "$engine/settings.json" ]]; then
  python3 "$SRCROOT/SweetpadFlutterLocalEngine.py" --flutter-root "$FLUTTER_ROOT" \
    --compiler "$compiler" --out "$engine" >/dev/null
fi
python3 - "$engine" "$compiler" "$FLUTTER_ROOT" <<'PYCODE'
import json,sys
from pathlib import Path
engine,compiler,flutter=map(Path,sys.argv[1:])
settings=json.loads((engine/"settings.json").read_text())
if (engine/"out/ios_release/gen_snapshot_arm64").resolve() != compiler.resolve() or Path(settings["FLUTTER_ROOT"]).resolve() != flutter.resolve():
    raise SystemExit("Stale Flutter local engine; clear this fixture's DerivedData")
PYCODE
export FLUTTER_ENGINE="$engine"
export LOCAL_ENGINE=ios_release
export LOCAL_ENGINE_HOST=host_release
exec /bin/sh "$backend" "$@"
