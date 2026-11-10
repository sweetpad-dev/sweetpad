---
sidebar_label: SDK export and setup
---

# Export Apple SDK files and configure Ubuntu

These convenience commands are unreleased. CLI 0.1.18 does not include them.
Use a current source build until the next CLI release.
The [Ubuntu guide](ubuntu-usb.md#manual-toolchain-setup) retains the setup script for CLI 0.1.18.

From a current source checkout, build the CLI on each host:

```bash
cargo build --release --locked -p sweetpad-cli --bin sweetpad
export PATH="$PWD/target/release:$PATH"
```

This source build requires Rust. The complete Linux release package supplies the CLI and public setup sources without Rust.

This flow transfers the required Apple files from your Mac to Ubuntu 24.04 ARM64.
Use a Linux VM on Apple hardware under [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
Keep the SDK archive private. It contains Apple SDK data and contains no signing identities or provisioning profiles.

## Export on your Mac

Install Xcode and select a destination for the archive.
Estimate the input size and required disk space:

```bash
sweetpad sdk export --xcode /Applications/Xcode.app \
  --output-file "$HOME/apple-sdk.tar.gz" --dry-run --json --non-interactive
```

Create the archive:

```bash
sweetpad sdk export --xcode /Applications/Xcode.app \
  --output-file "$HOME/apple-sdk.tar.gz" --json --non-interactive
```

Export includes the iOS device, iOS Simulator, and macOS base files required by the pinned importer.
It also includes additional platform SDKs present in Xcode.
The archive contains compatibility headers, runtime libraries, platform metadata, test runners, sticker stubs, and localized App Intents resources.
It excludes compiler caches and Xcode host compiler executables.

The manifest records the Xcode build, required toolchain versions, file sizes, file checksums, and symbolic links.
Export creates a `.sha256` file beside the archive and refuses to overwrite existing exports.

## Transfer both files

Copy the archive and its checksum file to your private Linux host:

```bash
scp "$HOME/apple-sdk.tar.gz" "$HOME/apple-sdk.tar.gz.sha256" ubuntu:~/
```

Replace `ubuntu` with your SSH host.
An OrbStack machine can also read the Mac files at their original absolute paths.
Do not publish the archive as a public CI artifact.

## Configure Ubuntu

Install the complete Linux CLI package or use a current source build before this step.
Run setup on Ubuntu 24.04 ARM64:

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz" --json --non-interactive
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios --json --non-interactive
```

Setup requires Python 3 and sudo access for system packages.
With `--non-interactive`, configure passwordless sudo or authenticate with `sudo -v` before setup.
Setup installs the pinned Swift toolchain, xtool, patched Swift Build, and macro plugin.
It verifies the SDK archive and imports the Apple files without access to the original Xcode installation.
It does not install Rust or modify shell startup files.
Setup builds the packaged LinuxApp sample for iOS Simulator to validate the installed tools and SDK.
The setup report includes the sample build result and product path.

The generated environment file selects the SDK and compiler paths.
Source this file in each new shell or from your shell configuration.
In CI, source it in each build step.

Setup reuses existing tools. Import of the same archive reuses the completed SDK.
Swift Build uses the shipped dependency lockfile. New tool versions or incompatible caches use a separate source directory.
Setup preserves existing source checkouts and local edits.
A different archive requires explicit replacement:

```bash
sweetpad setup --sdk "$HOME/new-apple-sdk.tar.gz" --replace-sdk \
  --json --non-interactive
```

Import stages and verifies the new SDK before it replaces the previous SDK.
Failure before replacement leaves the previous SDK intact.
Disk checks include space for extracted inputs and the new SDK.
Toolchain compilation and your app builds require additional space.

## Import without toolchain setup

With xtool 1.21.0 and Swift 6.4.2 on PATH, import only the SDK:

```bash
sweetpad sdk import "$HOME/apple-sdk.tar.gz" --json --non-interactive
```

Use `--destination /absolute/path` for a separate SDK.
Set `SWEETPAD_DARWIN_SDK` to that path before builds.
Use `--replace` to replace a different SDK after successful import.
For full setup, `--sdk-path /absolute/path` selects the equivalent destination and records it in the environment file.
Use `--env-file /absolute/path` to write a separate environment file for a CI runner or an isolated setup.

## Read the doctor report

Linux doctor checks the host, Swift version, Swift Build, macro plugin, SDK inputs, target SDKs, linker, and disk space.
It reports optional development signing and iPhone adapter configuration separately.
It does not require Xcode, `xcodebuild`, or Simulator on Linux.
Missing localized App Intents phrase resources produce a warning, because ordinary app compilation does not require them.
Use `--target macos` for macOS app builds.

The command exits with a nonzero status when required tools are missing.
JSON output still contains the report. Read `data.summary.problems` and each check's `status` and `hint`.
Warnings about signing or the iPhone adapter do not prevent unsigned compilation.
Doctor does not pair a phone, validate entitlements, or prove that an app installs.

Continue with [a sample build and USB setup](ubuntu-usb.md#build-without-a-phone), then [development signing](signing-and-testing.md).
