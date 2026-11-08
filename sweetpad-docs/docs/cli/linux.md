---
sidebar_position: 10
sidebar_label: Building on Linux
---

# Building on Linux

`sweetpad build` can build an Xcode project on Linux. It uses Swift Build, Apple's open-source build system, against the
Apple SDKs taken from your own copy of Xcode. For the Xcode tools Linux doesn't have, sweetpad
runs its own replacements.

The command is the same one you run on a Mac:

```bash
sweetpad build --scheme MyApp --destination "generic/platform=iOS Simulator"
```

:::warning
Linux builds are experimental. The Homebrew release of sweetpad is macOS only, so on Linux you
build sweetpad from source, as described below.
:::

## What builds

Most of what an iOS or macOS app is made of builds the way Xcode builds it:

- iOS and macOS apps, with their app extensions and widgets, frameworks and static libraries.
- tvOS, watchOS, and visionOS simulator builds, including modern standalone SwiftUI apps.
- Swift packages, local and remote, including packages that ship a binary XCFramework.
- Asset catalogs, storyboards and XIBs, String Catalogs and `.strings` files, and Core Data
  models and common Core Data mapping models.
- SwiftData's macros, `#Predicate`, `#Preview`, SwiftUI `@Entry`, and TipKit macros.
- Package macros built for the Linux host, including Tuist-generated macro tools.
- Explicit schemes shipped by resolved Git packages, including custom DerivedData locations.
- App Intents metadata and language models for the supported intent schemas.
- Run script phases, including scripts that call `xcrun`, `ditto` or `lipo`.
- Device builds, signed with your development identity (see
  [Signing a device build](#signing-a-device-build)).
- Development archives and IPA export that preserve the build's signing identity.

The real-project suite includes Mastodon, Swiftfin, Wikipedia, Flutter, React Native, and Capacitor.
See the repository's `sweetpad-lib/ci/LINUX_PROJECTS.md` for source pins, results, and setup requirements.
Framework projects need their generated dependencies before compilation.
All seven larger fixtures pass two launches on an iPhone 13.
Wikipedia and Mastodon use explicit app-local startup fixtures with omitted extensions and unsupported capabilities.
The checks reuse an existing wildcard profile and do not validate those capabilities.
Flutter Release uses a Linux-hosted iOS AOT compiler built from pinned Dart sources; its engine remains a prebuilt dependency.

## What stays on a Mac

Some things need macOS itself, or an Apple tool that exists only for macOS:

- Running the app in the Simulator and executing tests. Linux can compile test products with `sweetpad test build`.
- Mac Catalyst builds remain unverified.
- Metal shaders, Core ML model classes, and SceneKit and RealityKit
  assets.
- Run scripts that need macOS, like a Swift script that imports Darwin.

Other limits include advanced mapping expressions, unsupported App Intents schemas,
distribution IPA export, and some Swift/C++ interoperability cases.
The tested MMKV example needs an explicit fixture shim for Linux Swift's debug-only C++ thunk emission.
Both products pass storage checks, including persistence after relaunch.
See `sweetpad-lib/ci/LINUX_PROJECTS.md` for the pinned patch and dependency preparation.

iOS Icon Composer support produces static light, dark, and tinted appearances.
Interactive glass lighting and native material effects remain unsupported.
Sticker packs support ordered PNG/GIF/JPEG members and message-extension icons.
Sticker sequences and localized sticker-pack strings remain unsupported.

A build that needs one of these fails with an error, the same way a broken project would.

Current Xcode and Swift Build reject deprecated WatchKit extension products.
Use a modern single-target watchOS app. The new platform probes verify compilation and linking;
runtime and device-signing checks for tvOS, watchOS, and visionOS remain unverified.

## Setting up a Linux machine

A setup script in the sweetpad repository installs what a Linux build needs. It expects
Ubuntu 24.04 and is tested on arm64. On x86_64, set `XTOOL_SHA256` to the checksum of xtool's
x86_64 release. The script installs:

- the Swift toolchain, checked against swift.org's signing keys;
- xtool, and the Darwin SDK it extracts from your Xcode;
- additional tvOS, watchOS, and visionOS SDK data when those platforms exist in your Xcode;
- Swift back-deployment libraries from your Xcode for apps with older deployment targets;
- swbuild, Swift Build's command-line driver, built from source with a small patch from sweetpad;
- the plugin sweetpad uses for Apple's macros;
- Rust, to build sweetpad itself.

The current setup pins Swift 6.4.2 and Swift Build 6.4.0 separately.
It also installs Bash, bubblewrap, rsync, and LLVM's Darwin libtool.
For script phases, a private mount makes `/bin/sh` use Bash, as macOS scripts expect.
This does not change the machine's default shell or restrict the script's filesystem access.

Every step is skipped when its result is already there, so you can run the script again after
an update.
If Xcode's SDK version changes, rerun with `SWEETPAD_REFRESH_SDK=1` to replace the extracted SDK.

### On a Mac, in an OrbStack machine

With [OrbStack](https://orbstack.dev) installed, the script creates an Ubuntu machine called
sweetpad-linux and sets it up. The machine sees your Mac's files at the same paths, Xcode
included.

```bash
git clone https://github.com/sweetpad-dev/sweetpad.git
cd sweetpad
sweetpad-lib/ci/linux-setup.sh
```

### On a Linux host

Copy Xcode.app from your Mac to the Linux machine, then run the script there, pointing
`XCODE_APP` at the copy:

```bash
git clone https://github.com/sweetpad-dev/sweetpad.git
cd sweetpad
XCODE_APP=/opt/Xcode.app sweetpad-lib/ci/linux-setup.sh
```

:::note
Apple's SDK license ties the SDKs to Apple-branded hardware. The script only copies them from
the Xcode you point it at; it never downloads or redistributes them.
:::

### Building sweetpad

The script puts swbuild and the macro plugin in `~/bin`, and sweetpad looks for both on your
`PATH`. Build sweetpad from the same checkout, inside the Linux machine. In OrbStack, open a
shell there with `orb -m sweetpad-linux`.

```bash
export PATH="$HOME/bin:$(ls -d ~/swift/swift-*-RELEASE | tail -1)/usr/bin:$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR="$HOME/sweetpad-target"
cargo build --release --locked -p sweetpad-cli --bin sweetpad
```

The binary is `~/sweetpad-target/release/sweetpad`. A target folder of its own keeps the Linux
build apart from a Mac build of the same checkout. Keep the Swift toolchain first on your
`PATH` when you build an app, since Swift Build uses the first `swift` it finds.

## Building an app

On Linux, `sweetpad build` takes the same flags as on a Mac. Pick the destination as you would
there; a generic one is all a build needs:

```bash
sweetpad build --scheme MyApp --destination "generic/platform=iOS Simulator"
sweetpad build --scheme MyMacApp --destination "generic/platform=macOS"
```

The product lands in DerivedData, in the same folder Xcode would put it in.

For a test-only scheme, use `sweetpad test build --scheme MyTests`.
Remote package schemes become available after the project resolves its dependencies.
Listing schemes reads the existing checkout graph and does not fetch packages.

### Running Linux-built tests

Copy the complete test products directory to a Mac with Xcode. Keep the directory layout
referenced by the `.xctestrun` manifest. Then run:

```bash
sweetpad test run --project MyApp.xcodeproj --scheme MyApp \
  --xctestrun /path/to/Products/prebuilt.xctestrun --on SIMULATOR_UDID
```

This uses `test-without-building`. The project supplies result history; Xcode loads the
products named by the manifest. The command does not compile them or replace build diagnostics.
Test selectors, result bundles, and reports remain available. A selection that runs zero tests fails.

Linux `test build` automatically writes an `.xctestrun` for iOS app-hosted and logic unit tests.
JSON output includes `xctestrunPath`. The manifest remains relative to its `Products` directory after export.
It includes the enabled scheme targets, literal environment values, argument rows, and test filters.
Quoted arguments retain their quotes for the XCTest launcher.

```bash
sweetpad test build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS Simulator' --json --non-interactive \
  -- -derivedDataPath /path/to/TestBuild
```

Copy `/path/to/TestBuild/Build/Products` to the Mac, including the generated manifest.
Then pass that manifest to `test run --xctestrun`.
The simulator and test runtime still run on macOS.

UI tests, test plans, locale overrides, and arguments with build-setting expressions require a supplied manifest.
Unsupported inputs leave the successful build intact and report `xctestrunPath: null` with `xctestrunNote`.
The repository helper `sweetpad-lib/ci/prebuilt_tests.py` remains available for explicitly selected custom products.
For a custom build directory, pass `-derivedDataPath` after `--` or configure it in `sweetpad.toml`.

Build settings after `--` work as they do with xcodebuild, and so do `-derivedDataPath` and
`-xcconfig`. Flags that only xcodebuild understands are refused with an error, since the build
doesn't go through xcodebuild.

## Signing a device build

A build for a device signs the app with an identity from a `.p12` file, since Linux has no
keychain. Export your Apple Development certificate and its private key from Keychain Access on
your Mac, then point sweetpad at the file:

```bash
export SWEETPAD_CODE_SIGN_P12=~/signing/development.p12
export SWEETPAD_CODE_SIGN_P12_PASSWORD="$(cat ~/signing/password)"
sweetpad build --scheme MyApp --destination "generic/platform=iOS"
```

sweetpad picks the provisioning profile the way Xcode's automatic signing does. It looks in the
folders Xcode keeps profiles in under `~/Library`, or in the folder
`SWEETPAD_PROVISIONING_PROFILES` names. Copy your profiles from the Mac to either place.

Without an identity, a device build isn't signed. A simulator build needs no identity.

For an existing private key, export the `.p12` file with OpenSSL 3 and AES encryption.
The macOS system OpenSSL can select 40-bit RC2 encryption, which the Linux signer does not support.
Device signing includes Swift package bundles that contain resources without an executable.

A Linux-built parity app passed seven tests on an iPhone 13 with iOS 27.0.1.
The Mac installed the signed app and ran the tests without recompilation.
The exported executable hashes stayed unchanged.
See `sweetpad-lib/ci/linux-device-checks.py` for the repeatable device checks.

## Archives and IPA export

On Linux, `sweetpad archive` uses Swift Build's install action to create an `.xcarchive`.
It collects dSYMs for the app and its embedded extensions and frameworks.

```bash
sweetpad archive --scheme MyApp --destination "generic/platform=iOS" \
  --export-method debugging --output-file ./artifacts
```

Development export preserves the identity and provisioning profiles from the build.
It requires a signed device archive. Distribution methods and custom export options require Xcode.
Use `--no-export` to create an archive without an IPA.

## Device transport

Linux device discovery, installation, removal, launch, and logs use `pymobiledevice3` on `PATH`.
The command integration was checked against version 11.50.0.
Install it in a Python virtual environment and expose its executable on `PATH`.
USB access also needs `usbmuxd`, a paired device, Developer Mode, and an unlocked device.
Newer iOS developer services can also need a pymobiledevice3 tunnel.

Seven Linux-built app fixtures passed iPhone startup checks through macOS transport.
Linux USB and tunnel transport remain unverified. Linux readiness checks, process inspection,
and process termination are not implemented. Simulator operations still require a Mac.

## App Intents

The portable processors generate registration metadata, extracted strings, and shortcut language models.
Supported inputs include primitive parameters, optional and array values, image file inputs,
value dialogs, dialog/snippet results, enums, basic entities and queries, widget intents, and basic App Shortcuts.
Dynamic frameworks and embedded extensions keep separate registrations.
Advanced schemas, static dependency registrations, and unsupported language-model options fail with explicit errors.
Native metadata comparisons cover the supported forms. Siri behavior on a physical device is not verified.
Localized shortcut models use the app's translated names and AppShortcuts string tables.
The model trainer writes per-language bundles and removes models for deleted shortcuts.
The catalog validator checks supported AppShortcuts strings and catalogs.
AssistantIntents catalogs and catalog variations fail explicitly.

Mac execution of `linux-setup.sh` exports system phrase expansions from your Xcode
into your private Darwin SDK. Existing SDKs can use `ci/linux-appintents-resources.py`
to refresh `appintents-phrases.json`. Missing localized resources produce a preparation error.
The export covers 81 locale identifiers; `--locale` adds another project locale.

Ice Cubes' 19 localized models match native decoded contents. Native and Linux
products launch in the simulator. Voice invocation through Siri remains unverified.

## Environment variables

| Variable                          | What it sets                                                              |
| --------------------------------- | ------------------------------------------------------------------------- |
| `SWEETPAD_DARWIN_SDK`             | The Darwin SDK to build against, if it isn't where xtool installs it.     |
| `SWEETPAD_SWBUILD`                | The swbuild to run, in place of the one on `PATH`.                        |
| `SWEETPAD_MACROS`                 | The macro plugin, in place of the one on `PATH`.                          |
| `SWEETPAD_CODE_SIGN_P12`          | The `.p12` file with the identity a device build signs with.              |
| `SWEETPAD_CODE_SIGN_P12_PASSWORD` | That file's password.                                                     |
| `SWEETPAD_PROVISIONING_PROFILES`  | A folder of provisioning profiles, in place of Xcode's folders.           |

`sweetpad help environment` lists them too.

## How it works

On Linux, sweetpad does three things that Xcode would otherwise do.

**It describes the project to Swift Build.** sweetpad translates the workspace, its projects and
its schemes into the project description Swift Build reads. It asks SwiftPM for the same
description of the project's packages, so packages build exactly as in Xcode.

**It points Swift Build at the Darwin SDK.** The SDK carries the iOS and macOS SDKs, Xcode's
runtime libraries and a linker that writes Apple's binary format.

**It stands in for Xcode's tools.** The asset catalog, Interface Builder, String Catalog and
Core Data compilers, `codesign`, `lipo`, `ditto`, `xcrun` and a few smaller tools are all the
sweetpad binary, started under the tool's name. Each one is checked against what Xcode's tool
writes for the same input.
