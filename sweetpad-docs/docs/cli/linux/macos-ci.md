---
sidebar_label: macOS apps in CI
---

# Build macOS apps in GitHub Actions

Most pages in this section target iOS and iPhone.
A macOS app has a different execution and distribution path.

A Linux runner can compile a supported macOS app against the extracted macOS SDK.
A Mac runs the application, executes native tests, and performs Developer ID distribution signing and notarization.

## Use a hosted Mac for a native workflow

A hosted Mac is the direct path when your CI job needs build, test, and macOS execution together.
This uses SweetPad's normal Xcode engine.

Replace the project and scheme names:

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
      - name: Build
        run: |
          sweetpad build --project MyMacApp.xcodeproj --scheme MyMacApp \
            --configuration Release --on mac --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
      - name: Run tests
        run: |
          sweetpad test run --project MyMacApp.xcodeproj --scheme MyMacApp \
            --on mac --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
      - name: Package products
        run: |
          tar -C "$RUNNER_TEMP/DerivedData/Build" \
            -czf "$RUNNER_TEMP/macos-products.tar.gz" Products
      - uses: actions/upload-artifact@v7
        with:
          name: macos-products
          path: ${{ runner.temp }}/macos-products.tar.gz
          if-no-files-found: error
```

The scheme must include the tests that this job runs.
The test command can build its test products on the Mac.
The example retains unsigned products for CI validation, not a notarized distribution.

GitHub's [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) lists the architecture and available labels.
Use an explicit architecture when your app or dependencies require one.

## Compile the app on Linux

Use this path when compilation is the part that you want on Linux.
The same [private SDK runner prerequisites](github-actions.md#choose-the-runner) apply.

The destination selects macOS instead of iOS:

```bash
sweetpad build --project MyMacApp.xcodeproj --scheme MyMacApp \
  --configuration Release --destination 'generic/platform=macOS' \
  --json --non-interactive \
  -- -derivedDataPath "$RUNNER_TEMP/DerivedData" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
```

Build products use the macOS bundle layout, including the Contents directory.
Keep the complete app and embedded frameworks when you package the result:

```bash
tar -C "$RUNNER_TEMP/DerivedData/Build" \
  -czf "$RUNNER_TEMP/macos-products.tar.gz" Products
```

Upload the tar archive with the artifact action from the iOS workflow.
A subsequent Mac job can download and extract it for native validation or a separate signing stage.

The Linux build path does not provide a macOS runtime.
Frameworks, run scripts, or assets that require a native Apple tool can still require a Mac build.
Check [supported features](reference.md) before moving an existing job.

## Tests and distribution

Automatic Linux XCTest manifests currently cover the documented iOS test forms.
For a complete macOS test workflow, use the hosted-Mac example or your existing native test pipeline.
Do not assume that an iOS test-manifest recipe also covers macOS tests.

Developer ID distribution requires the matching certificate, entitlements, hardened-runtime settings, and Apple notarization.
Keep that release stage on a Mac with its existing signing configuration.
The unsigned build example does not replace this stage.

The CLI's Linux development IPA export applies to iOS device archives.
It is not a macOS distribution workflow.

## Verify the actual products

Use the architecture and deployment targets that your application supports.
After a toolchain change, run the app and its tests on a compatible Mac.
A successful Linux compilation establishes build coverage; native execution establishes runtime coverage.

The iPhone transport guide does not apply to macOS app execution.
For shared compiler and SDK behavior, see [how Linux builds work](how-it-works.md).
