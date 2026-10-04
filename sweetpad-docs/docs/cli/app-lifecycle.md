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
| `sweetpad app container`  | Print the app's data container, `.app`, or App Group paths.      |
| `sweetpad app debug`      | Run it under lldb.                                               |
| `sweetpad app diagnose`   | Run it under lldb, catch the first crash, report, and quit.      |
| `sweetpad app screenshot` | Capture the app: a macOS window, or the simulator it's on.       |
| `sweetpad app sample`     | Sample it and say whether its main thread is idle or stuck.      |
| `sweetpad app ui`         | Read and drive a macOS app's UI through accessibility.           |

When a scheme builds more than one app, such as a helper app or a watch app beside the main one, the
`app` verbs act on the app the scheme's Run action names, as Xcode does. If that app can't run on the
destination, they take the app that can.

SweetPad remembers which app it last launched, so most of these need no arguments. `app stop` stops
the thing you just started. Flags that describe that same launch, such as `--scheme MyAppMac --on mac`
after `app launch --mac`, still find it, even when it runs out of a `--derived-data-path` build. Flags
that name a different scheme, configuration, or destination look up that app instead.

:::note

The verbs that build (`run`, `install`, `debug`, `diagnose`) accept a `--` tail of xcodebuild
arguments. The ones that only act on an installed app (`launch`, `stop`, `logs`, `uninstall`) reject
it, rather than swallow arguments that would reach no build. If you built with
`-- -derivedDataPath <dir>`, launch that build with `app launch --derived-data-path <dir>`.

:::

## Windows from the last run

SweetPad launches macOS apps with `-ApplePersistenceIgnoreState YES`, so the app opens fresh
instead of reopening the windows it had last time. Without it, a relaunch after a crash can stop at
AppKit's question about reopening the app's windows, and the app's main thread waits on that dialog
until someone answers. The app looks launched but does nothing, and `app sample` reports it as idle.

To get the app's own behavior back, pass `--restore-state` to `run`, `app launch`, `app debug`, or
`app diagnose`. SweetPad also leaves the argument out if you set `-ApplePersistenceIgnoreState`
yourself, with `--arg` or in the scheme's launch arguments. Simulators and devices aren't affected.

AppKit answers the argument with a line on stderr at every launch, `ApplePersistenceIgnoreState:
Existing state will not be touched…`. When SweetPad added the argument, `sweetpad run` and
`app logs` leave that line out of the app's output. When you set it yourself, the line shows as
usual.

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

### Why the app stopped

`--exits` lists the app's recent terminations instead of its logs, each with when it happened and
why. The answer comes from the record launchd keeps when a process ends, so it covers deaths that
leave no crash report, like a watchdog or the simulator host ending the app. Crash reports add any
crash launchd didn't log, and on macOS SweetPad adds its own record of the apps it runs attached.

```bash
sweetpad app logs --exits              # the last ten minutes
sweetpad app logs --exits --last 1h
sweetpad app logs --exits -o json
```

Each line shows the process id, the raw reason with its code, and launchd's explanation when there
is one:

```text
17:31:15.990  pid 57171  Termination requested by simulator host (OS_REASON_SPRINGBOARD 0xfbfbfbfb), ran 3.8s
17:33:10.087  pid 59571  crashed with SIGABRT (sent by ExitProbe[59571]), ran 1.4s
    crash report: /Users/you/Library/Logs/DiagnosticReports/ExitProbe-2026-09-26-173313.ips
17:33:23.131  pid 59915  exited with status 3, ran 1.4s
20:37:03.524  pid 91599  crashed with SIGSEGV (sent by exc handler[91599]; EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x0000000000000010), ran 371ms
    crash report: /Users/you/Library/Logs/DiagnosticReports/SweetpadCIApp-2026-09-26-203704.ips
```

A plain-words label appears only when the meaning is well known, such as a crash signal, memory
pressure, or a watchdog timeout. Other codes show as they are. When a crash report names the fault,
it follows the sender after a semicolon. In `-o json`, the sender is `sentBy`, the fault is
`exception`, and each exit's `source` says where it came from: `launchd`, `crashReport`, or
`sweetpad`.

A crash can come without a crash report. macOS saves only so many for one app (25 on macOS 27),
so an app that crashes again and again stops getting them, and the note under the list says so.

:::note

`--exits` works for simulators and macOS apps, not physical devices. On macOS, launchd keeps a
record only for apps started through LaunchServices, like `open` or the Finder. For an app that
`sweetpad run --mac` starts and stays attached to, SweetPad records the exit itself. A detached
launch (`app launch --mac`, `run --detach`) shows up only if it crashes, through its crash report. A
clean exit or an outside kill of one leaves no record.

:::

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

The file starts with a line SweetPad writes at launch. `app logs` shows it as a separator, such as
`── sweetpad launched MyApp at 14:02:11.031 · args: --flag ──`. It isn't part of the `--json` stream,
and `--until` doesn't match it.

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

