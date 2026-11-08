#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} == --help ]]; then
  echo 'Usage: bash install.sh [--prefix /absolute/path] (default: ~/.local)'
  exit 0
fi
prefix="${HOME:?}/.local"
if [[ $# != 0 ]]; then
  [[ $# == 2 && $1 == --prefix ]] || { echo 'Use --help for installation options.' >&2; exit 2; }
  prefix="$2"
fi
[[ "$prefix" == /* ]] || { echo 'The installation prefix must be absolute.' >&2; exit 2; }
[[ $(uname -s) == Linux && $(uname -m) == aarch64 ]] || { echo 'This package requires ARM64 Linux.' >&2; exit 1; }
source_dir="$(cd "$(dirname "$0")" && pwd)"
mkdir -p "$prefix/bin" "$prefix/share/sweetpad"
install -m 755 "$source_dir/sweetpad" "$prefix/bin/sweetpad"
ln -sfn sweetpad "$prefix/bin/sweetpad-dap"
for file in README.md LINUX_DEVICES.md LICENSE.md manifest.json; do
  install -m 644 "$source_dir/$file" "$prefix/share/sweetpad/$file"
done
cp -R "$source_dir/setup" "$source_dir/licenses" "$source_dir/examples" "$prefix/share/sweetpad/"
printf 'Installed %s\n' "$prefix/bin/sweetpad"
printf 'Toolchain setup: SWEETPAD_SKIP_RUST=1 XCODE_APP=/path/to/Xcode.app bash %q\n' "$prefix/share/sweetpad/setup/sweetpad-lib/ci/linux-setup.sh"
