#!/usr/bin/env bash
# Host dependencies and the pinned Swift compiler. Also used by SDK-free CI.
set -euo pipefail

sweetpad_linux_host() {
  local release_file="${1:-/etc/os-release}" arch="${2:-$(uname -m)}"
  local ID= VERSION_ID= PRETTY_NAME=
  [ ! -r "$release_file" ] || . "$release_file"
  [[ "$arch" = aarch64 || "$arch" = x86_64 ]] || {
    echo 'error: setup requires ARM64 or x86_64 Linux' >&2; return 1;
  }
  SWEETPAD_HOST_NAME="${PRETTY_NAME:-${ID:-unknown Linux}}"
  SWEETPAD_HOST_PACKAGES=()
  SWEETPAD_PACKAGE_MANAGER=
  SWEETPAD_SWIFT_PLATFORM=ubuntu24.04
  case "$ID:$VERSION_ID" in
    ubuntu:24.04) SWEETPAD_PACKAGE_MANAGER=apt; SWEETPAD_LLVM_PACKAGE=llvm-18 ;;
    debian:12|debian:13)
      SWEETPAD_PACKAGE_MANAGER=apt
      SWEETPAD_SWIFT_PLATFORM="debian$VERSION_ID"
      SWEETPAD_LLVM_PACKAGE=llvm
      ;;
    arch:*)
      [ "$arch" = x86_64 ] || {
        echo 'error: Arch setup supports x86_64; Arch Linux ARM needs separate validation' >&2; return 1;
      }
      SWEETPAD_PACKAGE_MANAGER=pacman
      # Arch has no official Swift tarball. Use the signed Ubuntu compiler
      # with Arch's compatibility libraries, then check an actual host build.
      SWEETPAD_HOST_PACKAGES=(base-devel bash binutils bubblewrap ca-certificates cpio curl file git gnupg
        libedit libxml2-legacy libyaml llvm ncurses openssl patchelf pkgconf python rsync sqlite strace tzdata unzip xz zip zlib z3)
      ;;
    alpine:*)
      echo 'error: the CLI runs on Alpine, but automatic Apple toolchain setup requires glibc. Run setup in an Ubuntu or Debian container.' >&2
      return 1
      ;;
    *)
      echo "warning: automatic package installation is not configured for $SWEETPAD_HOST_NAME. Install the host dependencies and use --skip-system-packages, or use an Ubuntu/Debian container." >&2
      if [ "${SWEETPAD_SKIP_SYSTEM_PACKAGES:-0}" != 1 ]; then return 1; fi
      echo 'warning: trying the signed Ubuntu 24.04 Swift toolchain on this untested distribution' >&2
      ;;
  esac
  if [ "$SWEETPAD_PACKAGE_MANAGER" = apt ]; then
    SWEETPAD_HOST_PACKAGES=(bash binutils bubblewrap build-essential ca-certificates cpio curl file git gnupg
      libc6-dev libcurl4-openssl-dev libedit2 libncurses-dev libpython3-dev libsqlite3-dev libssl-dev
      libxml2-dev libyaml-dev libz3-dev "$SWEETPAD_LLVM_PACKAGE" pkg-config python3 rsync strace tzdata unzip zip xz-utils zlib1g-dev)
  fi
  [ "$arch" != aarch64 ] || SWEETPAD_SWIFT_PLATFORM+=-aarch64
}

