---
sidebar_position: 17
---

# SweetPad CLI

The SweetPad CLI is a command-line tool named `sweetpad` that builds, runs, and tests your Xcode and
Swift Package apps from the terminal — no editor needed. If you've ever wished `xcodebuild` were
friendlier, this is that.

It's the command-line side of SweetPad, alongside the [VSCode extension](./intro.md).

:::tip

New here? Start with [Get started with the CLI](./getting-started-cli.md) — install, then build and
run your app in a few minutes. This page is the fuller tour.

:::

## Installing and updating

Install with [Homebrew](https://brew.sh/):

```bash
brew install sweetpad-dev/tap/sweetpad
```

Update it later the same way you update anything else with Homebrew:

```bash
brew upgrade sweetpad
```

You'll need Xcode installed too — SweetPad uses Xcode's own tools to do the actual building.

## How commands are shaped

Most commands read as a thing followed by an action — for example `sweetpad simulator list` or
`sweetpad scheme list`. The common actions have a shortcut, so you can usually just say the thing:

- `sweetpad build` builds your app.
- `sweetpad test` runs your tests.
- `sweetpad run` builds, launches, and shows the logs.

Not sure what's available? `--help` always works:

```bash
sweetpad --help              # every command
sweetpad run --help          # options for one command
```

## Everyday commands

**Start a project.** `sweetpad project new` scaffolds a minimal SwiftUI app — run it with no options
for a short wizard, or pass a name to use defaults. Already have a project? Skip this and just `cd`
into it.

```bash
sweetpad project new              # answer a few questions
sweetpad project new MyApp        # or use defaults
```

The rest of these run from inside your project folder.

**Build and run.** `sweetpad run` is the one you'll use most — it builds the app, launches it on your
chosen simulator or device, and streams the logs into your terminal.

```bash
sweetpad run                       # build, launch, and follow logs
sweetpad run --on "iPhone 16 Pro"  # ...on a specific simulator
sweetpad build                     # just build
sweetpad test                      # run the tests
```

**See where you can run.** `sweetpad devices` lists every simulator, connected device, and macOS —
each with a copy-paste-ready name.

```bash
sweetpad devices
```

**Look at your project.** Handy when you're not sure what's inside a project:

```bash
sweetpad scheme list             # the schemes SweetPad found
sweetpad project                 # targets and configurations
sweetpad settings get            # the resolved build settings
sweetpad dependency list         # Swift Package dependencies
```

**Tidy up and fix things.**

```bash
sweetpad format                  # format your Swift files
sweetpad clean                   # remove build artifacts
sweetpad doctor                  # check your Xcode setup for problems
sweetpad open xcode              # open the project in Xcode
```

**Ship a build.** `sweetpad archive` produces an `.ipa` you can upload or distribute.

```bash
sweetpad archive
```

There's more — Swift Package tools, git merge helpers, and shell completions among them. Run
`sweetpad --help` to see the whole list.

## Choosing where to run

Commands like `build`, `run`, and `test` need to know which scheme to build and where to run it. The
easiest way to say where is `--on`, which understands plain descriptions:

```bash
sweetpad run --on "iPhone 16 Pro"   # a simulator by name
sweetpad build --on mac             # your Mac
sweetpad test --on booted           # whatever simulator is already open
```

You usually don't have to say any of this, though. The first time you build in a project, SweetPad
asks which scheme and destination to use, then **remembers your answer** so it won't ask again. Check
what's currently chosen — and where each choice came from — with:

```bash
sweetpad status
```

To change or clear the remembered choices, use `sweetpad context`. For all the ways to describe a
destination, run `sweetpad help destinations`.

## Live reload while you edit

`sweetpad run --hot` keeps your app running and applies each Swift file you save without a full
rebuild — so the app updates in place, keeping its current screen and state. It works on the iOS
Simulator. SwiftUI needs one small setup step; run `sweetpad help hot-reload` for the details.

```bash
sweetpad run --hot
```

## Saving your settings

If you'd rather not answer the scheme/destination prompt each time — or you want your whole team to
share the same defaults — you can write them down.

- Put your personal defaults in `~/.config/sweetpad/config.toml`.
- Put shared, checked-in defaults in a `sweetpad.toml` file next to your project, so everyone who
  clones the repo gets them.

For the exact keys you can set, run:

```bash
sweetpad help config
```

## Using the CLI in scripts and CI

Every command can print JSON instead of text, which makes it easy to use from scripts, git hooks, and
CI pipelines. Add `--json` for a single JSON result:

```bash
sweetpad --json settings get
```

A few options help in automation:

- `--non-interactive` never stops to ask a question — a missing scheme or destination becomes an error
  instead of a prompt. (SweetPad also turns this on automatically when it detects a CI environment.)
- `-C <folder>` runs as if you'd started in that folder.
- `--gh-annotations` makes build and test errors show up inline on your GitHub pull request.

Commands also return meaningful exit codes — `0` when everything's fine, and specific non-zero codes
for a failed build, a missing tool, and so on — so a script can react to what happened. Run
`sweetpad help exit-codes` for the list.

:::tip

When you ask for JSON, a successful response means "the command ran," not "the result was good." A
failing test run, for example, still reports its own failure inside the result — so check the status
in the payload, not just whether the command completed.

:::

## Built-in guides

Alongside `--help` on each command, the tool ships a few longer explanations you can read offline:

```bash
sweetpad help                 # list the guides
sweetpad help config          # settings you can save
sweetpad help destinations    # how to describe where to run
sweetpad help exit-codes      # what each exit code means
sweetpad help hot-reload      # setting up live reload
```

## Shell completions

Turn on tab-completion for your shell. For example, with zsh:

```bash
sweetpad completions zsh > /path/to/completions/_sweetpad
```

Bash, zsh, fish, elvish, and PowerShell are all supported.

## Controlling VSCode from the terminal

The `sweetpad` tool has a second side — `sweetpad vscode` — that talks to a running VSCode window so a
script or an AI coding agent can trigger builds and read results inside your editor session. It's an
advanced topic with its own page: [Agent CLI & RPC server](./agent-cli.md).
