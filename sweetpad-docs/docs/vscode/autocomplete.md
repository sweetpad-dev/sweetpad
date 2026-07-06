---
sidebar_position: 7
slug: /autocomplete
---

# Autocomplete

SweetPad wires Xcode's build information into [SourceKit-LSP](https://github.com/swiftlang/sourcekit-lsp) so you get
real autocomplete, jump-to-definition, hover docs, and Swift compiler diagnostics in VSCode.

![autocomplete](/images/autocomplete-preview.png)

SourceKit-LSP needs a **build server** to tell it how each file is compiled. SweetPad supports two:

- **xcode-build-server** (the default) — a battle-tested external tool that parses Xcode build logs. Works with
  workspaces, projects, and SPM, but needs a successful build before autocomplete comes alive.
- **SweetPad's built-in build server** (experimental) — ships inside the extension and reads compiler arguments
  straight from the project, so autocomplete works **without building first**. Currently limited to plain
  `.xcodeproj` projects.

:::info

Swift Packages don't need any of this — SourceKit-LSP understands `Package.swift` natively. If your workspace root is
a Swift Package, autocomplete works as soon as the [Swift extension](https://marketplace.visualstudio.com/items?itemName=swiftlang.swift-vscode)
is installed.

:::

## Setup with xcode-build-server (default)

1. Install the [Swift](https://marketplace.visualstudio.com/items?itemName=swiftlang.swift-vscode) extension from the
   Marketplace and [xcode-build-server](https://github.com/SolaWing/xcode-build-server) from Homebrew:

   ```bash
   brew install xcode-build-server --head
   ```

2. From the command palette, run `> SweetPad: Generate Build Server Config`. This creates a `buildServer.json` at
   the workspace root that points SourceKit-LSP at your Xcode build outputs.

3. Build the project once (▶️ in the Build view). Without a successful build there are no build logs for
   `xcode-build-server` to parse, so autocomplete looks "stuck".

After that, autocomplete should work. ✅

## Setup with the built-in build server (experimental)

If your project is a plain `.xcodeproj`, you can skip installing `xcode-build-server` entirely:

1. Run `> SweetPad: Set up Swift code intelligence (BSP)` from the command palette. This switches
   `sweetpad.buildServer.provider` to `sweetpad` and writes a `buildServer.json` that launches the bundled server.
2. Open a Swift file — SourceKit-LSP starts the server, which resolves build settings directly from the project.
   No prior build needed.

A few things to know:

- The server runs on Node.js, so `node` must be on your `PATH`.
- `.xcworkspace` and Swift Package projects aren't handled by the built-in server yet — SweetPad falls back to the
  `xcode-build-server` flow for those.
- The `buildServer.json` it writes points at a file inside the installed extension. After an extension update that
  path can go stale — if autocomplete stops working, re-run `> SweetPad: Generate Build Server Config`.

To switch back, set the provider in your settings:

```json title=".vscode/settings.json"
{
  "sweetpad.buildServer.provider": "xcode-build-server"
}
```

## When autocomplete doesn't work

Run `> SweetPad: Diagnose BSP (Doctor)` from the command palette. It checks the whole chain for your active
provider — `buildServer.json` is present and valid, the build-server tool (or Node.js) is available, a scheme is
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
