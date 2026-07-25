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
  `code` is one of: `generic`, `build_failure`, `target_resolution`,
  `tool_missing`, `user_cancel`.
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
- `app logs` — a live log stream.

Prefer the finite forms below. If you genuinely need a log stream, run it in the
background with your own timeout.

## Pick a destination

`sweetpad devices` lists everything runnable — macOS, simulators, connected
devices — each with a ready destination specifier and a short name, most-used
first, the remembered one marked.

```bash
sweetpad devices -o json
```

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

## Read build errors without rebuilding

After a build, pull the diagnostics from the last run — no recompile:

```bash
sweetpad build diagnostics -o json
```

This is the fast path for "why did it fail": build once, then read structured
errors and warnings (file, line, message) and fix from there.

## Run the app without blocking

Build, install, and launch, then return instead of following logs:

```bash
sweetpad run --on booted --no-logs --non-interactive
```

Pass arguments and environment to the app with `--arg` and `--env KEY=VALUE`
(both repeatable). `--detach` leaves the app running after the CLI exits.

## Test

`sweetpad test` runs the scheme's tests and returns a report.

```bash
sweetpad test -o json
sweetpad test --failed -o json                          # only last run's failures
sweetpad test --only-testing MyAppTests/LoginTests -o json
```

`--coverage` adds a coverage summary; `--junit <path>` writes a CI report;
`--retry-flaky <N>` retries each failing test up to N times before calling it
failed.

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
