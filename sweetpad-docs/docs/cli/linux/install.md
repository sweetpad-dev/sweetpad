---
sidebar_label: Install the CLI
---

# Install SweetPad on Linux

The CLI is a static executable for x64 or ARM64 Linux, with Swift Build, the macro plugin and an LLVM linker for Apple platforms in a `libexec/sweetpad` folder beside it.
It doesn't include Apple's SDKs or the Swift toolchain. [Set up the Apple toolchain](sdk-transfer.md) after installing it.

| Method | Use it for |
| --- | --- |
| [APT](#apt) | Ubuntu and Debian, with normal package updates. |
| [Debian package](#debian-package) | A pinned release on Ubuntu or Debian. |
| [Packslip or Mise](#packslip-or-mise) | User installs and per-project version pins. |
| [Container image](#container-image) | Copying the CLI into your own build image. |
| [Tarball](#tarball) | Any distribution, without root access. |

## APT

Download the repository key and check its fingerprint:

```bash
sudo apt-get install ca-certificates curl gnupg
curl -fsSLo /tmp/sweetpad-archive-keyring.asc \
  https://sweetpad-dev.github.io/sweetpad/apt/sweetpad-archive-keyring.asc
gpg --show-keys --with-fingerprint /tmp/sweetpad-archive-keyring.asc
```

The fingerprint must be:

```text
BD4A 4E59 600E EECC 7D9A  9DA7 BA44 850D FB32 ACC9
```

Then add the repository and install:

```bash
sudo install -Dm644 /tmp/sweetpad-archive-keyring.asc \
  /etc/apt/keyrings/sweetpad-archive-keyring.asc
sudo tee /etc/apt/sources.list.d/sweetpad.sources >/dev/null <<'EOF'
Types: deb
URIs: https://sweetpad-dev.github.io/sweetpad/apt
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/sweetpad-archive-keyring.asc
EOF
sudo apt-get update
sudo apt-get install sweetpad
```

`sudo apt-get install --only-upgrade sweetpad` upgrades it, and `sudo apt-get remove sweetpad` removes it.
Removing it keeps your toolchain, SDK, signing files, and configuration.

## Debian package

```bash
CLI_VERSION=0.1.26
CLI_ARCH="$(dpkg --print-architecture)"
BASE="https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION"
curl -fLO "$BASE/sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb"
curl -fLO "$BASE/sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb.sha256"
sha256sum -c "sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb.sha256"
sudo apt-get install "./sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb"
```

To upgrade, install the next release's package the same way.

## Packslip or Mise

With [Packslip](https://packslip.dev/docs/bootstrap/), which checks the release's signature before installing:

```bash
packslip install github.com/sweetpad-dev/sweetpad/tools/cli
```

Run the same command to upgrade.

With a [Mise that supports Packslip](https://mise.jdx.dev/dev-tools/backends/packslip.html):

```bash
mise use -g packslip:github.com/sweetpad-dev/sweetpad/tools/cli
```

Drop `-g` and add `@0.1.26` to pin a version for one project, and use `mise upgrade` to upgrade.

## Container image

The image at `ghcr.io/sweetpad-dev/sweetpad` holds the CLI and its Swift helpers, for `linux/amd64` and `linux/arm64`.
Copy both into your own build image, which supplies the Apple toolchain:

```dockerfile
FROM ghcr.io/sweetpad-dev/sweetpad:0.1.26 AS sweetpad
FROM ubuntu:24.04
COPY --from=sweetpad /usr/local/bin/sweetpad /usr/local/bin/sweetpad
COPY --from=sweetpad /usr/local/libexec/sweetpad /usr/local/libexec/sweetpad
RUN ln -s sweetpad /usr/local/bin/sweetpad-dap
RUN apt-get update && apt-get install -y ca-certificates python3 bash libsqlite3-0
```

Run `sweetpad setup` in that image, and bring the SDK and signing files in only when the job runs.

## Tarball

```bash
CLI_VERSION=0.1.26
case "$(uname -m)" in
  aarch64|arm64) CLI_ARCH=arm64 ;;
  x86_64|amd64) CLI_ARCH=x64 ;;
esac
BASE="https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION"
curl -fLO "$BASE/sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz"
curl -fLO "$BASE/sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz.sha256"
sha256sum -c "sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz.sha256"
mkdir sweetpad-cli
tar -xzf "sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz" -C sweetpad-cli
install -Dm755 sweetpad-cli/sweetpad "$HOME/.local/bin/sweetpad"
ln -sfn sweetpad "$HOME/.local/bin/sweetpad-dap"
rm -rf "$HOME/.local/libexec/sweetpad"
mkdir -p "$HOME/.local/libexec"
cp -R sweetpad-cli/libexec/sweetpad "$HOME/.local/libexec/sweetpad"
```

Make sure `~/.local/bin` is on your `PATH`.
SweetPad finds its helpers in `../libexec/sweetpad` next to the `bin` folder, so keep the two together.
Swift Build needs the host's SQLite library: install `libsqlite3-0` on Ubuntu and Debian, or `sqlite` on Arch.
To upgrade, install the next release's tarball the same way.

## Upgrade

Swift Build, the macro plugin and the linker come with the package, so upgrading the CLI upgrades them too.
Keep the tarball's `libexec` folder next to the `sweetpad` binary.
If a release changes the SDK import recipe or the toolchain, run setup again with the source you used the first time, and add `--replace-sdk` if it asks:

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz"
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios
```

Setup in older releases compiled Swift Build into `~/src/swift-build*` and linked it from `~/bin`.
Rerun setup once to drop those from the environment file, then delete the folders.
