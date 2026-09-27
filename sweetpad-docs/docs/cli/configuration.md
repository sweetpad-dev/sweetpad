---
sidebar_position: 15
sidebar_label: Configuration
---

# Configuration

You can drive the CLI entirely with flags, but you won't want to. There are three places a setting can
live, and they exist because they answer different questions:

- **`sweetpad.toml`**, committed next to your project: *what does this project need?* Everyone who
  clones the repo gets it.
- **`~/.config/sweetpad/config.toml`**, yours alone: *what do I prefer?* SweetPad writes only its
  `[feedback]` table there, and only when you run `sweetpad feedback off` or `on`.
- **Remembered context**, managed for you: *what did I pick last time?* This is where the answers to
  interactive prompts go.

`sweetpad status` always reports which of these a value came from, so a build shaped by a file you
didn't write still explains itself.

## The project file

`sweetpad.toml` is the team-shared layer. Put it next to your project, commit it, and everyone gets
the same defaults:

```toml
# sweetpad.toml
scheme = "MyApp"
configuration = "Debug"
destination = "platform=iOS Simulator,name=iPhone 16 Pro"

[xcodebuild]
args = ["-skipMacroValidation"]
```

SweetPad looks for it by walking up from your working directory to the git root, so one file at the
repo root serves the whole checkout, and you don't need a copy per subdirectory.

### Every key

The top level takes the same targeting values as the flags:

| Key             | What it does                                                        |
| --------------- | ------------------------------------------------------------------- |
| `scheme`        | Default scheme.                                                     |
| `configuration` | Default build configuration.                                        |
| `destination`   | Default destination, as a raw specifier.                            |
| `sdk`           | SDK override. Rarely needed, since the destination usually implies it.   |
| `developer_dir` | Pin the Xcode this project uses, for builds and manifest reads.     |
| `workspace`     | Name the `.xcworkspace`, relative to this file. See below.          |
| `project`       | Name the `.xcodeproj`, relative to this file. See below.            |
| `generator`     | Declare the project as [generated](./generated-projects.md), e.g. `"xcodegen"`. |

Then four tables:

| Table                     | Key               | What it does                                                   |
| ------------------------- | ----------------- | -------------------------------------------------------------- |
| `[run]`                   | `hot`             | Default `sweetpad run` to hot reload. `--no-hot` opts out.     |
|                           | `hot_recompiler`  | `"resolver"` or `"buildlog"`.                                  |
|                           | `auto_unsandbox`  | Whether a hot macOS build may strip the App Sandbox. Default true. |
| `[format]`                | `tool`            | `"swift-format"` or `"swiftlint"`.                             |
| `[testing]`               | `scheme`, `configuration`, `destination`, `target` | Test-only overrides, layered over the build values. |
| `[xcodebuild]`            | `args`            | Arguments added to every command that builds.                  |

A fuller example:

```toml
# sweetpad.toml
scheme = "MyApp"
developer_dir = "/Applications/Xcode-16.4.app/Contents/Developer"

[run]
hot = true

[format]
tool = "swiftlint"

[testing]
configuration = "Test"
destination = "platform=iOS Simulator,name=iPhone SE (3rd generation)"

[xcodebuild]
args = ["-skipMacroValidation", "-disablePackageRepositoryCache"]
```

### Pointing at a project somewhere else

By default the file sits beside the container it configures. When it doesn't, as with a repo root and the
Xcode project a few directories down, name the container relative to the file:

```toml
# sweetpad.toml, at the repo root
project = "ios/App.xcodeproj"
```

Now every command works from anywhere in the checkout with no `-C`. `workspace` does the same for an
`.xcworkspace` and wins when both are set.

You usually don't need this. Auto-discovery already searches upward to the git root and then up to two
levels down, skipping the usual noise (Pods, node_modules, Carthage, vendor, DerivedData, build, and
dotfile directories), so a layout like `ios/App.xcodeproj` works with no setup. The key earns its keep
when two projects sit at the same depth. SweetPad reports that as an error listing both rather than
guessing, and this is how you settle it.

### Test-only flags

`[xcodebuild] args` reaches every command that runs `xcodebuild`, but some flags only work when
testing. `xcodebuild build`, `archive`, and `clean` fail on `-enableCodeCoverage`, `-testPlan`,
`-testLanguage`, `-testRegion`, and `-testProductsPath`. SweetPad leaves these out of every command
except `sweetpad test` and `sweetpad test build`, so you can keep them in the file:

```toml
[xcodebuild]
args = ["-skipMacroValidation", "-testPlan", "CI"]   # -testPlan reaches only the test runs
```

`-resultStreamPath` needs a result bundle to stream into, so it stays out of `sweetpad clean` and out
of `sweetpad archive` unless you pass `-resultBundlePath` after `--`. Run with `-v` to see each flag a
command left out.

The rest of the testing flags, such as `-test-iterations` and `-parallel-testing-enabled`, work with
every action and reach every command. Flags you type after `--` are never left out.

### Arguments SweetPad won't let you put here

`[xcodebuild] args` refuses the arguments SweetPad settles itself, naming the key to use instead:
`-scheme`, `-configuration`, `-destination`, `-sdk`, `-workspace`, and `-project`.

