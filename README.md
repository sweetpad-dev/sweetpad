# SweetPad

SweetPad is a family of tools for developing Swift/iOS projects — building, running, debugging, and
testing your Xcode projects for iOS, macOS, tvOS, watchOS, and visionOS without living inside Xcode.

Everything is built on top of Xcode's own command-line tools (so you still need Xcode installed) and a
shared Rust core. There are two separate products, and you can use either on its own:

## VS Code extension

Build, run, debug, and test straight from the editor sidebar — with device/simulator logs,
format-on-save, autocomplete via SourceKit-LSP, and native Testing-panel integration.

- Install from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad)
  (works in [Cursor](https://www.cursor.com/) too).
- Read the [documentation](https://sweetpad.hyzyla.dev/docs/intro).

## SweetPad CLI

A single native `sweetpad` binary ("xcodebuild for humans") that builds, runs, and tests Xcode and
Swift Package projects from the terminal, with no editor running. Every command speaks JSON, so it
drops into scripts, git hooks, and CI.

```bash
brew install sweetpad-dev/tap/sweetpad
```

Read the [CLI documentation](https://sweetpad.hyzyla.dev/docs/cli).

## Repository layout

This repository is a monorepo. Both products are built from a shared Rust core:

- [`sweetpad-vscode/`](./sweetpad-vscode) — VS Code extension ([Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad)); the N-API addon bridging it to the Rust core lives in [`sweetpad-vscode/native/`](./sweetpad-vscode/native)
- [`sweetpad-cli/`](./sweetpad-cli) — the standalone `sweetpad` CLI
- [`sweetpad-core/`](./sweetpad-core) — business logic shared by the CLI and the extension (build-settings resolution, BSP server)
- [`sweetpad-lib/`](./sweetpad-lib) — interface-agnostic Xcode file/format utilities (in development)
- [`sweetpad-docs/`](./sweetpad-docs) — [documentation site](https://sweetpad.hyzyla.dev)
