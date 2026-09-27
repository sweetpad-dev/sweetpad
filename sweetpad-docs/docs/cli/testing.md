---
sidebar_position: 4
sidebar_label: Testing
---

# Testing

`sweetpad test` builds your test targets, runs them on the destination you picked, and prints a
summary instead of xcodebuild's transcript:

```console
$ sweetpad test
testing SweetpadCIApp for platform=iOS Simulator,id=F92801F8-…
  Linking SweetpadCIAppTests
Suite All tests
Suite SweetpadCIAppTests.xctest
Suite AppTests
  ✓ SweetpadCIAppTests.AppTests.testArithmetic (0.001 seconds)
✓ Tests succeeded (32.8s)
1 passed, 0 failed, 0 skipped (1 total)
result bundle: /Users/you/.local/state/sweetpad/results/SweetpadCIApp-fb7b1c91.xcresult
```

The result bundle on the last line is kept per project, replacing the previous run's. Several
commands read it back, so the run you just did stays inspectable after the output has scrolled away.

## When something fails

A failure prints where it happened, what was expected, and a repeat of the failing tests at the end so
you don't have to scroll up past a long run:

```console
$ sweetpad test
…
  ✓ SweetpadCIAppTests.AppTests.testArithmetic (0.001 seconds)
error: /path/to/Tests/AppTests/AppTests.swift:10: -[SweetpadCIAppTests.AppTests testGreeting] : XCTAssertEqual failed: ("Hello, SweetPad") is not equal to ("Hello, Sweetpad")
  ✗ SweetpadCIAppTests.AppTests.testGreeting
✗ Tests failed
1 passed, 1 failed, 0 skipped (2 total)
  ✗ SweetpadCIAppTests/AppTests/testGreeting: XCTAssertEqual failed: ("Hello, SweetPad") is not equal to ("Hello, Sweetpad")
```

When a test records more than one failure, each one after the first gets a line of its own. A
message that runs over several lines, like the `n → 2` a failed Swift Testing expectation adds, has
its extra lines indented deeper under it:

```console
  ✗ SweetpadCIAppTests/AppTests/testGreeting: XCTAssertEqual failed: ("hello") is not equal to ("world")
      XCTAssertTrue failed - second failure in the same test
  ✗ SweetpadCIAppTests/GreetingSuite/suiteGreeting(): Expectation failed: n == 1
        n → 2
```

With `-o json`, each failure has `message`, the first of them, and `messages`, which lists all of
them in the order the test recorded them.

If the tests don't compile, `sweetpad test` stops with the compile errors, and
`sweetpad build diagnostics` reads them back afterwards, as it does after `sweetpad build`.

Red tests exit with code `3`, the same code a failed build uses, since both mean "the work ran and
the answer was no". A missing scheme or an unresolvable destination is code `4` instead, so a CI
script can tell a genuine test failure apart from a broken invocation.

On Xcode 26 and later, xcodebuild normally collects a system diagnostic report after a failed run,
which can hold the run open for up to ten minutes. `sweetpad test` turns that off, so a failing run
ends as soon as the tests do. To get the report, pass `-- -collect-test-diagnostics on-failure`.

### When the app goes away mid-test

If the app under test crashes or is killed during a test, XCTest only reports that it's gone, with
a message like "Failed to application … is not running" or "… crashed". A unit test's host app that
crashes or calls `exit` gets "Crash: MyApp at …" or "The test runner exited with code 3 …". For
those failures, SweetPad looks up the system's record of how the process ended and prints it under
the failure:

```console
  ✗ ExitProbeUITests/ProbeUITests/testAppAbortsMidTest: dev.sweetpad.exitprobe.app crashed
      Failed to application dev.sweetpad.exitprobe.app is not running
      app terminated: crashed with SIGABRT (sent by ExitProbe[96265])
```

This also covers kills that leave no crash report, such as the simulator host ending the app. With
`-o json`, the same failure carries a `terminationReason` object with the raw reason, code, and
explanation, plus the crash report's path and the fault it names (`exception`) when there is one.
Other failures get no such field, including an assertion whose own text mentions a crash, such as
`XCTFail("the helper crashed")`.
`sweetpad app logs --exits` shows the same records outside a test run.

A crash doesn't always come with a crash report. macOS saves only so many for one app (25 on
macOS 27), so a suite that crashes the app many times in a day stops getting them, and the fault
is missing from the `app terminated:` line. SweetPad adds a line under the failure when that may
be the reason, and the JSON failure gets a `note` with the same text:

```console
      app terminated: crashed with SIGTRAP (sent by exc handler[79175])
      no crash report was found; macOS may have reached its limit of crash reports for this app
```

If SweetPad can't find the exit at all, the line under the failure says so and gives the
`sweetpad app logs --exits` command for the simulator or Mac the tests ran on, which lists the
app's recent exits.
The JSON failure's `note` has the same text.

