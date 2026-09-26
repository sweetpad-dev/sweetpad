---
name: sweetpad
description: Build, run, and test Xcode and Swift Package apps from the terminal with the sweetpad CLI ("xcodebuild for humans"). Use when the user wants to compile an iOS/macOS/Swift app, run it on a simulator or device, read build errors, run tests, or inspect resolved build settings from the command line — with agent-safe, non-blocking, JSON-mode invocations instead of raw xcodebuild.
---

# Drive the sweetpad CLI

`sweetpad` is a headless command-line tool for Xcode and Swift Package apps —
"xcodebuild for humans". Use it to build, run, test, and inspect iOS/macOS/Swift
projects. This skill covers the everyday flows and how to discover the rest.

## Before you start

- Confirm it's installed: `sweetpad --version`. If it's missing, install with
  `brew install sweetpad-dev/tap/sweetpad`.
- Run from inside the project directory, or pass `-C <dir>` to point at it.
  sweetpad auto-discovers the `.xcworkspace` / `.xcodeproj`.
- Add `-o json` to any command for a structured `{schema, ok, data}` envelope,
  and `--non-interactive` so it never prompts or waits on a terminal. Both are
  the right default when you're an agent.

## Read results correctly

- Success is `{"schema":N,"ok":true,"data":{…}}` on stdout. Errors are
  `{"schema":1,"ok":false,"error":{"code":…,"message":…}}` on stderr, where
  `code` is one of: `generic`, `usage_error`, `build_failure`,
  `target_resolution`, `tool_missing`, `user_cancel`.
- `ok: true` means "the command ran", not "the outcome was good". A failing test
  run still reports `ok: true` with `data.passed: false` — read the payload's own
  status field, not just `ok`.
- Exit codes: `0` ok · `1` generic · `2` bad flags · `3` build/test failure ·
  `4` target resolution (unknown scheme/destination) · `5` missing tool ·
  `6` cancelled.

## Avoid the commands that never return

Some modes stream forever and will hang an agent loop. Don't use these
non-interactively:

- `sweetpad run` **without** `--no-logs` follows the app's logs until it exits.
- `build --watch`, `test --watch`, `run --hot` — long-lived watch/session modes.
- `app logs` **without** `--last`, `--until`, `--timeout`, or `--exits` — an
  unbounded stream that follows until killed.

Prefer the finite forms below. If you genuinely need a live stream, run it in the
background with your own timeout.

## Pick a destination

`sweetpad devices` lists everything runnable — macOS, simulators, connected
devices — each with a ready destination specifier and a short name, most-used
first, the remembered one marked.

```bash
sweetpad devices -o json
```

A physical device's entry also has devicectl's `connection`, `transport`
(`wired` or `localNetwork`) and `pairing`. An idle device always shows
`connection: "disconnected"`, because xcodebuild connects when it needs to, so
don't read that as a fault.

Before building to a phone, check it:

```bash
sweetpad device info "<name or UDID>" -o json
```

It connects to the device and returns `ready` and a `reason` that names the fix
(locked, Developer Mode off, not paired, unreachable). It waits at most
`--timeout` seconds (10 by default) and exits 1 when the device isn't ready.
When a build or test can't reach a device, its error carries this command in
`error.tip`.

Target one with `--on <ref>`: a fuzzy name (`"iPhone 16 Pro"`), `booted`, `mac`,
`device`, a platform word (`ios`, `watchos`, …), a UDID, or a saved context
alias. `--destination "<raw>"` is the escape hatch for an exact xcodebuild
specifier.

## Build

`sweetpad build` compiles the resolved scheme and returns when the compile
finishes.

```bash
sweetpad build --on "iPhone 16 Pro" -o json
```

Common flags: `--clean` (clean first), `--scheme <name>`,
`--configuration <Debug|Release>`, `--show-command` (print the exact xcodebuild
invocation and exit without building).

In a build-fix loop, add `-q`: it drops progress chatter and keeps only what you
can't ignore — errors, warnings, and the failure banner. A clean build prints
nothing.

```bash
sweetpad build -q                                       # silent unless it matters
sweetpad test build -q                                  # the same, for test code
```

`sweetpad build` compiles only the scheme's Run targets, so a test that no
longer compiles still passes it. After you change test code, check with
`sweetpad test build -q`. It compiles the test targets without running them and
reports the same way, including `build diagnostics` afterwards.

Never pipe a build through `tail -N` to save context — truncation drops the
diagnostics precisely when you need them.

## Read build errors without rebuilding

After a build, pull the diagnostics from the last run — no recompile:

```bash
sweetpad build diagnostics -o json
```

This is the fast path for "why did it fail". The whole agent loop is
`sweetpad build -q` (or `sweetpad test build -q` when you touched tests), and on
a non-zero exit `sweetpad build diagnostics -o json` for structured errors and
warnings (file, line, message) without a second compile.

