---
sidebar_position: 17
sidebar_label: Scripts & CI
---

# Scripts and CI

The CLI is built to be called by other programs as much as by you. Every command has a machine-readable
output mode, a meaningful exit code, and a way to refuse prompting, which together are what a git
hook, a Makefile, a CI job, or an AI coding agent needs to work with an Xcode project without a human
watching.

## Exit codes

Every command exits with a code that says *what kind* of thing went wrong, not just that something
did:

| Code | Meaning                                                                    |
| ---- | -------------------------------------------------------------------------- |
| 0    | Success.                                                                   |
| 1    | Generic failure.                                                           |
| 2    | Usage error: bad flags or arguments.                                       |
| 3    | The build or the tests failed.                                             |
| 4    | Couldn't resolve a target: unknown scheme, destination, simulator, device. |
| 5    | A required tool is missing (xcodebuild, simctl, …).                        |
| 6    | Cancelled: a declined prompt, or Ctrl-C.                                   |

The distinction that matters most in CI is **3 versus 4**. Code 3 means your code is broken; code 4
means the job is misconfigured and never got as far as compiling anything. Treating them the same is
how a typo in a simulator name gets reported to your team as a failing test suite.

A signal that kills the process exits `128 + signo` (130 for Ctrl-C, 143 for SIGTERM) after the
terminal is restored and child processes are reaped.

## JSON output

`-o json` (or `--json`) returns the whole command as one envelope on stdout:

```console
$ sweetpad scheme list -o json
{
  "data": {
    "container": "/path/to/MyApp/MyApp.xcodeproj",
    "schemes": [
      { "name": "MyApp", "selected": true },
      { "name": "MyAppMac", "selected": false }
    ],
    "selected": "MyApp"
  },
  "ok": true,
  "schema": 1
}
```

Three fields, always: `schema` for the envelope version, `ok` for whether the command executed, and
`data` for the payload. Errors take the same shape on stderr, with `error` in place of `data`:

```console
$ sweetpad build --on "nope-does-not-exist" -o json
{"error":{"code":"target_resolution","message":"--on \"nope-does-not-exist\" matches nothing (try one of: …)"},"ok":false,"schema":1}
```

`error.code` is a name, not a number: `generic`, `build_failure`, `target_resolution`, `tool_missing`,
or `user_cancel`. It mirrors the exit-code taxonomy, so a script can branch on either.

:::warning

`"ok": true` means "the command executed", not "the outcome was good". A red test suite exits 3 and
reports its failures inside `data` with a perfectly valid envelope. Check the payload's own status, which for tests is `data.passed`, rather than the envelope.

:::

## Streaming with NDJSON

A build can take minutes, and a single envelope at the end tells you nothing while it runs. `-o ndjson`
emits one JSON object per line as things happen, ending with a result event:

```console
$ sweetpad build -o ndjson
{"event":"task","kind":"compile","name":"ContentView.swift"}
{"event":"task","kind":"copy","name":"MyApp.swiftsourceinfo"}
{"data":{"built":true,"configuration":"Debug","destination":"platform=iOS Simulator,id=F92801F8-…","durationMs":2451,"errors":0,"productPath":"/…/MyApp.app","scheme":"MyApp","warnings":0},"event":"result","ok":true}
```

Every line has an `event` field. The last one is always `"event":"result"` and carries the same
`ok`/`data` pair the one-shot envelope would have, so a consumer can stream progress and still get a
single authoritative answer at the end.

Use `ndjson` for the long-running verbs (`build`, `test`, `app logs`) and plain `json` for anything
that returns immediately.

## Quiet mode

`-o quiet` prints nothing at all and communicates only through the exit code. It's the right mode for
a check whose output you'd throw away:

```bash
sweetpad build -o quiet || echo "build is broken"
```

There's also `-q`, which suppresses progress chatter but keeps results, and `-v`, which prints the raw
tool output. `-q` wins if you pass both.

## Never prompting

