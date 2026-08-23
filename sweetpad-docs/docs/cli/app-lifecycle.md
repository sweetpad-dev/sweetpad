---
sidebar_position: 9
sidebar_label: App lifecycle & debugging
---

# App lifecycle and debugging

`sweetpad run` does the whole loop: build, install, launch, follow logs. The `app` group is that loop
taken apart, plus the tools for looking at an app that's already running. Every verb works on
simulators, and most work on physical devices and native macOS apps too.

| Command                   | What it does                                                     |
| ------------------------- | ---------------------------------------------------------------- |
| `sweetpad app run`        | The full loop. `sweetpad run` is shorthand for it.               |
| `sweetpad app install`    | Build and install, without launching.                            |
| `sweetpad app launch`     | Launch what's already installed.                                 |
| `sweetpad app stop`       | Terminate the running app.                                       |
| `sweetpad app uninstall`  | Remove it from the simulator or device.                          |
| `sweetpad app logs`       | Follow or replay its logs.                                       |
| `sweetpad app open-url`   | Open a deep link or universal link on a simulator.               |
| `sweetpad app debug`      | Run it under lldb.                                               |
| `sweetpad app diagnose`   | Run it under lldb, catch the first crash, report, and quit.      |
| `sweetpad app screenshot` | Capture the app: a macOS window, or the simulator it's on.       |
| `sweetpad app ui`         | Read and drive a macOS app's UI through accessibility.           |

SweetPad remembers which app it last launched, so most of these need no arguments. `app stop` stops
the thing you just started.

:::note

The verbs that build (`run`, `install`, `debug`, `diagnose`) accept a `--` tail of xcodebuild
arguments. The ones that only act on an installed app (`launch`, `stop`, `logs`, `uninstall`) reject
it, rather than swallow arguments that would reach no build.

:::

## Logs

`sweetpad app logs` is the most useful verb in the group, because a running app's output is otherwise
awkward to get at.

```bash
sweetpad app logs                 # follow the running app
sweetpad app logs --last 5m       # print the last five minutes and exit
sweetpad app logs --json          # one JSON object per line
```

### Narrowing what you see

```bash
sweetpad app logs --subsystem com.example.MyApp.networking
sweetpad app logs --category Requests
sweetpad app logs --level debug
```

Logs stream at the `info` level by default. Your app's own `debug` entries are hidden until you ask
for them, and because the system doesn't persist debug entries, `--last` can never show them however
low you set the level. Debug output exists only while you're following live.

`--predicate` is the escape hatch: a raw `log stream` predicate that replaces the process match
entirely, for the filters the flags don't cover.

### Waiting for something to happen

`--until` follows the log until a line contains some text, then exits 0. This turns "start it, wait
for the thing" into a single call instead of a background stream and a guessed `sleep`:

```bash
sweetpad app logs --until "Sync complete"
sweetpad app logs --until "Sync complete" --timeout 60
```

The match is a plain substring against the rendered line, so what you see is what it matches. With
`--timeout`, missing the deadline exits non-zero, so a script can tell "it happened" from "it never
did". On its own, `--timeout` just bounds the follow and exits 0.

### macOS: two streams

A macOS app produces two separate kinds of output, and by default you get both, interleaved as they
arrive:

- **`oslog`**: the unified log, what `os_log` and `Logger` write.
- **`stdout`**: what a detached launch captured, meaning `print`, C `printf`, and NSLog's stderr leg.

`--source` picks one:

```bash
sweetpad app logs --mac --source stdout
```

That second stream exists because `sweetpad run --detach` on macOS redirects the app's output to a
file. See [Build and run](./build-and-run.md#running-in-the-background). Simulators and devices are
os_log only, so `--source` doesn't apply there.

:::note

Streaming logs from a physical device needs pymobiledevice3, which isn't part of Xcode. `sweetpad
doctor` reports whether you have it, and `brew install pymobiledevice3` supplies it. `--last` isn't
available for devices at all.

:::

## Debugging under lldb

`sweetpad app debug` builds, installs, and hands you an lldb session. On a simulator it launches the
app suspended and attaches; on macOS it gives the executable to lldb and runs it.

```bash
sweetpad app debug
sweetpad app debug --mac
sweetpad app debug --arg -MyFlag --env LOG_LEVEL=debug
```

`--wait-for-debugger` launches suspended without attaching, for when you want to bring your own
debugger. The app waits for `lldb -p <pid>`.

### Driving lldb from a script

`--batch` runs lldb non-interactively: it executes the commands you give it and lets the session end,
instead of handing over a prompt. Commands are forwarded verbatim and run in order, so you supply your
own `run` and `quit`:

```bash
sweetpad app debug --batch \
  --cmd "breakpoint set --name applicationDidFinishLaunching" \
  --cmd "run" \
  --cmd "bt" \
  --cmd "quit"
```

`--on-crash` adds commands that run only if the target crashes:

```bash
sweetpad app debug --batch --cmd run --on-crash "bt all" --on-crash quit
```

A batch session is killed after `--timeout` seconds (300 by default, `0` to disable), so an
unattended run can't hang on an app that stays up.

:::warning

The exit code of `--batch` says whether the session *launched*, not what lldb found. A breakpoint that
never hit and a clean run look the same from outside. Parse the streamed output, or use `app diagnose`
when what you want is a verdict.

:::

## Catching a crash unattended

`sweetpad app diagnose` is the command for "run this and tell me if it breaks". It launches the app
under lldb, waits for the first Objective-C exception or crash, prints a structured report, and quits:

```bash
sweetpad app diagnose
sweetpad app diagnose --timeout 60
sweetpad app diagnose -o json
```

It's bounded by `--timeout`, 30 seconds by default, and reports a timeout if the app neither crashes
nor exits in that window. The result is the report, not the exit code, so read the output (or the JSON
payload) rather than branching on `$?`. Simulator and macOS only.

## Screenshots of the app

```bash
sweetpad app screenshot
sweetpad app screenshot --output-file ./bug.png --clipboard
```

On a simulator this captures the simulator the app launched on. On macOS it captures the app's own
window, where `--window N` picks among several, front-to-back, and `--pid` targets a process SweetPad
didn't launch.

Files default to `./sweetpad-shots/<app>-<time>.png`. For simulator-level captures with a clean status
bar, see [Simulators](./simulators.md#screenshots-worth-shipping).

## Driving a macOS app's UI

`sweetpad app ui` reads and operates a native macOS app through accessibility. Start with the tree,
which is also what tells you the labels and roles the other verbs match on:

```bash
sweetpad app ui tree
```

Then act on it:

```bash
sweetpad app ui click --label "Save"
sweetpad app ui click --label "Save" --role button
sweetpad app ui click --label "Open" --nth 2
sweetpad app ui type "hello@example.com" --label "Email"
```

`--label` matches an element's identifier or visible label. Exact matches beat substring ones, and
case is ignored. `--role` narrows to one kind of element (`button`, `textfield`, `menuitem`; the `AX`
prefix is optional), and `--nth` picks among ties, 1-based and front-to-back. `--pid` drives a process
SweetPad didn't launch.

:::note

`ui type` sets the field's value rather than synthesizing keystrokes. An app that watches for
individual key events, such as a live-validating field or a search-as-you-type box, may not react to it.

:::
