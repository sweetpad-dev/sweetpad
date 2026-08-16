---
sidebar_position: 3
slug: /cli-reference
sidebar_label: Reference
---

# CLI reference

A map of the `sweetpad` command-line tool: the commands you'll reach for, the global flags, the
configuration files, destination specifiers, and exit codes. For a guided tour, start with
[Get started with the CLI](./getting-started-cli.md) or the [overview](./cli.md).

Commands follow a resource-then-action grammar — `sweetpad scheme list`, `sweetpad simulator boot` —
and the everyday actions have top-level shortcuts (`sweetpad build`, `sweetpad test`, `sweetpad run`).
The CLI describes itself, and that is the authority: `sweetpad --help` lists the full command tree,
`sweetpad <command> --help` covers the flags and subcommands this page doesn't repeat, and
`sweetpad help <topic>` explains config, environment, exit codes, destinations, and hot reload.

## Commands

### Everyday

| Command            | What it does                                                                     |
| ------------------- | ---------------------------------------------------------------------------------- |
| `sweetpad run`      | Build, install, launch, and follow logs — the flagship loop. Press `r` to rebuild. |
| `sweetpad build`    | Compile the resolved scheme. `--watch` rebuilds on every Swift save; `--clean` first cleans. |
| `sweetpad test`     | Run the tests. Supports `--only-testing`, `--skip-testing`, `--failed`, `--retry-flaky`, `--coverage`, `--junit`, `--watch`. |
| `sweetpad clean`    | Clean build artifacts; `--purge` also deletes DerivedData.                        |
| `sweetpad archive`  | Archive and export an `.ipa` (`--export-method`, `--output`).                     |
| `sweetpad format`   | Format Swift sources (`--check` to verify only; `--tool swiftlint` to lint). Alias: `fmt`. |
| `sweetpad devices`  | List everything runnable — macOS, simulators, connected devices — most-used first. |
| `sweetpad status`   | Show the effective build context and where each value comes from.                 |
| `sweetpad doctor`   | Diagnose the local Xcode/Swift toolchain.                                          |

### Project inspection

