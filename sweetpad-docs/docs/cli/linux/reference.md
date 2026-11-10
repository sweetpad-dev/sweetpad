---
sidebar_label: Support and configuration
---

# Linux support and configuration

The released archive targets Ubuntu 24.04 ARM64.
The validated setup uses an Apple-hardware Linux VM, Swift 6.4.2, Swift Build 6.4.0, and SDK data from Xcode.

## Platform coverage

| Operation | Status |
| --- | --- |
| Build iOS apps and extensions | Supported by the experimental Linux engine. |
| Sign development iPhone products | Supported with a P12 identity and matching profiles. |
| Discover, install, launch, stop, and log an iPhone app | Validated through Linux USB and Wi-Fi. |
| Build iOS unit and UI test products | Supported, including the documented automatic manifests. |
| Execute Apple tests | Requires a Mac with Xcode. |
| Operate an iOS Simulator | Requires a Mac. |
| Build macOS apps | Supported for the validated resource and compiler paths; native execution requires a Mac. |
| Build tvOS, watchOS, and visionOS simulator targets | Compilation and linking coverage exists; runtime and device signing remain unverified. |
| Build Mac Catalyst targets on Linux | Remains unverified. |
| Export a development iOS IPA | Supported from a signed device archive. |
| Export distribution IPAs or custom export options | Requires Xcode. |
| Debug a device or inspect device readiness on Linux | Unsupported. |

Current Xcode and Swift Build reject deprecated WatchKit extension products.
Use modern standalone watchOS apps for new platform validation.

## Project and resource support

The portable build path covers these common inputs:

- Local and remote Swift packages, including binary XCFrameworks
- Swift, Objective-C, C, and C++ targets
- Frameworks, static libraries, app extensions, and widgets
- Asset catalogs, storyboards, XIBs, String Catalogs, and ordinary strings
- Core Data models and common mapping models
- Supported SwiftData, Predicate, Preview, SwiftUI Entry, and TipKit macros
- Package macros whose sources build for the Linux host
- Supported App Intents metadata and shortcut language models.

Explicit schemes from resolved Git packages are available after dependency resolution.
Listing schemes does not fetch missing packages.
Package manifests run on Linux. Host-only conditions that select Apple resources or libraries can require a compatibility patch.

Metal shaders, Core ML model classes, SceneKit assets, and RealityKit assets require native tools.
Scripts that import Darwin or call other macOS-only tools need adaptation or a Mac preparation stage.
Advanced Core Data mapping expressions remain outside the supported mapping-model forms.

Phone startup checks do not validate App Groups, push, payments, or extension behavior.
Those flows need matching profiles and their own execution checks.

### Modern icons and stickers

Icon Composer support produces static light, dark, and tinted appearances.
Interactive glass lighting and native material effects remain unsupported.

Sticker packs support ordered PNG, GIF, and JPEG members and message-extension icons.
Sticker sequences and localized sticker-pack strings remain unsupported.

### Swift and C++ interoperability

The MMKV validation uses the current generated-header correction rather than its former fixture shim.
The affected bridge helpers use the supported lazy inline-helper behavior.
Other compiler versions and interoperability forms need separate validation.

## App Intents

The portable processors generate registration metadata, extracted strings, and shortcut language models.
Supported forms include primitive parameters, optional and array values, file images, dialog or snippet results, enums, basic entities, widget intents, and basic shortcuts.

Dynamic frameworks and embedded extensions retain separate registrations.
Unsupported schemas, static dependency registration, and unsupported language-model options fail explicitly.
Siri voice invocation on a physical device remains unverified.

Localized shortcut models use the app's translated names and AppShortcuts string tables.
The trainer emits per-language models and removes models for deleted shortcuts.
AssistantIntents catalogs and catalog variations remain unsupported.

### Localized phrase resources

Mac execution of toolchain setup exports the system phrase resources into the private SDK.
For an independently prepared Ubuntu host, export those resources on the Mac using the setup sources from the archive:

```bash
python3 /path/to/sweetpad-install/setup/sweetpad-lib/ci/linux-appintents-resources.py \
  --xcode /Applications/Xcode.app --out /path/to/appintents-phrases.json
```

Transfer the export privately, then provide it to Linux setup:

```bash
SWEETPAD_SKIP_RUST=1 \
  SWEETPAD_APPINTENTS_RESOURCE_EXPORT=/path/to/appintents-phrases.json \
  XCODE_APP=/path/to/Xcode.app \
  bash "$HOME/.local/share/sweetpad/setup/sweetpad-lib/ci/linux-setup.sh"
```

Missing localized resources produce a preparation error.
The export is SDK data and stays outside your source repository and public artifacts.

## Environment variables

For export, import, setup, and Linux diagnostics, see [SDK export and setup](sdk-transfer.md).
Those convenience commands require a current source build until the next CLI release.

| Variable | Purpose |
| --- | --- |
| `SWEETPAD_DARWIN_SDK` | Select another extracted Darwin SDK bundle. |
| `SWEETPAD_SWBUILD` | Select the Swift Build driver instead of the executable on PATH. |
| `SWEETPAD_MACROS` | Select the macro launcher instead of the executable on PATH. |
| `SWEETPAD_CODE_SIGN_P12` | Select the P12 development identity. |
| `SWEETPAD_CODE_SIGN_P12_PASSWORD` | Supply that identity's password. |
| `SWEETPAD_PROVISIONING_PROFILES` | Select a directory of matching profiles. |
| `SWEETPAD_DEVICE_PYTHON` | Select Python with the pinned modern device adapter. |
| `SWEETPAD_DEVICE_NETWORK_CONFIG` | Select a private Wi-Fi device configuration. |
| `SWEETPAD_SKIP_RUST` | Setup only: use `1` to omit Rust for the prebuilt CLI. |
| `SWEETPAD_REFRESH_SDK` | Setup only: use `1` to refresh an existing SDK extraction. |
| `SWEETPAD_SDK_ARCHIVE` | Setup script only: import a private archive instead of reading an Xcode directory. |

Run `sweetpad help environment` for runtime variables.
The two setup controls apply to the setup script.

## Wi-Fi transport

With trusted USB available, create an isolated network record:

```bash
sweetpad device pair-network IPHONE_UDID \
  --out "$HOME/.local/share/sweetpad/iphone-network" --json --non-interactive
export SWEETPAD_DEVICE_NETWORK_CONFIG="$HOME/.local/share/sweetpad/iphone-network/devices.json"
sweetpad device list --json --non-interactive
```

The command preserves existing default pairing records and rejects an existing output directory.
The new directory contains private credentials.
Discovery uses multicast advertisements or the system DNS-SD resolver.

The validated OrbStack bridge uses DNS-SD because raw LAN multicast does not cross the bridge.
For networks without discovery, the configuration can select an explicit RemotePairing host and port.

Run `sweetpad help linux-devices` for configuration details and bounded app-log commands.
See [Ubuntu and USB setup](ubuntu-usb.md) for the primary iPhone workflow.