## Run the app without blocking

Build, install, and launch, then return instead of following logs:

```bash
sweetpad run --on booted --no-logs --non-interactive
```

Pass arguments and environment to the app with `--arg` and `--env KEY=VALUE`
(both repeatable) — both work on `app run`, `app debug`, and `app diagnose`
alike. `--detach` (`app run` only) leaves the app running after the CLI exits.

`app launch` starts the installed app (on macOS, the built one) without building
it again. If a macOS build ran with `-- -derivedDataPath <dir>`, pass
`app launch --mac --derived-data-path <dir>` so it finds that build.

If a run fails with `cannot bind 127.0.0.1:8887 for hot reload`, a dead session
left the listener behind: `sweetpad hot status` names the holder and
`sweetpad hot reset` clears it. Prefer that over `--no-hot`, which works by
giving up hot reload entirely.

## Read the app's logs

`app logs` follows the running app forever, which will hang you. Three flags
bound it — use one of them:

```bash
sweetpad app logs --last 2m -o ndjson                   # recent history, then exit
sweetpad app logs --until "Ready to serve" --timeout 30s # wait for one line
sweetpad app logs --timeout 10s                          # bounded tail
```

`--until` is the one to reach for when you need to start something, poke it, and
read what it said: it exits 0 on the first line containing that text and non-zero
if the deadline passes, so you branch on the exit code instead of backgrounding a
stream and guessing a `sleep`. It's a plain substring, not a regex.

Logs are a stream, so use `-o ndjson` — one JSON object per line, not the
`{schema, ok, data}` envelope `-o json` gives other commands.

Two things reliably mislead agents here:

- The stream starts at `info`, so the app's `.debug` entries stay hidden until
  you pass `--level debug`. An app logging correctly through `os.Logger` looks
  completely silent otherwise.
- The system never persists `.debug` entries, so `--last` cannot show them at
  any level. They exist only while you follow live.

On macOS this merges the app's `os_log` with the stdout/stderr a `--detach`ed
launch captured; `--source oslog|stdout` narrows it to one of the two.

To catch a crash or Objective-C exception without a terminal, `sweetpad app
diagnose` runs the app under lldb, prints a structured report, and quits —
bounded by `--timeout` (default 30s).

When the app is running but seems stuck, `sweetpad app sample -o json` samples
it for 3 seconds (`--seconds` to change) and reports what the main thread was
doing. `idle` means it is waiting for events and isn't hung, so look for a
callback or queue that never fired. `blocked` names the wait and your function
that waits, and `busy` lists the functions the time went to. The full `sample`
report is saved, and `reportPath` says where.

When the app already died and you need to know why, `sweetpad app logs --exits
-o json` lists its recent terminations (the last 10m, or `--last`) from
launchd's exit records: the signal of a crash with its crash report, or the
reason code and explanation of a kill that left no report, such as
`Termination requested by simulator host`. Simulator and macOS only.

## Test

`sweetpad test` runs the scheme's tests and returns a report.

```bash
sweetpad test -o json
sweetpad test --failed -o json                          # only last run's failures
sweetpad test --only-testing MyAppTests/LoginTests -o json
sweetpad test build -q                                  # compile the tests, run none
```

`--coverage` adds a coverage summary; `--junit <path>` writes a CI report;
`--retry-flaky <N>` retries each failing test up to N times before calling it
failed.

Each failure's `messages` lists every failure the test recorded, in order;
`message` is only the first. When one of them says the app vanished (`… is not
running`, `… crashed`, `Test crashed with signal …`), read that failure's
`terminationReason`: launchd's record of how the app or test runner ended,
including kills that leave no crash report.

## Inspect resolved build settings

```bash
sweetpad settings show -o json
```

The fully resolved settings for the active scheme/target — the thing
`xcodebuild -showBuildSettings` makes painful.

## See and set what would build

`sweetpad status` shows the resolved scheme, configuration, and destination, and
where each value came from (flag / env / config / remembered / default). The
remembered selection lives in the project's context:

```bash
sweetpad status -o json
sweetpad context set scheme MyApp          # remember a scheme (no prompt)
sweetpad context alias work-phone <UDID>   # then use: --on work-phone
```

## Discover everything else

This skill covers the common flows. For anything not here, the CLI is
self-describing — prefer these over guessing:

- `sweetpad --help` — the full command tree (scheme, simulator, app, archive,
  dependency, clean, format, pbxproj, bsp, vscode, …).
- `sweetpad <command> --help` — flags and subcommands for one command, e.g.
  `sweetpad app --help`, `sweetpad simulator --help`.
- `sweetpad help <topic>` — prose guides: `config`, `environment`,
  `exit-codes`, `destinations`, `hot-reload`.
- Add `-o json` to any read command for structured output.
