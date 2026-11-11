---
sidebar_label: Install the CLI
---

# Install Sweetpad on Linux

Choose a package for your Linux host: x64 or ARM64.
Both hosts can build ARM64 iPhone apps after [Apple toolchain setup](sdk-transfer.md).
The CLI is a static executable with embedded public setup files.

| Method | Use it for |
| --- | --- |
| APT | Ubuntu machines that need normal package updates. |
| Local `.deb` | A pinned release on Ubuntu or Debian. |
| Packslip or Mise | Verified user installs and project version pins. |
| GHCR image | Docker builds that copy the CLI into another image. |
| Tarball | A manual install without root access or a package manager. |

Apple SDKs, signing credentials, Swift, and Swift Build are separate requirements.
The CLI packages and container image contain no Apple SDKs or signing material.
Ubuntu 24.04 is the validated platform for Apple toolchain setup.

## APT

Install the repository key and check its fingerprint:

```bash
sudo apt-get update
sudo apt-get install ca-certificates curl gnupg
curl -fsSLo /tmp/sweetpad-archive-keyring.asc \
  https://sweetpad-dev.github.io/sweetpad/apt/sweetpad-archive-keyring.asc
gpg --show-keys --with-fingerprint /tmp/sweetpad-archive-keyring.asc
```

The signing fingerprint is:

```text
BD4A 4E59 600E EECC 7D9A  9DA7 BA44 850D FB32 ACC9
```

After checking it, configure this key for the Sweetpad repository:

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
sweetpad --version
```

APT selects `amd64` or `arm64` from your host architecture.
The repository uses its own `stable` suite, independent of your Ubuntu release name.
The package installs `/usr/bin/sweetpad` and `/usr/bin/sweetpad-dap` without maintainer scripts.

Upgrade or remove it with APT:

```bash
sudo apt-get update
sudo apt-get install --only-upgrade sweetpad
sudo apt-get remove sweetpad
```

Removal keeps your compiler, private SDK, signing files, and user configuration.

## Local Debian package

Download a release package and its checksum:

```bash
CLI_VERSION=0.1.21
CLI_ARCH="$(dpkg --print-architecture)"
curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb"
curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb.sha256"
sha256sum -c "sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb.sha256"
sudo apt-get install "./sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb"
```

A local package install does not register the APT feed.
Install the next verified `.deb` in the same way, or configure the feed above.

## Packslip and Mise

With [Packslip](https://packslip.dev/docs/bootstrap/) installed:

```bash
packslip install github.com/sweetpad-dev/sweetpad/tools/cli
sweetpad --version
```

Packslip verifies the signed release metadata and artifact digest before exporting the commands.
Run the same install command to upgrade.
The project path ends with `tools/cli` because the repository also publishes editor extension releases.

With a version of [Mise that supports Packslip](https://mise.jdx.dev/dev-tools/backends/packslip.html):

```bash
mise use -g packslip:github.com/sweetpad-dev/sweetpad/tools/cli
```

For a project version pin, omit `-g` and add the version:

```bash
mise use packslip:github.com/sweetpad-dev/sweetpad/tools/cli@0.1.21
```

Use `mise upgrade` for Mise-managed installs.
On Linux, install `ca-certificates` for HTTPS and `python3` and `bash` for SDK preparation.

## GHCR container image

The image supports `linux/amd64` and `linux/arm64`:

```bash
docker run --rm ghcr.io/sweetpad-dev/sweetpad:0.1.21 --version
```

Copy the static CLI into an existing Ubuntu build image:

```dockerfile
FROM ghcr.io/sweetpad-dev/sweetpad:0.1.21 AS sweetpad
FROM ubuntu:24.04
COPY --from=sweetpad /usr/local/bin/sweetpad /usr/local/bin/sweetpad
RUN ln -s sweetpad /usr/local/bin/sweetpad-dap
RUN apt-get update && apt-get install -y ca-certificates python3 bash
```

The published image contains the CLI and CA certificates, with no shell or Apple build toolchain.
Run `sweetpad setup` in your own prepared build image.
Import SDK files and signing credentials privately at build or job execution time.
Use an image digest when your CI requires immutable dependencies.

## Standalone tarball

Install CA certificates for HTTPS, plus Python and Bash for SDK preparation:

```bash
sudo apt-get install ca-certificates python3 bash curl
```

Download the archive for your host and verify it:

```bash
CLI_VERSION=0.1.21
case "$(uname -m)" in
  aarch64|arm64) CLI_ARCH=arm64 ;;
  x86_64|amd64) CLI_ARCH=x64 ;;
  *) echo "Unsupported Linux host architecture" >&2; exit 1 ;;
esac
mkdir -p "$HOME/downloads/sweetpad-$CLI_VERSION"
cd "$HOME/downloads/sweetpad-$CLI_VERSION"
curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz"
curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz.sha256"
sha256sum -c "sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz.sha256"
tar -xzf "sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz"
install -Dm755 sweetpad "$HOME/.local/bin/sweetpad"
ln -sfn sweetpad "$HOME/.local/bin/sweetpad-dap"
export PATH="$HOME/.local/bin:$PATH"
sweetpad --version
```

No Rust compiler, source checkout, or installation script is required.
The binary needs no glibc or OpenSSL shared libraries.
Setup files are extracted into a content-addressed user cache when needed.

To upgrade, verify the next archive and replace the executable.
To remove the default manual installation:

```bash
rm -f "$HOME/.local/bin/sweetpad" "$HOME/.local/bin/sweetpad-dap"
```

## Prepare the build environment

After installing the CLI, follow [SDK export and setup](sdk-transfer.md).
Then use [Ubuntu and USB setup](ubuntu-usb.md) to connect an iPhone.
