---
sidebar_label: GitHub Actions for iOS
---

# iOS builds in GitHub Actions

A Linux job can build iPhone apps and test products, archive them, and upload them to TestFlight.
Running Apple tests still needs a Mac.

## Prepare the runner

Hosted Ubuntu runners don't have your Apple SDK, so iOS builds need a self-hosted runner.
Set it up once with [the Apple toolchain](sdk-transfer.md), then run `sweetpad doctor --target ios` as the runner's user.
The validated runner is a Linux VM on Apple hardware, and the example below uses the custom label `sweetpad-linux`.

Run these jobs only for trusted code, because a pull request's code can read a self-hosted runner's files.
GitHub documents [self-hosted runners](https://docs.github.com/en/actions/reference/runners/self-hosted-runners).

## Build iPhone products

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
          CLI_VERSION=0.1.26
          BASE="https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION"
          mkdir "$RUNNER_TEMP/cli" && cd "$RUNNER_TEMP/cli"
          curl -fLO "$BASE/sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz"
          curl -fLO "$BASE/sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz.sha256"
          sha256sum -c "sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz.sha256"
          tar -xzf "sweetpad-cli-$CLI_VERSION-linux-arm64.tar.gz"
          install -Dm755 sweetpad "$RUNNER_TEMP/sweetpad/bin/sweetpad"
          mkdir -p "$RUNNER_TEMP/sweetpad/libexec"
          cp -R libexec/sweetpad "$RUNNER_TEMP/sweetpad/libexec/sweetpad"
          echo "$RUNNER_TEMP/sweetpad/bin" >> "$GITHUB_PATH"
      - name: Build iPhone products
        shell: bash
        run: |
          set -euo pipefail
          source "$HOME/.config/sweetpad/linux-env.sh"
          sweetpad build --project MyApp.xcodeproj --scheme MyApp \
            --configuration Release --destination 'generic/platform=iOS' \
            --json --non-interactive \
            -- -derivedDataPath "$RUNNER_TEMP/DerivedData" \
            CODE_SIGNING_ALLOWED=NO > "$RUNNER_TEMP/build.json"
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

Each step sources the environment file that setup wrote, which selects its toolchain and SDK.
The tar archive keeps the products' permissions and symbolic links, which artifact uploads lose.
On an x86_64 runner, use the `linux-x64` archive and the `X64` label. The build itself doesn't change.

## Add signing

Keep the P12 file, its password, and the profiles in your secret store.
Decode them into a private folder under `RUNNER_TEMP` with `umask 077`, and set `SWEETPAD_CODE_SIGN_P12`, `SWEETPAD_CODE_SIGN_P12_PASSWORD`, and `SWEETPAD_PROVISIONING_PROFILES` as in [the signing guide](signing-and-testing.md#configure-development-signing).
Then drop `CODE_SIGNING_ALLOWED=NO`, and delete the signing files in a final step with `if: always()`.

## Build tests

Swap the build command for a test build, and keep the Products archive:

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath "$RUNNER_TEMP/DerivedData" CODE_SIGNING_ALLOWED=NO
```

A later job on a Mac, such as `macos-15`, extracts the archive and runs it with `sweetpad test run --xctestrun`, as in [running the tests on a Mac](signing-and-testing.md#run-the-tests-on-a-mac).

## Install on a connected iPhone

On a runner with a [paired phone](ubuntu-usb.md#connect-the-iphone), install the signed build and launch it.
Run one job at a time per phone, and use a profile that lists it:

```bash
export SWEETPAD_DEVICE_PYTHON="$HOME/.local/share/sweetpad/device-tools/bin/python"
APP_PATH="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["data"]["productPath"])' "$RUNNER_TEMP/build.json")"
sweetpad device install "$APP_PATH" --device "$IPHONE_UDID" --json --non-interactive
sweetpad app launch --project MyApp.xcodeproj --scheme MyApp \
  --configuration Release --device-id "$IPHONE_UDID" \
  --derived-data-path "$RUNNER_TEMP/DerivedData" --json --non-interactive
```

## Upload to TestFlight

With your Apple Distribution identity, App Store profiles, and an App Store Connect API key in the secret store, replace the build step:

```yaml
      - name: Archive and upload
        shell: bash
        env:
          SWEETPAD_ASC_KEY_ID: ${{ secrets.ASC_KEY_ID }}
          SWEETPAD_ASC_ISSUER_ID: ${{ secrets.ASC_ISSUER_ID }}
          SWEETPAD_ASC_KEY: ${{ secrets.ASC_KEY }}
        run: |
          set -euo pipefail
          source "$HOME/.config/sweetpad/linux-env.sh"
          sweetpad archive --project MyApp.xcodeproj --scheme MyApp \
            --configuration Release --destination 'generic/platform=iOS' \
            --upload --output-file "$RUNNER_TEMP/dist" \
            --json --non-interactive \
            -- CURRENT_PROJECT_VERSION="$GITHUB_RUN_NUMBER" > "$RUNNER_TEMP/archive.json"
```

The step ends when Apple accepts or rejects the build, and a rejection fails the job.
Each upload needs a new build number, which the run number supplies here.

React Native and Flutter apps need their own preparation first; see the [React Native](react-native.md) and [Flutter](flutter.md) guides.
