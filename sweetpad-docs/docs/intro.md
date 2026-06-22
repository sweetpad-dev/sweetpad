---
sidebar_position: 1
---

import ReactPlayer from 'react-player'

# Introduction

SweetPad is a VSCode extension that lets you build, run, debug, and test your Xcode projects for iOS, macOS, tvOS,
watchOS, and visionOS without leaving VSCode. It's built on top of the Xcode CLI tools, plus a handful of open-source tools
like [xcode-build-server](https://github.com/SolaWing/xcode-build-server),
[xcbeautify](https://github.com/cpisciotta/xcbeautify),
[swift-format](https://github.com/swiftlang/swift-format), and
[pymobiledevice3](https://github.com/doronz88/pymobiledevice3).

:::info

You still need to have Xcode installed on your machine to use the extension because it heavily relies on the Xcode CLI
tools to build and run your project.

:::

## What you get

- 🛠️ **[Build & Run](./build.md)** apps on simulators, macOS, and physical devices straight from the SweetPad sidebar
  — with support for Xcode workspaces, Xcode projects, [Tuist](./tuist.md), XcodeGen, and Swift Package Manager
  (`Package.swift`).
- 🐞 **[Debug](./debug.md)** with breakpoints, step, watch, and the rest of LLDB via the CodeLLDB extension — on the
  Simulator and on physical iOS devices.
- 📋 **Logs from devices and simulators** stream `os_log` / `Logger` / `print` / `NSLog` into the build terminal so
  you don't have to keep Console.app open.
- 🧪 **[Tests](./tests.md)** show up in VSCode's native Testing panel with gutter ▶️ buttons; supports XCTest and
  Swift Testing.
- ✍️ **[Format on save](./format.md)** with `swift-format` (Xcode's bundled copy by default) or any other Swift
  formatter you prefer.
- 💡 **[Autocomplete](./autocomplete.md)** via SourceKit-LSP backed by `xcode-build-server`, including inline
  compiler diagnostics in the Problems panel.
- 🌳 **[Git worktrees](./worktree.md)** — switch the active workspace between parallel checkouts of the same project
  in one command.
- 🤖 **[Agent CLI / RPC server](./agent-cli.md)** — opt-in JSON-RPC server and bundled `sweetpad` CLI so scripts and
  AI coding agents can drive your VSCode session from the outside.

## Getting started

:::tip

This tutorial also works for [Cursor](https://www.cursor.com/), an AI-first code editor that's a fork of VSCode.

:::

First, install [VSCode](https://code.visualstudio.com/) and the extension from the
[VSCode Marketplace](https://marketplace.visualstudio.com/items?itemName=SweetPad.sweetpad).

![Install extension](/images/intro/install-extension.png)

Next, create an Xcode project. We highly recommend [XcodeGen](https://github.com/yonaskolb/XcodeGen) or
[Tuist](https://tuist.io/), which let you define the project structure in configuration files — but plain Xcode is
fine too. SweetPad also works directly with Swift Packages: open a folder that contains a `Package.swift` and you're
good to go.

![Xcode](/images/intro/create-project.png)

Once you have a working Xcode project, open the project's root folder in VSCode — not the `.xcodeproj` or
`.xcworkspace` folder itself.

If you installed the extension correctly, you should see the SweetPad lollipop icon 🍭 in the left sidebar of the
editor. This is the main entry point for using the extension.

The main panels of the extension are:

1. **Build** — shows the list of schemes and the "Launch" button to build and run the project.
2. **Destinations** — lists every place you can run on: recently used destinations, simulators, and connected devices.
3. **Tools** — installs and links to docs for the third-party tools SweetPad uses.

![Opened project](/images/intro/open-project.png)

To build and run the project, click ▶️ next to the scheme and wait for the build to finish. SweetPad then boots the
simulator and launches the app.

That's it — you've built and run your first Xcode project in VSCode. From here:

- [Configure format on save](./format.md) so Swift files reformat themselves on save.
- Install `xcbeautify` for readable build logs — the [Tools](./tools.md) panel handles it.
- Explore the rest of the [features](#what-you-get).

## Demo

Here is a short demo of building and running an Xcode project in VSCode:

<ReactPlayer src="/images/intro/build-demo.mp4" controls style={{ width: '100%', height: '100%' }} />