sweetpad_install_host_tools() {
  sweetpad_linux_host
  local -a privilege=()
  if [ "${SWEETPAD_SKIP_SYSTEM_PACKAGES:-0}" != 1 ]; then
    if [ "$(id -u)" != 0 ]; then
      command -v sudo >/dev/null || { echo 'error: install sudo or use --skip-system-packages' >&2; return 1; }
      privilege=(sudo)
      if [ "${SWEETPAD_NONINTERACTIVE:-0}" = 1 ]; then privilege+=(--non-interactive); fi
    fi
    echo "Installing host packages for $SWEETPAD_HOST_NAME"
    case "$SWEETPAD_PACKAGE_MANAGER" in
      apt)
        "${privilege[@]}" env DEBIAN_FRONTEND=noninteractive apt-get update -qq
        "${privilege[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -yqq "${SWEETPAD_HOST_PACKAGES[@]}"
        ;;
      pacman)
        # Do not refresh Arch's database without a full system upgrade.
        # Users update their system with pacman -Syu before running setup.
        "${privilege[@]}" pacman -S --needed --noconfirm "${SWEETPAD_HOST_PACKAGES[@]}" || {
          echo 'error: update Arch with pacman -Syu, then rerun setup' >&2; return 1;
        }
        ;;
    esac
  fi
  mkdir -p "$HOME/bin" "$HOME/swift" "$HOME/downloads"
  local libtool=
  for candidate in /usr/bin/llvm-libtool-darwin /usr/bin/llvm-libtool-darwin-*; do
    if [ -x "$candidate" ]; then libtool="$candidate"; break; fi
  done
  [ -n "$libtool" ] || {
    echo 'error: install LLVM with llvm-libtool-darwin before running setup' >&2; return 1;
  }
  ln -sf "$libtool" "$HOME/bin/llvm-libtool-darwin"
  # Preserve an existing user-selected libtool.
  if [ ! -e "$HOME/bin/libtool" ]; then ln -sf "$libtool" "$HOME/bin/libtool"; fi

  local tag="swift-$SWIFT_VERSION-RELEASE"
  local swift_dir="$HOME/swift/$tag" downloads="$HOME/downloads"
  if [ ! -x "$swift_dir/usr/bin/swift" ]; then
    local platform="$SWEETPAD_SWIFT_PLATFORM"
    local url="https://download.swift.org/swift-$SWIFT_VERSION-release/${platform//./}/$tag/$tag-$platform.tar.gz"
    # Include the platform so caches cannot mix Ubuntu and Debian archives.
    local archive="$downloads/$tag-$platform.tar.gz"
    curl -fL --retry 3 -sS -o "$archive" "$url"
    curl -fL --retry 3 -sS -o "$archive.sig" "$url.sig"
    curl -fL --retry 3 -sS --compressed -o "$downloads/swift-keys.asc" https://www.swift.org/keys/all-keys.asc
    gpg --batch --quiet --import "$downloads/swift-keys.asc"
    gpg --batch --verify "$archive.sig" "$archive"
    local stage
    stage="$(mktemp -d "$HOME/swift/.sweetpad-toolchain.XXXXXX")"
    if ! tar -xzf "$archive" -C "$stage" --strip-components=1; then
      rm -rf "$stage"; return 1
    fi
    # A failed extraction must never leave a reusable, incomplete compiler.
    if [ -e "$swift_dir" ]; then
      rm -rf "$stage"
      echo "error: incomplete compiler at $swift_dir; move it aside before rerunning setup" >&2
      return 1
    fi
    mv "$stage" "$swift_dir"
  fi
  if [ "$SWEETPAD_PACKAGE_MANAGER" = pacman ]; then
    # Arch's swift-bin recipe makes the same ncurses dependency adjustment:
    # https://aur.archlinux.org/cgit/aur.git/tree/PKGBUILD?h=swift-bin
    # Change only this private toolchain, never libraries under /usr/lib.
    python3 - "$swift_dir/usr" <<'PY'
from pathlib import Path
import subprocess
import sys

for path in Path(sys.argv[1]).rglob("*"):
    if path.is_symlink() or not path.is_file() or not (path.stat().st_mode & 0o111 or ".so" in path.name):
        continue
    with path.open("rb") as source:
        header = source.read(18)
    # Relocatable .o files have no dynamic section and need no adjustment.
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[16:18], "little") not in (2, 3):
        continue
    needed = subprocess.run(["patchelf", "--print-needed", str(path)], check=True, capture_output=True, text=True).stdout.splitlines()
    for name in ("ncurses", "panel", "form"):
        old = f"lib{name}.so.6"
        if old in needed:
            subprocess.run(["patchelf", "--replace-needed", old, f"lib{name}w.so.6", str(path)], check=True)
PY
  fi
  export PATH="$HOME/bin:$swift_dir/usr/bin:$PATH"
  swift --version
  # Starting swift alone misses linker and Foundation dependencies.
  local probe
  probe="$(mktemp -d "$HOME/swift/.sweetpad-host-check.XXXXXX")"
  printf '%s\n' 'import Foundation' 'print("Sweetpad host toolchain: " + ProcessInfo.processInfo.operatingSystemVersionString)' > "$probe/main.swift"
  if ! swiftc "$probe/main.swift" -o "$probe/check" || ! "$probe/check"; then
    rm -rf "$probe"
    echo "error: Swift cannot compile and run on $SWEETPAD_HOST_NAME; check the host dependencies or use an Ubuntu/Debian container" >&2
    return 1
  fi
  rm -rf "$probe"
  llvm-libtool-darwin --version
}

sweetpad_install_xtool() {
  local arch="$(uname -m)" downloads="$HOME/downloads"
  if [ "$("$HOME/bin/xtool" --version 2>/dev/null)" != "xtool $XTOOL_VERSION" ]; then
    local image="$downloads/xtool-$XTOOL_VERSION-$arch.AppImage"
    curl -fL -sS -o "$image" \
      "https://github.com/xtool-org/xtool/releases/download/$XTOOL_VERSION/xtool-$arch.AppImage"
    # The manifest pins a separate AppImage checksum for each host architecture.
    echo "$XTOOL_SHA256  $image" | sha256sum -c -
    chmod +x "$image"
    # Extracting sidesteps FUSE, which an OrbStack machine doesn't have.
    rm -rf "$downloads/squashfs-root" "$HOME/xtool"
    (cd "$downloads" && "$image" --appimage-extract >/dev/null)
    mv "$downloads/squashfs-root" "$HOME/xtool"
    ln -sf "$HOME/xtool/AppRun" "$HOME/bin/xtool"
  fi
  xtool --version

}

if [[ "${BASH_SOURCE[0]}" = "$0" ]]; then
  SWIFT_VERSION="${SWIFT_VERSION:-$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["swift"])' "$(dirname "$0")/linux-toolchain.json")}"
  read -r XTOOL_VERSION XTOOL_SHA256 < <(python3 -c 'import json,sys; p=json.load(open(sys.argv[1])); print(p["xtool"],p["xtoolSha256"][sys.argv[2]])' "$(dirname "$0")/linux-toolchain.json" "$(uname -m)")
  sweetpad_install_host_tools
  sweetpad_install_xtool
fi
