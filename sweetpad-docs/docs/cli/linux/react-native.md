---
sidebar_label: React Native
---

# React Native iPhone apps on Linux

SweetPad builds the iOS workspace after JavaScript dependencies, native code generation, and Pods are ready.
It does not replace those preparation steps.

The validated examples use React Native 0.85.3 with Hermes.
The MMKV example also uses NitroModules 0.35.9.
Release products pass iPhone startup checks, and MMKV passes storage and relaunch checks.

:::info Prerequisites
Complete [Ubuntu and USB setup](ubuntu-usb.md) first.
The pinned Hermes compiler is a Linux x86_64 executable.
The validated ARM64 environment runs it through OrbStack's Linux binary translation.
:::

## Prepare the validated example

The repository includes a pinned preparation recipe for the community template.
It captures dependency locks and applies the same scene-lifecycle compatibility patch to native and Linux controls.

Get the matching helper sources:

```bash
git clone --branch cli-v0.1.18 --depth 1 https://github.com/sweetpad-dev/sweetpad.git "$HOME/src/sweetpad"
cd "$HOME/src/sweetpad"
bash sweetpad-lib/ci/linux-framework-tools.sh
source "$HOME/.cache/sweetpad/framework-tools/env.sh"
```

The helper installs Node 24.22.2, Ruby 3.4.8, Bundler 4.0.20, and the frozen JavaScript tools.
Its files stay in a private tool directory.

Prepare a fresh template checkout:

```bash
python3 -B sweetpad-lib/ci/linux-project-prepare.py \
  --only react-native-template \
  --corpus-root "$HOME/sweetpad-examples" \
  --tool-bin "$HOME/.cache/sweetpad/framework-tools/js/node_modules/.bin" \
  --gem-home "$HOME/.cache/sweetpad/framework-tools/gems" \
  --out "$HOME/sweetpad-examples/template-preparation.json"
```

This uses the captured npm, Ruby, and Pod dependency graph.
Linux metadata adapters supply the Xcode information that dependency preparation expects.
Generated Pod scripts select the portable signing tool through PATH.

The recipe also records paths for Node and generated dependencies.
An existing Pods directory from another machine is not a substitute for preparation on this host.

## Build the iOS workspace

Select the Linux Hermes compiler explicitly:

```bash
APP_ROOT="$HOME/sweetpad-examples/react-native-template/template"

sweetpad build --workspace "$APP_ROOT/ios/HelloWorld.xcworkspace" \
  --scheme HelloWorld --configuration Release \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- -derivedDataPath "$HOME/builds/ReactNative" \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO \
  HERMES_CLI_PATH="$APP_ROOT/node_modules/hermes-compiler/hermesc/linux64-bin/hermesc"
```

This unsigned build verifies compilation.
For the phone, configure [development signing](signing-and-testing.md#configure-development-signing).
Remove the unsigned setting. Then rebuild.

Install and launch the resulting app using [the USB workflow](ubuntu-usb.md#build-and-install-the-iphone-app).
Use the product path from JSON output rather than assuming a bundle name.

## Release and Debug behavior

Release builds embed their JavaScript and Hermes products.
The validated Release app launches without a Metro server.

Debug builds need a reachable Metro server.
Start it from the app directory. Configure the iPhone's bundle-server address for your network.
The VM's loopback address is not the phone's loopback address.

Keep the host compiler and target artifacts distinct.
The Hermes compiler must execute on Linux; the app and embedded frameworks must target iOS.

## MMKV

The same preparation recipe supports the separate MMKV example:

```bash
cd "$HOME/src/sweetpad"
python3 -B sweetpad-lib/ci/linux-project-prepare.py \
  --only react-native-mmkv \
  --corpus-root "$HOME/sweetpad-examples" \
  --tool-bin "$HOME/.cache/sweetpad/framework-tools/js/node_modules/.bin" \
  --gem-home "$HOME/.cache/sweetpad/framework-tools/gems" \
  --out "$HOME/sweetpad-examples/mmkv-preparation.json"
```

Its workspace is in the example's iOS directory, and its scheme is MmkvExample.
Its Hermes compiler comes from the repository-root dependency directory.

The current build path does not need the former MMKV debug-thunk fixture shim.
The setup script includes the Swift Build correction for the affected generated C++ bridge helpers.
This covers the pinned compiler and example; other Swift/C++ combinations can expose different issues.

## Use your own app

Prepare your app's declared Node, Ruby, code-generation, and Pod dependencies.
Then pass its workspace and scheme to the same build command.
Adjust the Hermes path for your package layout.

The pinned recipe is a reference workflow, not a generator for arbitrary React Native applications.
Other versions and native modules need their own validation.
A preparation script that requires macOS can still require a Mac before Linux compilation.

When you compare native and Linux products, keep the same source and dependency locks.

## Troubleshooting

| Symptom | What to inspect |
| --- | --- |
| Hermes cannot execute | Check its executable format and the host's x86_64 translation support. |
| A Pod preparation tool asks for Xcode | Use the validated metadata adapters or prepare that dependency on a Mac. |
| Generated headers or code are missing | Complete code generation and Pod preparation before the build. |
| The app cannot load JavaScript | Check Release bundling or the Debug Metro address. |
| The SDK reports a scene-lifecycle failure | Check the app's scene adoption; the pinned template needs its explicit compatibility patch. |

The [CI guide](github-actions.md) explains where preparation belongs in a Linux build job.
