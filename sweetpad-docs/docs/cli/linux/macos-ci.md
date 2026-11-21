---
sidebar_label: Build macOS apps on Linux
---

# Build and export macOS apps on Linux

SweetPad can compile, sign, and export a macOS app on Linux. Choose a macOS destination to build against the Mac SDK.

## Build the app on Linux {#on-linux}

On Linux, [set up the Apple toolchain](sdk-transfer.md). From your project directory, run an unsigned Release build. Replace the project and scheme names with yours.

```bash
sweetpad build --project MyMacApp.xcodeproj --scheme MyMacApp \
  --configuration Release --destination 'generic/platform=macOS' \
  --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/MyMacApp" CODE_SIGNING_ALLOWED=NO
```

A generic destination uses the project's standard architectures for its deployment target. To run the app, copy the product at `data.productPath` to a compatible Mac. Use a tar archive to preserve bundle permissions and symbolic links.

## Distribute from Linux

Choose the export method for your distribution channel.

- `--export-method app-store-connect` writes a Mac App Store `.pkg`, and `--upload` sends it to TestFlight.
- `--notarize` exports with Developer ID, then notarizes and staples the app and any installer package. The ZIP contains the stapled app.

```bash
sweetpad archive --project MyMacApp.xcodeproj --scheme MyMacApp \
  --configuration Release --destination 'generic/platform=macOS' \
  --notarize --output-file "./dist" --json --non-interactive
```

Provide the identities and profiles the method needs, as in [the signing guide](signing-and-testing.md#archive-and-distribute).
A Mac App Store build needs the App Sandbox, and a Developer ID build needs the hardened runtime.
SweetPad refuses an export without them and names the build setting to change.

## Run tests on a hosted Mac {#on-a-hosted-mac}

Running macOS tests requires a Mac with Xcode. SweetPad generates test manifests on Linux for iOS only. This GitHub Actions job builds the macOS tests from your checkout and runs them on a hosted Mac.

```yaml
name: macOS app
on:
  push:
    branches: [main]
  pull_request:
permissions:
  contents: read

jobs:
  macos:
    runs-on: macos-15
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v7
      - name: Install SweetPad
        run: brew install sweetpad-dev/tap/sweetpad
      - name: Build and test
        run: |
          sweetpad test run --project MyMacApp.xcodeproj --scheme MyMacApp \
            --configuration Release --on mac --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
```
