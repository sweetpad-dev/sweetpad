---
sidebar_label: Signing, tests, and archives
---

# Signing, tests, and archives

An iPhone app needs a development identity and a provisioning profile before installation.
Linux builds and signs the products. A Mac with Xcode supplies Apple test execution.

## Configure development signing

Prepare an Apple Development certificate and private key as a P12 file.
Use OpenSSL 3 with AES encryption for the export.
Legacy P12 files that use 40-bit RC2 encryption are not supported by the Linux signer.

Provide profiles for the app's bundle identifiers, entitlements, and registered iPhone.
Extensions and UI test runners can require additional matching profiles.

Set the paths and enter the password in your local terminal:

```bash
export SWEETPAD_CODE_SIGN_P12="$HOME/signing/development.p12"
export SWEETPAD_PROVISIONING_PROFILES="$HOME/signing/profiles"
read -r -s -p 'P12 password: ' SWEETPAD_CODE_SIGN_P12_PASSWORD
printf '\n'
export SWEETPAD_CODE_SIGN_P12_PASSWORD

sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyApp" ARCHS=arm64
```

The signer selects a matching profile from the configured directory.
Without a configured directory, it also checks the profile locations that Xcode uses under the user's Library.

An unsigned device build can compile successfully, but the phone cannot install it.
Simulator builds require no development identity.

## Build test products

Use the test action for a scheme that contains unit or UI tests:

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyAppTests" ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
```

JSON output includes `built` and `xctestrunPath`.
A successful build with an unsupported manifest policy reports a null path and an explanatory `xctestrunNote`.
Check both the build result and the manifest path before scheduling test execution.

For an iPhone test build, select `generic/platform=iOS` and configure signing for every required bundle.
Unit, UI runner, and target app signatures must match the device's provisioning requirements.

### Select a test plan

Linux supports version-1 test plans and their named configurations.
The default plan supplies enabled test targets and manifest settings.
Select another referenced plan after the passthrough separator:

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -testPlan Smoke -derivedDataPath "$HOME/builds/MyAppSmoke" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
```

Supported settings include environment, arguments, locale, selected or skipped tests, expansion target, timeouts, parallelization, and attachment lifetime.
Sanitizer policies and other unsupported settings require a supplied manifest.

## Run Linux-built tests

Copy the complete Products directory and its generated manifest to a Mac.
Preserve its directory layout, executable permissions, and symbolic links.
A tar archive preserves these properties when CI artifact storage does not:

```bash
tar -C "$HOME/builds/MyAppTests/Build" -czf MyAppTests.tar.gz Products
```

On the Mac, extract the archive and run the emitted manifest:

```bash
tar -xzf MyAppTests.tar.gz
sweetpad test run --project MyApp.xcodeproj --scheme MyApp \
  --xctestrun /path/to/Products/prebuilt.xctestrun \
  --on SIMULATOR_UDID --json --non-interactive
```

Use the actual manifest filename from `xctestrunPath`.
For signed iPhone products, select the phone's UDID instead of a simulator.
The selected runtime must support the product's platform, architecture, and deployment target.

This uses Xcode's test-without-building action.
It does not compile the application or tests again.
The project supplies result history and context; the manifest supplies the prebuilt products.
A selection that runs zero tests fails.

The automatic manifests preserve enabled targets, environment values, quoted arguments, locale overrides, and test filters.
UI test manifests also identify the built runner and target application.

## Create a development archive

Configure development signing, then archive the device build:

```bash
sweetpad archive --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' \
  --export-method debugging --output-file ./artifacts \
  --json --non-interactive
```

The archive includes app, extension, and embedded-framework debug symbols.
Development IPA export preserves the build's identity and profiles.
Use `--no-export` to retain only the archive.

Distribution export and custom export options require Xcode.
The Linux export path rejects unsigned IPA export and simulator archives.

## Validate on an iPhone

The [Ubuntu and USB guide](ubuntu-usb.md#build-and-install-the-iphone-app) covers installation, launch, and bounded logs.
A startup check confirms that the signed products launch.
It does not validate every entitlement, extension, payment flow, or push service.

For CI, keep signing files separate from retained build artifacts.
The [GitHub Actions guide](github-actions.md) describes the runner and artifact layout.
