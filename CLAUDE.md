# SweetPad Development Guide

## Project Overview

SweetPad is a VSCode extension for iOS/Swift development. It wraps Xcode CLI tools
(xcodebuild, xcrun, simctl, devicectl) and provides a VSCode UI for building, running,
testing, and debugging iOS/macOS/watchOS/tvOS/visionOS apps.

## Build & Test

```bash
npm run build          # Build extension with esbuild
npm test               # Run all Jest tests
npm run check:all      # Format + lint + typecheck
```

## Architecture

- `src/build/` — xcodebuild integration (build, run, clean, test)
- `src/common/cli/scripts.ts` — CLI command wrappers (the boundary to Xcode tools)
- `src/common/xcode/` — Xcode workspace/project parsing
- `src/devices/` — Physical device management (devicectl)
- `src/simulators/` — Simulator management (simctl)
- `src/destination/` — Unified device/simulator abstraction
- `src/debugger/` — LLDB debugging
- `src/format/` — Swift formatting
- `src/testing/` — XCTest integration

### Key pattern: pure logic vs. side effects

Code that parses CLI output, computes destination strings, or processes workspace XML
is pure and testable without mocks. Code that calls `vscode.*` APIs or spawns processes
requires mocking at the boundary.

**Pure (test directly):** types.ts, utils.ts, helpers.ts, cache.ts, parsers
**Side effects (mock at boundary):** commands.ts, manager.ts, provider.ts

## Testing

Tests use Jest + ts-jest. VSCode API is mocked via `src/__mocks__/vscode.ts`.

```bash
npm test                                    # All tests
npx jest --testPathPattern='build/utils'    # Specific test
npx jest --no-coverage                      # Skip coverage
```

### Writing tests

- Colocate tests: `src/foo/bar.ts` → `src/foo/bar.spec.ts`
- Prefer testing pure functions without mocks
- For CLI output parsing, use fixture files in `tests/` directories
- Existing fixture directories:
  - `tests/devicectl-data/` — devicectl JSON output samples
  - `tests/xcdevice-data/` — xcdevice JSON output samples
  - `tests/contents-data/` — Xcode workspace XML samples
  - `tests/examples/` — Real Xcode projects (run `npm run download-examples`)

## Reproducing Bug Reports

When a bug report involves Xcode behavior, use these scripts to reproduce it locally.
This requires macOS with Xcode installed.

### Step 1: Create a matching project structure

```bash
# List available templates
npm run setup-test-project -- --list

# Create a project matching the bug report's setup
npm run setup-test-project -- ios-app
npm run setup-test-project -- multi-project
npm run setup-test-project -- spm-package
npm run setup-test-project -- multi-platform

# Or output to a specific directory
npm run setup-test-project -- ios-app --output /tmp/bug-123
```

Templates:
- `ios-app` — Single iOS app, one scheme (requires `xcodegen`)
- `multi-project` — Workspace with app + framework (requires `xcodegen`)
- `spm-package` — Swift Package Manager package (no xcodegen needed)
- `multi-platform` — iOS + watchOS + tvOS targets (requires `xcodegen`)

If the bug requires a custom project structure, create a new template in
`scripts/setup-test-project.ts` by adding to the `TEMPLATES` object.

### Step 2: Capture real Xcode output

```bash
# Capture all CLI output for a workspace
npm run capture-fixtures -- ./tests/fixtures/projects/ios-app/TestApp.xcodeproj/project.xcworkspace

# Tag the capture for a specific bug
npm run capture-fixtures -- ./path/to/workspace --tag bug-123
```

This runs real `xcodebuild`, `simctl`, and `devicectl` commands and saves the output
to `tests/fixtures/captured/<tag>/`. The manifest.json lists what was captured.

### Step 3: Write a failing test using the captured fixture

```typescript
// Example: testing that scheme parsing handles a specific output format
import { readFileSync } from "node:fs";
import { parseCliJsonOutput } from "../src/common/cli/scripts";

it("parses schemes from captured xcodebuild output", () => {
  const raw = readFileSync("tests/fixtures/captured/bug-123/xcodebuild-list.json", "utf-8");
  const parsed = parseCliJsonOutput(raw);
  expect(parsed.workspace.schemes).toContain("ExpectedScheme");
});
```

### Step 4: Fix the bug and verify

Run `npm test` to confirm the fix. The captured fixture becomes a permanent regression
test — copy it from `tests/fixtures/captured/` to the appropriate committed fixture
directory (e.g., `tests/devicectl-data/`) so it's tracked in git.

### Workflow summary

```
Bug report → setup-test-project → capture-fixtures → write failing test → fix → commit fixture + fix
```

## Simulators & Devices

Useful commands when reproducing issues locally:

```bash
# List available simulators
xcrun simctl list devices available

# Boot a simulator
xcrun simctl boot <UDID>

# List connected physical devices
xcrun devicectl list devices

# List schemes for a workspace
xcodebuild -list -json -workspace <path>

# Build
xcodebuild -workspace <path> -scheme <scheme> -destination 'platform=iOS Simulator,id=<UDID>' build
```
