---
sidebar_position: 1
---

# Introduction

SweetPad helps you build, run, debug, and test your Xcode projects for iOS, macOS, tvOS, watchOS, and
visionOS — without living inside Xcode. It works with Xcode workspaces and projects, [Tuist](./tuist.md),
XcodeGen, and Swift Packages.

SweetPad comes in two forms — pick whichever fits how you work:

- **[VSCode extension](./getting-started-vscode.md)** — build, run, and debug from the editor sidebar,
  with logs, tests, formatting, and autocomplete built in. This works in [Cursor](https://www.cursor.com/) too.
- **[SweetPad CLI](./getting-started-cli.md)** — the `sweetpad` command-line tool ("xcodebuild for
  humans") that does the same from the terminal, no editor needed.

:::info

Both products drive Xcode's own command-line tools under the hood, so you still need Xcode installed on
your Mac.

:::

## Get started

- New to the extension? Follow [Get started with the extension](./getting-started-vscode.md).
- Prefer the terminal? Follow [Get started with the CLI](./getting-started-cli.md).

## What you get

The VSCode extension gives you:

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

And the same building, running, and testing is available from the terminal:

- 💻 **[SweetPad CLI](./cli.md)** — the standalone `sweetpad` command-line tool to build, run, and test
  your projects, and to script them into git hooks and CI.
- 🤖 **[Agent CLI / RPC server](./agent-cli.md)** — an opt-in server so scripts and AI coding agents can
  drive your VSCode session from the outside.
