---
sidebar_position: 15
---

# SweetPad CLI

The SweetPad CLI is a standalone `sweetpad` command-line tool — "xcodebuild for humans". It builds,
runs, tests, and inspects Xcode and Swift Package projects from the terminal, with no editor running.

It's one of the two SweetPad products, a sibling to the [VSCode extension](./intro.md) — use either on
its own. It's a single native macOS binary (no Node runtime), and it shares the same build-settings
resolver the extension uses, so a scheme, destination, or configuration you pick on the command line
resolves exactly the way it would in the sidebar.

:::info

Like the extension, the CLI drives Xcode's own command-line tools under the hood, so you still need
Xcode installed.

:::

## Install

The CLI is distributed via Homebrew as a signed, notarized universal binary, independent of the VSCode
extension:

```bash
brew install sweetpad-dev/tap/sweetpad
```

Verify it:

```bash
sweetpad --version
```

Upgrade it later with `brew upgrade sweetpad`.

## Quick start

Run `sweetpad` from inside a project — a folder containing an `.xcworkspace`, `.xcodeproj`, or
`Package.swift`. It finds the project by walking up from the current directory, the same way `git`
finds its repo.

```bash
cd ~/Developer/MyApp

# Where am I? Show the resolved build context.
sweetpad status

# List everything you can run on — simulators, devices, macOS.
sweetpad devices

# Build, install, launch, and stream logs — the flagship loop.
sweetpad run
```

The first time you run inside a project, SweetPad prompts you to pick a scheme and a destination, then
remembers them per project so later commands don't ask again. Run `sweetpad status` any time to see
what's currently selected and where each value came from.

## What the CLI can do

The command tree is resource-first — a noun, then a verb (`sweetpad simulator list`). Most nouns also
have a bare shortcut for their most common verb, so `sweetpad build` is `sweetpad build start` and
`sweetpad test` is `sweetpad test run`. Run `sweetpad --help` to explore the full tree, or
`sweetpad <command> --help` for one command.

The main groups:

- **Build & run** — `build`, `run` (build + install + launch + logs), `test`, `archive` (export an
  `.ipa`), and `clean`.
- **Explore the project** — `scheme`, `project` (targets and configurations), `settings` (resolved
  build settings), and `dependency` (Swift Package dependencies).
- **Pick where to run** — `devices` lists every runnable target with its ready specifier; `simulator`
  and `context` (the remembered selection) manage the details.
- **Format** — `format` runs or lints Swift sources with `swift-format` or a formatter you point it at.
- **Maintenance** — `doctor` diagnoses the local toolchain, `derived-data` inspects and purges Xcode's
  DerivedData, and `open` jumps to the project in Xcode, the Simulator, or the config file.
- **Utilities** — `bsp` sets up SourceKit-LSP autocomplete, `merge` installs git merge drivers for
  `project.pbxproj` and `Package.resolved`, `completions` generates shell completions, and
  `self-update` upgrades the binary.

### Hot reload

`sweetpad run --hot` recompiles and injects each Swift save into the running app without relaunching,
preserving state — the CLI counterpart of the extension's [hot reload](./hot-reload.md). It's iOS
Simulator only. See `sweetpad help hot-reload` for the SwiftUI setup and recompiler options.

## Selecting a target

Every build command needs to know four things: which project container, which scheme, which
configuration, and which destination. You can supply any of them explicitly, but you rarely have to —
SweetPad resolves each value from the first source that has it, highest priority first:

1. An explicit flag (`--scheme`, `--configuration`, `--on`, `--destination`).
2. A `SWEETPAD_*` environment variable.
3. Your personal config file.
4. A committed `sweetpad.toml` next to the project (team-shared defaults).
5. The selection SweetPad remembered from a previous run.
6. Auto-discovery (a single obvious scheme, a booted simulator, and so on).

The friendliest way to choose a destination is `--on`, which takes a human reference — a fuzzy
simulator name, `booted`, `mac`, `device`, or a platform word:

```bash
sweetpad run --on "iPhone 16 Pro"
sweetpad build --on mac
sweetpad test --on booted
```

`--destination` remains the raw escape hatch when you need to pass an exact `xcodebuild` specifier.
Run `sweetpad help destinations` for the full grammar.

## Configuration

SweetPad reads two hand-authored files, and never writes to either:

- Your personal `~/.config/sweetpad/config.toml` holds global defaults plus per-project overrides.
- A committed `sweetpad.toml` next to the project is the team-shared defaults layer — scheme,
  configuration, destination, the Xcode to use, and more — that everyone on the project inherits.

Remembered selections live separately in a machine-managed state file; inspect and edit them with
`sweetpad context`, not by hand. For the full list of keys and the resolution precedence, run:

```bash
sweetpad help config
```

## Scripting and CI

Every command speaks JSON, so the CLI drops into scripts, git hooks, and CI without screen-scraping.
Pass `--json` (or `-o json`) for a single result envelope, or `-o ndjson` to stream one event per line
from the long-running verbs (`build`, `test`, logs):

```bash
sweetpad -o json settings get
sweetpad -o ndjson build
```

A few flags earn their keep in automation:

- `--non-interactive` (also `SWEETPAD_NONINTERACTIVE`, and implied by `CI`) never prompts — a missing
  scheme or destination becomes an error instead of a picker.
- `-C DIR` runs as if started in `DIR`, like `git -C`.
- `--developer-dir` pins the Xcode every spawned tool uses.
- `--gh-annotations` emits GitHub Actions `::error` annotations for build and test diagnostics, so
  failures surface inline on the PR.

Commands exit with a small, stable set of codes — `0` success, `3` build/test failure, `4` target
resolution failed, `5` a required tool is missing, and so on — so a script can branch on the outcome.
Run `sweetpad help exit-codes` for the full table.

:::tip

`ok: true` in the JSON envelope means the command ran, not that the outcome was good. A red test suite
still exits non-zero with `passed: false` in its payload — read the payload's own status field.

:::

## Built-in help topics

Beyond `--help` on each command, the CLI ships longer-form topics you can read offline:

```bash
sweetpad help                 # list the topics
sweetpad help config          # the config file: keys and precedence
sweetpad help environment     # every SWEETPAD_* variable
sweetpad help destinations    # destination specifiers and the picker
sweetpad help exit-codes      # what each exit code means
sweetpad help hot-reload      # requirements and recompilers for --hot
```

## Shell completions

Generate a completion script for your shell and load it however your shell prefers:

```bash
sweetpad completions zsh > /path/to/completions/_sweetpad
```

`bash`, `zsh`, `fish`, `elvish`, and `powershell` are all supported.

## Driving a live VSCode session

The same binary has a second half — `sweetpad vscode <method>` — that talks over JSON-RPC to a running
VSCode window, so a script or AI agent can read state and trigger builds _inside_ your editor session
rather than in a headless project. That's covered on its own page:
[Agent CLI & RPC server](./agent-cli.md).