A unit-test bundle with no host app runs in `xctest`, and a crash there reads "Crash: xctest at …".
The system keeps no exit record for `xctest`, so SweetPad reads the crash log XCTest attached to the
test instead:

```console
  ✗ AppTests/CrashTests/testCBadPointer: Crash: xctest at static xctest.main()
      xctest terminated: crashed with SIGSEGV (sent by exc handler[18468]; EXC_BAD_ACCESS KERN_INVALID_ADDRESS at 0x0000000000000010)
```

With `-o json`, the failure gets `terminationReason` and `crashedIn` from that crash log.
`bundleId` is `null`, since `xctest` has none, and `crashReport` is the same report in
`~/Library/Logs/DiagnosticReports`. When that file is missing, or the result bundle holds no crash
log SweetPad can read, the line under the failure gives a `sweetpad test attachments` command that
exports the crash log XCTest attached.

A crash in a unit test's host app doesn't always fail the test that caused it. When tests run in
parallel, XCTest fails every test that was running at the time, all with the same message. When the
crash comes from work a test left running after it passed, XCTest fails whichever test runs next.
So SweetPad reads the crash report's backtrace, and when the crash is in another test's code it adds
a line under the failure. With `-o json`, the failure's `crashedIn` names that test:

```console
  ✗ AppTests/ParallelSuite/a_recordsCrashed(): Crash: MyApp at specialized static Runner._applyScopingTraits(for:testCase:_:)
      app terminated: crashed with SIGTRAP (sent by exc handler[42305]; EXC_BREAKPOINT)
      the crash report's backtrace is in AppTests/ParallelSuite/b_crashesHost(), not this test
```

When the backtrace doesn't go through any test, or there's no crash report, SweetPad can't tell
which test caused the crash. If the crash failed more than one test, the line says so, and the JSON
failure's `crashCandidates` lists every test it failed.

## Compiling the tests without running them

`sweetpad test build` compiles the test targets and stops, the way `sweetpad build` does for the app.
Nothing launches, so it's a quick way to check that a change didn't break a test:

```console
$ sweetpad test build
building SweetpadCIApp's tests (Debug) for platform=iOS Simulator,id=F92801F8-…
  Compiling AppTests.swift
  Linking SweetpadCIAppTests
✓ Build succeeded (5.6s)
```

Its output matches `sweetpad build`'s: `-q` and `-o json` work the same, a failure exits `3`, and
`sweetpad build diagnostics` reads its errors back. It picks the scheme, configuration, and
destination the way `sweetpad test` does. It doesn't touch the retained result bundle, so
`--failed`, `test output`, and `test attachments` still read your last run.

There's no `--only-testing` here. xcodebuild compiles every test target in the scheme even when
given a filter, so the flag would narrow nothing.

## Running just some of the tests

`--only-testing` and `--skip-testing` narrow the run. Both are repeatable, and both take an
identifier in the form `Target/Class/method`, and you can stop at any level:

```bash
sweetpad test --only-testing SweetpadCIAppTests                              # one target
sweetpad test --only-testing SweetpadCIAppTests/AppTests                     # one class
sweetpad test --only-testing SweetpadCIAppTests/AppTests/testGreeting        # one test
sweetpad test --skip-testing MyAppUITests                                    # everything but the UI tests
```

The target here is the **test target**, the one that produces the `.xctest` bundle rather than the class the
tests live in. It's the first component of the `Suite` line in the output above.

`--failed` reruns only what failed last time, reading the identifiers out of the retained result
bundle:

```bash
sweetpad test --failed
```

When the test process fails outside any test, for example when the host app crashes at launch,
XCTest records a failure named like `MyApp (36652) encountered an error`. That isn't a test, and
given it as a selector xcodebuild runs nothing and reports success. So `--failed` leaves it out and
says so. If it's the only failure, `--failed` stops with an error, and a plain `sweetpad test` is
the rerun.

The failure lines at the end of a run use the same `Target/Class/method` form, so you can paste any
of them into `--only-testing`. A Swift Testing test keeps its parentheses, as in
`SweetpadCIAppTests/GreetingSuite/suiteGreeting()`. Without them xcodebuild selects nothing.

## Watching, retrying, and measuring

Three flags cover most of what you'd otherwise script by hand.

**`--watch`** reruns the suite on every Swift save and keeps going after a failure. Pair it with
`--only-testing` so the loop stays fast while you work on one area:

```bash
sweetpad test --watch --only-testing SweetpadCIAppTests/AppTests
```

**`--retry-flaky N`** runs each failing test up to N times before calling it failed. A test that
passes on retry is reported as flaky rather than broken.

**`--coverage`** collects code coverage and folds the summary into the report.

```bash
sweetpad test --retry-flaky 3
sweetpad test --coverage
```

## Looking at what the run left behind

