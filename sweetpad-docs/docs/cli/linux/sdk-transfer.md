---
sidebar_label: SDK import and setup
---

# Import Apple SDK files and configure Linux

Install CLI 0.1.19 or later on each host.
The [CLI installation guide](../getting-started.md) covers macOS.
The [Ubuntu guide](ubuntu-usb.md#install-the-release) covers Linux.

The complete Linux package supplies the CLI and public setup sources without Rust.
For manual toolchain setup, use the [setup script](ubuntu-usb.md#manual-toolchain-setup).

You can export the required Apple files from your Mac or import a private Xcode download directly on Linux.
CLI 0.1.22 supports Ubuntu, Debian, and Arch setup on the [validated host architectures](distributions.md).
Use a Linux VM on Apple hardware under [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
Keep the SDK archive private. It contains Apple SDK data and contains no signing identities or provisioning profiles.

## Import a Xcode download on Linux

CLI 0.1.22 can import Xcode `.xip` files and has experimental support for `.aea` files.
Apple introduced `.aea` downloads in [Xcode 27.3](https://developer.apple.com/documentation/xcode-release-notes/xcode-27_3-release-notes).
Download Xcode from Apple and keep the archive private.
Then run setup with the downloaded file:

```bash
sweetpad setup --sdk "$HOME/Downloads/Xcode.xip" --json --non-interactive
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios --json --non-interactive
```

Use your `.aea` filename in the same command for an AEA download.
Setup extracts the required SDK inputs and installs Linux host tools.
It does not install or execute the Xcode application.
The archive remains on disk, so allow space for the download, extracted inputs, and staged SDK.

The importer checks archive checksums and AEA authentication data.
It does not validate Apple's publisher certificate chain. Use a trusted Apple download and check its published checksum when available.
An adjacent `Xcode.xip.sha256` or `Xcode.aea.sha256` file is checked when present.
Use the usual `sha256sum` format: the digest, two spaces, and the archive filename.

AEA support covers signed or AES-CTR profiles 0, 1, and 2.
It supports embedded keys and Apple FCS key envelopes.
FCS import can retrieve public distribution key material from approved Apple HTTPS hosts without account cookies.
For offline import or archives without embedded keys, pass a key file:

```bash
sweetpad setup --sdk "$HOME/Downloads/Xcode.aea" \
  --aea-key-file /private/path/archive-key --json --non-interactive
```

The key file can contain a raw 32-byte symmetric key, a `hex:` or `base64:` value, or an Apple FCS private key.
For signed archives without an embedded signing key, use `--aea-public-key-file /private/path/signing-public.pem`.
The same options work with `sweetpad sdk import`.
Development builds after CLI 0.1.22 also resolve shared file data in Apple Archive hard-link, clone, and identical-data clusters.
The importer can recover data stored outside the selected SDK paths and preserves each file's permissions.
These builds retain the macOS XCTest runner and preserve permissions when platform metadata is patched for Linux.
Missing data, conflicting cluster contents, and invalid checksums fail before SDK replacement.
Large archives are decoded twice to find shared data before extraction.
Development builds after CLI 0.1.22 report phases and decoded sizes on stderr during `sweetpad sdk import`.
Unsupported encryption profiles fail with an explicit error.
Use a Mac SDK export if your download needs an unsupported archive feature.

Direct import does not generate localized App Intents phrase resources.
Ordinary app compilation can proceed; doctor reports a warning for the missing resources.
Use the Mac export flow below for localized shortcut training.
Development builds after CLI 0.1.22 imported a real Xcode 27.3 beta (27C5020n) profile 0 archive on ARM64 and emulated x64 Linux.
Both hosts built an ARM64 iPhone sample with that SDK.
Validation also uses Apple-tool-generated archive fixtures and full SDK inputs from Xcode 27.2.
A real encrypted Xcode download remains unverified.

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

## Configure Linux from an SDK export

Install the complete Linux CLI package or use a current source build before this step.
Run setup on a [supported Linux distribution](distributions.md):

```bash
sweetpad setup --sdk "$HOME/apple-sdk.tar.gz" --json --non-interactive
source "$HOME/.config/sweetpad/linux-env.sh"
sweetpad doctor --target ios --json --non-interactive
```

Setup requires Python 3 and sudo access for system packages.
With CLI 0.1.22, use `--skip-system-packages` to supply existing host dependencies.
With `--non-interactive`, configure passwordless sudo or authenticate with `sudo -v` before setup.
Setup installs the pinned Swift toolchain, xtool, patched Swift Build, and macro plugin.
It verifies the SDK archive and imports the Apple files without access to the original Xcode installation.
It does not install Rust or modify shell startup files.
Setup builds the packaged LinuxApp sample for iPhone to validate the installed tools and SDK.
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

This command also accepts private `.xip` and `.aea` archives.
Direct archive import needs Python 3 and a host C compiler. AEA also needs Python `cryptography`.
Setup installs these dependencies on the supported distributions.

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
