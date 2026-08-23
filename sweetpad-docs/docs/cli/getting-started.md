---
sidebar_position: 1
sidebar_label: Get started
---

# Get started with the CLI

The `sweetpad` command-line tool builds, runs, and tests your Xcode apps from the terminal, with no editor
needed. This page gets you from install to a running app in a few minutes. You need a Mac with Xcode
installed.

## 1. Install

Install with [Homebrew](https://brew.sh/):

```bash
brew install sweetpad-dev/tap/sweetpad
```

Check that it worked:

```bash
sweetpad --version
```

## 2. Get a project

You can start a brand-new app or point SweetPad at one you already have.

### Create a new project

`sweetpad project new` scaffolds a minimal SwiftUI app. Run it with no options and it walks you through
a short set of questions (project name, iOS or macOS, bundle identifier, and so on) and then creates the
project for you:

```bash
sweetpad project new
```

Prefer to skip the questions? Pass a name (and any options you want) and it uses sensible defaults:

```bash
sweetpad project new MyApp --platform ios
```

When it's done, hop into the new folder:

```bash
cd MyApp
```

[Starting a project](./project-new.md) has every option and what the scaffold contains.

### Or use an existing project

Already have a project? Just `cd` into it, anywhere inside a folder that has an `.xcworkspace`,
`.xcodeproj`, or `Package.swift`. SweetPad finds the project by looking in the current folder and its
parents, just like `git` does.

```bash
cd ~/Developer/MyApp
```

## 3. See where you are

Run `sweetpad status` to see what SweetPad thinks it's working with:

```bash
sweetpad status
```

The first time, it may not have picked a scheme or a place to run yet. That's fine; the next step
sorts it out.

## 4. Build and run

Run your app with a single command:

```bash
sweetpad run
```

The first time in a project, SweetPad asks which scheme to build and which simulator or device to run
on, then remembers your choice so it won't ask again. It builds the app, launches it, and streams the
app's logs right in your terminal.

Want to skip the questions and just say where to run? Use `--on` with a simulator name, `mac`, or
`booted` (whatever simulator is already open):

```bash
sweetpad run --on "iPhone 16 Pro"
```

## 5. Try a few more commands

Here are the everyday ones:

```bash
# See everywhere you can run — simulators, devices, and macOS
sweetpad devices

# Just build, don't run
sweetpad build

# Run your tests
sweetpad test

# Format your Swift files
sweetpad format
```

Most commands ask you to pick a scheme or destination the first time, then remember it. Run
`sweetpad status` any time to see the current choices, or change them with `sweetpad context`.

## Getting help

Every command explains itself with `--help`:

```bash
sweetpad --help              # all commands
sweetpad run --help          # options for one command
```

And there are a few longer guides built right into the tool:

```bash
sweetpad help                # list the guides
sweetpad help destinations   # how to pick where to run
sweetpad help config         # settings you can save
```

## Where to go next

The [Overview](./overview.md) is the fuller tour if you'd rather read one page than pick a topic.
Otherwise:

**The daily loop.** [Build and run](./build-and-run.md) goes deeper on what you just did, including
reading a failed build. [Testing](./testing.md) covers narrowing a run, watch mode, and getting at
what the tests recorded. [Formatting](./formatting.md) is short. [Hot reload](./hot-reload.md) skips
the rebuild entirely and injects each save into the running app.

**Where it runs.** [Destinations and devices](./destinations.md) is everything about choosing where a
build goes. [Simulators](./simulators.md) drives one: screenshots, push payloads, permissions.
[App lifecycle and debugging](./app-lifecycle.md) covers logs, lldb, and crash reports.

**Your project.** [Starting a project](./project-new.md) scaffolds a new one.
[Project and dependencies](./project.md) reads and edits what's in the project.
[Tuist and XcodeGen](./generated-projects.md) covers generated ones.
[Archive and distribute](./archive.md) ships it. [Git merge drivers](./merge.md) stop `.pbxproj`
conflicts from ruining your afternoon.

**Setup and automation.** [Configuration](./configuration.md) is how you stop answering the same
prompts, for you or your team. [Editor autocomplete](./autocomplete.md) wires up completions in
Neovim, Zed, Helix, or Emacs. [Scripts and CI](./scripts-and-ci.md) covers JSON output, exit codes,
and a working GitHub workflow.

**When something's wrong.** [Troubleshooting](./troubleshooting.md) starts from `sweetpad doctor` and
works outward. The [CLI reference](./reference.md) lists every command, flag, config key, and exit
code on one page.

Working in an editor? The [VS Code extension](../vscode/getting-started.md) is a separate product that
does all of this from the VS Code sidebar. You don't need it for anything above.
