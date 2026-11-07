#!/usr/bin/env bash
#
# Provision a Linux machine that builds Xcode projects without Xcode, for
# ci/linux-build.sh. Run it on the Mac and it creates (or reuses) an OrbStack
# machine and provisions it; run it on a Linux host and it provisions that host.
# Every step is skipped when its result already exists, so a rerun is cheap.
#
#   Swift      the release toolchain, PGP-checked against swift.org's keys
#   xtool      its Darwin Swift SDK: the iOS and macOS SDKs plus an ld64.lld
#              that links for Apple platforms, extracted from an Xcode.app
#   swbuild    Swift Build's command-line driver, which the toolchain doesn't
#              ship, built from the separately pinned swiftlang/swift-build tag,
#              with swift-build-linux.patch, and linked into ~/bin
#   sweetpad-macros
#              the plugin for Apple's macros that Linux has no working build
#              of (SwiftData's, #Predicate, #Preview), built from
#              sweetpad-cli/macros into ~/bin
#   Rust       rustup, for building sweetpad itself
#
# The SDK comes from your own Xcode (XCODE_APP, default /Applications/Xcode.app,
# visible inside OrbStack at the same path). Apple's SDK license ties the SDK to
# Apple-branded hardware; this script never downloads or redistributes it.
#
# Knobs (env): SWEETPAD_VM (OrbStack machine name, default sweetpad-linux),
# XCODE_APP, SWIFT_VERSION, SWIFT_BUILD_VERSION, XTOOL_VERSION, XTOOL_SHA256,
# SWEETPAD_REFRESH_SDK=1 (replace a stale slim SDK, with rollback on error).
set -euo pipefail

VM="${SWEETPAD_VM:-sweetpad-linux}"
XCODE_APP="${XCODE_APP:-/Applications/Xcode.app}"
SWIFT_VERSION="${SWIFT_VERSION:-6.4.2}"
SWIFT_BUILD_VERSION="${SWIFT_BUILD_VERSION:-6.4.0}"
XTOOL_VERSION="${XTOOL_VERSION:-1.21.0}"
XTOOL_SHA256="${XTOOL_SHA256:-470d55880379de9696a048f4293a0b05a7fc43beba423ffe6c294a4975dcebf7}"

if [ "$(uname -s)" = Darwin ]; then
  command -v orb >/dev/null || { echo "error: OrbStack's 'orb' is not installed" >&2; exit 1; }
  orb status >/dev/null 2>&1 || orb start
  orb list -q | grep -qx "$VM" || orb create ubuntu:noble "$VM"
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  exec orb -m "$VM" env XCODE_APP="$XCODE_APP" SWIFT_VERSION="$SWIFT_VERSION" \
    SWIFT_BUILD_VERSION="$SWIFT_BUILD_VERSION" \
    XTOOL_VERSION="$XTOOL_VERSION" XTOOL_SHA256="$XTOOL_SHA256" \
    SWEETPAD_REFRESH_SDK="${SWEETPAD_REFRESH_SDK:-0}" bash "$self"
fi

ARCH="$(uname -m)"
TAG="swift-$SWIFT_VERSION-RELEASE"
SWIFT_DIR="$HOME/swift/$TAG"
DOWNLOADS="$HOME/downloads"
mkdir -p "$DOWNLOADS" "$HOME/swift" "$HOME/src" "$HOME/bin"

section() { echo; echo "==== $* ===="; }

section "apt packages"
sudo DEBIAN_FRONTEND=noninteractive apt-get update -qq
sudo DEBIAN_FRONTEND=noninteractive apt-get install -yqq \
  bash binutils bubblewrap build-essential ca-certificates cpio curl file git gnupg2 libc6-dev \
  libcurl4-openssl-dev libedit2 libgcc-13-dev libncurses-dev libpython3-dev \
  libsqlite3-dev libssl-dev libstdc++-13-dev libxml2-dev libz3-dev llvm-18 pkg-config \
  python3 rsync tzdata unzip zip xz-utils zlib1g-dev >/dev/null
# Swift Build's Darwin archive tasks use Apple's libtool command syntax.
# LLVM supplies the portable implementation; GNU libtool is a different tool.
if [ ! -e "$HOME/bin/libtool" ]; then
  ln -sf /usr/bin/llvm-libtool-darwin-18 "$HOME/bin/libtool"
fi
ln -sf /usr/bin/llvm-libtool-darwin-18 "$HOME/bin/llvm-libtool-darwin"