Interactive commands ask questions: which scheme, which simulator. `--non-interactive` turns every
such question into an error instead:

```bash
sweetpad build --non-interactive
```

You rarely need to pass it. SweetPad enables it automatically when `CI` is set, which every CI runner
does. Set `SWEETPAD_NONINTERACTIVE=1` to force it anywhere else.

## Pinning the context

A script shouldn't depend on what someone last picked at a prompt. Two ways to be explicit:

**Per command**, with flags:

```bash
sweetpad test --scheme MyApp --destination 'platform=iOS Simulator,name=iPhone 16 Pro'
```

**Once for the whole job**, with environment variables:

```bash
export SWEETPAD_SCHEME=MyApp
export SWEETPAD_DESTINATION='platform=iOS Simulator,name=iPhone 16 Pro'
sweetpad build && sweetpad test
```

Prefer `--destination`, the raw specifier, over `--on` in CI. Fuzzy name matching against whatever
simulators a runner happens to have installed is a liability. A pinned specifier fails loudly instead
of quietly building for something else. See [Configuration](./configuration.md#environment-variables)
for the full list of variables.

`-C <dir>` runs as if started somewhere else, like `git -C`, which saves a subshell when a job builds
more than one project:

```bash
sweetpad -C ./ios build
```

## GitHub Actions

`--gh-annotations` turns build and test diagnostics into workflow commands, so failures land on the
diff instead of only in the log:

```console
$ sweetpad build --gh-annotations
::error file=/path/to/Sources/App/ContentView.swift,line=12,col=14::cannot find 'greeting' in scope
```

It writes those to stdout, which is where `-o json` and `-o ndjson` put their own output, so the two
are mutually exclusive, and SweetPad says so rather than interleaving them.

A workflow that builds, tests, and reports both inline and as a JUnit report:

```yaml title=".github/workflows/ios.yml"
name: iOS

on: [push, pull_request]

jobs:
  build-and-test:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@v4

      - name: Install SweetPad
        run: brew install sweetpad-dev/tap/sweetpad

      - name: Build
        run: sweetpad build --gh-annotations
        env:
          SWEETPAD_SCHEME: MyApp
          SWEETPAD_DESTINATION: platform=iOS Simulator,name=iPhone 16 Pro

      - name: Test
        run: sweetpad test --gh-annotations --junit ./reports/tests.xml
        env:
          SWEETPAD_SCHEME: MyApp
          SWEETPAD_DESTINATION: platform=iOS Simulator,name=iPhone 16 Pro

      - name: Upload test report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: test-report
          path: ./reports/
```

`--non-interactive` isn't needed, because the `CI` variable GitHub sets already implies it.

## A pre-commit hook

The cheapest useful hook checks formatting and that the project still compiles, and says which one
failed:

```bash title=".git/hooks/pre-commit"
#!/bin/sh
set -e

sweetpad format --check || {
  echo "unformatted Swift files — run 'sweetpad format'" >&2
  exit 1
}

sweetpad build -o quiet || {
  echo "build failed — run 'sweetpad build' to see why" >&2
  exit 1
}
```

Bear in mind `sweetpad build` compiles the app target and not your test targets, so a hook like this
won't catch a test that no longer compiles.

## Reading results in a script

Anything the CLI knows, it will hand over as JSON. A few patterns worth having:

```bash
# The path to the built .app
sweetpad build -o json | jq -r '.data.productPath'

# One build setting, as a bare string — no jq needed
sweetpad settings show --key PRODUCT_BUNDLE_IDENTIFIER

# Every booted simulator's UDID
sweetpad devices -o json | jq -r '.data.destinations[] | select(.booted) | .udid'

# Did the tests pass?
sweetpad test -o json | jq -e '.data.passed' >/dev/null
```

For an AI coding agent, there's a better starting point than raw JSON: the
[agent skills](./agent-skills.md) teach an agent this whole surface directly, so it drives the CLI
instead of guessing at xcodebuild.
