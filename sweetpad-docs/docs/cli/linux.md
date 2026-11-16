---
sidebar_label: Linux overview
---

# Build iPhone apps on Linux

SweetPad builds an Xcode project on Linux, signs it, and installs it on an iPhone.
It also exports release builds, uploads them to App Store Connect, and notarizes Developer ID apps.
The commands, project, scheme, and build settings are the same as on a Mac:

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive
```

:::warning Experimental
The CLI runs on ARM64 and x86_64 Linux. The Apple toolchain installs on Ubuntu 24.04, Debian 12 and 13, and Arch Linux on x86_64.
The validated setup is a Linux VM on Apple hardware, with Apple's SDKs from your own Xcode.
:::

## Start here

1. [Install the CLI](linux/install.md).
2. [Set up the Apple toolchain](linux/sdk-transfer.md) from your Mac's Xcode or an Xcode download.
3. [Set up signing](linux/signing-and-testing.md) with your development identity and profiles.
4. [Connect an iPhone](linux/ubuntu-usb.md) over USB or Wi-Fi, then build, install, and launch.

Then, as you need them:

- [GitHub Actions for iOS](linux/github-actions.md) builds, tests, and uploads from a Linux runner.
- [React Native](linux/react-native.md) and [Flutter](linux/flutter.md) apps need extra preparation.
- [macOS apps in CI](linux/macos-ci.md) covers Mac apps built on Linux.
- [How it works](linux/how-it-works.md) explains the build pipeline.
- [Support and configuration](linux/reference.md) lists what works and the environment variables.

## What stays on a Mac

The iOS Simulator and running Apple tests need macOS with Xcode.
Linux builds the test products and their test manifest, and a Mac runs them without compiling them again.
Debugging an app on a device from Linux isn't supported.
