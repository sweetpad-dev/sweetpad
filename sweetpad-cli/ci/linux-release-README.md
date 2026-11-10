# Sweetpad for Linux ARM64

This experimental package supports Ubuntu 24.04 ARM64 (glibc 2.39).
It contains the CLI and public toolchain setup sources, including pinned patches and macro sources.
It does not contain Apple SDKs, Xcode, signing identities, provisioning profiles, or build caches.

## Install the CLI

Install the runtime dependencies:

```bash
sudo apt-get update
sudo apt-get install ca-certificates libssl3t64 libgcc-s1 python3
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

On your Mac, export the required files from your Xcode with a CLI version that includes SDK export:

```bash
sweetpad sdk export --xcode /Applications/Xcode.app \
  --output-file "$HOME/apple-sdk.tar.gz" --json --non-interactive
scp "$HOME/apple-sdk.tar.gz" "$HOME/apple-sdk.tar.gz.sha256" ubuntu:~/
```

On Ubuntu, import the archive and install the pinned tools:

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
The original setup script also accepts a visible Xcode copy through `XCODE_APP`.
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

## Upgrade or remove the CLI

Rerun `install.sh` to upgrade. It replaces the bundled setup sources, licenses, and example app,
including files removed from the new package. Keep your app source outside the installed example directory.

For the default prefix, remove the CLI with:

```bash
rm -f "$HOME/.local/bin/sweetpad" "$HOME/.local/bin/sweetpad-dap"
rm -rf "$HOME/.local/share/sweetpad"
```

The compiler, private SDK, signing files, and user configuration remain in their separate locations.
