---
sidebar_position: 3
sidebar_label: Build & run
---

# Build and run

The loop you'll spend the most time in is one command:

```bash
sweetpad run
```

It builds the app, installs it on your chosen simulator or device, launches it, and streams its logs
into your terminal. When you're done, Ctrl-C stops following the logs.

The first time you run it in a project, SweetPad asks which scheme and destination to use and
remembers the answer, so every run after that is just `sweetpad run`. See
[Destinations and devices](./destinations.md) for how that choice is made and changed.

## What a run looks like

```console
$ sweetpad run --no-logs
▶ SweetpadCIApp · Debug · iPhone 15
✓ Build succeeded (2.8s)
Launched dev.sweetpad.ci.app → dev.sweetpad.ci.app: 35549
```

The header line names the three things that decide what gets built: the scheme, the configuration,
and where it's going. If any of them is wrong, that line tells you before the build spends any time.

While the app is running and the logs are following, two keys work:

- **`r`** rebuilds and relaunches, the same loop again, without leaving the command.
- **`q`** quits.

## Building without running

`sweetpad build` compiles and stops there. It's the right command for a pre-commit hook, a quick
"does this still compile", or any moment you don't want a simulator to pop up.

```bash
sweetpad build            # compile the resolved scheme
sweetpad build --clean    # clean first, then compile
```

Add `--watch` to leave it running: every Swift file you save triggers a rebuild, and a failure keeps
watching instead of exiting.

```bash
sweetpad build --watch
```

:::note

`sweetpad build` builds the app target. It does not compile your test targets, so a green build can
sit next to a test target that no longer compiles. Run [`sweetpad test`](./testing.md) for that.

:::

## Reading a failed build

A failure prints the diagnostics and nothing else. There's no transcript to scroll back through
looking for the one line that matters:

```console
$ sweetpad build
building SweetpadCIApp (Debug) for platform=iOS Simulator,id=F92801F8-…
  Compiling ContentView.swift
error: /path/to/Sources/App/ContentView.swift:12:14: cannot find 'greeting' in scope
✗ Build failed
error: building the project
  xcodebuild exited with a non-zero status
```

A failed build exits with code `3`, which is how a script tells "the build broke" apart from "the
scheme doesn't exist" or "Xcode isn't installed". Run `sweetpad help exit-codes` for the full list.

If you've scrolled past the errors, or something else has since filled your terminal, you can ask for
them again without rebuilding:

```console
$ sweetpad build diagnostics
last build: FAILED (1 error(s), 0 warning(s))
  error: /path/to/Sources/App/ContentView.swift:12:14: cannot find 'greeting' in scope
```

That reads the last build's results from disk, so it's instant and safe to run as many times as you
like.

:::tip

Want the raw output instead of the summary? `-v` prints everything xcodebuild said. It's the first
thing to reach for when a build fails in a way the diagnostics don't explain.

:::

## Giving the app arguments and environment

Launch arguments and environment variables go to the app process, not to the build. Both are
repeatable:

```bash
sweetpad run --arg -MyFlag --arg YES
sweetpad run --env API_BASE=https://staging.example.com --env LOG_LEVEL=debug
```

## Running in the background

By default `run` stays in the foreground so it can show you the logs. Two flags change that:

- `--no-logs` launches the app and returns immediately, leaving it running.
- `--detach` also returns immediately, but on macOS it spawns the app in its own session with its
  output redirected to a log file, so the app survives the CLI exiting and `--env` is still honored.
  On a simulator or device the app already outlives the CLI, so this behaves like `--no-logs`.

Either way, `sweetpad app logs` picks the logs back up afterwards, and `sweetpad app stop` ends the
app.

## The rest of the app lifecycle

`sweetpad run` is shorthand for `sweetpad app run`, and the other verbs in that group let you take the
loop apart when you need to:

```bash
sweetpad app install      # build and install, don't launch
sweetpad app launch       # launch what's already installed
sweetpad app logs         # follow the running app's logs
sweetpad app stop         # terminate it
sweetpad app uninstall    # remove it from the simulator or device
sweetpad app open-url URL # open a deep link or universal link on a simulator
```

`sweetpad app logs` is the one you'll reach for most: `--last 5m` prints recent history and exits
instead of following, and `--until "some text"` follows until a line matches and then stops, which helps
in a script that needs to wait for the app to reach a known state.

[App lifecycle and debugging](./app-lifecycle.md) covers every verb, including the debugging ones
(`app debug`, `app diagnose`) and the macOS-only `app screenshot` and `app ui`.

## Seeing the exact xcodebuild command

When a build behaves differently from Xcode's and you want to know why, ask what SweetPad is actually
running:

```console
$ sweetpad build --show-command
xcodebuild build -scheme SweetpadCIApp -configuration Debug -destination 'platform=iOS Simulator,id=F92801F8-…' -resultBundlePath /Users/you/.local/state/sweetpad/results/SweetpadCIApp-…-build.xcresult -project /path/to/SweetpadCIApp.xcodeproj
in /path/to/project
```

Nothing runs. It prints the invocation and exits. `sweetpad test --show-command` does the same for a
test run.

## Adding your own xcodebuild flags

Anything after `--` is handed to xcodebuild untouched, so one unusual option doesn't send you back to
the raw tool:

```bash
sweetpad build -- -parallelizeTargets
sweetpad run -- -derivedDataPath ./build
sweetpad app install -- -allowProvisioningUpdates DEVELOPMENT_TEAM=ABCDE12345
```

If your project always needs the same flag, write it into `sweetpad.toml` instead of typing it every
time. See [Extra xcodebuild arguments](./reference.md#extra-xcodebuild-arguments) for the details and
the handful of arguments SweetPad settles itself.

## Where to go next

- [Testing](./testing.md): running, narrowing, and reading the results of your test suite.
- [Destinations and devices](./destinations.md): everything about choosing where a build runs.
- [Hot reload](./hot-reload.md): skip the rebuild entirely and inject each save into the running app.
