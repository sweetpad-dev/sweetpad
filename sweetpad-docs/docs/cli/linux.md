---
sidebar_label: Linux overview
---

# Build iPhone apps on Linux

SweetPad builds an Xcode project on Linux, signs it with your development identity, and installs it on an iPhone.
The command uses the same project, scheme, and build settings as the macOS CLI.

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive
```

:::warning Experimental
Release 0.1.18 targets Ubuntu 24.04 ARM64. Current source builds add x86_64 hosts, including ARM64 iPhone cross-compilation. The validated setup uses Linux in a VM on Apple hardware.
Apple SDKs and signing credentials come from your own development environment.
:::

## Start here

These guides follow the path from a fresh machine to a running iPhone app:

| Guide | What you will do |
| --- | --- |
| [How it works](linux/how-it-works.md) | Understand the compiler, SDK, resource tools, and device transport. |
| [SDK export and setup](linux/sdk-transfer.md) | Export private Apple files, configure Ubuntu, and diagnose the Linux toolchain. |
| [Ubuntu and USB setup](linux/ubuntu-usb.md) | Install the CLI and toolchain, pair an iPhone, and launch an app through USB. |
| [Signing, tests, and archives](linux/signing-and-testing.md) | Configure development signing and run Linux-built test products on a Mac. |
| [GitHub Actions for iOS](linux/github-actions.md) | Build iPhone products on a provisioned Linux runner and retain usable artifacts. |
| [React Native](linux/react-native.md) | Prepare Pods and Hermes, then build the iOS workspace. |
| [Flutter](linux/flutter.md) | Prepare a matching Linux iOS AOT compiler and build the Runner project. |
| [macOS apps in CI](linux/macos-ci.md) | Build macOS bundles on Linux or use a hosted Mac for the full native workflow. |
| [Support and configuration](linux/reference.md) | Find supported features, environment variables, and known limits. |

## What builds

The iOS build path covers Swift, Objective-C, C, and C++ targets, local and remote packages, frameworks, widgets, and app extensions.
Portable tools compile asset catalogs, Interface Builder files, strings, Core Data models, and supported App Intents metadata.

The validation suite includes native Swift apps, React Native with Hermes and MMKV, Flutter AOT, and Capacitor.
Device checks cover startup, storage persistence, and direct Linux USB and Wi-Fi transport.
Some corpus apps require explicit compatibility patches. A successful fixture check does not promise compatibility with every framework version.

See [the support reference](linux/reference.md) for the exact boundaries.

## What stays on a Mac

The iOS Simulator and Apple test execution require macOS with Xcode.
Linux can build test products and write their test manifest; a Mac runs those products without recompiling them.

Development signing and development IPA export work on Linux.
App Store distribution export and Linux device debugging remain outside the supported release scope.

## Signing a device build

An iPhone requires a signed app and a matching provisioning profile.
Follow [signing, tests, and archives](linux/signing-and-testing.md) before the device-build step in the Ubuntu guide.

## Device transport

A connected iPhone can use Linux USB transport directly.
The [Ubuntu guide](linux/ubuntu-usb.md#connect-the-iphone) covers trust, Developer Mode, and USB passthrough.

Wi-Fi pairing and discovery use the same pinned device adapter.
Run `sweetpad help linux-devices` for the command guide.
