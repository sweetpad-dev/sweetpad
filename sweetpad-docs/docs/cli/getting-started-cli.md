---
sidebar_position: 1
slug: /getting-started-cli
sidebar_label: Get started
---

# Get started with the CLI

The `sweetpad` command-line tool builds, runs, and tests your Xcode apps from the terminal — no editor
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
a short set of questions — project name, iOS or macOS, bundle identifier, and so on — then creates the
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

### Or use an existing project

Already have a project? Just `cd` into it — anywhere inside a folder that has an `.xcworkspace`,
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

The first time, it may not have picked a scheme or a place to run yet — that's fine, the next step
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

- The [SweetPad CLI](./cli.md) page covers every command group, saving your settings, and using the
  CLI in scripts and CI.
- The [CLI reference](./reference.md) lists every command, flag, config key, and exit code on one page.
- Prefer working in an editor? The [VSCode extension](../vscode/getting-started-vscode.md) does all of this
  from a sidebar.
