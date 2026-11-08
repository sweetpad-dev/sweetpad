# Sweetpad for Linux ARM64

This experimental package supports Ubuntu 24.04 ARM64 (glibc 2.39).
It contains the CLI and public toolchain setup sources, including pinned patches and macro sources.
It does not contain Apple SDKs, Xcode, signing identities, provisioning profiles, or build caches.

## Install the CLI

Install the runtime dependencies:

```bash
sudo apt-get update
sudo apt-get install ca-certificates libssl3t64 libgcc-s1
```

Verify the downloaded archive before extraction:

```bash
sha256sum -c sweetpad-cli-VERSION-linux-arm64.tar.gz.sha256
tar -xzf sweetpad-cli-VERSION-linux-arm64.tar.gz -C /path/to/empty-directory
cd /path/to/empty-directory
bash install.sh
export PATH="$HOME/.local/bin:$PATH"
sweetpad --version
```

`--prefix /absolute/path` selects another installation directory. Installation requires no compiler or root access.
The `sweetpad-dap` alias and the setup sources install with the CLI.

## Prepare the Apple build toolchain

Provide your own Xcode copy, then run the installed setup script:

```bash
SWEETPAD_SKIP_RUST=1 XCODE_APP=/path/to/Xcode.app \
  bash "$HOME/.local/share/sweetpad/setup/sweetpad-lib/ci/linux-setup.sh"
export PATH="$HOME/.local/bin:$HOME/bin:$HOME/swift/swift-6.4.2-RELEASE/usr/bin:$PATH"
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive
```

The setup script downloads the pinned public Linux tools and imports SDK files from your Xcode copy.
The SDK stays private. The prebuilt CLI requires no Rust installation or Sweetpad source checkout.
App signing requires your development identity and profiles, configured separately.

## Support scope

The validated environment is Ubuntu 24.04 ARM64 in OrbStack on Apple hardware.
Core compilation, resources, development signing, and physical iPhone USB/Wi-Fi transport pass validation.
The packaged CLI also passes installation in a clean Ubuntu 24.04 ARM64 container.
Simulator operation and Apple test execution still require a Mac with Xcode.
Distribution IPA export, Linux device debugging, and device readiness inspection remain unsupported.
Flutter AOT requires the separately built Linux ARM64 iOS snapshot compiler.
The tested React Native Hermes compiler requires OrbStack's x86 translation in this ARM64 environment.
Other Linux architectures and distributions are not release targets yet.

Run `sweetpad help linux-devices` for pairing and bounded logs.
See the [Linux guide](https://sweetpad.hyzyla.dev/docs/cli/linux) for build setup and current limitations.
