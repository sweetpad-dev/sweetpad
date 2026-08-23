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
  ✗ SweetpadCIAppTests/testGreeting(): XCTAssertEqual failed: ("Hello, SweetPad") is not equal to ("Hello, Sweetpad")
```

Red tests exit with code `3`, the same code a failed build uses, since both mean "the work ran and
the answer was no". A missing scheme or an unresolvable destination is code `4` instead, so a CI
script can tell a genuine test failure apart from a broken invocation.

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

:::note

If `--failed` comes back with "isn't a member of the specified test plan or scheme", the identifier it
recovered is missing its test target. Copy the full `Target/Class/method` form into `--only-testing`
instead.

:::

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
enough, and neither reruns anything.

### What the tests printed

`sweetpad test output` shows each test's own stdout and stderr, grouped by test:

```console
$ sweetpad test output
AppTests/testGreeting
    greeting under test
    /path/to/Tests/AppTests/AppTests.swift:10: error: -[SweetpadCIAppTests.AppTests testGreeting] : XCTAssertEqual failed: ("Hello, SweetPad") is not equal to ("Hello, Sweetpad")
1 test wrote output (recorded less than a minute ago)
```

Long output is trimmed to the last few KB per test; `--full` prints all of it. A test's own `print`
lands here. XCTest's assertion messages and a UI test's screenshots do not.

### Screenshots and UI dumps

`sweetpad test attachments` exports what a UI test recorded, meaning screenshots and view hierarchy dumps, as
files on disk:

```bash
sweetpad test attachments                      # everything, next to the result bundle
sweetpad test attachments --only-failures      # only what a failing test recorded
sweetpad test attachments --output-dir ./out   # somewhere you choose
```

Without `--output-dir` the files land beside the retained result bundle, replacing the previous
export.

## Tests in CI

Three things make a test run pipeline-friendly.

**A JUnit report.** `--junit <path>` writes one alongside the normal output, for whatever your CI
displays test results with:

```bash
sweetpad test --junit ./reports/tests.xml
```

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
job archives it as a build artifact.

For the machine-readable form, `-o json` returns the run as a single envelope and `-o ndjson` streams
one event per line as tests finish. In both cases a successful envelope means the command ran, and the
pass/fail counts are inside the payload, under `data.passed`.

[Scripts and CI](./scripts-and-ci.md) has the whole automation surface, including a workflow file that
does the above.

## Where the tests run

Tests use the same destination machinery as everything else, so `--on` works the way it does for
builds:

```bash
sweetpad test --on "iPhone 16 Pro"
sweetpad test --on booted
sweetpad test --on mac
```

SweetPad keeps a **separate remembered destination for testing**, so you can develop against one
simulator and test against another without re-answering the question each time. Set it with:

```bash
sweetpad context select --testing
```

See [Destinations and devices](./destinations.md) for the rest of the story.
