---
sidebar_position: 10
sidebar_label: Building on Linux
---

# Building on Linux

`sweetpad build` can build an Xcode project on Linux, with no Xcode and no changes to the
project. There it builds with Swift Build, Apple's open-source build system, against the
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
- Swift packages, local and remote, including packages that ship a binary XCFramework.
- Asset catalogs, storyboards and XIBs, String Catalogs and `.strings` files, and Core Data
  models.
- SwiftData's macros, `#Predicate` and `#Preview`.
- Run script phases, including scripts that call `xcrun`, `ditto` or `lipo`.
- Device builds, signed with your development identity (see
  [Signing a device build](#signing-a-device-build)).

## What stays on a Mac

Some things need macOS itself, or an Apple tool that exists only for macOS:

- Running the app in the Simulator, running tests, and installing on a device.
- watchOS, tvOS and visionOS apps, and Mac Catalyst. The SDKs sweetpad takes from Xcode cover
  iOS and macOS only.
- Metal shaders, Core ML model classes, Core Data mapping models, and SceneKit and RealityKit
  assets.
- Run scripts that need macOS, like a Swift script that imports Darwin.
- Swift macros that a project builds as a macOS command-line tool, which some generated
  projects do with macro packages.

A build that needs one of these fails with an error, the same way a broken project would.

## Setting up a Linux machine

A setup script in the sweetpad repository installs what a Linux build needs. It expects
Ubuntu 24.04 and is tested on arm64. On x86_64, set `XTOOL_SHA256` to the checksum of xtool's
x86_64 release. The script installs:

- the Swift toolchain, checked against swift.org's signing keys;
- xtool, and the Darwin SDK it extracts from your Xcode;
- swbuild, Swift Build's command-line driver, built from source with a small patch from sweetpad;
- the plugin sweetpad uses for Apple's macros;
- Rust, to build sweetpad itself.

Every step is skipped when its result is already there, so you can run the script again after
an update.

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