section "Swift $SWIFT_VERSION"
if [ ! -x "$SWIFT_DIR/usr/bin/swift" ]; then
  # Ubuntu 24.04 is the only distro this pins; the URL is swift.org's layout.
  platform="ubuntu24.04"
  [ "$ARCH" = aarch64 ] && platform="$platform-aarch64"
  url="https://download.swift.org/swift-$SWIFT_VERSION-release/${platform//./}/$TAG/$TAG-$platform.tar.gz"
  curl -fL --retry 3 -sS -o "$DOWNLOADS/$TAG.tar.gz" "$url"
  curl -fL -sS -o "$DOWNLOADS/$TAG.tar.gz.sig" "$url.sig"
  # swift.org serves the key file gzip-encoded.
  curl -fL -sS --compressed -o "$DOWNLOADS/swift-keys.asc" https://www.swift.org/keys/all-keys.asc
  gpg --batch --quiet --import "$DOWNLOADS/swift-keys.asc"
  gpg --batch --verify "$DOWNLOADS/$TAG.tar.gz.sig" "$DOWNLOADS/$TAG.tar.gz"
  mkdir -p "$SWIFT_DIR"
  tar -xzf "$DOWNLOADS/$TAG.tar.gz" -C "$SWIFT_DIR" --strip-components=1
fi
"$SWIFT_DIR/usr/bin/swift" --version
export PATH="$HOME/bin:$SWIFT_DIR/usr/bin:$PATH"

section "xtool $XTOOL_VERSION"
if [ "$("$HOME/bin/xtool" --version 2>/dev/null)" != "xtool $XTOOL_VERSION" ]; then
  image="$DOWNLOADS/xtool-$XTOOL_VERSION-$ARCH.AppImage"
  curl -fL -sS -o "$image" \
    "https://github.com/xtool-org/xtool/releases/download/$XTOOL_VERSION/xtool-$ARCH.AppImage"
  # The default digest is the aarch64 AppImage's; another arch passes its own.
  echo "$XTOOL_SHA256  $image" | sha256sum -c -
  chmod +x "$image"
  # Extracting sidesteps FUSE, which an OrbStack machine doesn't have.
  rm -rf "$DOWNLOADS/squashfs-root" "$HOME/xtool"
  (cd "$DOWNLOADS" && "$image" --appimage-extract >/dev/null)
  mv "$DOWNLOADS/squashfs-root" "$HOME/xtool"
  ln -sf "$HOME/xtool/AppRun" "$HOME/bin/xtool"
fi
xtool --version

section "Darwin SDK from $XCODE_APP"
BUNDLE="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle"
if [ "${SWEETPAD_REFRESH_SDK:-0}" = 1 ] && [ -d "$BUNDLE" ]; then
  backup="$(mktemp -d "$DOWNLOADS/sweetpad-sdk-backup.XXXXXX")"
  mv "$BUNDLE" "$backup/bundle"
  if xtool sdk install --slim "$XCODE_APP"; then
    rm -rf "$backup"
  else
    rm -rf "$BUNDLE"
    mv "$backup/bundle" "$BUNDLE"
    rmdir "$backup"
    echo "error: SDK refresh failed; restored the previous SDK" >&2
    exit 1
  fi
elif [ ! -f "$BUNDLE/toolset-swb.json" ]; then
  [ -d "$XCODE_APP" ] || { echo "error: $XCODE_APP is not visible here" >&2; exit 1; }
  # --slim copies just the files the SDK needs; without it an unpacked
  # Xcode.app is copied whole and then hard-linked.
  xtool sdk install --slim "$XCODE_APP"
fi
# An Xcode update does not update an existing slim SDK. Compare the SDKs
# actually installed, rather than the Xcode.app directory's name.
python3 - "$XCODE_APP/Contents" "$BUNDLE" <<'PY'
from pathlib import Path
import plistlib
import sys

def versions(root):
    found = {}
    for path in Path(root).glob("Developer/Platforms/*.platform/Developer/SDKs/*.sdk/SDKSettings.plist"):
        with path.open("rb") as source:
            name = plistlib.load(source).get("CanonicalName")
        found.setdefault(path.parents[3].name, set()).add(name)
    return found

source, installed = map(versions, sys.argv[1:])
for platform, sdk in installed.items():
    if source.get(platform) != sdk:
        raise SystemExit(f"error: stale extracted SDK for {platform}: {sorted(sdk)}; Xcode has {sorted(source.get(platform, set()))}. Rerun with SWEETPAD_REFRESH_SDK=1.")
PY
# xtool's slim SDK includes iOS and macOS. Add the other platforms present
# in this Xcode so embedded watch products and their SDKs resolve as well.
python3 "$(dirname "$0")/linux-sdk-platforms.py" "$XCODE_APP" "$BUNDLE"
# xtool leaves out the version files, which an app's Info.plist takes
# DTXcode, DTXcodeBuild and DTPlatformBuild from. Xcode's own goes beside the
# bundle's Developer folder, where Xcode.app keeps it, for sweetpad to read;
# each platform's goes in the platform, where Swift Build reads it.
if [ -f "$XCODE_APP/Contents/version.plist" ]; then
  cat "$XCODE_APP/Contents/version.plist" > "$BUNDLE/version.plist"
