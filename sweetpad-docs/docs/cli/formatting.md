---
sidebar_position: 5
sidebar_label: Formatting
---

# Formatting

`sweetpad format` runs a Swift formatter over your project. With no arguments it formats the whole
project directory:

```bash
sweetpad format
```

Give it paths to narrow the job: files, directories, or both.

```bash
sweetpad format Sources/App
sweetpad format Sources/App/ContentView.swift Tests/
```

`fmt` is an alias, if you type this often.

## Checking without changing

`--check` reports what would change and exits non-zero if anything would, without touching a file.
That's the form for a hook or a CI job:

```bash
sweetpad format --check
```

## Choosing the formatter

Two tools are supported, and they're different in kind: swift-format rewrites your code, swiftlint
checks it against rules.

```bash
sweetpad format --tool swift-format   # the default
sweetpad format --tool swiftlint
```

swift-format ships inside the Xcode toolchain, so it's already there. swiftlint doesn't, so install it
with Homebrew if you want it:

```bash
brew install swiftlint
```

`sweetpad doctor` reports which of the two it can find, so that's the quickest way to check what's
available before you commit to one.

## Settling on one for the project

Put the choice in `sweetpad.toml` and everyone who clones the repo formats the same way:

```toml
# sweetpad.toml
[format]
tool = "swiftlint"
```

`--tool` still overrides it for a single run.

:::note

SweetPad chooses and runs the formatter; it doesn't wrap its configuration. A `.swift-format` or
`.swiftlint.yml` in your repo is read by the tool itself exactly as it would be if you ran it
directly, so your existing setup carries over untouched.

:::

## In a hook or a pipeline

The `--check` exit code is the whole integration. A pre-commit hook:

```bash title=".git/hooks/pre-commit"
#!/bin/sh
sweetpad format --check || {
  echo "unformatted Swift files — run 'sweetpad format'" >&2
  exit 1
}
```

And in GitHub Actions, as a step that fails the job before a build is spent on it:

```yaml
- name: Check formatting
  run: sweetpad format --check
```

[Scripts and CI](./scripts-and-ci.md) covers the rest of the automation surface.
