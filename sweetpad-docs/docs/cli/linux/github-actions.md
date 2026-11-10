---
sidebar_label: GitHub Actions for iOS
---

# iOS builds in GitHub Actions

A Linux job can compile iPhone products, generate test manifests, and create development archives.
Apple test execution still runs on a Mac with Xcode.

## Choose the runner

| Job | Runner |
| --- | --- |
| Run the CLI without Apple SDKs | A hosted Ubuntu 24.04 runner for the package architecture. |
| Compile an iOS app on Linux | A provisioned ARM64 or x86_64 Linux runner with your private SDK and matching host tools. |
| Run iOS Simulator tests | A Mac with Xcode and a compatible simulator runtime. |
| Install on an iPhone | A trusted USB or paired Wi-Fi connection from the selected host. |

The validated app-build setup uses a Linux VM on Apple hardware.
Use SDK files under [Apple's Xcode agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
A hosted Ubuntu runner supplies neither your licensed SDK environment nor a connected iPhone.

Provision the app-build runner using [Ubuntu setup](ubuntu-usb.md).
Current source builds also provide [SDK export and setup commands](sdk-transfer.md).
With those commands, source `$HOME/.config/sweetpad/linux-env.sh` in each build step and run `sweetpad doctor --target ios` before compilation.
Its user needs the Linux Swift toolchain, Swift Build, macro plugin, and extracted SDK on the documented paths.
The example uses the custom runner label `sweetpad-linux`.

### Repository validation workflows

The `Linux packages` workflow builds release CLI archives on both `ubuntu-24.04` (x64) and `ubuntu-24.04-arm`.
It runs on relevant pull requests and main pushes. Each job verifies installation in a clean Ubuntu container.
The library workflow also runs Rust tests and clippy on both hosts.

SDK-dependent app checks stay in the existing manually dispatched `Linux Apple SDK integration` workflow.
They require a provisioned self-hosted SDK runner and do not run on public pull requests.

### x86_64 runners

Current source builds support the `X64` self-hosted runner label and ARM64 iPhone targets.
The x64 archive is prepared for the next release. The example that follows keeps the released 0.1.18 ARM64 archive.
After an x64 package release, select `linux-x64` and use `runs-on: [self-hosted, Linux, X64, sweetpad-linux]`.
Keep the iPhone destination and `ARCHS=arm64` settings unchanged.
An SDK imported on ARM64 contains ARM64 host tools. Import the private export on the x64 runner to select x64 tools.

## Build and retain iPhone products

This workflow installs the released CLI into a temporary prefix.
The toolchain and SDK are existing runner prerequisites.

Replace the project and scheme names with your own:

```yaml
name: iPhone build on Linux
on:
  workflow_dispatch:
permissions:
  contents: read

jobs:
  build:
    runs-on: [self-hosted, Linux, ARM64, sweetpad-linux]
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false
      - name: Install SweetPad
        shell: bash
        run: |
          set -euo pipefail
          CLI_VERSION=0.1.18
          mkdir "$RUNNER_TEMP/cli"
          cd "$RUNNER_TEMP/cli"
          curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz"
          curl -fLO "https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION/sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz.sha256"
          sha256sum -c "sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz.sha256"
          tar -xzf "sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz"
          bash install.sh --prefix "$RUNNER_TEMP/sweetpad"
          echo "$RUNNER_TEMP/sweetpad/bin" >> "$GITHUB_PATH"
          echo "$HOME/bin" >> "$GITHUB_PATH"
          echo "$HOME/swift/swift-6.4.2-RELEASE/usr/bin" >> "$GITHUB_PATH"
      - name: Build iPhone products
        shell: bash
        run: |
          set -euo pipefail
          export PATH="$RUNNER_TEMP/sweetpad/bin:$HOME/swift/swift-6.4.2-RELEASE/usr/bin:$HOME/bin:$PATH"
          sweetpad build --project MyApp.xcodeproj --scheme MyApp \
            --configuration Release --destination 'generic/platform=iOS' \
            --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" \
            ARCHS=arm64 CODE_SIGNING_ALLOWED=NO > "$RUNNER_TEMP/build.json"
          tar -C "$RUNNER_TEMP/DerivedData/Build" \
            -czf "$RUNNER_TEMP/Products.tar.gz" Products
      - uses: actions/upload-artifact@v7
        with:
          name: iphone-products
          path: |
            ${{ runner.temp }}/Products.tar.gz
            ${{ runner.temp }}/build.json
          if-no-files-found: error
```

The example creates unsigned device products for a build check.
[Development signing](signing-and-testing.md#configure-development-signing) is required before installation on a phone.

The tar archive preserves executable permissions and framework symbolic links.
Upload the archive rather than individual app-bundle files.
Retain build products and reports, not the private SDK or signing keys.

## Add development signing

Store the P12, its password, and profiles in your CI secret store.
Decode the files into a private directory under `RUNNER_TEMP` with `umask 077`.
Set these CLI variables for the signed build:

| Variable | CI value |
| --- | --- |
| `SWEETPAD_CODE_SIGN_P12` | Path to the decoded identity file. |
| `SWEETPAD_CODE_SIGN_P12_PASSWORD` | The identity's password from the secret store. |
| `SWEETPAD_PROVISIONING_PROFILES` | Directory containing matching profiles. |

Remove `CODE_SIGNING_ALLOWED=NO` from the build.
Remove temporary signing files in a final cleanup step that uses `if: always()`.

This example uses a manual trigger.
Keep private SDK and signing jobs on trusted code. Untrusted pull-request code can read a self-hosted runner's files.
GitHub documents [self-hosted runner operation](https://docs.github.com/en/actions/reference/runners/self-hosted-runners) and [runner labels](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).

## Build tests on Linux

Replace the app-build command with a test-build command:

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$RUNNER_TEMP/DerivedData" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO
```

Retain the complete Products directory, including its generated XCTest run manifest.
Check `xctestrunPath` before publishing the test artifact.
See [test plans and supported policies](signing-and-testing.md#select-a-test-plan).

A following Mac job can extract the same archive and invoke `sweetpad test run --xctestrun`.
That uses the Linux products without a Mac rebuild.
Choose a simulator with the matching architecture and a sufficient OS version.

GitHub currently lists `macos-15` as an ARM64 hosted runner.
Check [GitHub's runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners) when you change labels or architectures.

## Deploy to a connected iPhone

Use a trusted runner with a paired phone and the pinned Python adapter.
Serialize jobs that share the same phone.
The signed build must use a profile that includes that device.

After the signed build, install its reported product and launch it:

```bash
export SWEETPAD_DEVICE_PYTHON="$HOME/.local/share/sweetpad/device-tools/bin/python"
APP_PATH="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["data"]["productPath"])' "$RUNNER_TEMP/build.json")"
sweetpad device install "$APP_PATH" --device "$IPHONE_UDID" \
  --json --non-interactive
sweetpad app launch --project MyApp.xcodeproj --scheme MyApp \
  --configuration Release --device-id "$IPHONE_UDID" \
  --derived-data-path "$RUNNER_TEMP/DerivedData" --json --non-interactive
```

Set `IPHONE_UDID` in your runner configuration.
For live logs and USB ownership, follow [the iPhone guide](ubuntu-usb.md#connect-the-iphone).
Distribution delivery through App Store Connect remains a separate Xcode workflow.

## Framework apps and repeatable builds

React Native and Flutter need their own dependency preparation before the Xcode project can compile.
Use the [React Native](react-native.md) or [Flutter](flutter.md) guide for the validated versions and Linux-specific tools.

Keep dependency locks and pin the CLI version.
A fresh DerivedData path prevents CI from accepting stale app products.
Cache compiler and package inputs separately from signing material and SDK files.

For macOS app targets, use the separate [macOS CI guide](macos-ci.md).
