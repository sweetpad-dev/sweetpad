# Sweetpad for Linux

This package contains a statically linked Linux CLI for ARM64 or x86_64.
The executable embeds the public setup sources, pinned patches, macros, and sample project.
Apple toolchain setup supports Ubuntu 24.04 and Debian 12/13 on both architectures, plus Arch Linux x64.
Other distributions warn and require existing compatible dependencies. Alpine toolchain setup remains unsupported.
The CLI itself needs no glibc or OpenSSL shared libraries.
It does not contain Apple SDKs, Xcode, signing identities, provisioning profiles, or build caches.

## Install the CLI

Install CA certificates for HTTPS and Python/Bash for SDK preparation:

```bash
sudo apt-get update
sudo apt-get install ca-certificates python3 bash
```

Select `linux-arm64` for an ARM64 host, or `linux-x64` for an x86_64 host.
The package architecture selects the tools that run on Linux. An iPhone build targets ARM64 on either host.

Verify the downloaded archive before extraction (the example uses ARM64):

```bash
sha256sum -c sweetpad-cli-VERSION-linux-arm64.tar.gz.sha256
tar -xzf sweetpad-cli-VERSION-linux-arm64.tar.gz -C /path/to/empty-directory
cd /path/to/empty-directory
install -Dm755 sweetpad "$HOME/.local/bin/sweetpad"
ln -sfn sweetpad "$HOME/.local/bin/sweetpad-dap"
export PATH="$HOME/.local/bin:$PATH"
sweetpad --version
```

Installation requires no compiler, root access, or Sweetpad installation script.
You can copy the executable into another directory on PATH.
The CLI materializes its embedded setup files in its content-addressed user cache when needed.

## Package managers and containers

The release also provides native `amd64` and `arm64` Debian packages.
Install the matching local package with `sudo apt install ./sweetpad_VERSION_ARCH.deb`.
The signed APT feed supplies normal APT upgrades.

Signed Packslip metadata identifies the CLI separately from the editor extension:

```bash
packslip install github.com/sweetpad-dev/sweetpad/tools/cli
mise use -g packslip:github.com/sweetpad-dev/sweetpad/tools/cli
```

GHCR supplies a multi-platform CLI image at `ghcr.io/sweetpad-dev/sweetpad`.
It contains the static executable, CA certificates, and license notices. Docker builds can copy `/usr/local/bin/sweetpad` into their own image.
Swift, Swift Build, and the private SDK remain separate build-environment requirements.

## Prepare the Apple build toolchain

On your Mac, export the required files from your Xcode with a CLI version that includes SDK export:

```bash
sweetpad sdk export --xcode /Applications/Xcode.app \
  --output-file "$HOME/apple-sdk.tar.gz" --json --non-interactive
scp "$HOME/apple-sdk.tar.gz" "$HOME/apple-sdk.tar.gz.sha256" ubuntu:~/
```

On Linux, import the archive and install the pinned tools:

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz" --json --non-interactive
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios --json --non-interactive
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive
```

Setup downloads the pinned public Linux tools, imports SDK files, and builds the sample app.
It requires sudo access for system packages, or an existing root shell. Use `sudo -v` first if your account requires a password.
An unchanged archive reuses the installed SDK. A different archive requires `--replace-sdk`.
The SDK stays private. The prebuilt CLI requires no Rust installation or Sweetpad source checkout.
App signing requires your development identity and profiles, configured separately.

CLI 0.1.22 also accepts private Xcode `.xip` downloads directly:

```bash
sweetpad setup --sdk "$HOME/Downloads/Xcode.xip" --json --non-interactive
```

Apple Encrypted Archive (`.aea`) import is experimental and supports profiles 0–2.
It supports embedded keys, Apple FCS envelopes, `--aea-key-file`, and `--aea-public-key-file`.
Use trusted Apple downloads: integrity checks do not validate Apple's publisher certificate chain.
Direct import omits localized App Intents phrase resources; use a Mac export for localized shortcut training.
See [SDK import and setup](https://sweetpad.hyzyla.dev/docs/cli/linux/sdk-transfer) for key formats and validation limits.

## Support scope

The ARM64 validation environment uses Ubuntu 24.04 in OrbStack on Apple hardware.
The x64 cross-compilation checks use an x86_64 Ubuntu container on the same hardware.
Core compilation, resources, development signing, and physical iPhone USB/Wi-Fi transport pass validation.
The standalone CLI passes installation in clean containers for both host architectures.
Simulator operation and Apple test execution still require a Mac with Xcode.
Distribution IPA export, Linux device debugging, and device readiness inspection remain unsupported.
Flutter AOT requires the separately built Linux ARM64 iOS snapshot compiler.
The tested React Native Hermes compiler requires OrbStack's x86 translation in this ARM64 environment.
Other Linux architectures, including 32-bit x86, are not release targets.
Public host-toolchain checks cover Ubuntu, Debian, and Arch. Private SDK app builds cover these distributions separately.
Additional distributions remain unverified.

Run `sweetpad help linux-devices` for pairing and bounded logs.
See the [Linux guide](https://sweetpad.hyzyla.dev/docs/cli/linux) for build setup and current limitations.

## Upgrade or remove the CLI

Use the package manager that installed the CLI to upgrade or remove it.
For manual installations, verify the next archive and replace the executable.
Its embedded setup files move with the binary, so an upgrade cannot retain older helper sources.

For the default prefix, remove the CLI with:

```bash
rm -f "$HOME/.local/bin/sweetpad" "$HOME/.local/bin/sweetpad-dap"
```

The compiler, private SDK, signing files, and user configuration remain in their separate locations.
