---
sidebar_position: 10
---

# Tests

SweetPad wires Swift test targets into VSCode's native Testing UI, so XCTest and Swift Testing tests behave like any
other VSCode tests — click the gutter ▶️ next to a test, browse them in the **Testing** panel, or jump straight to a
failure from the Problems list.

![Tests](/images/test-menu-run.png)

## Run a test

1. Open a Swift file containing an `XCTestCase` subclass or a `@Test` function (Swift Testing).
2. Click the green ▶️ in the gutter next to a test method, or open the **Testing** panel from the activity bar to
   browse the full test hierarchy.
3. SweetPad builds the test target, runs the test on your active destination, and reports pass/fail inline.

The first run for a target builds the test bundle (this can take a moment). Subsequent runs reuse the build.

## Pick a different test target

If your project has multiple test targets (for example one for the app and a separate one for an SPM module), pin the
one SweetPad should use:

- Run `> SweetPad.Testing: Select testing target` from the command palette and pick a scheme.

That choice is remembered per-workspace.

## Pick a scheme for testing

SweetPad keeps a separate scheme for testing from the one you build with — common in apps that split **`MyApp`**
(build) from **`MyAppTests`** (test). The first time you run a test it asks which scheme to use and remembers your
choice per-workspace.

To change it later — or pin it up front so you're not prompted — run `> SweetPad.Testing: Set scheme for testing`
from the command palette.

## Use a different configuration for testing

Some test suites need a non-Debug configuration (e.g. a `Testing` configuration that disables analytics or points at a
mock backend). Set `sweetpad.testing.configuration` to override the configuration `xcodebuild` uses when running tests:

```json title=".vscode/settings.json"
{
  "sweetpad.testing.configuration": "Testing"
}
```

Or pick interactively with `> SweetPad.Testing: Select configuration for testing`.

## Pick a different destination for testing

SweetPad's testing flow keeps a separate destination from your normal build destination, so you can target an iPad
Simulator for tests while still building/running the app on an iPhone. Pick it with
`> SweetPad.Testing: Select destination for testing`.

## Build without running tests

If you only want to verify the test target compiles (e.g. as part of a pre-commit check):

- `> SweetPad.Testing: Build for testing (without running tests)`

To run already-built tests without rebuilding:

- `> SweetPad.Testing: Test without building`

This pair mirrors `xcodebuild build-for-testing` / `test-without-building` and is much faster than a full
build-and-test cycle when you're iterating on test code.

## Tuist projects

For Tuist projects you can shortcut straight to `tuist test`, which builds and runs every target Tuist knows about
without needing a scheme selection:

- `> SweetPad: Test Generated project using Tuist`

See [Tuist](./tuist.md) for the rest of the Tuist integration.

## Tasks for `tasks.json`

You can wire SweetPad's test action into VSCode tasks:

```json title=".vscode/tasks.json"
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "sweetpad: test",
      "type": "sweetpad",
      "action": "test",
      "scheme": "MyAppTests",
      "configuration": "Debug",
      "problemMatcher": ["$sweetpad-watch"]
    }
  ]
}
```
