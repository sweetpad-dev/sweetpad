# SweetPad

Develop Swift/iOS projects from VS Code and the terminal.

SweetPad builds, runs, debugs, and tests your Xcode projects for iOS, macOS, tvOS, watchOS, and
visionOS. It's built on top of Xcode's own command-line tools, so you still need Xcode installed — but
you don't need to open it. There are two ways to use it: the VS Code extension and the standalone CLI.

## VS Code extension

The primary way to use SweetPad. Build, run, debug, and test straight from the editor sidebar — with
device/simulator logs, format-on-save, autocomplete via SourceKit-LSP, and native Testing-panel
integration.

- Install from the [VS Code Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad)
  (works in [Cursor](https://www.cursor.com/) too).
- Read the [documentation](https://sweetpad.hyzyla.dev/docs/intro).

## SweetPad CLI

The same power from the terminal — a single native `sweetpad` binary ("xcodebuild for humans") that
builds, runs, and tests the same projects with no editor running. Every command speaks JSON, so it
drops into scripts, git hooks, and CI.

```bash
brew install sweetpad-dev/tap/sweetpad
```

Read the [CLI documentation](https://sweetpad.hyzyla.dev/docs/cli).

## Repository layout

This repository is a monorepo. The two products above are built from a shared Rust core:

- [`sweetpad-vscode/`](./sweetpad-vscode) — VS Code extension ([Marketplace](https://marketplace.visualstudio.com/items?itemName=sweetpad.sweetpad)); the N-API addon bridging it to the Rust core lives in [`sweetpad-vscode/native/`](./sweetpad-vscode/native)
- [`sweetpad-cli/`](./sweetpad-cli) — the standalone `sweetpad` CLI
- [`sweetpad-core/`](./sweetpad-core) — business logic shared by the CLI and the extension (build-settings resolution, BSP server)
- [`sweetpad-lib/`](./sweetpad-lib) — interface-agnostic Xcode file/format utilities (in development)
- [`sweetpad-docs/`](./sweetpad-docs) — [documentation site](https://sweetpad.hyzyla.dev)
