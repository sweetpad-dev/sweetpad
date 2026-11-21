---
sidebar_label: Signing, tests, and archives
---

# Signing, tests, and archives

Linux signs builds, exports and uploads release builds, and notarizes Developer ID apps.
Running Apple tests still needs a Mac.

## Configure development signing

On Linux, put your Apple Development identity in `~/signing/development.p12` and the provisioning profiles in `~/signing/profiles/`. Include a profile for the app and each extension. The P12 contains the certificate and private key used to sign the code. The profiles specify the permitted bundle identifiers, entitlements, certificates, and devices.

If your identity is in Keychain Access on a Mac, export the certificate **with its private key** as a password-protected P12. Transfer it privately to Linux.

In Bash on Linux, set the paths and enter the P12 password at the prompt.

```bash
export SWEETPAD_CODE_SIGN_P12="$HOME/signing/development.p12"
export SWEETPAD_PROVISIONING_PROFILES="$HOME/signing/profiles"
read -r -s -p 'P12 password: ' SWEETPAD_CODE_SIGN_P12_PASSWORD
printf '\n'
export SWEETPAD_CODE_SIGN_P12_PASSWORD

sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive
```

The signer selects a matching profile from that folder. A generic iOS destination has no device ID, so check that the development profile includes the iPhone you intend to use. When the destination names a device, SweetPad checks its ID during profile selection.
If you omit `SWEETPAD_PROVISIONING_PROFILES`, SweetPad searches the standard Xcode profile directories.
Build scripts, plugins, and scheme actions don't see the password or the App Store Connect key in their environment.

`sweetpad doctor --target ios` lists the identities and profiles it finds.
It warns about an expired certificate, a missing issuer certificate, and profiles that list none of your certificates.
Simulator builds need no signing.

### CocoaPods

A generated CocoaPods script can call `/usr/bin/codesign` directly. On Linux, that path can cause a signed build to fail in the “[CP] Embed Pods Frameworks” phase.
If your generated scripts contain this absolute path, point them at the signer on the build's PATH. Repeat the change after regenerating the scripts.

```bash
sed -i 's|/usr/bin/codesign|codesign|' "ios/Pods/Target Support Files"/*/*-frameworks.sh
```

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

## Build test products

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp --configuration Debug \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyAppTests" CODE_SIGNING_ALLOWED=NO
```

Read `data.xctestrunPath` in the build report. It gives the generated manifest for this scheme, configuration, and platform.
When the manifest needs something Linux can't write, such as code coverage or sanitizers, the path is null and `data.xctestrunNote` explains why. A successful compile alone does not guarantee a runnable manifest.
Add `-testPlan Smoke` after `--` to use another test plan from the scheme.

For an iPhone, build with `generic/platform=iOS` and signing for the app, its tests, and the UI test runner.
Unit tests on a device need a host app.

## Run the tests on a Mac

Copy the whole Products folder to the Mac as a tar archive, which keeps permissions and symbolic links:

```bash
tar -C "$HOME/builds/MyAppTests/Build" -czf MyAppTests.tar.gz Products
```

Copy the archive to a Mac with a compatible Xcode and simulator runtime. From the same project revision, extract the archive. Use the filename from `data.xctestrunPath` under the extracted `Products/` directory. For the example above with the Debug configuration, run:

```bash
tar -xzf MyAppTests.tar.gz
sweetpad test run --project MyApp.xcodeproj --scheme MyApp \
  --xctestrun Products/MyApp-Debug-iphonesimulator.xctestrun --on SIMULATOR_UDID --json --non-interactive
```

This runs the Linux-built products without compiling them again.
Paths the build put into test environment variables and arguments are changed to the Mac's project and products folders.
Use the phone's UDID for signed iPhone products.