fi
for platform in "$BUNDLE"/Developer/Platforms/*.platform; do
  source="$XCODE_APP/Contents/Developer/Platforms/$(basename "$platform")/version.plist"
  [ ! -f "$source" ] || cat "$source" > "$platform/version.plist"
done
# Slim SDK extraction omits the executable Swift Build copies into Messages
# and sticker extensions.
for platform in "$BUNDLE"/Developer/Platforms/*.platform; do
  source="$XCODE_APP/Contents/Developer/Platforms/$(basename "$platform")/Library/Application Support/MessagesApplicationExtensionStub"
  destination="$platform/Library/Application Support/MessagesApplicationExtensionStub"
  if [ -d "$source" ] && ! cmp -s "$source/MessagesApplicationExtensionStub" "$destination/MessagesApplicationExtensionStub"; then
    mkdir -p "$destination"
    # Stream the copy: optimized sparse copies from OrbStack's shared Mac
    # mount can produce a correctly sized file containing only zeroes.
    tar -C "$source" -cf - . | tar -C "$destination" -xf -
  fi
done
swift sdk list

# Cross-compilation uses the extracted toolchain's resource directory. The
# Swift/C++ interoperability headers are beside it, outside the platform SDK.
source="$XCODE_APP/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift"
destination="$BUNDLE/Developer/Toolchains/XcodeDefault.xctoolchain/usr/include/swift"
if [ -d "$source" ]; then
  mkdir -p "$destination"
  tar -C "$source" -cf - . | tar -C "$destination" -xf -
fi

SWB_TAG="swift-$SWIFT_BUILD_VERSION-RELEASE"
section "swbuild ($SWB_TAG)"
SWB="$HOME/src/swift-build"
[ -d "$SWB/.git" ] || git clone -q --depth 1 --branch "$SWB_TAG" https://github.com/swiftlang/swift-build.git "$SWB"
if [ "$(git -C "$SWB" rev-parse HEAD)" != "$(git -C "$SWB" rev-parse "$SWB_TAG^{commit}")" ]; then
  echo "error: $SWB must use $SWB_TAG; use a separate cache for another revision" >&2
  exit 1
fi
# swift-build-linux.patch ports host-specific build tasks. A macOS app's build registers
# it with Launch Services through /System/…/lsregister, and Swift Build fails
# the build when that can't start, where it only notes a run that fails; the
# patch notes both. Swift Build's console asks no one for a signing identity
# or provisioning profile, so it signs only ad hoc; the patch has it ask the
# tool SWEETPAD_PROVISIONING names, which sweetpad sets for a device build.
# It runs /usr/bin/derq by absolute path for a signed target's DER
# entitlements; the patch has it look derq up on the search path first.
# Its Mach-O reader takes the file's structures from the Darwin SDK and
# fails every read elsewhere; the patch defines them in a new file,
# SWBUtil/MachOStructures.swift, and names the architectures itself. And it
# finds a text encoding by name only through CoreFoundation, so a Mac app's
# .strings can't be written as UTF-16; the patch names the encodings
# Foundation converts on every platform and detects Unicode BOMs. LLVM
# libtool version discovery avoids treating its version as a cctools version.
# A checkout holding an older version of the patch is reset before this one
# goes on. The new file is marked intent-to-add, so `git diff` in the
# checkout writes the whole patch back; `git apply --intent-to-add` would
# drop every other file from the index.
PATCH="$(cd "$(dirname "$0")" && pwd)/swift-build-linux.patch"
if ! git -C "$SWB" apply --reverse --check "$PATCH" 2>/dev/null; then
  git -C "$SWB" reset -q --hard
  git -C "$SWB" clean -fdq -- Sources
  git -C "$SWB" apply "$PATCH"
  git -C "$SWB" add --intent-to-add -- $(git -C "$SWB" ls-files --others --exclude-standard -- Sources)
  rm -f "$SWB/.build/release/swbuild"
fi
if [ ! -x "$SWB/.build/release/swbuild" ] || [ ! -x "$SWB/.build/release/SWBBuildServiceBundle" ]; then
  (cd "$SWB" && swift build -c release --product swbuild && swift build -c release --product SWBBuildServiceBundle)
fi
# sweetpad looks swbuild up on PATH; it finds its service bundle through the
# link.
ln -sf "$SWB/.build/release/swbuild" "$HOME/bin/swbuild"
ls "$SWB/.build/release/swbuild" "$SWB/.build/release/SWBBuildServiceBundle"

section "sweetpad-macros"
# Built outside the checkout, which the Mac shares, so the two builds keep
# apart.
MACROS="$(cd "$(dirname "$0")/../../sweetpad-cli/macros" && pwd)"
swift build -c release --package-path "$MACROS" --scratch-path "$HOME/src/sweetpad-macros" --product sweetpad-macros
ln -sf "$HOME/src/sweetpad-macros/release/sweetpad-macros" "$HOME/bin/sweetpad-macros"
ls "$HOME/bin/sweetpad-macros"

section "Rust"
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
"$HOME/.cargo/bin/cargo" --version

echo
echo "ready: ci/linux-build.sh builds an Xcode project here"
