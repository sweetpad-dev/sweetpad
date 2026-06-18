# Build Backends: a pluggable build pipeline

> Status: **Design / RFC.** Generalizes the [xtool exploration](./xtool-linux-build.md)
> into a build-tool-agnostic architecture. The goal: try different build tools
> (xcodebuild, xtool, …) against the **same `.xcodeproj`/`.xcworkspace`** without
> migrating the project, by normalizing the project into an intermediate Build
> Plan and letting each backend either consume it as CLI parameters or generate
> its own config on the fly.

## Motivation

Several tools can build an iOS app (xcodebuild, xtool, swift build, bazel/buck,
…), but each expects the project in its own shape, so adopting one normally means
**migrating** the project. We don't want migration. We want to keep the Xcode
project canonical and *adapt* to whichever backend the user selects — ideally
switchable on the fly to experiment.

## Concept

```
.xcodeproj / .xcworkspace
        │  (frontend: read once)
        ▼
   BuildPlan (IR)  ── backend-agnostic description of WHAT to build
        │  (select backend via `sweetpad.build.backend`)
        ▼
   BuildBackend.prepare(plan)
        ├── param mode:      map IR → CLI flags            (e.g. xcodebuild)
        └── config-gen mode: materialize IR → config file  (e.g. xtool.yml)
        ▼
   PreparedBuild { command, args, cwd, env, resolveProduct() }
        │  (existing executor — unchanged)
        ▼
   run / install / debug  (unchanged, uses resolveProduct())
```

The key asymmetry: some backends accept the **whole config as parameters**
(xcodebuild: `-scheme`, `-configuration`, `KEY=VALUE` build settings); others
**cannot** and need a config file generated for them (xtool: `Package.swift` +
`xtool.yml`). This is modeled as a per-backend **capability flag** that decides
whether `prepare()` writes files or just assembles an argv.

## Where it slots into the current code

The whole xcodebuild invocation is assembled in one place today:

- `src/build/manager.ts:918` — `BuildManager.buildApp()` builds settings, drives
  `XcodeCommandBuilder`, runs `terminal.execute(...)`, and parses diagnostics.
- `src/build/utils.ts:653` — `XcodeCommandBuilder` is, in effect, the
  *xcodebuild-specific* command builder already.
- `src/common/cli/scripts.ts` — `getBuildSettings*` already resolves the ~1.4k
  build-settings map in-process; this feeds most of the IR for free.

Refactor: the body of `buildApp()` becomes the **xcodebuild backend's
`prepare()`**, with one dispatch step inserted before it. The `terminal.execute`
executor, diagnostics, run/install/debug paths stay as-is (they consume
`PreparedBuild.resolveProduct()` instead of `getBuildSettingsToLaunch` directly).

```ts
const plan     = await buildPlanner.fromXcodeWorkspace({ scheme, configuration, destination, xcworkspace });
const backend  = backends.get(getWorkspaceConfig("build.backend") ?? "xcodebuild");
const prepared = await backend.prepare(plan, ctx);   // xtool.yml generated here, if needed
await terminal.execute(prepared);
const product  = await prepared.resolveProduct();
```

## Interfaces (sketch)

