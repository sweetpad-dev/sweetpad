---
sidebar_label: Install the CLI
---

# Install SweetPad on Linux

Choose a package for the machine that runs SweetPad. An x86_64 Linux host uses the x64 package even when it builds an ARM64 iPhone app. The CLI itself is static. The packaged Swift Build driver uses the host's SQLite library.

This guide describes the package layout with `libexec/sweetpad/`. It contains Swift Build, the macro plugin, and LLVM tools for Apple targets. The tarballs from CLI 0.1.26 and 0.1.27 use an older layout. For a pinned release, check its included README before following the setup instructions. [SweetPad releases](https://github.com/sweetpad-dev/sweetpad/releases)

After installation, [run setup](sdk-transfer.md) to install the Linux Swift compiler and import your Apple SDKs. The prebuilt package requires no Rust toolchain or source checkout.

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

Run these commands in Bash. At the prompt, enter the release number without the `cli-v` prefix.

```bash
read -r -p 'SweetPad release number: ' CLI_VERSION
CLI_ARCH="$(dpkg --print-architecture)"
BASE="https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION"
curl -fLO "$BASE/sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb"
curl -fLO "$BASE/sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb.sha256"
sha256sum -c "sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb.sha256" || exit 1
sudo apt-get install "./sweetpad_$CLI_VERSION-1_$CLI_ARCH.deb"
```

APT installs the host packages listed as dependencies of the selected release. With the current package, that includes the tools and libraries that setup checks. To upgrade, install the next release’s package the same way.

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

For a project pin, omit `-g` and append `@VERSION` to the tool name, replacing `VERSION` with your release number. Use `mise upgrade` to upgrade.

## Container image

The current image recipe for `ghcr.io/sweetpad-dev/sweetpad` starts from Debian with the pinned Linux Swift compiler. It adds the CLI, packaged helpers, and host dependencies for `linux/amd64` and `linux/arm64`. Supply the private SDK at runtime.
For a tag with packaged helpers, pass its numeric version as the Docker build argument `SWEETPAD_VERSION`. Copy the helpers with the CLI so it can find the matching build tools.

```dockerfile
ARG SWEETPAD_VERSION
FROM ghcr.io/sweetpad-dev/sweetpad:${SWEETPAD_VERSION} AS sweetpad
FROM ubuntu:24.04
COPY --from=sweetpad /usr/local/bin/sweetpad /usr/local/bin/sweetpad
COPY --from=sweetpad /usr/local/libexec/sweetpad /usr/local/libexec/sweetpad
RUN ln -s sweetpad /usr/local/bin/sweetpad-dap
RUN apt-get update && apt-get install -y ca-certificates python3 bash libsqlite3-0 \
    && ln -sf bash /bin/sh
```

If you copy just the CLI and helpers into Ubuntu as above, the Swift compiler stays in the first image. To prepare the public host tools in the Ubuntu image, run `sweetpad setup --no-sdk --install-system-packages` as root. For local Apple builds, make your private SDK available when you run the container. Point `SWEETPAD_DARWIN_SDK` at the mounted bundle. Supply signing files only for signed builds.

## Tarball

In Bash, enter the release number without the `cli-v` prefix. The commands verify the archive and keep the extracted CLI and helpers together.

```bash
read -r -p 'SweetPad release number: ' CLI_VERSION
case "$(uname -m)" in
  aarch64|arm64) CLI_ARCH=arm64 ;;
  x86_64|amd64) CLI_ARCH=x64 ;;
  *) echo "Unsupported Linux architecture" >&2; exit 1 ;;
esac
BASE="https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION"
curl -fLO "$BASE/sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz"
curl -fLO "$BASE/sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz.sha256"
sha256sum -c "sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz.sha256" || exit 1
release_dir="$HOME/.local/share/sweetpad/releases/$CLI_VERSION-linux-$CLI_ARCH"
mkdir -p "$release_dir" "$HOME/.local/bin"
tar -xzf "sweetpad-cli-$CLI_VERSION-linux-$CLI_ARCH.tar.gz" -C "$release_dir"
ln -sfn "$release_dir/sweetpad" "$HOME/.local/bin/sweetpad"
ln -sfn "$release_dir/sweetpad-dap" "$HOME/.local/bin/sweetpad-dap"
export PATH="$HOME/.local/bin:$PATH"
sweetpad --version
```

Keep the extracted directory intact. SweetPad follows the link to its executable and looks beside it for the matching helpers. Swift Build needs `libsqlite3-0` on Ubuntu and Debian, or `sqlite` on Arch. To switch versions, repeat the procedure with the new release number.

## Upgrade

Swift Build, the macro plugin and the linker come with the package, so upgrading the CLI upgrades them too.
Keep the tarball's `libexec` folder next to the `sweetpad` binary.
If a release changes the SDK import recipe or the toolchain, run setup again with the source you used the first time, and add `--replace-sdk` if it asks:

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz"
sweetpad doctor --target ios
```

Setup in older releases compiled Swift Build into `~/src/swift-build*` and linked it from `~/bin`.
After upgrading from that layout, rerun setup to regenerate the environment file. Start a fresh shell so old helper overrides do not take precedence over the packaged tools.
