---
sidebar_label: Signing, tests, and archives
---

# Signing, tests, and archives

An iPhone app needs a development identity and a provisioning profile before installation.
Linux builds and signs the products, and exports, uploads, and notarizes release builds.
A Mac with Xcode supplies Apple test execution.

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
When the destination names a device, the signer selects only a profile that lists that device.

The P12 file must contain the certificate's issuer, unless the issuer is an Apple WWDR authority (G3 to G6) or a Developer ID authority.
SweetPad includes those. If the issuer is missing, signing stops and names the missing authority.

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

## Archive and distribute

Archive the device build with the export method you need:

```bash
sweetpad archive --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' \
  --export-method debugging --output-file ./artifacts \
  --json --non-interactive
```

The archive includes app, extension, and embedded-framework debug symbols.
A development export keeps the build's identity and profiles.
Use `--no-export` to retain only the archive, and `sweetpad archive export` to export it later.

### Identities and profiles for each method

Every other method signs the archived app again, as Xcode's export does.
SweetPad picks the identity by its certificate type and the profile by its kind:

| Method | Identity | Profiles |
| --- | --- | --- |
| `app-store-connect` | Apple Distribution, plus Mac Installer Distribution for a macOS package | App Store or Mac App Store |
| `release-testing` | Apple Distribution | Ad Hoc |
| `enterprise` | Apple Distribution | In-House |
| `developer-id` | Developer ID Application, plus Developer ID Installer for the package | Developer ID, when the app uses capabilities that need one |

List every P12 file in `SWEETPAD_CODE_SIGN_P12`, separated by `:`.
A file exported from Keychain Access can hold several identities.
`SWEETPAD_CODE_SIGN_P12_PASSWORD` holds one password for all of them, or one per file in the same order.
Put the profiles for the app and each extension in `SWEETPAD_PROVISIONING_PROFILES`.

When no identity can sign an archive, SweetPad signs it ad hoc and keeps its entitlements.
The distribution export then signs it properly, so the archive step needs no distribution identity.
Run `sweetpad doctor --target ios` to list the identities and profiles it finds.
It warns about an expired certificate, an issuer the P12 file lacks, and profiles that list none of your certificates.

### Upload and notarize

Add `--upload` to send an App Store Connect export to TestFlight, or `--notarize` for a Developer ID build.
Both need an App Store Connect API key:

```bash
export SWEETPAD_ASC_KEY_ID=KEY_ID
export SWEETPAD_ASC_ISSUER_ID=ISSUER_ID
export SWEETPAD_ASC_KEY_PATH="$HOME/signing/AuthKey_KEY_ID.p8"

sweetpad archive --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --upload \
  --output-file ./artifacts --json --non-interactive
```

[Archive and distribute](../archive.md#uploading-to-app-store-connect) covers the key, what an upload waits for, and notarization.
`sweetpad doctor --network` checks that Apple's services answer from the runner.

The Linux export path rejects unsigned archives and simulator archives.

## Validate on an iPhone

The [Ubuntu and USB guide](ubuntu-usb.md#build-and-install-the-iphone-app) covers installation, launch, and bounded logs.
A startup check confirms that the signed products launch.
It does not validate every entitlement, extension, payment flow, or push service.

For CI, keep signing files separate from retained build artifacts.
The [GitHub Actions guide](github-actions.md) describes the runner and artifact layout.