The summary is deliberately small. Two commands dig into the retained result bundle when it isn't
enough, and neither reruns anything. SweetPad keeps one bundle per project, from its last run, so
neither command takes a scheme, configuration, or destination flag.

### What the tests printed

`sweetpad test output` shows each test's own stdout and stderr, grouped by test, in the order the
tests started:

```console
$ sweetpad test output
SweetpadCIAppTests/AppTests/testGreeting
    greeting under test
    /path/to/Tests/AppTests/AppTests.swift:10: error: -[SweetpadCIAppTests.AppTests testGreeting] : XCTAssertEqual failed: ("Hello, SweetPad") is not equal to ("Hello, Sweetpad")
1 test wrote output (recorded less than a minute ago)
```

Long output is trimmed to the last few KB per test; `--full` prints all of it. A test's own `print`
lands here. XCTest's assertion messages and a UI test's screenshots do not.

Parallel testing doesn't change this, since each worker writes its own output. A test that runs
another test inside itself keeps its own lines, and the inner test's lines are listed under the
inner test. If two tests run at the same time in one process, their lines can't be told apart.
SweetPad then warns and names the directory that holds the output as it was written.

Lines written outside any test, such as the app's own logging at launch, are listed only when no
test wrote anything. When some of them were written between two tests, a note says how many and
where to read them.

### Screenshots and UI dumps

`sweetpad test attachments` exports what a UI test recorded, meaning screenshots and view hierarchy dumps, as
files on disk:

```bash
sweetpad test attachments                      # everything, next to the result bundle
sweetpad test attachments --only-failures      # only the failed tests' files
sweetpad test attachments --output-dir ./out   # somewhere you choose
```

Without `--output-dir` the files land beside the retained result bundle, replacing the previous
export. Each test gets its own directory, and the listing names tests in the same
`Target/Class/method` form as the failure lines, so `--only-testing` takes any of them.

`--only-failures` keeps everything a failed test attached, including the crash log and screen
recording Xcode adds to a UI test whose app crashed, and the crash log it adds on macOS when a test
crashes its host app or `xctest`. Xcode's own marking can't be relied on for this: on an iOS simulator it marks
none of a UI test's files as belonging to a failure, not even the crash log, so SweetPad goes by
which tests failed. When nothing is left, the note says whether
the run had no failures or its failing tests attached nothing. The listing marks a failed test's
files with `(failure)`, and `-o json` sets `failure` to `true` for them.

## Tests in CI

Three things make a test run pipeline-friendly.

**A JUnit report.** `--junit <path>` writes one alongside the normal output, for whatever your CI
displays test results with:

```bash
sweetpad test --junit ./reports/tests.xml
```

The report has a `<testcase>` for every test, including the ones that passed or were skipped, with
the time each took. A test's `classname` is `Target.Class` and its `name` is the method, as in
`classname="SweetpadCIAppTests.AppTests" name="testGreeting"`, so report viewers group the tests by
target and then by class. A failure's `message` is the first line of its first message, and the
`<failure>` element's text has every message the test recorded, in full. When the app went away
mid-test, the `app terminated:` line comes after them.

**Inline annotations on GitHub.** `--gh-annotations` emits GitHub Actions annotations, so failures
show up on the diff instead of only in the log:

```bash
sweetpad test --gh-annotations
```

**No prompts.** `--non-interactive` turns a missing scheme or destination into an error rather than a
question. SweetPad enables it automatically when it detects a CI environment, so you rarely have to
pass it. A raw specifier is still worth pinning, since fuzzy name matching against whatever
simulators the runner happens to have is a liability:

```bash
sweetpad test --destination 'platform=iOS Simulator,name=iPhone 16 Pro' --junit ./reports/tests.xml
```

`--result-bundle <path>` puts the `.xcresult` somewhere you control, which is what you want when the
job archives it as a build artifact. Use it rather than `-- -resultBundlePath`, which `sweetpad test`
refuses because it passes its own.

For the machine-readable form, `-o json` returns the run as a single envelope and `-o ndjson` streams
one event per line as tests finish. In both cases a successful envelope means the command ran, and the
pass/fail counts are inside the payload, under `data.passed`.

Each finished test streams as `{"event":"test","status":"passed","name":…}`, with `failed` or
`skipped` in place of `passed`. XCTest and Swift Testing tests both stream, in serial and parallel
runs.

[Scripts and CI](./scripts-and-ci.md) has the whole automation surface, including a workflow file that
does the above.

## Where the tests run

Tests use the same destination machinery as everything else, so `--on` works the way it does for
builds:

```bash
sweetpad test --on "iPhone 16 Pro"
sweetpad test --on booted
sweetpad test --on mac
sweetpad test --mac     # the same thing
```

SweetPad keeps a **separate remembered destination for testing**, so you can develop against one
simulator and test against another without re-answering the question each time. Set it with:

```bash
sweetpad context select --testing
```

See [Destinations and devices](./destinations.md) for the rest of the story.
