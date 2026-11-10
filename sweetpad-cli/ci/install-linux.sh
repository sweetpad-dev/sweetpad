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
[[ $(uname -s) == Linux ]] || { echo 'This package requires Linux.' >&2; exit 1; }
source_dir="$(cd "$(dirname "$0")" && pwd)"
python3 - "$source_dir/manifest.json" "$(uname -m)" <<'PYTHON'
import json,sys
expected = {"aarch64": "linux-arm64", "x86_64": "linux-x64"}.get(sys.argv[2])
with open(sys.argv[1]) as source:
    actual = json.load(source)["platform"]
if actual != expected:
    raise SystemExit(f"Package architecture {actual} does not match host {sys.argv[2]}")
PYTHON
mkdir -p "$prefix/bin" "$prefix/share/sweetpad"
stage="$(mktemp -d "$prefix/share/sweetpad/.install.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
cp -R "$source_dir/setup" "$source_dir/licenses" "$source_dir/examples" "$stage/"
install -m 755 "$source_dir/sweetpad" "$prefix/bin/sweetpad"
ln -sfn sweetpad "$prefix/bin/sweetpad-dap"
for file in README.md LINUX_DEVICES.md LICENSE.md manifest.json; do
  install -m 644 "$source_dir/$file" "$prefix/share/sweetpad/$file"
done
for directory in setup licenses examples; do
  rm -rf "$prefix/share/sweetpad/$directory"
  mv "$stage/$directory" "$prefix/share/sweetpad/$directory"
done
printf 'Installed %s\n' "$prefix/bin/sweetpad"
printf 'Toolchain setup: %q setup --sdk /path/to/apple-sdk.tar.gz\n' "$prefix/bin/sweetpad"
printf 'Add the CLI to PATH: export PATH=%q:"$PATH"\n' "$prefix/bin"
printf 'Remove the CLI: rm -f %q %q; rm -rf %q\n' "$prefix/bin/sweetpad" "$prefix/bin/sweetpad-dap" "$prefix/share/sweetpad"
printf 'The SDK, compiler, signing files, and user configuration are installed separately.\n'