| Command                    | What it does                                                       |
| --------------------------- | -------------------------------------------------------------------- |
| `sweetpad project info`     | Show targets, configurations, and schemes.                          |
| `sweetpad project new`      | Scaffold a minimal SwiftUI app (see [options](#sweetpad-project-new)). |
| `sweetpad scheme list`      | List the schemes SweetPad found.                                     |
| `sweetpad settings show`    | Show resolved build settings. `--key NAME` prints one bare value for scripts. |
| `sweetpad dependency list`  | List SPM dependencies and their locked versions. Alias: `dep`.       |
| `sweetpad dependency add`   | Add a package by URL and link a product to a target.                 |
| `sweetpad dependency remove`| Remove a package, or unlink one product from one target.             |
| `sweetpad dependency update`| Update resolved versions, or change a requirement.                   |
| `sweetpad dependency resolve` | Resolve dependencies into the lockfile.                            |

### App lifecycle

`sweetpad run` is shorthand for `sweetpad app run`; the rest of the lifecycle lives under `app`:

| Command                  | What it does                                              |
| ------------------------- | ----------------------------------------------------------- |
| `sweetpad app install`    | Build and install without launching.                       |
| `sweetpad app launch`     | Launch an already-installed app.                           |
| `sweetpad app debug`      | Run under lldb — attached to a suspended simulator launch, or owning the launch on macOS. `--batch` with `--cmd` drives lldb from a script. |
| `sweetpad app diagnose`   | Run under lldb, catch the first crash or Objective-C exception, print a structured report, and quit. Bounded by `--timeout`; `-o json` for the machine-readable form. |
| `sweetpad app logs`       | Follow the running app's logs — simulator, device, or macOS, where os_log and a detached launch's captured stdout arrive on one stream. `--last <dur>` prints history instead; `--until <text>` stops at a match. |
| `sweetpad app stop`       | Terminate the running app.                                 |
| `sweetpad app uninstall`  | Remove the app from a simulator or device.                 |
| `sweetpad app open-url`   | Open a URL on a simulator — deep links and universal links. |
| `sweetpad app screenshot` | Save a PNG of the running app — a macOS app's window, or the simulator it launched on. |
| `sweetpad app ui`         | Read or drive a macOS app's UI through accessibility: `ui tree`, `ui click`, `ui type`. |

### Extra xcodebuild arguments

Anything after `--` goes to `xcodebuild` verbatim — its own flags, or `KEY=VALUE` build-setting
overrides:

```bash
# xcodebuild flags
sweetpad app install -- -allowProvisioningUpdates
sweetpad build -- -parallelizeTargets
sweetpad archive -- -allowProvisioningUpdates

# build-setting overrides
sweetpad build -- SWIFT_ACTIVE_COMPILATION_CONDITIONS="DEBUG STAGING"
sweetpad app install --device -- DEVELOPMENT_TEAM=ABCDE12345

# both at once, and combined with SweetPad's own flags
sweetpad app install --on "iPhone 16 Pro" -- -derivedDataPath ./build ENABLE_TESTABILITY=YES
```

The commands that run a build accept it: `build`, `test`, `archive`, and `app run`, `app install`,
`app debug`, `app diagnose`. The `app` commands that only act on an already-installed app —
`launch`, `uninstall`, `logs`, `stop` — reject it rather than accept arguments that would reach no
`xcodebuild`.

A `-derivedDataPath` in the tail is honored when locating the built `.app`, so the bundle SweetPad
installs is the one the build just wrote:

```bash
sweetpad app install -- -derivedDataPath /tmp/dd   # builds and installs from /tmp/dd
```

Overrides that move the product somewhere the locator can't follow are rejected up front, before a
build is spent on them — use `-derivedDataPath` instead:

```bash
sweetpad app install -- SYMROOT=/tmp/out
# error: `-- SYMROOT=…` relocates the built product where the app locator
#        can't follow; use `-- -derivedDataPath <dir>` instead
```

#### Writing them down for the whole repo

An argument every build in a project needs belongs in `sweetpad.toml`, not in your shell history.
The `[xcodebuild] args` list is added to every command that builds, so it reaches the builds inside
`app run`/`install`/`debug`/`diagnose` as well as `build`, `test`, and `archive`:

```toml
# sweetpad.toml (committed)
scheme = "MyApp"

[xcodebuild]
args = ["-skipMacroValidation", "-disablePackageRepositoryCache"]
```

A `--` tail is appended after the file's arguments, so typing one wins — `xcodebuild` takes the last
value for a repeated flag or setting:

```bash
sweetpad build -- SWIFT_ACTIVE_COMPILATION_CONDITIONS=DEBUG   # beats the file's value
```

`sweetpad status` prints the effective list, so a build shaped by a file you didn't write still says
where it came from.

Arguments SweetPad settles itself are refused in the file, naming the key to use instead: `-scheme`,
`-configuration`, `-destination`, `-sdk`, `-workspace`, `-project`, and `-resultBundlePath` (SweetPad
writes and reads back its own). `-derivedDataPath` is refused too — a relative value in a committed
file would resolve against the working directory rather than the file, meaning a different place
depending on where the command ran. Pass it per command instead. Swift packages ignore the table
entirely: they build with `swift build`, which knows none of `xcodebuild`'s flags.

:::tip

For `KEY=VALUE` build settings, an `.xcconfig` is usually the better committed home — Xcode honors it
too, so ⌘B and `sweetpad build` stay in agreement. Put flags in `[xcodebuild] args`; put build
settings in an xcconfig unless you specifically want them only when building through SweetPad.

:::

### Simulators

Alias: `sim`. Most take an optional target (name or UDID) and default to the booted simulator.

| Command                        | What it does                                                 |
| ------------------------------- | --------------------------------------------------------------- |
| `sweetpad simulator list`       | List available simulators.                                     |
| `sweetpad simulator boot`       | Boot a simulator (prompts when omitted; `--wait` blocks until ready). |
| `sweetpad simulator shutdown`   | Shut down a simulator.                                         |
| `sweetpad simulator open`       | Open the Simulator.app window.                                 |
| `sweetpad simulator screenshot` | Save a PNG (`--clipboard` copies instead).                     |
| `sweetpad simulator record`     | Record the screen to an mp4 — Ctrl-C stops and finalizes.      |
| `sweetpad simulator appearance` | Switch light/dark appearance.                                  |
| `sweetpad simulator status-bar` | Set a clean status bar for screenshots (9:41, full bars); `--clear` resets. |
| `sweetpad simulator location`   | Set the simulated GPS location.                                |
| `sweetpad simulator push`       | Deliver an APNs push payload from a JSON file.                 |
| `sweetpad simulator privacy`    | Grant, revoke, or reset a privacy permission for an app.       |
| `sweetpad simulator media-add`  | Add photos and videos to the media library.                    |
| `sweetpad simulator create`     | Create a new simulator.                                        |
| `sweetpad simulator clone`      | Clone a shut-down simulator under a new name.                  |
| `sweetpad simulator erase`      | Erase contents and settings (simulator must be shut down).     |
| `sweetpad simulator delete`     | Delete a simulator — no undo (`--yes` skips the prompt).       |

### Context and configuration

| Command                     | What it does                                                              |
| ---------------------------- | ---------------------------------------------------------------------------- |
| `sweetpad context show`      | Show the remembered scheme, configuration, and destination.                 |
| `sweetpad context select`    | Change a remembered value interactively (`--testing` for the test context). |
| `sweetpad context set`       | Set a value without prompting — script- and CI-friendly.                    |
| `sweetpad context remove`    | Clear one remembered value, or `--all` for the whole context.               |
| `sweetpad context alias`     | Name a destination (`context alias work-phone <UDID>`), then use `--on work-phone` anywhere. |
| `sweetpad open <what>`       | Open the project in `xcode`, the `sim` window, the `dd` (DerivedData) folder, or your `config` file. |

### Maintenance and integration

| Command                        | What it does                                                        |
| ------------------------------- | ---------------------------------------------------------------------- |
| `sweetpad derived-data path`    | Print this project's DerivedData folder (`--all` for the whole store). Alias: `dd`. |
| `sweetpad derived-data size`    | Report DerivedData's on-disk size.                                    |
| `sweetpad derived-data purge`   | Delete DerivedData (`--yes` skips the prompt).                        |
| `sweetpad merge install`        | Register git merge drivers that resolve `.pbxproj` and `Package.resolved` conflicts semantically (`--global` for all repos). |
| `sweetpad merge run`            | Resolve conflicted project files in the current merge by hand.        |
| `sweetpad bsp init`             | Write `buildServer.json` so SourceKit-LSP autocomplete works in any editor. |
| `sweetpad bsp doctor`           | Check the autocomplete wiring.                                        |
| `sweetpad hot status`           | Report whether the hot-reload port is free, and which process holds it. |
| `sweetpad hot reset`            | End a hot-reload listener a dead `--hot` session left behind (`--force` for a non-sweetpad holder). |
| `sweetpad completions <shell>`  | Generate completions for bash, zsh, fish, elvish, or PowerShell.      |
| `sweetpad self-update`          | Update sweetpad (Homebrew installs run brew upgrade instead).         |
| `sweetpad help [topic]`         | Built-in guides: `config`, `environment`, `exit-codes`, `destinations`, `hot-reload`. |
| `sweetpad vscode <method>`      | Drive a running VSCode window — see [Agent CLI & RPC server](./agent-cli.md). |

## Global flags

These work on every command:

| Flag                 | What it does                                                                                    |
| --------------------- | -------------------------------------------------------------------------------------------------- |
| `-C <dir>`            | Run as if started in that folder, like `git -C`.                                                  |
| `-o, --output <mode>` | Output mode: `human` (default), `json`, `ndjson`, or `quiet`.                                     |
| `--json`              | Shorthand for `-o json`.                                                                           |
| `--non-interactive`   | Never prompt — missing choices become errors. Auto-enabled when `CI` is set.                       |
| `--gh-annotations`    | Emit GitHub Actions annotations for build/test errors. Not combinable with `-o json`/`ndjson`.     |
| `--developer-dir <dir>` | Pin the Xcode to use (same as the `DEVELOPER_DIR` environment variable).                          |
| `--no-color`          | Disable colored output (also honors `NO_COLOR`).                                                   |
| `-v, --verbose`       | Show raw tool output.                                                                              |
| `-q, --quiet`         | Suppress progress chatter (wins over `--verbose`).                                                 |

Commands that build or run also take targeting flags: `--workspace`, `--project`, `--scheme`,
`--configuration`, `--sdk`, and the two ways to say where — `--destination` (a raw specifier) or
`--on` (a human-friendly reference; the two are mutually exclusive).

## Environment variables

Every targeting flag has an environment-variable twin, so CI pipelines can set the context once:
`SWEETPAD_WORKSPACE`, `SWEETPAD_PROJECT`, `SWEETPAD_SCHEME`, `SWEETPAD_CONFIGURATION`,
`SWEETPAD_DESTINATION`, `SWEETPAD_ON`, and `SWEETPAD_SDK`. `SWEETPAD_NONINTERACTIVE=1` (or `CI=1`)
turns prompts into errors. `NO_COLOR` and `CLICOLOR_FORCE` control color. Run
`sweetpad help environment` for the full story.

## Configuration files

Three layers, from personal to shared:

- **`~/.config/sweetpad/config.toml`** — your personal defaults. A `[defaults]` table for global
  values, plus `[projects."<path to .xcodeproj/.xcworkspace/Package.swift>"]` tables for per-project
  overrides. SweetPad never writes this file; it's yours.
- **`sweetpad.toml`** next to the project — team defaults, meant to be committed. Same keys
  (`scheme`, `configuration`, `destination`, `sdk`), plus `developer_dir` and `[run]`, `[format]`,
  `[testing]`, and `[xcodebuild]` tables.
- **Remembered state** — the answers you gave to interactive prompts, stored per project. Inspect and
  change it with `sweetpad context`, not by editing files.

When the same value is set in several places, the most explicit one wins:

```
flag  >  environment variable  >  config.toml  >  sweetpad.toml  >  remembered answer  >  auto-detect
```

Typos are never silently ignored — unknown keys produce a warning on every run. See
`sweetpad help config` for every key.

## Destinations

`--on` accepts plain descriptions and resolves them against your live device list:

| You write                  | You get                                            |
| --------------------------- | ----------------------------------------------------- |
| `--on "iPhone 16 Pro"`      | The simulator or device with the closest name.       |
| `--on booted`               | Whatever simulator is already running.               |
| `--on mac`                  | Your Mac (for macOS schemes).                        |
| `--on device`               | Your connected physical device.                      |
| `--on ios` / `--on watchos` | Any destination of that platform.                    |
| `--on work-phone`           | An alias you created with `sweetpad context alias`.  |
| `--on <UDID>`               | That exact simulator or device.                      |

`--destination` takes the raw, exact form instead — e.g.
`platform=iOS Simulator,name=iPhone 16 Pro` or `platform=macOS` — which is handy in CI where fuzzy
matching would be a liability. `sweetpad devices` prints a copy-paste-ready specifier for everything
it lists.

## Exit codes and JSON output

| Code | Meaning                                                            |
| ---- | -------------------------------------------------------------------- |
| 0    | Success.                                                             |
| 1    | Generic failure.                                                     |
| 2    | Usage error — bad flags or arguments.                                |
| 3    | The build or the tests failed.                                       |
| 4    | Couldn't resolve a target — unknown scheme, destination, simulator…  |
| 5    | A required tool is missing (xcodebuild, simctl, …).                  |
| 6    | Cancelled by you — a declined prompt or Ctrl-C.                      |

With `-o json`, results arrive as a one-shot envelope `{"schema": 1, "ok": true, "data": …}`; errors
go to stderr as `{"schema": 1, "ok": false, "error": {"code", "message"}}` where the code mirrors the
exit-code taxonomy. `-o ndjson` streams one JSON event per line and ends with a result event — useful
for long commands like builds where you want progress as it happens.

:::tip

`"ok": true` means "the command executed", not "the outcome was good". A red test suite exits with
code 3 and reports its failures inside `data` — check the payload, not just the envelope.

:::

## sweetpad project new

Scaffolds a minimal SwiftUI app. With no flags on a terminal it runs a short wizard; any flag you pass
skips its question.

| Flag                        | Default                | What it does                              |
| ---------------------------- | ---------------------- | -------------------------------------------- |
| `--platform <ios\|macos>`    | `ios`                  | Target platform.                            |
| `--bundle-id <id>`           | `com.example.<Name>`   | Bundle identifier.                          |
| `--deployment-target <ver>`  | iOS 17.0 / macOS 14.0  | Minimum OS version.                         |
| `--current-dir`              | off                    | Scaffold into the current folder instead of a new one. |
| `--no-git`                   | off                    | Skip the initial git init.                   |
| `--force`                    | off                    | Allow scaffolding into a non-empty folder.   |