It also refuses the paths SweetPad names for itself. `sweetpad test` writes and reads back its own
`-resultBundlePath`, so name one per run with `sweetpad test --result-bundle`, or pass it after `--`
on a build. `sweetpad archive` names its own `-archivePath`, `-exportPath`, and `-exportOptionsPlist`,
so use `--output-file` and `--export-options` instead.

`-enableCodeCoverage` and `-test-iterations` can stay in the file. When you pass `--coverage` or
`--retry-flaky` to `sweetpad test`, the flag replaces the file's copy, and `-v` says so.

`-derivedDataPath` is refused too. Only the builds, `sweetpad clean`, and the `app` commands would
follow it, while `sweetpad clean --purge`, `sweetpad derived-data`, and the editor's index would keep
using the DerivedData location Xcode's settings name. A relative value would also resolve against the project's directory, where SweetPad runs
`xcodebuild`, and not against this file the way `project` does. Pass that one per command, after
`--` on the build and as `--derived-data-path` on `app launch`.

Swift packages ignore the table entirely: they build with `swift build`, which knows none of
xcodebuild's flags.

:::tip

For `KEY=VALUE` build settings, an `.xcconfig` is usually the better committed home, because Xcode honors it
too, so ⌘B and `sweetpad build` stay in agreement. Put flags in `[xcodebuild] args`; put build
settings in an xcconfig unless you specifically want them only when building through SweetPad.

:::

## Your personal config

`~/.config/sweetpad/config.toml` holds your own preferences. It's a file you own, and it honors
`XDG_CONFIG_HOME` if you set one. SweetPad writes to it in one case: `sweetpad feedback off` sets
`enabled = false` under `[feedback]` (see [Feedback reports](./feedback.md)), and `sweetpad feedback on`
sets it back. Both change only that key and keep the rest of the file, comments included.

It has a `[defaults]` table for values that apply everywhere, plus per-project tables:

```toml
# ~/.config/sweetpad/config.toml
[defaults]
configuration = "Debug"

[projects."/Users/me/code/MyApp/MyApp.xcodeproj"]
scheme = "MyApp"
destination = "platform=iOS Simulator,name=iPhone 15"

[projects."/Users/me/code/MyApp/MyApp.xcodeproj".testing]
configuration = "Test"
target = "MyAppTests"
```

:::warning

The project key is the **container**: the `.xcodeproj`, `.xcworkspace`, or `Package.swift` itself,
not the directory holding it. A key that can't match a real container is reported as a warning on
every run, which is how you'll notice you wrote the directory path instead.

:::

Personal config beats `sweetpad.toml`, so this is also where you override a team default you disagree
with, without touching the committed file.

## Remembered context

The third layer isn't a file you edit. When SweetPad prompts you for a scheme or a destination, it
saves the answer per project and stops asking. Inspect and change it with `sweetpad context`:

```bash
sweetpad context show
sweetpad context select
sweetpad context set scheme MyApp
sweetpad context remove --all
```

[Destinations and devices](./destinations.md#changing-whats-remembered) covers this in full.

## Which setting wins

Highest first:

```
flag  >  environment variable  >  config.toml  >  sweetpad.toml  >  remembered answer  >  auto-detect
```

Read it as most-explicit-wins. A flag you typed beats everything; auto-detection only happens when
nothing else has an opinion. Note that your personal config outranks the committed project file, and
that both outrank whatever you last picked at a prompt.

## Environment variables

Every targeting value has an environment twin, folded into the flag layer. A typed flag still wins,
and a variable set to the empty string counts as unset:

| Variable                 | Sets                                              |
| ------------------------ | ------------------------------------------------- |
| `SWEETPAD_WORKSPACE`     | Path to the `.xcworkspace`.                       |
| `SWEETPAD_PROJECT`       | Path to the `.xcodeproj`.                         |
| `SWEETPAD_SCHEME`        | Scheme name.                                      |
| `SWEETPAD_CONFIGURATION` | Build configuration.                              |
| `SWEETPAD_DESTINATION`   | Raw destination specifier.                        |
| `SWEETPAD_ON`            | Human destination reference. Overrides `SWEETPAD_DESTINATION`. |
| `SWEETPAD_SDK`           | SDK override.                                     |
| `DEVELOPER_DIR`          | The Xcode every spawned tool uses.                |

Two more control behavior rather than targeting. `SWEETPAD_NONINTERACTIVE` turns prompts into errors,
and `CI` does the same thing, which is why a pipeline usually needs neither. Both parse loosely: `0`,
`false`, `no`, `off`, and empty all mean off.

Color follows the usual conventions: `NO_COLOR` disables it, `CLICOLOR_FORCE` and `FORCE_COLOR` force
it on even when output is piped, and an explicit `--no-color` still wins.

## Typos are never silent

Unknown keys produce a warning on every run, in both config files. So does a project key that can't
match a real container. A setting that quietly does nothing is worse than one that complains, and
this is the one part of the config system that's deliberately noisy.

## Where things live

| Path                                   | What                                              |
| -------------------------------------- | ------------------------------------------------- |
| `sweetpad.toml`                        | Project defaults. Committed, hand-authored.       |
| `~/.config/sweetpad/config.toml`       | Your defaults. Hand-authored; `sweetpad feedback off` sets one key. |
| `~/.local/state/sweetpad/state.toml`   | Remembered context. Managed; use `sweetpad context`. |

`sweetpad open config` opens your personal config in your editor, and `sweetpad help config` has this
material offline.
