---
sidebar_position: 16
sidebar_label: Editor debugging
---

# Editor debugging

Breakpoints, stepping, and variables in your editor's own debugger come from Xcode's `lldb-dap`, a
Debug Adapter Protocol server that ships with Xcode. It can launch a program or attach to a process,
but it can't build a scheme or start an app on a simulator or a device.

`sweetpad dap` fills that gap. Your editor starts it as its debug adapter. It builds the app, installs
it, starts it, and then hands the process to lldb-dap. From there the editor talks to lldb-dap as usual.

```bash
sweetpad dap init --editor nvim   # or --editor zed
```

It works with simulators, the Mac (including Mac Catalyst), and devices on iOS 17 and later. Swift
variables and expressions behave as they do in Xcode, since lldb-dap uses Xcode's own LLDB.

:::note

Using VS Code? The extension uses `sweetpad dap` when the CLI is installed, with CodeLLDB as a
fallback. See the [extension's debugging page](../vscode/debug.md) instead.

:::

## What a session does

When you start debugging, the debug console shows the same short build log `sweetpad run` prints.
Errors and warnings link to their file and line, and the editor's progress indicator names the file
being compiled. Cancelling that progress, or stopping the session, stops the build.

Once the build succeeds, the app starts suspended, the debugger attaches, and your breakpoints are set
before any of your code runs. The app's `print` output and its `os_log` and `Logger` entries go to the
debug console, at the `info` level `sweetpad run` uses.

Stopping the session stops the app, unless the editor asks to detach. Restarting is stop and start
again, which rebuilds.

## Neovim

`dap init --editor nvim` writes a block into your project's `.nvim.lua`. The block registers the adapter
with [nvim-dap](https://github.com/mfussenegger/nvim-dap) and adds a "SweetPad: Build and Run"
configuration for Swift, Objective-C, C, and C++ files:

```lua
dap.adapters.sweetpad = { type = "executable", command = "/opt/homebrew/bin/sweetpad", args = { "dap" } }
```

Neovim reads `.nvim.lua` when `exrc` is on (`vim.o.exrc = true` in your config) and you have trusted
the file. Then start debugging with `:DapContinue`. Running `dap init` again replaces the block and
leaves the rest of the file alone.

## Zed

`dap init --editor zed` adds a "SweetPad: Build and Run" scenario to `.zed/debug.json`, for the debug
adapter of Zed's Swift extension. That adapter runs whichever binary your Zed settings name for it, so
point it at SweetPad:

```json
{
  "dap": {
    "Swift": { "binary": "/opt/homebrew/bin/sweetpad-dap" }
  }
}
```

Zed starts the adapter without arguments, and `sweetpad-dap` is the name under which SweetPad serves a
debug session without any. Homebrew installs it next to `sweetpad`; for a build from source, create it
with `ln -s sweetpad sweetpad-dap` in the same directory. `dap init` prints the line with the right path.

With that setting, Zed sends every Swift debug session to SweetPad. A configuration that names a
`program` (a Swift package's executable, say) goes to lldb-dap unchanged, so those keep working.

## Launch configuration

Every field is optional. Whatever you leave out falls back to the scheme and destination in
`sweetpad.toml`, in your config, or remembered from your last `sweetpad run`, the same way the
command-line verbs resolve them.

```json
{
  "type": "sweetpad",
  "request": "launch",
  "name": "SweetPad: Build and Run",
  "cwd": "${workspaceFolder}",
  "scheme": "MyApp",
  "configuration": "Debug",
  "destination": "iPhone 16 Pro",
  "args": ["-MyFlag", "YES"],
  "env": { "LOG_LEVEL": "debug" },
  "xcodebuildArgs": ["-allowProvisioningUpdates"],
  "lldb": { "sourceMap": [] }
}
```

| Field                    | Meaning                                                                                  |
| ------------------------ | ---------------------------------------------------------------------------------------- |
| `cwd`                    | Where to find the project from. Use your workspace folder.                               |
| `workspace`, `project`   | A specific `.xcworkspace` or `.xcodeproj`, when the folder has several.                  |
| `scheme`, `configuration` | As for `sweetpad run --scheme` and `--configuration`.                                   |
| `destination`            | What `--on` takes: a simulator or device name, a UDID, `booted`, or `mac`. See below for Mac Catalyst. |
| `args`                   | Arguments for the app.                                                                   |
| `env`                    | Environment for the app, as an object or a list of `KEY=VALUE`.                          |
| `xcodebuildArgs`         | Extra xcodebuild arguments, as `sweetpad run` takes after `--`.                          |
| `lldb`                   | lldb-dap's own fields (`sourceMap`, `initCommands`, `preRunCommands`, ...), passed to it as they are. |

A debug session has no terminal to prompt in. When no scheme or destination is set anywhere, the
launch fails with the field to set. Running `sweetpad run` once in a terminal also works: the scheme
and simulator you pick there are remembered.

### Mac Catalyst and Designed for iPad

A `destination` that contains `=` is passed to xcodebuild as its `-destination` value. That's how to
name the two ways an iOS app runs on the Mac:

```json
"destination": "platform=macOS,variant=Mac Catalyst"
"destination": "platform=macOS,variant=Designed for iPad"
```

A Designed for iPad app only runs when macOS starts it, so the debugger waits for it to appear and
attaches then. The first few milliseconds of launch run before the debugger is attached.

### Attaching to a running app

An `attach` request reaches an app that's already running, without building it:

```json
{ "type": "sweetpad", "request": "attach", "scheme": "MyApp", "destination": "booted" }
```

SweetPad finds the process of the app that the scheme builds. Pass `pid` instead to attach to a
specific process on the Mac, which includes simulator apps. Stopping an attached session detaches and
leaves the app running.

## When it doesn't start

```bash
sweetpad dap doctor
```

It checks that `xcrun lldb-dap` resolves for the selected Xcode, that the Xcode is 16 or later, and that
lldb-dap answers. To use another lldb-dap, for example a newer LLVM's, set `SWEETPAD_LLDB_DAP` to its
path in the environment your editor starts the adapter with.

To see exactly what the editor and lldb-dap send each other, set `SWEETPAD_DAP_LOG` to a file path. The
file records every message in both directions.

Physical devices older than iOS 17 aren't supported, because SweetPad reaches devices through
`devicectl`. The VS Code extension keeps a CodeLLDB route for those.

The first session on a device can take minutes to reach your code. Until this Mac has a copy of the
device's system libraries, the debugger reads each one from the device as the app loads it. Xcode makes
that copy the first time it runs an app on the device: open any project in Xcode, pick the device, and
press Run, ideally with the device on a cable. The debug console says when the copy is missing.
