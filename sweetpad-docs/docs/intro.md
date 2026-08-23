---
sidebar_position: 1
slug: /
sidebar_label: Which one do I need?
---

# CLI or VS Code extension?

SweetPad is two separate tools that solve the same problem from different places: building, running,
debugging, and testing Xcode projects for iOS, macOS, tvOS, watchOS, and visionOS without living inside
Xcode. Both work with Xcode workspaces and projects, Tuist, XcodeGen, and Swift Packages.

**You do not need both.** Pick the one that matches where you work.

## SweetPad CLI

A single native binary named `sweetpad`, or "xcodebuild for humans". You install it with Homebrew and run
it from any terminal:

```bash
brew install sweetpad-dev/tap/sweetpad
```

It does not need VS Code, and it does not care what editor you use. It works the same from a terminal
in Xcode, Vim, Zed, a git hook, or a CI job.

Start at [Get started with the CLI](./cli/getting-started.md).

## VS Code extension

An extension that puts the same builds, runs, and tests in the VS Code sidebar, with breakpoints, a
Testing panel, format-on-save, and autocomplete. It installs from the VS Code Marketplace and works
in [Cursor](https://www.cursor.com/) too.

It does not need the CLI to build, run, debug, or test. Those run in the extension itself.

Start at [Get started with the extension](./vscode/getting-started.md).

## The one place they meet

The extension's default autocomplete setup runs SweetPad's build server, and that server ships in the
CLI binary. So if you want code intelligence in VS Code, install the CLI as well. The
[Autocomplete](./vscode/autocomplete.md) page walks through it, and the Tools panel offers the install
in one click.

That is the only overlap. Nothing else in the extension shells out to the CLI, and nothing in the CLI
looks for VS Code.

:::info

Both tools drive Xcode's own command-line tools underneath, so you need Xcode installed on your Mac
either way.

:::

## Still not sure?

- You spend your day in a terminal, or you want builds in scripts, git hooks, and CI → **CLI**.
- You want an editor that can build and debug your app like Xcode does → **extension**.
- You use VS Code *and* want a terminal command → install both. They share nothing but a name and a
  Rust core, so they will not fight.
