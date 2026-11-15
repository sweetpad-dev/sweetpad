---
sidebar_label: Support and configuration
---

# Linux support and configuration

Release 0.1.22 provides static ARM64 and x86_64 packages, signed APT updates, Packslip metadata, and a CLI container image.
See [Install the CLI](install.md) for installation and upgrades.
Apple toolchain setup supports Ubuntu 24.04 and the validated [Debian and Arch hosts](distributions.md).

## Host and target architectures

| Linux host | Package suffix | iPhone target |
| --- | --- | --- |
| ARM64 (`aarch64`) | `linux-arm64` | ARM64 (`arm64-apple-ios`) |
| x86_64 (`amd64`) | `linux-x64` | ARM64 (`arm64-apple-ios`) |

The CLI, Swift compiler, Swift Build service, macro plugins, and SDK tools must match the Linux host.
The build destination selects the app platform and target architecture.
For example, `--destination 'generic/platform=iOS'` selects an ARM64 iPhone build on either host.
Simulator and macOS destinations default to the host architecture. An explicit destination `arch=` selects another supported architecture.

The same private SDK export can supply Apple files to both hosts.
SDK import installs host tools for the importing machine. An installed SDK bundle contains host executables and needs replacement after a host-architecture change.
Flutter AOT and React Native keep the additional host-tool requirements in their respective guides.

The validated setup uses an Apple-hardware Linux VM, Swift 6.4.2, Swift Build 6.4.0, and SDK data from Xcode.
Setup builds Swift Build with SweetPad's patches and records which ones it used.
A newer SweetPad can carry newer patches, so after an upgrade `sweetpad doctor` warns until `sweetpad setup` runs again.
The same report lists the tvOS, watchOS, and visionOS platforms that the SDK includes.

## Platform coverage

| Operation | Status |
| --- | --- |
| Build iOS apps and extensions | Supported by the experimental Linux engine. |
| Sign iPhone and macOS products | Supported with P12 identities and matching profiles. |
| Discover, install, launch, stop, and log an iPhone app | Validated through Linux USB and Wi-Fi. |
| Build iOS unit and UI test products | Supported, including the documented automatic manifests. |
| Execute Apple tests | Requires a Mac with Xcode. |
| Operate an iOS Simulator | Requires a Mac. |
| Build macOS apps | Supported for the validated resource and compiler paths; native execution requires a Mac. |
| Build tvOS, watchOS, and visionOS simulator targets | Compilation and linking coverage exists; runtime and device signing remain unverified. |
| Build Mac Catalyst targets on Linux | Remains unverified. |
| Export development, ad hoc, enterprise, and App Store Connect IPAs | Supported. Each export signs the app again for its method. |
| Export a Mac App Store package, or a Developer ID app and package | Supported. |
| Upload to App Store Connect | Supported with an App Store Connect API key. |
| Notarize and staple a Developer ID app or package | Supported with a team API key. |
| Custom export options | Supported for the keys SweetPad acts on. Other keys that change the result are refused by name. |
| Build universal macOS archives | Linux archives contain arm64 only. |
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
- Build-tool plugins on package targets, such as SwiftLint or SwiftGen applied in a package
- Supported App Intents metadata and shortcut language models.

Explicit schemes from resolved Git packages are available after dependency resolution.
Listing schemes does not fetch missing packages.
Package manifests run on Linux. Host-only conditions that select Apple resources or libraries can require a compatibility patch.

A build-tool plugin on an Xcode project target doesn't run on Linux.
Xcode runs those plugins itself through `XcodeProjectPlugin`, which the Linux Swift toolchain doesn't include.
The build warns for each such plugin and goes on without the files it would generate.

Metal shaders, Core ML model classes, SceneKit assets, and RealityKit assets require native tools.
When a build needs a tool that Linux doesn't have, such as `metal`, it stops and names the tool and the file that needs it.
Scripts that import Darwin or call other macOS-only tools need adaptation or a Mac preparation stage.
Linux file names are case-sensitive and a Mac's usually aren't, so a project that names `Core.swift` for a file saved as `core.swift` builds on a Mac and fails on Linux.
The failed build's tip names the file as it is on disk.

