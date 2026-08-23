---
sidebar_position: 2
sidebar_label: Overview
---

# VS Code extension

The SweetPad extension turns VS Code into a place you can build, run, debug, and test an Xcode app
without switching to Xcode. It installs from the
[VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad) and works
with Xcode workspaces and projects, [Tuist](./tuist.md), [XcodeGen](./xcodegen.md), and Swift Packages.

Everything here works in [Cursor](https://www.cursor.com/) too. It's a fork of VS Code, so the
extension installs and behaves the same way.

:::tip

New here? Start with [Get started with the extension](./getting-started.md): install it, open a
project, and run your app on a simulator in about five minutes. This page is the fuller tour.

:::

## What you get

- 🛠️ **[Build & Run](./build.md)** apps on simulators, macOS, and physical devices straight from the
  SweetPad sidebar.
- 🐞 **[Debug](./debug.md)** with breakpoints, step, watch, and the rest of LLDB via the CodeLLDB
  extension, on the Simulator and on physical iOS devices.
- 📋 **Logs from devices and simulators** stream `os_log`, `Logger`, `print`, and `NSLog` into the build
  terminal so you don't have to keep Console.app open.
- 🧪 **[Tests](./tests.md)** show up in VS Code's native Testing panel with gutter ▶️ buttons; supports
  XCTest and Swift Testing.
- ✍️ **[Format on save](./format.md)** with `swift-format` (Xcode's bundled copy by default) or any
  other Swift formatter you prefer.
- 💡 **[Autocomplete](./autocomplete.md)** via SourceKit-LSP, including inline compiler diagnostics in
  the Problems panel.
- 🔥 **[Hot reload](./hot-reload.md)** applies a saved Swift file to the running app without a rebuild.
- 🌳 **[Git worktrees](./worktree.md)** switch the active workspace between parallel checkouts of the
  same project in one command.
- 🧰 **[Tools](./tools.md)**: one-click installs for the helper tools SweetPad can use.

## Choosing where to run

The **Destinations** view in the sidebar lists every simulator, connected device, and macOS. Pick one
and SweetPad remembers it for the workspace. See [Destinations](./destinations.md), and
[Simulators](./simulators.md) and [Devices](./devices.md) for managing each kind.

## Reference

When you need the exact name of something:

- [Settings reference](./settings.md): every extension setting, grouped by area.
- [Commands reference](./commands.md): every command-palette command.
- [Troubleshooting](./troubleshooting.md): what to check when something doesn't work.

## Do I need the CLI too?

No, with one exception. Building, running, debugging, testing, and formatting all happen inside the
extension. The default autocomplete setup is the exception: it runs SweetPad's build server, which
ships inside the `sweetpad` CLI binary, so that one feature asks you to install the CLI. The
[Autocomplete](./autocomplete.md) page covers it.

If you *want* the terminal tool as well, it's a separate product with its own docs:
[SweetPad CLI](../cli/getting-started.md).
