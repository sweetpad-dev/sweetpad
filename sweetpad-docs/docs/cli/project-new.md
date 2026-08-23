---
sidebar_position: 10
sidebar_label: Starting a project
---

# Starting a new project

`sweetpad project new` scaffolds a minimal SwiftUI app: an `.xcodeproj`, a shared scheme, two Swift
files, and a git repository. It exists so you can try something, reproduce a bug, or start a project
without opening Xcode's template picker.

If you already have a project, skip this page entirely. `cd` into it and go to
[Build and run](./build-and-run.md).

## The short version

```console
$ sweetpad project new DocsDemo
Created DocsDemo at /path/to/DocsDemo

Next steps:
  cd DocsDemo
  sweetpad app run
```

That's a working app. `cd` in and run it:

```bash
cd DocsDemo
sweetpad run
```

Run it with no name and no flags at a terminal and you get a short wizard instead: name, platform,
bundle identifier, deployment target. Any flag you pass skips its question, and `--json` skips the
wizard entirely.

## What you get

```
DocsDemo/
├── .gitignore
├── DocsDemo/
│   ├── DocsDemoApp.swift
│   └── ContentView.swift
└── DocsDemo.xcodeproj/
    ├── project.pbxproj
    ├── project.xcworkspace/
    └── xcshareddata/xcschemes/DocsDemo.xcscheme
```

Deliberately small: an `@main` App struct and a `ContentView` with the globe-and-hello-world body
Xcode's own template produces. The scheme is shared rather than user-specific, so it's committed and
everyone who clones the repo sees the same one.

`git init` runs unless you pass `--no-git`.

## Options

| Flag                        | Default                | What it does                                       |
| --------------------------- | ---------------------- | --------------------------------------------------- |
| `--platform <ios\|macos>`   | `ios`                  | Target platform.                                   |
| `--bundle-id <id>`          | `com.example.<Name>`   | Bundle identifier.                                 |
| `--deployment-target <ver>` | iOS 17.0 / macOS 14.0  | Minimum OS version.                                |
| `--current-dir`             | off                    | Scaffold into the current directory instead of creating `./<Name>/`. |
| `--no-git`                  | off                    | Skip the initial `git init`.                       |
| `--force`                   | off                    | Allow scaffolding into a non-empty directory.      |

The name is positional and optional. With `--current-dir` it defaults to the directory's own name:

```bash
mkdir MyApp && cd MyApp
sweetpad project new --current-dir
```

A macOS app instead of an iOS one:

```bash
sweetpad project new MyMacApp --platform macos --bundle-id com.example.mymacapp
```

## After scaffolding

The first build asks which scheme and destination to use and remembers the answer, so from then on
it's one word:

```bash
cd MyApp
sweetpad run --on "iPhone 16 Pro"   # or just `sweetpad run` and answer once
```

Worth doing next: [`sweetpad bsp init`](./autocomplete.md) for completions in your editor, and a
`sweetpad.toml` if you want to skip the prompt entirely. See
[Configuration](./configuration.md#the-project-file).
