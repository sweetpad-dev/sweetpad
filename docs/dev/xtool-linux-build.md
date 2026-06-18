# Exploration: Building on Linux with `xtool`

> Status: **Exploration / RFC** — no code committed yet. This document captures
> research into integrating [`xtool`](https://xtool.sh/documentation/xtool/) so
> that SweetPad can build (and eventually run/deploy) iOS apps from Linux, where
> Xcode is unavailable.

## 1. What is `xtool`?

`xtool` ([xtool-org/xtool](https://github.com/xtool-org/xtool)) is a
cross-platform, open-source "Xcode replacement". It builds, signs, and deploys
iOS apps from a **SwiftPM package** on **Linux, Windows (WSL), and macOS** —
without Xcode or `xcodebuild`.

Key characteristics:

- **Input is a Swift Package, not an `.xcodeproj`/`.xcworkspace`.** `xtool`
  builds a `Package.swift`-based project plus an `xtool.yml` manifest (app name,
  bundle id, etc.) into a `.app`/`.ipa`. It does *not* read Xcode project files.
- **No simulator on Linux/Windows.** The iOS Simulator is a macOS-only Apple
  technology. On Linux/Windows `xtool` targets **physical devices** (sign +
  install over USB/network); the simulator path only works on macOS.
- **Signing requires an Apple Developer account.** `xtool auth` /
  `xtool ds` talk to Apple Developer Services to provision/sign. This is a hard
  requirement for on-device deployment, even free accounts (7-day certs).
- **Needs a Darwin Swift SDK.** `xtool setup` / `xtool sdk` install a
  cross-compilation toolchain (Swift Static Linux SDK + Darwin platform SDK) so a
  Linux host can emit Mach-O iOS binaries.
- Ships **XKit**, a SwiftPM library for talking to Apple Developer Services and
  iOS devices programmatically.

### Relevant CLI surface (subject to change)

| Group | Commands | Purpose |
|-------|----------|---------|
| Setup | `xtool setup`, `xtool sdk`, `xtool auth` | Install toolchain/SDK, authenticate with Apple |
| Dev | `xtool new`, `xtool dev`, `xtool ds` | Scaffold project, build/run dev cycle, Developer Services |
| Device | `xtool devices`, `xtool install`, `xtool uninstall`, `xtool launch` | Enumerate devices, install/launch the built app |

## 2. How SweetPad builds today (the constraint)

SweetPad is currently **macOS + Xcode only**. The whole pipeline assumes
`xcodebuild` and Apple's command-line tooling:

- `src/build/commands.ts:377` — `diagnoseBuildSetupCommand` hard-rejects any host
  where `process.platform !== "darwin"` with *"depends on Xcode which is
  available only on macOS"*.
- `src/build/manager.ts` — `buildApp()` constructs `xcodebuild` invocations;
  `getXcodeBuildCommand()` / `getXcodeBuildDestinationString()` produce
  `xcodebuild`-shaped args and `-destination` strings.
- `src/common/cli/scripts.ts` — schemes, targets, configurations and build
  settings all come from `xcodebuild -showBuildSettings`, `xcrun`, `xcodebuild
  -list`, etc. SPM packages are *already* a recognized workspace type
  (`detectWorkspaceType() === "spm"`, `getSwiftPMDirectory()`), **but they are
  still built by handing the package to `xcodebuild`**, not `swift build`.
- Running branches by Apple-specific destination types (`runOnMac`,
  `runOniOSSimulator`, device install via `xcrun devicectl`) in
  `src/build/manager.ts`.
- `package.json` `activationEvents` already include `**/Package.swift`, so the
  extension activates for SPM projects — it just can't build them off-Mac.

**Implication:** SweetPad's central abstraction is "an Xcode workspace + scheme +
configuration + destination". `xtool`'s abstraction is "a SwiftPM package +
`xtool.yml` + a device". These don't line up 1:1, which is the core integration
challenge.

## 3. What "build on Linux via xtool" actually requires

To make even a minimal Linux build work, several Xcode assumptions must become
pluggable:

1. **Host gate.** Replace the hard `darwin` check with capability detection
   (macOS+Xcode → existing path; Linux/Windows with `xtool` installed → xtool
   path).
2. **A build "backend" abstraction.** Introduce a seam so the build manager can
   dispatch to either an `xcodebuild` backend (today) or an `xtool` backend
   (new). The cleanest seam is around `BuildManager.buildApp()` /
   `runSchemeTask()` and the `scripts.ts` query functions.
3. **Project model translation.** On Linux there are no schemes/configurations in
   the Xcode sense. SweetPad would need to treat the SwiftPM package (+
   `xtool.yml`) as the unit of build, and synthesize a degenerate
   scheme/destination model so the existing UI (tree views, status bar, pickers)
   keeps working.
4. **Destination model.** No simulators on Linux → the destination picker must
   surface only physical devices discovered via `xtool devices` (XKit), not
   `xcrun simctl`/`xcdevice`.
5. **Toolchain bootstrap & Apple auth.** `xtool setup`/`auth` are interactive and
   account-bound. SweetPad's "Tools" UX (currently Homebrew-centric) would need
   an `xtool`-aware setup flow.
6. **Diagnostics.** Build-error parsing currently keys off `xcodebuild`/clang
   output (`src/build/diagnostics-parser.ts`). `xtool dev` ultimately runs
   `swift build`, so SwiftPM/clang diagnostics are similar but the wrapper output
   differs and needs its own parsing.

> **Update:** the integration is now framed as the first non-native backend of a
> general pluggable build pipeline **in the Rust `sweetpad` CLI** (`sweetpad-lib`),
> not the VS Code extension — see [Build Backends](./build-backends.md). `xtool`
> is a *config-generating* backend (it can't read `.xcodeproj`, so its `prepare()`
> materializes `Package.swift` + `xtool.yml` from the normalized Build Plan into a
> scratch dir). The VS Code extension just passes `--backend xtool`.

## 4. Integration options

### Option A — Minimal "build only" backend (recommended first step)
Add an `xtool` build backend gated behind host/tool detection. On Linux:
- Detect `xtool` on `PATH`; if absent, guide the user to install it.
- Detect SwiftPM packages with an `xtool.yml`; treat the package dir as the
  build unit.
- Map **Build** → `xtool dev build` (or `xtool dev`), streamed into the existing
  task terminal.
- Skip simulator/run for now; deliver compile + diagnostics only.

Smallest surface area, immediately useful for "edit + compile" on Linux, and
proves the backend seam without touching signing or device flows.

### Option B — Build + deploy to device
Extend Option A with `xtool devices`/`install`/`launch`, plus `xtool auth`/`ds`
for signing. Much larger: interactive Apple auth, certificate/provisioning UX,
USB device discovery, and a new run/debug path. High value but high effort and
many failure modes outside our control (Apple account state, 2FA, cert limits).

### Option C — Full Xcode-parity replacement
Make `xtool` a first-class peer of `xcodebuild` everywhere (schemes,
configurations, tests, debug, hot reload). Effectively a second product surface;
not justified until A/B prove demand.

## 5. Key challenges & risks

- **Project shape mismatch.** Most existing SweetPad users have `.xcworkspace`/
  `.xcodeproj` projects, which `xtool` cannot build. The Linux story realistically
  only serves **pure SwiftPM + `xtool.yml`** projects. We should be explicit that
  this is a *new project type*, not a port of existing projects.
- **No simulator off-Mac.** The flagship "run in simulator" demo doesn't exist on
  Linux; the value prop is narrower (compile + on-device).
- **Apple account coupling.** Anything beyond compiling needs Developer Services
  auth, which is interactive, rate-limited, and 2FA-bound — hard to make
  frictionless inside VS Code.
- **Toolchain weight.** `xtool setup` pulls a Swift toolchain + Darwin SDK;
  first-run UX and disk/network cost are significant.
- **Moving target.** `xtool` is young and pre-1.0; CLI/manifest surface may shift.
  An abstraction layer (don't hard-code arg strings everywhere) reduces churn.
- **LSP/autocomplete already works on Linux.** `sourcekit-lsp` natively supports
  SwiftPM (see `src/common/cli/scripts.ts:700`), so editing/autocomplete is the
  *easy* part — building is the gap `xtool` fills.

## 6. Recommended path

1. **Ship Option A (build-only) behind a setting/feature flag.**
   - New host-capability detection replacing the `darwin` hard-gate
     (`src/build/commands.ts:377`).
   - A `BuildBackend` interface with two implementations: `XcodebuildBackend`
     (extract current logic) and `XtoolBackend` (new). Dispatch in
     `BuildManager`.
   - `xtool`-presence detection wired into the Tools view.
   - `xtool dev build` mapped to the Build command; reuse the task terminal and
     add a SwiftPM-output diagnostics parser.
2. **Validate with an example project** (a `Package.swift` + `xtool.yml` "hello
   iOS" app) added under `examples/`.
3. **Then evaluate Option B** (device deploy + signing) based on user demand.

## 7. Open questions

- Should Linux support be a separate VS Code extension capability flag, or fully
  integrated with conditional activation?
- How do we present "no simulator on Linux" without confusing users who expect
  parity?
- Do we want to *generate* `xtool.yml`/`Package.swift` (à la the existing
  XcodeGen/Tuist integrations) for users starting fresh on Linux?
- Minimum supported `xtool` version, and how to detect/communicate it.

## References

- xtool docs: <https://xtool.sh/documentation/xtool/>
- xtool repo: <https://github.com/xtool-org/xtool>
- Swift Forums announcement:
  <https://forums.swift.org/t/xtool-cross-platform-xcode-replacement-build-ios-apps-on-linux-and-more/79803>
