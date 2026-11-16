---
sidebar_label: Signing, tests, and archives
---

# Signing, tests, and archives

Linux signs builds, exports and uploads release builds, and notarizes Developer ID apps.
Running Apple tests still needs a Mac.

## Configure development signing

Export your Apple Development certificate and its private key from Keychain Access as a P12 file, and collect the provisioning profiles for the app and each extension.
Then point SweetPad at them and enter the password at a prompt:

```bash
export SWEETPAD_CODE_SIGN_P12="$HOME/signing/development.p12"
export SWEETPAD_PROVISIONING_PROFILES="$HOME/signing/profiles"
read -r -s -p 'P12 password: ' SWEETPAD_CODE_SIGN_P12_PASSWORD; printf '\n'
export SWEETPAD_CODE_SIGN_P12_PASSWORD

sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive
```

The signer picks a matching profile from that folder, and only one that lists the device when the destination names one.
Without the variable, it looks where Xcode keeps profiles.
Build scripts, plugins, and scheme actions don't see the password or the App Store Connect key in their environment.

`sweetpad doctor --target ios` lists the identities and profiles it finds.
It warns about an expired certificate, a missing issuer certificate, and profiles that list none of your certificates.
Simulator builds need no signing.

### CocoaPods

CocoaPods writes `/usr/bin/codesign` into its embed-frameworks scripts, and Linux has no such file, so a signed build fails in "[CP] Embed Pods Frameworks".
After each `pod install`, point the scripts at SweetPad's signer:

```bash
sed -i 's|/usr/bin/codesign|codesign|' "ios/Pods/Target Support Files"/*/*-frameworks.sh
```

## Build test products

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyAppTests" CODE_SIGNING_ALLOWED=NO
```

The JSON output's `xctestrunPath` names the test manifest that a Mac runs.
When the manifest needs something Linux can't write, such as code coverage or sanitizers, the path is null and `xctestrunNote` says why.
Add `-testPlan Smoke` after `--` to use another test plan from the scheme.

For an iPhone, build with `generic/platform=iOS` and signing for the app, its tests, and the UI test runner.
Unit tests on a device need a host app.

## Run the tests on a Mac

Copy the whole Products folder to the Mac as a tar archive, which keeps permissions and symbolic links:

```bash
tar -C "$HOME/builds/MyAppTests/Build" -czf MyAppTests.tar.gz Products
```

On the Mac, extract it and run the manifest from `xctestrunPath`:

```bash
tar -xzf MyAppTests.tar.gz
sweetpad test run --project MyApp.xcodeproj --scheme MyApp \
  --xctestrun Products/prebuilt.xctestrun --on SIMULATOR_UDID --json --non-interactive
```

This runs the Linux-built products without compiling them again.
Paths the build put into test environment variables and arguments are changed to the Mac's project and products folders.
Use the phone's UDID for signed iPhone products.

## Archive and distribute

```bash
sweetpad archive --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' \
  --export-method release-testing --output-file ./artifacts \
  --json --non-interactive
```

The export signs the archived app again for its method, as Xcode does, picking the identity by certificate type and the profiles by kind:

| Method | Identity | Profiles |
| --- | --- | --- |
| `debugging` | Apple Development | Development |
| `release-testing` | Apple Distribution | Ad Hoc |
| `enterprise` | Apple Distribution | In-House |
| `app-store-connect` | Apple Distribution, plus Mac Installer Distribution for a Mac package | App Store or Mac App Store |
| `developer-id` | Developer ID Application, plus Developer ID Installer for the package | Developer ID, when the app's capabilities need one |

Without `SWEETPAD_CODE_SIGN_P12`, a `debugging` export keeps the build's signature, and `mac-application` always does.

List every P12 file in `SWEETPAD_CODE_SIGN_P12`, separated by `:`.
`SWEETPAD_CODE_SIGN_P12_PASSWORD` holds one password for all of them, or one per file.
When no identity can sign the archive itself, SweetPad signs it ad hoc and the export signs it properly.
`--no-export` keeps only the archive, and `sweetpad archive export` exports it later.

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

[Archive and distribute](../archive.md#uploading-to-app-store-connect) covers the key and the upload options.
`sweetpad doctor --network` checks that Apple's services answer from the machine.
