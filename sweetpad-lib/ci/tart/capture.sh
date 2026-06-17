#!/usr/bin/env bash
# capture.sh — back-compat shim. Capture is now one mode of the unified driver.
#   ci/tart/capture.sh <version> [--keep] [--no-test] [--image …] [--name …]
# is equivalent to:
#   ci/tart/env.sh capture <version> [...]
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$HERE/env.sh" capture "$@"