To debug from your editor's own debugger instead of a terminal, see
[Editor debugging](./editor-debugging.md).

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

The exit code of `--batch` says whether the session *ran*, not what lldb found. A breakpoint that
never hit and a clean run look the same from outside. lldb's own exit status is not passed on: a
command that fails, such as `bt` after the app has exited, ends lldb's batch with status 1, and
sweetpad still exits 0. It exits non-zero when the build or launch fails or `--timeout` ends the
session. Parse the streamed output, or use `app diagnose` when what you want is a verdict.

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
nor exits in that window. The result is the report, not the exit code. It exits 0 for any report,
whether the app crashed, threw, exited, or timed out, so read the output (or the JSON payload) rather
than branching on `$?`. Simulator and macOS only.

`app diagnose` runs its own lldb commands and takes none from you, so it has no `--batch` or `--cmd`.
To run your own commands, use [`app debug --batch`](#driving-lldb-from-a-script).

lldb reports most crashes as the Mach exception the CPU raised, such as
`EXC_BAD_ACCESS (code=1, address=0x10)`, before the system turns it into a signal. The report gives the
signal the app dies of and says what happened:

```text
com.example.MyApp: crashed with SIGSEGV: a bad memory access at 0x10 (EXC_BAD_ACCESS)
```

In `-o json`, that line is `verdict`, the signal is `signal`, and lldb's own wording stays in
`stopReason`. A bad access at an unmapped address is `SIGSEGV` and any other bad access is `SIGBUS`.
`EXC_BREAKPOINT` is `SIGTRAP`, `EXC_BAD_INSTRUCTION` is `SIGILL`, and `EXC_ARITHMETIC` is `SIGFPE`.

lldb stops a Swift `fatalError`, a failed `precondition`, or an index out of range before the trap,
with the message Swift gives it. The report names `SIGTRAP`, the signal the app dies of, and quotes
the message:

```text
com.example.MyApp: crashed with SIGTRAP: Swift fatal error "Index out of range"
```

The report of a crash also lists its backtrace, with the frame lldb stopped in marked `*`.

The JSON also says how lldb itself ended. `lldbStatus` is lldb's exit code, and it's `null` when the
timeout killed lldb. `chainComplete` is `true` when lldb ran all of its commands through the
backtrace. A crash, an exception, and a clean exit all give `0` and `true`. If lldb fails partway,
for example because it can't attach to the app, it stops with status 1 and `chainComplete` is
`false`. The verdict then gives lldb's error:

```text
com.example.MyApp: lldb stopped partway with status 1: attach failed: no such process
```

## The app's files

`sweetpad app container` prints where the app keeps its files, for a script that seeds a fixture
before a test or reads back what the app wrote. It prints only the path, so it works inside `$(…)`:

```bash
sweetpad app container                # the data container: Documents, Library, tmp
sweetpad app container --kind app     # the installed .app bundle
sweetpad app container --kind groups  # one "id  path" line per App Group
cp fixture.pdf "$(sweetpad app container)/Documents/"
```

`-o json` gives `path`, `kind`, `bundleId`, and `destination`, or a `groups` list of `{id, path}` for
`--kind groups`.

On a simulator this works for any installed app, and SweetPad boots the simulator first if it has
to.

On macOS only a sandboxed app has a data container. SweetPad reads the App Sandbox entitlement from
the built app, and for an app without it the command reports that there is no container.

A physical device's containers stay on the device. For those, the error names the
`xcrun devicectl device copy to` and `copy from` commands to use.

## Is it hung, or just idle?

When an app looks alive but has stopped doing anything, `sweetpad app sample` samples it for a few
seconds and tells you what its main thread was doing:

```bash
sweetpad app sample
sweetpad app sample --seconds 10
sweetpad app sample -o json
```

The verdict is one of four:

- `idle`: the main thread is waiting for events in its run loop, so the app isn't hung. If work
  stopped, something it was waiting for never arrived: a callback, a completion handler, or a queue
  that never ran.
- `blocked`: the main thread is stuck on a lock, a semaphore, or a dispatch_sync. The verdict says
  which, and names the function in your code that's waiting.
- `busy`: the main thread is running code, and the verdict lists your functions the time went to.
- `unclassified`: no single state accounts for most of the samples, so you get the split instead of
  a guess.

It also warns when AppKit swallowed an Objective-C exception earlier in the run. The app keeps
running after that, but whatever the exception interrupted never finished. `app diagnose` stops at
the exception where it's thrown.

The full report from macOS's sample tool is saved every time, and its path is printed with the
verdict. Sampling works for macOS apps and for simulator apps, because a simulated app runs as a
process on your Mac. An app on a physical device can't be sampled. `--pid` samples a process
SweetPad didn't launch.

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
