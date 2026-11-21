---
sidebar_label: GitHub Actions for iOS
---

# Build iOS apps on Ubuntu in GitHub Actions

An Ubuntu 24.04 runner can cross-compile your Xcode project for an ARM64 iPhone. The workflow imports your Apple SDK, builds the app without signing, and uploads the app bundle as a GitHub Actions artifact.

## Build an unsigned iPhone app on Ubuntu

:::warning Apple SDK license
Using an Apple SDK on a GitHub-hosted Ubuntu runner may violate your agreement with Apple. Apple’s standard terms limit execution to Apple-branded hardware running macOS. They also restrict uploading or hosting the SDK. Use the Ubuntu example only if your terms permit both the runner and the way you supply the SDK. See the opening notice and sections 2.2 and 2.7 of the [Xcode and Apple SDKs Agreement](https://www.apple.com/legal/sla/docs/xcode.pdf).
:::

Save the following workflow in `.github/workflows/ios-ubuntu.yml`. It runs manually on a fresh Ubuntu 24.04 x64 runner. Replace `MyApp.xcodeproj` and `MyApp` with your project and scheme.

1. Set the repository variable `SWEETPAD_CLI_VERSION` to the release number for a [Linux package with bundled helpers](install.md). The workflow downloads its x64-host Debian package. The iPhone app itself will be ARM64.
2. If your terms permit this transfer, [export the SDK on your Mac](sdk-transfer.md#export-it-on-your-mac). Make the archive available at a private HTTPS URL. Store that URL in the Actions secret `APPLE_SDK_EXPORT_URL`. Set the variable `APPLE_SDK_SHA256` to the 64-character digest from the export’s checksum file.
3. Choose a runner with room for the compressed archive, extracted SDK, Swift toolchain, and build products. Run the workflow on a trusted revision because build scripts can read the SDK.

```yaml
name: Build iPhone app on Ubuntu
on: workflow_dispatch
permissions:
  contents: read

jobs:
  build:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v7
        with:
          persist-credentials: false

      - name: Install SweetPad
        working-directory: ${{ runner.temp }}
        env:
          CLI_VERSION: ${{ vars.SWEETPAD_CLI_VERSION }}
        run: |
          set -euo pipefail
          package="sweetpad_${CLI_VERSION:?Set SWEETPAD_CLI_VERSION}-1_amd64.deb"
          release="https://github.com/sweetpad-dev/sweetpad/releases/download/cli-v$CLI_VERSION"
          curl -fsSLO "$release/$package"
          curl -fsSLO "$release/$package.sha256"
          sha256sum -c "$package.sha256"
          sudo apt-get update -qq
          sudo apt-get install -y "./$package"

      - name: Import the SDK and prepare Swift
        working-directory: ${{ runner.temp }}
        env:
          SDK_URL: ${{ secrets.APPLE_SDK_EXPORT_URL }}
          SDK_SHA256: ${{ vars.APPLE_SDK_SHA256 }}
        run: |
          set -euo pipefail
          umask 077
          curl -fsSL --proto '=https' "$SDK_URL" -o apple-sdk.tar.gz
          printf '%s  apple-sdk.tar.gz\n' "$SDK_SHA256" > apple-sdk.tar.gz.sha256
          sha256sum -c apple-sdk.tar.gz.sha256
          sweetpad setup --sdk "$PWD/apple-sdk.tar.gz" --non-interactive

      - name: Build and package the app
        run: |
          set -euo pipefail
          sweetpad build --project MyApp.xcodeproj --scheme MyApp \
            --configuration Release --destination 'generic/platform=iOS' \
            --json --non-interactive -- -derivedDataPath "$RUNNER_TEMP/DerivedData" \
            CODE_SIGNING_ALLOWED=NO > "$RUNNER_TEMP/build.json"
          app="$(jq -er '.data.productPath' "$RUNNER_TEMP/build.json")"
          tar -C "$(dirname "$app")" -czf "$RUNNER_TEMP/iphone-app.tar.gz" "$(basename "$app")"

      - uses: actions/upload-artifact@v7
        with:
          name: unsigned-iphone-app
          path: ${{ runner.temp }}/iphone-app.tar.gz
          if-no-files-found: error
```

The artifact contains the unsigned app bundle. The tar archive preserves its permissions and symbolic links. Only the app is uploaded. To install it on an iPhone, [build with development signing](signing-and-testing.md#configure-development-signing). For simulator tests, [build test products and transfer them to a Mac](signing-and-testing.md#run-the-tests-on-a-mac).

## Optional simulator tests on macOS {#build-and-test-an-ios-app}

Running iOS simulator tests requires macOS. This separate job builds the tests from your checkout and runs them with Xcode.

Choose a macOS runner with an Xcode version and simulator runtime that your project supports. Set the repository variable `IOS_SIMULATOR` to the name of an installed simulator. Replace the project and scheme below with your own.

```yaml
name: iOS tests
on: [push, pull_request]
permissions:
  contents: read

jobs:
  test:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v7
      - name: Install SweetPad
        run: brew install sweetpad-dev/tap/sweetpad
      - name: Build and run tests
        env:
          IOS_SIMULATOR: ${{ vars.IOS_SIMULATOR }}
        run: |
          sweetpad test run --project MyApp.xcodeproj --scheme MyApp \
            --on "${IOS_SIMULATOR:?Set IOS_SIMULATOR to an installed simulator}" \
            --json --non-interactive
```

Xcode supplies the SDK and runs the simulator on the same machine. The command builds the test targets and exits with code 3 if compilation or tests fail. For React Native projects, prepare the dependencies and open the generated workspace with `--workspace`. See [React Native preparation](react-native.md).