```ts
// WHAT to build — neutral, derived from the Xcode project.
interface BuildPlan {
  action: "build" | "clean" | "test";
  scheme: string;
  configuration: string;                 // Debug / Release

  // product identity (from -showBuildSettings)
  productName: string;
  bundleIdentifier: string;

  // target / platform
  destination: Destination;              // device | simulator | mac (+ udid)
  platform: DestinationPlatform;         // iphoneos / iphonesimulator / ...
  deploymentTarget: string;
  archs?: string[];

  // signing
  developmentTeam?: string;
  codeSignStyle?: "automatic" | "manual";
  allowProvisioningUpdates: boolean;

  // source graph — populated ONLY for backends that can't read .xcodeproj
  sourceRoot: string;
  sources?: string[];
  resources?: string[];
  dependencies?: PackageDep[];

  // pass-through
  buildSettingOverrides: Record<string, string>;  // archs, debug symbols, hot-reload flags
  extraArgs: string[];
  env: Record<string, string>;
}

interface BuildBackend {
  id: string;                            // "xcodebuild" | "xtool" | ...
  capabilities: {
    readsXcodeProjectDirectly: boolean;  // false ⇒ must generate config
    acceptsConfigAsArgs: boolean;        // false ⇒ must generate config
    supportsSimulator: boolean;
    supportsDevice: boolean;
  };
  detect(): Promise<{ available: boolean; reason?: string; version?: string }>;
  prepare(plan: BuildPlan, ctx: BackendContext): Promise<PreparedBuild>;
}

interface PreparedBuild {
  command: string;
  args: string[];
  cwd: string;
  env: Record<string, string>;
  pipes?: Command[];
  diagnosticsMode: "xcodebuild" | "swiftpm" | "raw";
  generatedConfig?: { path: string; contents: string }[]; // inspection / cleanup
  resolveProduct(): Promise<{ appPath: string; bundleId: string }>;
}
```

### Backend examples

| Backend | `readsXcodeProjectDirectly` | `acceptsConfigAsArgs` | `prepare()` does |
|---------|:---:|:---:|---|
| `xcodebuild` | ✅ | ✅ | Today's `XcodeCommandBuilder` argv. No files written. |
| `xtool` | ❌ | ❌ | Generate `xtool.yml` (+ `Package.swift` if absent) into a derived dir; run `xtool dev build` there. |
| `swift build` | ❌ (SPM only) | partial | Only for already-SPM projects; pass flags, no manifest gen. |
| `bazel`/`buck` | ❌ | ❌ | Generate `BUILD` files (hard; future). |

> Note: tuist / xcodegen are the **reverse** direction (manifest → `.xcodeproj`),
> so they are *project generators*, not backends in this model.

## The hard part: reconstructing the source graph

Most IR fields (bundle id, product name, deployment target, signing,
build-setting overrides) come for free from the in-process `-showBuildSettings`
resolver. The **expensive** fields are `sources` / `resources` / `dependencies`:
`-showBuildSettings` does **not** report file membership, so a config-generating
backend must parse the `.pbxproj` to reconstruct an equivalent package. This is
the central risk and the main reason to keep IR generation **lazy** — only
populate the source graph when the selected backend's capabilities require it.

## UX

- `sweetpad.build.backend`: `"xcodebuild"` (default) | `"xtool"` | …
- A "Build with backend…" command to try a tool ad hoc without changing settings.
- `detect()` results surfaced in the Tools view + the *Diagnose Build Setup*
  command (`src/build/commands.ts:354`), replacing the hard `darwin` gate
  (`:377`) with per-backend capability checks.

## Phased plan

1. **Extract `XcodebuildBackend`** from `buildApp()` behind the `BuildBackend`
   interface, with `build.backend` defaulting to `xcodebuild`. Pure refactor —
   no behavior change, fully covered by existing tests.
2. **Add the `BuildPlan` frontend** (`fromXcodeWorkspace`) feeding the xcodebuild
   backend, so the IR is exercised on the happy path before any new tool.
3. **Add `XtoolBackend`** (config-gen mode) as the first non-native backend,
   starting with build-only on Linux (see [xtool doc](./xtool-linux-build.md)).
4. **Generalize run/install/debug** to consume `PreparedBuild.resolveProduct()`.

## Open questions

- Per-workspace vs per-scheme backend selection?
- How faithfully must the generated package reproduce the xcodeproj (build
  phases, scripts, entitlements) before a build is "equivalent"?
- Where do generated configs live — derived data, a temp dir, or committed for
  inspection? (Lean: derived/ignored, with an option to dump them.)
- Caching: regenerate config only when the project fingerprint changes.
