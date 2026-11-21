---
sidebar_label: Linux overview
---

# Build iPhone apps on Linux

SweetPad can compile an Xcode project on Linux and produce an iPhone app. An unsigned build needs three things on the Linux host: the SweetPad CLI, a Linux Swift compiler, and Apple SDKs from your copy of Xcode.

:::warning Experimental
The validated environment is a Linux VM on Apple hardware. Setup supports Ubuntu 24.04 and Debian 12 or 13 on ARM64 and x86_64. It also supports Arch Linux on x86_64. See [supported hosts](linux/sdk-transfer.md#supported-hosts).
:::

## Start here

1. [Install the CLI](linux/install.md) for your Linux host architecture.
2. [Set up the Apple toolchain](linux/sdk-transfer.md). Use an Xcode download on Linux or an SDK export from an existing Mac installation of Xcode. Setup imports the SDK and checks it by building an unsigned sample app.
3. From your own project, build for a generic iPhone. Replace the project and scheme names in this example.

```bash
sweetpad build --project MyApp.xcodeproj --scheme MyApp \
  --destination 'generic/platform=iOS' --json --non-interactive \
  -- CODE_SIGNING_ALLOWED=NO
```

The destination selects the iPhone SDK and CPU architecture. You can compile without a connected phone or signing identity. The JSON field `data.productPath` gives the path to the app.

## Choose your next step

| Goal | What you need |
| --- | --- |
| Run on an iPhone | [Configure development signing](linux/signing-and-testing.md#configure-development-signing), then [pair the phone, install, and launch](linux/ubuntu-usb.md). |
| Run iOS tests | Linux can [build test products](linux/signing-and-testing.md#build-test-products). A Mac with Xcode runs them on a simulator or phone. |
| Distribute an app | [Archive and export](linux/signing-and-testing.md#archive-and-distribute) with the identity and profiles for your distribution method. |
| Build an iPhone app in GitHub Actions | Use the [Ubuntu workflow](linux/github-actions.md). |
| Build a macOS app on Linux | [Compile and export for a macOS destination](linux/macos-ci.md). |
| Build a React Native workspace | [Prepare JavaScript dependencies, generated code, and Pods](linux/react-native.md) before compiling. |

The iOS Simulator, Apple test execution, and running macOS apps require a Mac. SweetPad does not support debugging an iPhone from Linux. See [how Linux builds work](/blog/2026/11/21/build-xcode-projects-on-linux) and the [support reference](linux/reference.md) for project and resource limits.
