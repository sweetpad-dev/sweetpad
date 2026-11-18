#!/bin/bash
# Run the private Linux release package check from macOS.
set -euo pipefail

usage() {
    echo "Usage: $0 [--check] [path/to/sweetpad-cli-VERSION-linux-arm64.tar.gz]"
    echo "--check verifies prerequisites without building the sample."
    echo "Defaults to the ARM64 archive in dist/ or target/arm64-release-check/verified/dist/."
    echo "Set SWEETPAD_LINUX_VM to select an OrbStack machine (default: sweetpad-linux)."
}

if [[ ${1:-} == --help || ${1:-} == -h ]]; then usage; exit 0; fi
mode=build
if [[ ${1:-} == --check ]]; then mode=check; shift; fi
if (( $# > 1 )); then usage >&2; exit 2; fi
fail() { echo "$*" >&2; exit 1; }
[[ $(uname -s) == Darwin ]] || fail "Run this launcher on your Mac."
command -v orb >/dev/null || fail "Start OrbStack and make its orb command available on PATH."

root=$(cd "$(dirname "$0")/../.." && pwd -P)
vm=${SWEETPAD_LINUX_VM:-sweetpad-linux}
if (( $# == 1 )); then
    archive=$1
else
    shopt -s nullglob
    for dir in "$root/dist" "$root/target/arm64-release-check/verified/dist"; do
        archives=("$dir"/sweetpad-cli-*-linux-arm64.tar.gz)
        if (( ${#archives[@]} > 1 )); then fail "Multiple archives in $dir. Pass the archive to check."; fi
        if (( ${#archives[@]} == 1 )); then archive=${archives[0]}; break; fi
    done
    [[ -n ${archive:-} ]] || fail "Pass a Linux release tarball with its .sha256 file, or place both in $root/dist/."
fi
[[ -f "$archive" ]] || fail "Missing archive: $archive"
archive=$(cd "$(dirname "$archive")" && pwd -P)/$(basename "$archive")
[[ -f "$archive.sha256" ]] || fail "Missing release checksum: $archive.sha256"
echo "Checking $archive"
expected=$(awk 'NR == 1 {print $1}' "$archive.sha256")
actual=$(shasum -a 256 < "$archive" | awk '{print $1}')
[[ $actual == "$expected" ]] || fail "SHA-256 mismatch for $archive"

swift_version=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["swift"])' "$root/sweetpad-lib/ci/linux-toolchain.json")
orb run -m "$vm" bash -s -- "/mnt/mac$root" "/mnt/mac$archive" "$swift_version" "$vm" "$mode" <<'LINUX'
set -euo pipefail
checker=$1/sweetpad-cli/ci/check-linux-sdk-install.py
swift=$HOME/swift/swift-$3-RELEASE
sdk=$HOME/.swiftpm/swift-sdks/darwin.artifactbundle
[[ -x "$swift/usr/bin/swiftc" ]] || { echo "Install Swift $3 in the VM at $swift." >&2; exit 1; }
[[ -f "$sdk/swift-sdk.json" ]] || { echo "Import your SDK in the VM with sweetpad sdk import. Expected $sdk." >&2; exit 1; }
command -v bwrap >/dev/null || { echo "Install bubblewrap in the VM." >&2; exit 1; }

[[ -r "$checker" && -r "$2" ]] || { echo "The VM cannot read the checkout or tarball through /mnt/mac." >&2; exit 1; }
command -v file >/dev/null || { echo "Install the file utility in the VM." >&2; exit 1; }
python3 "$checker" --help >/dev/null
python3 - "$2" <<'PYTHON'
import json
import platform
import sys
import tarfile

if not hasattr(tarfile, "data_filter"):
    sys.exit("The VM needs Python tarfile extraction filters. Use the current Ubuntu 24.04 Python package.")
with tarfile.open(sys.argv[1]) as archive:
    manifest = json.load(archive.extractfile("manifest.json"))
    arch = {"aarch64": "linux-arm64", "x86_64": "linux-x64"}.get(platform.machine())
    if not arch or manifest["platform"] != arch:
        sys.exit(f"Archive platform {manifest['platform']} does not match {platform.machine()}.")
    for program in ("swbuild", "sweetpad-macros", "llvm"):
        archive.getmember(f"libexec/sweetpad/{program}")
    print(f"Package: {manifest['packageVersion']} ({manifest['binaryVersion']}, {arch})")
PYTHON
"$swift/usr/bin/swiftc" --version
bwrap --unshare-net --unshare-pid --unshare-ipc --unshare-uts --ro-bind / / -- /bin/true
if [[ $5 == check ]]; then
    echo "Ready. Run the same command without --check to build the sample."
    exit 0
fi

mkdir -p "$HOME/.cache/sweetpad/p01"
run=$(mktemp -d "$HOME/.cache/sweetpad/p01/run.XXXXXXXX")
out=$run/check
echo "Results on your Mac: ~/OrbStack/$4$out"
python3 "$checker" --archive "$2" --swift "$swift" --sdk "$sdk" --out "$out"
# Keep the report, logs, and built app. The original tarball stays on the Mac.
rm -rf -- "$out/unpacked"
LINUX
