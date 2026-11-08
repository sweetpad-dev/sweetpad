---
sidebar_label: Flutter
---

# Flutter iPhone apps on Linux

A Flutter iOS app needs both its Xcode project and Dart application code.
For a Release build, that code includes an iOS AOT snapshot.

The validated workflow uses Flutter 3.47.9 and its matching engine artifacts.
A Linux-built ARM64 iOS snapshot compiler produces the application snapshot.
The Flutter engine framework remains a prebuilt dependency.

:::info Prerequisites
Complete [Ubuntu setup](ubuntu-usb.md) first.
This recipe supports the pinned Flutter revision and an ARM64 Linux host.
It does not turn the standard Flutter iOS command into a general Linux build command.
:::

## Install the matching Flutter SDK

Use the tested Flutter revision:

```bash
git clone --filter=blob:none https://github.com/flutter/flutter.git "$HOME/src/flutter"
git -C "$HOME/src/flutter" checkout c197ca5199e9ee5661e865596c9c07294dfab51c
export PATH="$HOME/src/flutter/bin:$PATH"
flutter --version
flutter precache --ios
```

The cached iOS engine artifacts include a snapshot compiler for a Mac host.
Downloading those artifacts alone does not provide Linux AOT compilation.

## Get the helper sources

Use an existing checkout of this tag, or create one:

```bash
git clone --branch cli-v0.1.18 --depth 1 https://github.com/sweetpad-dev/sweetpad.git "$HOME/src/sweetpad"
cd "$HOME/src/sweetpad"
bash sweetpad-lib/ci/linux-framework-tools.sh
source "$HOME/.cache/sweetpad/framework-tools/env.sh"
sudo apt-get install ninja-build
```

The frozen preparation recipe expects the documented Node tools even for this sample.
The compiler recipe also needs Python 3.12 and the Linux Clang installation from toolchain setup.

## Build the iOS snapshot compiler

Use a fresh output directory:

```bash
python3 sweetpad-lib/ci/linux-flutter-snapshot.py \
  --flutter-root "$HOME/src/flutter" \
  --clang-root "$HOME/swift/swift-6.4.2-RELEASE/usr" \
  --out "$HOME/src/flutter-ios-snapshot" --jobs 4
```

The recipe checks the Flutter-pinned Dart revision and builds only the product-mode snapshot compiler.
It records source revisions, downloads, compiler identity, and the resulting executable hash.
The result is a Linux ARM64 executable that generates iOS ARM64 code.

This step needs a matching Dart source and engine configuration.
A compiler from another Flutter version can produce a snapshot that the engine rejects.

## Prepare the sample iOS project

The validated app is the platform-design sample.
Prepare it with a new source directory and the pinned dependency graph:

```bash
python3 -B sweetpad-lib/ci/linux-project-prepare.py \
  --only flutter-samples \
  --corpus-root "$HOME/sweetpad-examples" \
  --flutter-root "$HOME/src/flutter" \
  --tool-bin "$HOME/.cache/sweetpad/framework-tools/js/node_modules/.bin" \
  --out "$HOME/sweetpad-examples/flutter-preparation.json"
```

The recipe resolves Dart dependencies and writes Linux paths into the generated iOS configuration.
It also applies the minimum-deployment migration required by the pinned engine.

Flutter's iOS configuration-only command is unavailable on Linux.
The preparation recipe supplies those inputs for this pinned sample rather than invoking a Mac executable.

## Select the Linux compiler

Create a local-engine directory with the matching cached engine artifacts:

```bash
python3 sweetpad-lib/ci/linux-flutter-local-engine.py \
  --flutter-root "$HOME/src/flutter" \
  --compiler "$HOME/src/flutter-ios-snapshot/out/SweetpadIOS/gen_snapshot_product" \
  --out "$HOME/builds/flutter-local-engine"
```

This directory points to the existing engine framework and the new Linux compiler.
It does not replace the installed Flutter SDK.
Use a fresh output directory for another preparation.

## Build for the iPhone

Pass the local-engine settings to the Xcode project build:

```bash
sweetpad build \
  --project "$HOME/sweetpad-examples/flutter-samples/platform_design/ios/Runner.xcodeproj" \
  --scheme Runner --configuration Release \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/FlutterRunner" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO \
  FLUTTER_ROOT="$HOME/src/flutter" \
  FLUTTER_ENGINE="$HOME/builds/flutter-local-engine" \
  LOCAL_ENGINE=ios_release LOCAL_ENGINE_HOST=host_release
```

The unsigned command verifies the build.
For installation, configure [development signing](signing-and-testing.md#configure-development-signing).
Remove the unsigned setting. Then rebuild.
Then use the [USB installation and launch steps](ubuntu-usb.md#build-and-install-the-iphone-app).

The validated Release app launches twice on an iPhone with the Linux-generated snapshot.
A successful app launch also confirms that the matching engine can load that snapshot.

## Use another Flutter app

Prepare its iOS project, generated configuration, plugin metadata, and native dependencies for the Linux paths.
The pinned sample helper does not prepare arbitrary plugin workspaces.

Keep the Flutter revision, engine artifacts, and snapshot compiler aligned.
The engine's required deployment target must also match the project.
Use the app's declared dependency locks for reproducible preparation.

Native plugins and preparation scripts can add requirements beyond this sample.
Debug, profile, hot reload, and other Flutter versions require separate validation.

## Troubleshooting

| Symptom | What to inspect |
| --- | --- |
| The compiler recipe rejects the revision | Use the exact Flutter revision supported by the recipe. |
| A Mac executable runs during preparation | Check the selected snapshot compiler and local-engine settings. |
| The engine rejects the snapshot | Check Dart, engine, pointer-mode, and compiler compatibility. |
| Generated configuration is missing | Complete the pinned preparation step before building. |
| A native plugin fails | Validate its Pods, generated project, host tools, and target artifacts separately. |

The [GitHub Actions guide](github-actions.md) covers runner prerequisites and product retention.
