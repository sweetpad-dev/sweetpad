---
sidebar_position: 16
slug: /settings
sidebar_label: Settings reference
---

# Settings reference

Every setting the SweetPad extension reads, in one place. Put them in `.vscode/settings.json` to keep
them per-project (recommended — most of these describe the project, not you), or in your user settings
to apply them everywhere.

Each table lists the setting key, its default, and what it does. Settings that deserve a longer story
link to the page that tells it.

## Build

Covered in depth in [Build & Run](./build.md).

| Setting                                  | Default | What it does                                                                                                       |
| ---------------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------------ |
| `sweetpad.build.xcodeWorkspacePath`      | —       | Path to the workspace or project to build, absolute or relative to the VSCode folder. Unset = auto-detect (asks if several are found). |
| `sweetpad.build.configuration`           | —       | Build configuration to use (e.g. `Debug`, `Release`). Unset = asks and remembers.                                  |
| `sweetpad.build.scheme`                  | —       | Scheme to build. Unset = asks and remembers.                                                                       |
| `sweetpad.build.destination`             | —       | Simulator, device, or Mac to build and run on (`id` + `type`). Unset = asks and remembers.                          |
| `sweetpad.build.args`                    | `[]`    | Extra arguments appended to every `xcodebuild` invocation.                                                          |
| `sweetpad.build.env`                     | `{}`    | Environment variables for the `xcodebuild` process itself. `${env:VAR}` expands; `null` unsets an inherited value. |
| `sweetpad.build.launchArgs`              | `[]`    | Arguments passed to your app when it launches.                                                                      |
| `sweetpad.build.launchEnv`               | `{}`    | Environment variables passed to your app when it launches.                                                          |
| `sweetpad.build.derivedDataPath`         | —       | Custom DerivedData location, absolute or relative to the VSCode folder.                                             |
| `sweetpad.build.xcodebuildCommand`       | —       | Alternative `xcodebuild` binary — e.g. Xcode-beta's copy. `${env:VAR}` expands.                                     |
| `sweetpad.build.swiftCommand`            | —       | Alternative `swift` binary, for Swift Package builds. `${env:VAR}` expands.                                          |
| `sweetpad.build.arch`                    | —       | Force an architecture (`arm64` or `x86_64`). Useful for Rosetta builds on Apple Silicon.                             |
| `sweetpad.build.rosettaDestination`      | `false` | Prefer the Rosetta variant of the target simulator.                                                                  |
| `sweetpad.build.allowProvisioningUpdates`| `true`  | Let Xcode refresh provisioning profiles during the build.                                                            |
| `sweetpad.build.xcbeautifyEnabled`       | `true`  | Pipe build logs through `xcbeautify` when it's installed.                                                            |
| `sweetpad.build.bringSimulatorToForeground` | `true` | Bring the Simulator window to the front when launching the app. Turn off to stay in VSCode.                        |
| `sweetpad.build.schemes.include`         | `[]`    | Only show schemes matching these patterns in the Build view (`*` wildcard). Empty = show all.                       |
| `sweetpad.build.schemes.exclude`         | `[]`    | Hide schemes matching these patterns, applied after `include`.                                                      |
| `sweetpad.build.autoRefreshSchemes`      | `true`  | Re-read the scheme list when project files change (`Package.swift`, `.xcodeproj`, `.xcworkspace`).                  |
| `sweetpad.build.autoRefreshSchemesDelay` | `500`   | Milliseconds to wait after a file change before refreshing schemes.                                                 |

## App logs

