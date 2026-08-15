---
sidebar_position: 7
slug: /autocomplete
---

# Autocomplete

SweetPad wires Xcode's build information into [SourceKit-LSP](https://github.com/swiftlang/sourcekit-lsp) so you get
real autocomplete, jump-to-definition, hover docs, and Swift compiler diagnostics in VSCode.

![autocomplete](/images/autocomplete-preview.png)

SourceKit-LSP needs a **build server** to tell it how each file is compiled. SweetPad supports two:

- **SweetPad's own build server** (the default) — reads compiler arguments straight from the project, so
  autocomplete works **without building first**. Runs through the `sweetpad` CLI. Handles `.xcodeproj` and
  `.xcworkspace`.
- **xcode-build-server** — a battle-tested external tool that parses Xcode build logs. Install it separately, and
  build the project once before autocomplete comes alive.

:::info

Swift Packages don't need any of this — SourceKit-LSP understands `Package.swift` natively. If your workspace root is
a Swift Package, autocomplete works as soon as the [Swift extension](https://marketplace.visualstudio.com/items?itemName=swiftlang.swift-vscode)
is installed.

:::

## Setup with the built-in build server (default)

1. Install the [Swift](https://marketplace.visualstudio.com/items?itemName=swiftlang.swift-vscode) extension from the
   Marketplace, and the `sweetpad` CLI that runs the build server:

   ```bash
   brew install sweetpad-dev/tap/sweetpad
   ```

2. From the command palette, run `> SweetPad: Generate Build Server Config`. This writes a `buildServer.json` at the
   workspace root that runs `sweetpad bsp serve`. SweetPad writes it for you on the first build too.
3. Open a Swift file — SourceKit-LSP starts the server, which resolves build settings directly from the project.

After that, autocomplete should work. ✅

:::note

Already using `xcode-build-server`? Nothing changes for you. A workspace that has a `buildServer.json` written by
another tool keeps that tool, so an existing setup is never swapped out underneath you — that includes a
`buildServer.json` you maintain by hand. To move such a workspace to the built-in server, run
`> SweetPad: Set up Swift code intelligence (BSP)`, or delete the file and build once.
`> SweetPad: Diagnose BSP (Doctor)` reports which server is in use and why.

:::

A few things to know:

- The server runs through the [`sweetpad` CLI](../cli/getting-started-cli.md), so it needs to be installed:
  `brew install sweetpad-dev/tap/sweetpad`, or the Tools panel. SweetPad tells you once per workspace if it's
  missing, and `> SweetPad: Diagnose BSP (Doctor)` reports it too.
- An `.xcworkspace` resolves each file against whichever member project declares its target, so a CocoaPods or
  multi-project workspace works the same as a single `.xcodeproj`.
- A Swift package takes a different route: SourceKit-LSP reads `Package.swift` and indexes it natively, so SweetPad
  writes no `buildServer.json` at all. If one is already sitting in the package directory, delete it — its presence
  overrides that native support.
- Headers borrow the sysroot, search paths and language dialect of a neighbouring source file — the `.m` beside
  `Foo.h`, or the nearest one in the same folder — so a `.h` resolves even though no target compiles it directly.
- The `buildServer.json` it writes names the installed CLI, so it keeps resolving across extension updates. If the
  CLI itself moves or is uninstalled, SweetPad rewrites the file the next time the window loads, and
  `> SweetPad: Generate Build Server Config` does the same on demand.

## Setup with xcode-build-server

To use the external tool instead, point the provider at it:

```json title=".vscode/settings.json"
{
  "sweetpad.buildServer.provider": "xcode-build-server"
}
```

1. Install the [Swift](https://marketplace.visualstudio.com/items?itemName=swiftlang.swift-vscode) extension from the
   Marketplace and [xcode-build-server](https://github.com/SolaWing/xcode-build-server) from Homebrew:

   ```bash
   brew install xcode-build-server --head
   ```

2. From the command palette, run `> SweetPad: Generate Build Server Config`. This creates a `buildServer.json` at
   the workspace root that points SourceKit-LSP at your Xcode build outputs.

3. Build the project once (▶️ in the Build view). Without a successful build there are no build logs for
   `xcode-build-server` to parse, so autocomplete looks "stuck".

To go back to the built-in server, run `> SweetPad: Set up Swift code intelligence (BSP)` — it resets the provider
and regenerates `buildServer.json` in one step.

## When autocomplete doesn't work

Run `> SweetPad: Diagnose BSP (Doctor)` from the command palette. It checks the whole chain for your active
provider — `buildServer.json` is present and valid, the build-server tool is available, a scheme is
selected, the Xcode developer directory resolves — and prints a ✓/✗ report with a fix hint for each failure in the
**SweetPad: BSP** output channel.

To watch what the build server is doing, run `> SweetPad: Show BSP logs`. With the built-in server this opens a
live log stream; tune its verbosity with `sweetpad.buildServer.logLevel` (`off`, `error`, `info`, or `debug` —
`debug` includes the raw JSON-RPC traffic). With `xcode-build-server`, logging goes to a file instead — set
`XBS_LOGPATH` via `sweetpad.xcodebuildserver.serverEnv` (see below).

## Auto-regenerate buildServer.json

By default SweetPad regenerates `buildServer.json` whenever you build or change the default scheme — handy if you
switch between schemes frequently. If you maintain a custom `buildServer.json` (e.g. backed by Swift Build, or a
language server with background indexing), turn that off so SweetPad doesn't overwrite your file:

```json title=".vscode/settings.json"
{
  "sweetpad.build.autoGenerateBuildServerConfig": false,
  "sweetpad.build.autoRestartSwiftLSP": false
}
```

The two settings are paired: `autoGenerateBuildServerConfig` controls the file; `autoRestartSwiftLSP` controls
whether the Swift language server is restarted after each build / scheme regeneration. Disable both if you have a
build server that does its own indexing.

The explicit `> SweetPad: Generate Build Server Config` command always regenerates and restarts the LSP, regardless
of these settings.

## Diagnostics from the build log

SweetPad surfaces Swift compiler errors and warnings from the build log as inline VSCode diagnostics — squiggles in
the editor and entries in the Problems panel. They're on by default.

If a third-party tool is providing diagnostics (Swift LSP with background indexing, or a custom error reporter) you
may want to silence SweetPad's pass to avoid duplicate squiggles:

- `> SweetPad: Disable LSP Diagnostics` — turns the live diagnostic stream off for this workspace.
- `> SweetPad: Enable LSP Diagnostics` — turns it back on.

Both drive `xcode-build-server`'s logging, so they apply only to that provider. With the built-in server, turn up
`sweetpad.buildServer.logLevel` and run `> SweetPad: Show BSP logs` instead.

## Use a custom xcode-build-server

If you've installed `xcode-build-server` somewhere outside `PATH`, or you're using a fork, point SweetPad at the
binary you want:

```json title=".vscode/settings.json"
{
  "sweetpad.xcodebuildserver.path": "/opt/homebrew/bin/xcode-build-server"
}
```

You can also pass environment variables to the long-running server process that SourceKit-LSP launches — useful for
turning on the server's own logging, or pointing it at a non-default cache:

```json title=".vscode/settings.json"
{
  "sweetpad.xcodebuildserver.serverEnv": {
    "XBS_LOGPATH": "/tmp/sweetpad-xbs.log"
  }
}
```
