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
#              ship, built from swiftlang/swift-build at the toolchain's tag and
#              linked into ~/bin
#   Rust       rustup, for building sweetpad itself
#
# The SDK comes from your own Xcode (XCODE_APP, default /Applications/Xcode.app,
# visible inside OrbStack at the same path). Apple's SDK license ties the SDK to
# Apple-branded hardware; this script never downloads or redistributes it.
#
# Knobs (env): SWEETPAD_VM (OrbStack machine name, default sweetpad-linux),
# XCODE_APP, SWIFT_VERSION, XTOOL_VERSION, XTOOL_SHA256.
set -euo pipefail

VM="${SWEETPAD_VM:-sweetpad-linux}"
XCODE_APP="${XCODE_APP:-/Applications/Xcode.app}"
SWIFT_VERSION="${SWIFT_VERSION:-6.4.0}"
XTOOL_VERSION="${XTOOL_VERSION:-1.21.0}"
XTOOL_SHA256="${XTOOL_SHA256:-470d55880379de9696a048f4293a0b05a7fc43beba423ffe6c294a4975dcebf7}"

if [ "$(uname -s)" = Darwin ]; then
  command -v orb >/dev/null || { echo "error: OrbStack's 'orb' is not installed" >&2; exit 1; }
  orb status >/dev/null 2>&1 || orb start
  orb list -q | grep -qx "$VM" || orb create ubuntu:noble "$VM"
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  exec orb -m "$VM" env XCODE_APP="$XCODE_APP" SWIFT_VERSION="$SWIFT_VERSION" \
    XTOOL_VERSION="$XTOOL_VERSION" XTOOL_SHA256="$XTOOL_SHA256" bash "$self"
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
  binutils build-essential ca-certificates cpio curl file git gnupg2 libc6-dev \
  libcurl4-openssl-dev libedit2 libgcc-13-dev libncurses-dev libpython3-dev \
  libsqlite3-dev libssl-dev libstdc++-13-dev libxml2-dev libz3-dev pkg-config \
  python3 tzdata unzip xz-utils zlib1g-dev >/dev/null

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
if [ ! -f "$HOME/.swiftpm/swift-sdks/darwin.artifactbundle/toolset-swb.json" ]; then
  [ -d "$XCODE_APP" ] || { echo "error: $XCODE_APP is not visible here" >&2; exit 1; }
  # --slim copies just the files the SDK needs; without it an unpacked
  # Xcode.app is copied whole and then hard-linked.
  xtool sdk install --slim "$XCODE_APP"
fi
# xtool leaves out the version files, which an app's Info.plist takes
# DTXcode, DTXcodeBuild and DTPlatformBuild from. Xcode's own goes beside the
# bundle's Developer folder, where Xcode.app keeps it, for sweetpad to read;
# each platform's goes in the platform, where Swift Build reads it.
BUNDLE="$HOME/.swiftpm/swift-sdks/darwin.artifactbundle"
if [ ! -f "$BUNDLE/version.plist" ] && [ -f "$XCODE_APP/Contents/version.plist" ]; then
  cp "$XCODE_APP/Contents/version.plist" "$BUNDLE/version.plist"
  for platform in "$BUNDLE"/Developer/Platforms/*.platform; do
    source="$XCODE_APP/Contents/Developer/Platforms/$(basename "$platform")/version.plist"
    [ -f "$source" ] && cp "$source" "$platform/version.plist"
  done
fi
swift sdk list

section "swbuild ($TAG)"
SWB="$HOME/src/swift-build"
if [ ! -x "$SWB/.build/release/swbuild" ] || [ ! -x "$SWB/.build/release/SWBBuildServiceBundle" ]; then
  [ -d "$SWB/.git" ] || git clone -q --depth 1 --branch "$TAG" https://github.com/swiftlang/swift-build.git "$SWB"
  (cd "$SWB" && swift build -c release --product swbuild && swift build -c release --product SWBBuildServiceBundle)
fi
# sweetpad looks swbuild up on PATH; it finds its service bundle through the
# link.
ln -sf "$SWB/.build/release/swbuild" "$HOME/bin/swbuild"
ls "$SWB/.build/release/swbuild" "$SWB/.build/release/SWBBuildServiceBundle"

section "Rust"
if [ ! -x "$HOME/.cargo/bin/cargo" ]; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi
"$HOME/.cargo/bin/cargo" --version

echo
echo "ready: ci/linux-build.sh builds an Xcode project here"
