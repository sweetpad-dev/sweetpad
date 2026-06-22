---
sidebar_position: 2
---

# Autocomplete

This extension wires Xcode's build output into [SourceKit-LSP](https://github.com/swiftlang/sourcekit-lsp) so you get
real autocomplete, jump-to-definition, hover docs, and Swift compiler diagnostics in VSCode.

![autocomplete](/images/autocomplete-preview.png)

## Setup

1. Install the [Swift](https://marketplace.visualstudio.com/items?itemName=swiftlang.swift-vscode) extension from the
   Marketplace and [xcode-build-server](https://github.com/SolaWing/xcode-build-server) from Homebrew:

   ```bash
   brew install xcode-build-server --head
   ```

2. From the command palette, run **`> SweetPad: Generate Build Server Config`**. This creates a `buildServer.json` at
   the workspace root that points SourceKit-LSP at your Xcode build outputs.

3. Build the project once (▶️ in the Build view). Without a successful build there are no build logs for
   `xcode-build-server` to parse, so autocomplete looks "stuck".

After that, autocomplete should work. ✅

## Auto-regenerate `buildServer.json`

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

## Use a custom `xcode-build-server`

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
