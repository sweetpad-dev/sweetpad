---
sidebar_label: macOS apps in CI
---

# Build macOS apps in GitHub Actions

A Linux runner can build a macOS app, then sign, package, upload, and notarize it.
Running the app and its tests needs a Mac.

## On a hosted Mac

When one job should build, test, and run, use a hosted Mac with SweetPad's usual Xcode engine:

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
          sweetpad build --project MyMacApp.xcodeproj --scheme MyMacApp \
            --configuration Release --on mac --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
          sweetpad test run --project MyMacApp.xcodeproj --scheme MyMacApp \
            --on mac --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
```

## On Linux

Linux uses the same [self-hosted runner](github-actions.md#prepare-the-runner) as iPhone builds, with a macOS destination:

```bash
sweetpad build --project MyMacApp.xcodeproj --scheme MyMacApp \
  --configuration Release --destination 'generic/platform=macOS' \
  --json --non-interactive \
  -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
```

The build holds arm64 and x86_64, as Xcode's does.
Keep the products in a tar archive, as for [iPhone builds](github-actions.md#build-iphone-products), and run them in a later Mac job.
Linux doesn't write test manifests for macOS tests, so run those on a Mac.

## Distribute from Linux

An archive of a Mac app exports what Xcode's does:

- `--export-method app-store-connect` writes a Mac App Store `.pkg`, and `--upload` sends it to TestFlight.
- `--notarize` exports with Developer ID, then notarizes and staples the app, its `.zip`, and the `.pkg`.

```bash
sweetpad archive --project MyMacApp.xcodeproj --scheme MyMacApp \
  --configuration Release --destination 'generic/platform=macOS' \
  --notarize --output-file "$RUNNER_TEMP/dist" --json --non-interactive
```

Provide the identities and profiles the method needs, as in [the signing guide](signing-and-testing.md#archive-and-distribute).
A Mac App Store build needs the App Sandbox, and a Developer ID build needs the hardened runtime.
SweetPad refuses an export without them and names the build setting to change.