`sweetpad open dd` opens the DerivedData folder with `xdg-open`.
`sweetpad open config` opens the config file in `$VISUAL` or `$EDITOR` at a terminal, and with `xdg-open` otherwise.
`open xcode` and `open sim` need a Mac, so on Linux they exit with code 5 (`tool_missing`).
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

`sweetpad sdk export` runs on your Mac and includes the system phrase resources in the private SDK archive.
Transfer that archive and its checksum privately, then run `sweetpad setup --sdk /path/to/apple-sdk.tar.gz` on Linux.
See [SDK export and setup](sdk-transfer.md) for the complete commands.

Missing localized resources produce a preparation error.
The export is SDK data and stays outside your source repository and public artifacts.

## Environment variables

For export, import, setup, and Linux diagnostics, see [SDK export and setup](sdk-transfer.md).
Those convenience commands require CLI 0.1.19 or later.

| Variable | Purpose |
| --- | --- |
| `SWEETPAD_DARWIN_SDK` | Select another extracted Darwin SDK bundle. |
| `SWEETPAD_SWBUILD` | Select the Swift Build driver instead of the executable on PATH. |
| `SWEETPAD_MACROS` | Select the macro launcher instead of the executable on PATH. |
| `SWEETPAD_CODE_SIGN_P12` | Select the P12 identity files, separated by `:`. A file can hold several identities. |
| `SWEETPAD_CODE_SIGN_P12_PASSWORD` | Supply their password, or one per file separated by `:`. |
| `SWEETPAD_PROVISIONING_PROFILES` | Select a directory of matching profiles. |
| `SWEETPAD_ASC_KEY_ID` | Select the App Store Connect API key for uploads and notarization. |
| `SWEETPAD_ASC_ISSUER_ID` | Supply the team key's issuer ID. Notarization needs a team key. |
| `SWEETPAD_ASC_KEY_PATH` | Select the key's `.p8` file. |
| `SWEETPAD_ASC_KEY` | Supply the key file's text instead of a path. |
| `SWEETPAD_DEVICE_PYTHON` | Select Python with the pinned modern device adapter. |
| `SWEETPAD_DEVICE_TIMEOUT` | Set the finite device-action timeout in seconds (default: 180, maximum: 3600). |
| `SWEETPAD_DEVICE_DISCOVERY_TIMEOUT` | Set each device's discovery timeout in seconds (default: 30, maximum: 3600). |
| `SWEETPAD_DEVICE_NETWORK_CONFIG` | Select a private Wi-Fi device configuration. |
| `SWEETPAD_SKIP_RUST` | Setup only: use `1` to omit Rust for the prebuilt CLI. |
| `SWEETPAD_REFRESH_SDK` | Setup only: use `1` to refresh an existing SDK extraction. |
| `SWEETPAD_SDK_ARCHIVE` | Setup script only: import a private archive instead of reading an Xcode directory. |
| `SWEETPAD_APPLE_SESSION_FILE` | Select private Sweetpad session JSON for terminal login and Xcode downloads. |
| `SWEETPAD_ENGINE` | Select `swift-build` or `xcodebuild` for build commands. |
| `SWEETPAD_SETUP_JOBS` | Limit concurrent setup compiler jobs (default: 4). |
| `SWEETPAD_VM` | Select the OrbStack machine for Mac execution of the setup script. |
| `XCODE_APP` | Select the Xcode directory for the manual setup script. |
| `SWEETPAD_APPINTENTS_RESOURCE_EXPORT` | Supply the private localized phrase export during manual setup. |
| `SWEETPAD_NONINTERACTIVE` | Disable interactive prompts. Setup also prevents sudo password prompts. |

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