How log streaming works — and which settings apply to which destination — is covered in
[Simulators](./simulators.md#logs-from-the-simulator) and [Devices](./devices.md#stream-os_log-and-print-output-from-the-device).

| Setting                                             | Default         | What it does                                                                             |
| ---------------------------------------------------- | --------------- | ----------------------------------------------------------------------------------------- |
| `sweetpad.build.logStreamEnabled`                    | `true`          | Stream your app's logs into the build terminal on launch.                                 |
| `sweetpad.build.logStreamPredicate`                  | —               | Custom predicate for Apple's log tool (simulators and macOS only). `${bundleId}` and `${processName}` are substituted. |
| `sweetpad.build.pymobiledevice3Path`                 | `pymobiledevice3` | Binary used to stream logs from physical devices. Leave as-is to resolve via `PATH`.     |
| `sweetpad.build.pymobiledevice3ExtraArgs`            | `[]`            | Extra arguments appended to the device log stream command.                                |
| `sweetpad.build.pymobiledevice3SubsystemAllowList`   | `[]`            | When non-empty, keep only device log entries whose subsystem matches (e.g. `com.myapp.*`). |
| `sweetpad.build.pymobiledevice3SubsystemDenyList`    | `["com.apple.*"]` | Drop device log entries whose subsystem matches.                                        |
| `sweetpad.build.deviceTunnelAutoStart`               | `false`         | Auto-start the iOS 17+ developer tunnel (needs sudo) before launching on a device.        |

## Autocomplete and code intelligence

Covered in depth in [Autocomplete](./autocomplete.md).

| Setting                                        | Default              | What it does                                                                              |
| ----------------------------------------------- | -------------------- | ------------------------------------------------------------------------------------------ |
| `sweetpad.buildServer.provider`                 | `sweetpad`           | Which build server backs SourceKit-LSP: `sweetpad` (built-in) or `xcode-build-server` (external, parses build logs). A workspace that already has a `buildServer.json` from another tool keeps it until you set this explicitly. |
| `sweetpad.buildServer.logLevel`                 | `info`               | Verbosity of the built-in server's live log stream (`off`, `error`, `info`, `debug`). The log file always captures everything. |
| `sweetpad.buildServer.logPath`                  | —                    | Custom location for the built-in server's log file. Relative paths and `${workspaceFolder}` resolve against the workspace. |
| `sweetpad.build.autoGenerateBuildServerConfig`  | `true`               | Regenerate `buildServer.json` on build and scheme change. Turn off if you maintain your own file. |
| `sweetpad.build.autoRestartSwiftLSP`            | `true`               | Restart the Swift language server after builds and project regeneration.                   |
| `sweetpad.xcodebuildserver.autogenerate`        | `true`               | Regenerate the build server config when the default scheme changes.                        |
| `sweetpad.xcodebuildserver.path`                | —                    | Custom `xcode-build-server` binary, if it's not on `PATH`.                                  |
| `sweetpad.xcodebuildserver.serverEnv`           | `{}`                 | Environment variables for the long-running xcode-build-server process (e.g. `XBS_LOGPATH`). |

## Testing

Covered in depth in [Tests](./tests.md).

| Setting                          | Default | What it does                                              |
| --------------------------------- | ------- | ----------------------------------------------------------- |
| `sweetpad.testing.configuration`  | —       | Build configuration used when running tests (e.g. `Testing`). Unset = same flow as building. |
| `sweetpad.testing.baseClasses`    | `[]`    | Extra class names to recognize as XCTest base classes, so test classes inheriting from a shared base are discovered. `XCTestCase` is always recognized. |

## Formatting

Covered in depth in [Format code](./format.md).

| Setting                          | Default | What it does                                                                                             |
| --------------------------------- | ------- | ---------------------------------------------------------------------------------------------------------- |
| `sweetpad.format.path`            | Xcode's bundled swift-format | Formatter executable — a command on `PATH` or an absolute path.                             |
| `sweetpad.format.args`            | —       | Arguments for the formatter. `${file}` is replaced with the file path (as its own array item).             |
| `sweetpad.format.selectionArgs`   | —       | Arguments for format-selection. Placeholders: `${file}`, `${startLine}`, `${endLine}`, `${startOffset}`, `${endOffset}`. Unset = whole file is formatted. |

## Hot reload

Covered in depth in [Hot reload](./hot-reload.md).

| Setting                        | Default | What it does                                                                    |
| ------------------------------- | ------- | --------------------------------------------------------------------------------- |
| `sweetpad.hotReload.enabled`    | `false` | Launch simulator/macOS builds with InjectionNext wired up so saves reload live.  |
| `sweetpad.hotReload.dylibPath`  | —       | Override the injection dylib path when InjectionNext isn't in `/Applications/`.   |

## Project generators

Covered in depth in [Tuist](./tuist.md) and [XcodeGen](./xcodegen.md).

| Setting                        | Default | What it does                                                                     |
| ------------------------------- | ------- | ---------------------------------------------------------------------------------- |
| `sweetpad.tuist.autogenerate`   | `false` | Re-run Tuist generate when Swift files are added or removed. Restart VSCode to apply. |
| `sweetpad.tuist.generate.env`   | `{}`    | Environment variables for every Tuist generate run (dynamic configuration).       |
| `sweetpad.xcodegen.autogenerate`| `false` | Re-run XcodeGen when Swift files are added or removed. Restart VSCode to apply.    |

## Agent CLI

Covered in depth in [Agent CLI & RPC server](../cli/agent-cli.md).

| Setting                      | Default | What it does                                                                        |
| ----------------------------- | ------- | ------------------------------------------------------------------------------------- |
| `sweetpad.cliServer.enabled`  | `false` | Run the JSON-RPC server so the `sweetpad vscode` CLI (and AI agents) can drive this window. |

## System

Most of these matter when something misbehaves — see [Troubleshooting](./troubleshooting.md).

| Setting                                | Default | What it does                                                                             |
| --------------------------------------- | ------- | ------------------------------------------------------------------------------------------ |
| `sweetpad.system.logLevel`              | `info`  | Extension log verbosity (`debug`, `info`, `warn`, `error`). Restart VSCode to apply.       |
| `sweetpad.system.taskExecutor`          | `v3`    | How tasks run: `v3` uses a real pseudoterminal (colors, TUI support); `v2` uses plain pipes. |
| `sweetpad.system.autoRevealTerminal`    | `true`  | Bring the terminal panel forward when a command or task starts.                             |
| `sweetpad.system.showProgressStatusBar` | `true`  | Show task progress in the status bar.                                                       |
| `sweetpad.system.xcodebuildFallback`    | `false` | If the bundled build-settings resolver fails, retry via `xcodebuild -showBuildSettings` (slower, but survives exotic projects). |
| `sweetpad.system.enableSentry`          | `false` | Send error reports to the maintainers. Off by default; reports may include project details. |
| `sweetpad.shellEnv.shell`               | `$SHELL` | Shell used to resolve your login environment (`PATH` etc.).                                |
| `sweetpad.shellEnv.timeout`             | `5000`  | Milliseconds to wait for the login shell to dump its environment.                           |
