---
sidebar_position: 20
sidebar_label: Troubleshooting
---

# Troubleshooting

Two commands answer most "why isn't this working" questions, and they answer different halves of it:
`sweetpad doctor` checks your machine, and `sweetpad status` checks this project's context.

## Is my toolchain OK?

```console
$ sweetpad doctor
[ok] Xcode  /Applications/Xcode-26.5.0.app/Contents/Developer
[ok] xcodebuild  Xcode 26.5
[ok] swift  Apple Swift version 6.3.2
[ok] Simulator runtimes  1 available
[ok] devicectl (physical devices)  518.31
[warn] pymobiledevice3 (device logs)  not found
    ↳ only required to stream logs from a physical device: brew install pymobiledevice3
[ok] swift-format  …/usr/bin/swift-format
[ok] swiftlint  0.63.2

0 problem(s), 1 warning(s)
```

Every line that isn't `[ok]` comes with the fix. The distinction between a problem and a warning is
whether it blocks anything: the run above is completely healthy for someone who doesn't stream logs
from a physical device.

If you have several Xcodes installed, `doctor` reports which one is active. `--developer-dir` changes
it for one command, and `developer_dir` in `sweetpad.toml` pins it for the project.

## Why did it build *that*?

```console
$ sweetpad status
/path/to/MyApp.xcodeproj (project)
  scheme        MyApp  (remembered)
  configuration Debug  (remembered)
  destination   platform=iOS Simulator,id=F92801F8-…  (remembered)
```

The parenthetical is the point: it names the layer each value came from: a flag, an environment
variable, a config file, a remembered answer, or auto-detection. When a build targets something you
didn't ask for, this says who asked.

`sweetpad status` also prints the effective `[xcodebuild] args`, so a build shaped by a committed file
you didn't write still explains itself.

## The build is wrong, not broken

When compilation succeeds but the result is stale (an asset that didn't update, a change that didn't
take, a build that fails only after a branch switch), the artifacts are usually the problem:

```bash
sweetpad clean            # xcodebuild clean
sweetpad clean --purge    # and delete this project's DerivedData
```

`--purge` is scoped to the current project and doesn't prompt; the flag itself is the consent. For the
whole store there's a separate group, which does prompt:

```bash
sweetpad derived-data size     # how much is it costing you?
sweetpad derived-data path     # where is it?
sweetpad derived-data purge --all
```

`dd` is the alias.

## It's asking the wrong questions, or none

A remembered answer that's gone stale, such as a simulator you deleted or a scheme that was renamed, shows up
as a resolution failure or a build in the wrong place:

```bash
sweetpad context show               # what's saved
sweetpad context remove destination # forget one value
sweetpad context remove --all       # start over
```

The next command asks again. See
[Destinations and devices](./destinations.md#changing-whats-remembered).

## A setting in my config does nothing

Unknown keys are warned about on every run, in both config files, and so is a `[projects."…"]` key
that can't match a real container. If you're not seeing a warning, the key is being read and something
above it in the precedence chain is winning, and `sweetpad status` says which.

The most common mistake is a project key naming a *directory* instead of the container. It has to be
the `.xcodeproj`, `.xcworkspace`, or `Package.swift` itself. See
[Configuration](./configuration.md#your-personal-config).

## Hot reload says the address is in use

A `--hot` session that died without cleaning up leaves its listener bound:

```bash
sweetpad hot status   # is the port free, and who holds it?
sweetpad hot reset    # end a leftover sweetpad listener
```

`hot reset` refuses to kill a process that isn't SweetPad's; `--force` overrides that, for example when
InjectionNext is holding the port. [Hot reload](./hot-reload.md#when-the-port-is-stuck) has the
details.

## Autocomplete stopped working

```bash
sweetpad bsp doctor
```

It checks each link in the chain and says which one broke. The usual causes are a `buildServer.json`
missing a required field, which sourcekit-lsp skips silently, or absolute paths in it
that went stale when the checkout moved. [Editor autocomplete](./autocomplete.md) covers both.

## A device build hangs looking for a destination

A connected iPhone has to be unlocked, trusted, and in Developer Mode before xcodebuild can reach it.
`sweetpad device list` reports the connection state:

```console
$ sweetpad device list
Iphone 13 (iPhone 13, iOS 26.6)  [disconnected]
    00008110-000559182E90401E
```

Device builds also need signing settings that a simulator build doesn't. See
[Destinations and devices](./destinations.md#physical-devices).

## I need to see what xcodebuild actually said

Three levels, in increasing order of noise:

```bash
sweetpad build diagnostics   # last build's errors and warnings, no rebuild
sweetpad build --show-command # the exact invocation, without running it
sweetpad build -v             # the full raw transcript
```

`build diagnostics` is the one to try first. It's instant, and it reads the last build's results from
disk rather than repeating the work.

## Reading an exit code

| Code | Meaning                                                                    |
| ---- | -------------------------------------------------------------------------- |
| 0    | Success.                                                                   |
| 1    | Generic failure.                                                           |
| 2    | Usage error: bad flags or arguments.                                       |
| 3    | The build or the tests failed.                                             |
| 4    | Couldn't resolve a target: unknown scheme, destination, simulator, device. |
| 5    | A required tool is missing.                                                |
| 6    | Cancelled: a declined prompt, or Ctrl-C.                                   |

The pair worth learning is 3 and 4: code 3 means your code is broken, code 4 means the invocation is.
[Scripts and CI](./scripts-and-ci.md#exit-codes) has the rest.

## Where SweetPad keeps things

Useful when you want to inspect state, or clear it:

| Path                                    | What                                         |
| --------------------------------------- | --------------------------------------------- |
| `~/.local/state/sweetpad/state.toml`    | Remembered context, per project.             |
| `~/.local/state/sweetpad/results/`      | Retained result bundles and build logs.      |
| `~/.local/state/sweetpad/logs/`         | Output captured from detached macOS launches. |
| `~/.config/sweetpad/config.toml`        | Your personal config.                        |

## Still stuck

Every built-in guide is available offline, which is often faster than the website:

```bash
sweetpad help                 # list them
sweetpad help config
sweetpad help environment
sweetpad help destinations
sweetpad help exit-codes
sweetpad help hot-reload
```

If it looks like a bug, [open an issue](https://github.com/sweetpad-dev/sweetpad/issues) with the
output of `sweetpad doctor` and `sweetpad status`. Between them they cover almost everything anyone
would ask you next.
