#!/usr/bin/env bash
# Swift starts compiler plugins with a minimal PATH and no LD_LIBRARY_PATH.
# The adjacent link records the host toolchain selected by the setup script.
set -euo pipefail
launcher_dir="$(cd "$(dirname "$0")" && pwd)"
swift_binary="$(readlink -f "$launcher_dir/sweetpad-macro-swift")"
runtime="$(dirname "$swift_binary")/../lib/swift/linux"
export LD_LIBRARY_PATH="$runtime${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$launcher_dir/sweetpad-macros.bin" "$@"
