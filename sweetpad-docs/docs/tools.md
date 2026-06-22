---
sidebar_position: 11
---

# Tools

The **Tools** panel in the SweetPad sidebar lists the third-party CLI tools SweetPad integrates with, installs them
via Homebrew, and links out to each one's documentation — so you don't have to remember which `brew install` to
run.

[![iOS tools](/images/tools-demo.gif)](/images/tools-demo.gif)

The tools listed in the panel:

- [**Homebrew**](https://brew.sh/) — package manager used to install most of the others.
- [**swift-format**](https://github.com/apple/swift-format) — the default Swift formatter. Xcode 16+ ships it
  bundled; older Xcodes need a Homebrew install. See [Format code](./format.md).
- [**XcodeGen**](https://github.com/yonaskolb/XcodeGen) — generates `.xcodeproj` from a YAML manifest.
- [**SwiftLint**](https://github.com/realm/SwiftLint) — Swift linter.
- [**xcbeautify**](https://github.com/cpisciotta/xcbeautify) — formats `xcodebuild` output into something readable.
- [**xcode-build-server**](https://github.com/SolaWing/xcode-build-server) — exposes Xcode's build outputs to
  SourceKit-LSP, which powers [autocomplete](./autocomplete.md), jump-to-definition, and hover docs.
- [**ios-deploy**](https://github.com/ios-control/ios-deploy) — installs and launches apps on physical iOS devices.
- [**tuist**](https://docs.tuist.io/) — declarative Xcode project generation. SweetPad has deeper integration here; see
  [Tuist](./tuist.md).

:::tip

[`pymobiledevice3`](https://github.com/doronz88/pymobiledevice3) — used for on-device log streaming and the iOS 17+
developer tunnel — is installed via Python (`uv`, `pipx`, or `pip`), not Homebrew. Run
`> SweetPad: Install pymobiledevice3` from the command palette. See [Devices](./devices.md) for details.

:::
