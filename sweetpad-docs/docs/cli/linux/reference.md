---
sidebar_label: Support and configuration
---

# Linux support and configuration

## Platforms and operations

| Operation | On Linux |
| --- | --- |
| Build iOS apps, extensions, and widgets | Supported. |
| Build macOS apps | Supported. Running them needs a Mac. |
| Build tvOS, watchOS, and visionOS simulator targets | Builds and links. Running them and device signing are untested. |
| Build Mac Catalyst targets | Untested. |
| Sign iPhone and macOS products | Supported, with P12 identities and matching profiles. |
| Install, launch, stop, and stream logs from an iPhone app | Supported over USB and Wi-Fi. |
| Debug an app on a device | Not supported. |
| Build unit and UI test products | Supported, with automatic test manifests for iOS. |
| Run Apple tests, use the iOS Simulator | Needs a Mac with Xcode. |
| Export IPAs, Mac App Store packages, and Developer ID apps | Supported. Each export signs the app again for its method. |
| Upload to App Store Connect, notarize and staple | Supported, with an App Store Connect API key. |

A generic destination builds every standard architecture, as `xcodebuild` does: arm64 and x86_64 for a simulator or a Mac, and arm64 with arm64_32 for a watch.
Add `arch=` to the destination to build one architecture. A named simulator builds the host's architecture.

Custom `ExportOptions.plist` keys that SweetPad doesn't act on, and that would change the result, are refused by name.
Current Xcode and Swift Build reject the old WatchKit extension products, so watchOS apps need to be standalone.

## Projects and resources

These build on Linux:

- Swift, Objective-C, C, and C++ targets
- Local and remote Swift packages, including binary XCFrameworks, and package macros that build on Linux
- Frameworks, static libraries, app extensions, and widgets
- Asset catalogs, storyboards, XIBs, String Catalogs, and strings files
- Core Data models and common mapping models
- SwiftData, Predicate, Preview, SwiftUI Entry, and TipKit macros
- Build-tool plugins on package targets and Xcode project targets. For project targets, SweetPad runs the SwiftPM `BuildToolPlugin` entry point. The plugin and its tools must run on Linux.
- App Intents metadata and shortcut language models
- Icon Composer icons, as static light, dark, and tinted images
- Sticker packs with PNG, GIF, and JPEG stickers

These don't:

- Metal shaders, Core ML model classes, SceneKit assets, and RealityKit assets, which need Apple's own tools. The build stops and names the tool and the file.
- Plugin commands implemented only in the `XcodeProjectPlugin` entry point. These require Xcode.
- Sticker sequences, localized sticker-pack strings, Icon Composer's glass and material effects, and advanced Core Data mapping expressions.

Linux file names are case-sensitive and a Mac's usually aren't.
A project that refers to `Core.swift` for a file saved as `core.swift` builds on a Mac and fails on Linux, and the error names the file as it is on disk.

Package manifests run on Linux, so a condition that only holds on macOS can pick different resources or libraries.

## Run scripts and scheme actions

Run-script phases run on Linux, without Apple's `sandbox-exec` profiles.
Their `PATH` holds the Swift toolchain and SweetPad's versions of these macOS tools:

- `xcrun`, which runs a tool by name, finds one with `--find`, and reports an SDK's path and versions with `--show-sdk-*`
- `ditto`, which copies files and folders, but leaves archives to `zip` and `unzip`
- `cc`, which compiles for an Apple target with Clang and for the host with the host's `cc`
- `codesign`, `lipo`, `strip`, and `dsymutil`
- `actool`, `ibtool`, `momc`, `mapc`, and `xcstringstool`

Other macOS commands, such as `/usr/libexec/PlistBuddy`, `plutil`, `agvtool`, and `sw_vers`, aren't there.
A script that needs one needs a Linux equivalent, or has to run on a Mac.

A process that a script starts in the background keeps running, and the build doesn't wait for it.
Ctrl-C stops the build along with anything its scripts left running.

The scheme's Build pre-actions and post-actions run, with the build settings of the target they select.
Pre-actions run before the build starts, so they can generate source files, and post-actions run after a successful build.
Archive actions run around the archive step.
Test actions don't run, since Linux stops at building the tests. Send Email actions are skipped, as `xcodebuild` skips them.

## App Intents

The App Intents processors write the registration metadata, extracted strings, and shortcut language models, per language for a localized app.
They support primitive, optional, and array parameters, file images, dialog and snippet results, enums, basic entities, widget intents, and basic shortcuts.
Other schemas and language-model options fail with an error that names them.
AssistantIntents catalogs and catalog variations aren't supported, and Siri voice invocation on a device is untested.

Localized shortcut training needs the phrase resources that a [Mac SDK export](sdk-transfer.md#export-it-on-your-mac) includes.

## Other commands

`sweetpad open dd` opens the DerivedData folder with `xdg-open`.
`sweetpad open config` opens the config file in `$VISUAL` or `$EDITOR`, or with `xdg-open` outside a terminal.
`open xcode` and `open sim` need a Mac and exit with code 5 (`tool_missing`).

## Environment variables

| Variable | Purpose |
| --- | --- |
| `SWEETPAD_DARWIN_SDK` | The Darwin SDK to build against. The setup environment file sets it. |
| `SWEETPAD_SWBUILD` | The Swift Build driver, in place of the one installed with SweetPad. |
| `SWEETPAD_MACROS` | The macro plugin, in place of the one installed with SweetPad. |
| `SWEETPAD_ENGINE` | `swift-build` or `xcodebuild`. Linux defaults to `swift-build`. |
| `SWEETPAD_CODE_SIGN_P12` | P12 identity files, separated by `:`. |
| `SWEETPAD_CODE_SIGN_P12_PASSWORD` | Their password, or one per file separated by `:`. |
| `SWEETPAD_PROVISIONING_PROFILES` | A folder of provisioning profiles. |
| `SWEETPAD_ASC_KEY_ID`, `SWEETPAD_ASC_ISSUER_ID` | The App Store Connect API key and its issuer. Notarization needs a team key, which has an issuer. |
| `SWEETPAD_ASC_KEY_PATH`, `SWEETPAD_ASC_KEY` | The key's `.p8` file, or its text. |
| `SWEETPAD_APPLE_SESSION_FILE` | The Apple session for Xcode downloads. |
| `SWEETPAD_DEVICE_PYTHON` | The Python with `pymobiledevice3==11.50.0` that device commands use. |
| `SWEETPAD_DEVICE_NETWORK_CONFIG` | The Wi-Fi pairing from `sweetpad device pair-network`. |
| `SWEETPAD_DEVICE_TIMEOUT` | Seconds a device action may take (default 180). |
| `SWEETPAD_DEVICE_DISCOVERY_TIMEOUT` | Seconds to look for each device (default 30). |

The two timeouts apply when `SWEETPAD_DEVICE_PYTHON` or `SWEETPAD_DEVICE_NETWORK_CONFIG` is set.
`sweetpad help environment` lists every variable.
