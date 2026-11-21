---
sidebar_label: React Native
---

# React Native iPhone apps on Linux

Build the iOS workspace after installing JavaScript dependencies, generating native code, and installing Pods. The workspace contains the app and the generated Pods targets that the app links against.

## Use your own app

On Linux, install the [CLI](install.md) and [Apple toolchain](sdk-transfer.md). Prepare the Node, Ruby, code-generation, and Pod dependencies your app declares. A preparation script that calls a macOS-only tool can require a Mac for that step.

From the app directory, build the generated workspace. Replace the workspace and scheme names with yours.

```bash
sweetpad build --workspace ios/MyApp.xcworkspace --scheme MyApp \
  --configuration Release --destination 'generic/platform=iOS' \
  --json --non-interactive -- CODE_SIGNING_ALLOWED=NO
```

If your project uses Hermes, make sure its compiler runs on the Linux host. The [template build](#build-the-ios-workspace) shows how to select it with `HERMES_CLI_PATH`.

## Prepare the validated example

The validated examples use React Native 0.85.3 with Hermes.
The MMKV example also uses NitroModules 0.35.9.
Release products pass iPhone startup checks, and MMKV passes storage and relaunch checks.

:::info Prerequisites
The pinned Hermes compiler is a Linux x86_64 executable.
The validated ARM64 environment runs it through OrbStack's Linux binary translation.
:::

The repository includes a pinned preparation recipe for the community template.
The recipe pins dependency versions and applies the scene-lifecycle patch used when testing the example on both macOS and Linux.

On Linux, start in a SweetPad source checkout matching the CLI build you are testing. Run these commands from the repository root.

```bash
bash sweetpad-lib/ci/linux-framework-tools.sh
source "$HOME/.cache/sweetpad/framework-tools/env.sh"
```

The helper downloads Node 24.22.2, builds Ruby 3.4.8, and installs Bundler 4.0.20 and the locked JavaScript tools. By default it installs under `~/.cache/sweetpad/framework-tools`. Sourcing its environment file puts those tools on your shell’s PATH.

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
Remove `CODE_SIGNING_ALLOWED=NO` and rebuild.

Install and launch the resulting app using [the USB workflow](ubuntu-usb.md#build-install-and-launch).
Use the product path from JSON output rather than assuming a bundle name.

## Release and Debug behavior

Release builds embed their JavaScript and Hermes products.
The validated Release app launches without a Metro server.

Debug builds need a reachable Metro server.
Start it from the app directory. Configure the iPhone's bundle-server address for your network.
Use an address the phone can reach. On the phone, `localhost` refers to the phone itself.

Hermes runs during the build to compile JavaScript to bytecode. Its compiler must execute on Linux. The app and embedded frameworks must target iOS.

## MMKV

The same preparation recipe supports the separate MMKV example:

```bash
python3 -B sweetpad-lib/ci/linux-project-prepare.py \
  --only react-native-mmkv \
  --corpus-root "$HOME/sweetpad-examples" \
  --tool-bin "$HOME/.cache/sweetpad/framework-tools/js/node_modules/.bin" \
  --gem-home "$HOME/.cache/sweetpad/framework-tools/gems" \
  --out "$HOME/sweetpad-examples/mmkv-preparation.json"
```

Use `example/ios/MmkvExample.xcworkspace` and the `MmkvExample` scheme under the prepared MMKV checkout.
Its Hermes compiler comes from the repository-root dependency directory.

When SweetPad merges the Swift-generated C++ header, it corrects two recursive generic bridge helpers used by the pinned MMKV example. The example therefore builds without the old Podfile patch for debug thunks.

## Troubleshooting

| Symptom | What to inspect |
| --- | --- |
| Hermes cannot execute | Check its executable format and the host's x86_64 translation support. |
| A Pod preparation tool asks for Xcode | Use the validated metadata adapters or prepare that dependency on a Mac. |
| Generated headers or code are missing | Complete code generation and Pod preparation before the build. |
| The app cannot load JavaScript | Check Release bundling or the Debug Metro address. |
| The SDK reports a scene-lifecycle failure | Check the app's scene adoption; the pinned template needs its explicit compatibility patch. |

In GitHub Actions, prepare the dependencies before the Ubuntu build step and pass the generated workspace with `--workspace`. See the [Ubuntu workflow](github-actions.md#build-an-unsigned-iphone-app-on-ubuntu).
