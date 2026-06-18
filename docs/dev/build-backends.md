# Build Backends: a pluggable build pipeline (Rust CLI)

> Status: **Design / RFC** (Phase 1 + the generate seam landed). Generalizes the
> [xtool exploration](./xtool-linux-build.md) into a build-tool-agnostic
> architecture. The goal: build the **same `.xcodeproj` / `.xcworkspace` /
> `Package.swift`** with different tools (xcodebuild, swift build, xtool, Bazel)
> without migrating the project.
>
> **This lives in the Rust CLI (`sweetpad-lib`, the `sweetpad` binary), not in the
> VS Code extension.** The extension is a thin caller that shells out to
> `sweetpad …`; it should need *no* changes to gain a new backend beyond passing
> `--backend <id>`.

## The model (pivoted): generate, then build

Two separate steps, not one fused "build that materializes config on the fly":

1. **Generate (explicit, persistent).** `sweetpad build generate --backend <tool>`
   reads the parsed Xcode project and **writes the tool's config as real files
   into the project directory, to be committed** — `Package.swift` + `xtool.yml`
   for xtool, `MODULE.bazel` + `BUILD.bazel` for Bazel. Output is inspectable,
   diffable, reproducible, and editable by hand; it is *not* a hidden scratch dir
   regenerated on every build.
2. **Build (routes by option).** `sweetpad build start --backend <tool>` selects
   the tool via the `--backend` flag (or config / auto) and runs it against the
   **already-generated** config. The build step does not generate anything.

```
.xcodeproj / .xcworkspace / Package.swift
        │
        │  sweetpad build generate --backend xtool        (explicit, occasional)
        ▼
   parsed project ──► render config ──► Package.swift + xtool.yml   (committed)
        │
        │  sweetpad build start --backend xtool            (every build)
        ▼
   route by --backend ──► run the tool against its committed config
        ▼
   run / install / launch
```

Why the split (vs the earlier on-the-fly idea):

- **Debuggable & trustworthy** — you can read exactly what the tool will build,
  and `git diff` shows config drift when the Xcode project changes.
- **Hand-editable** — generation is a starting point; users can tweak the
  generated `xtool.yml` / `BUILD.bazel` and keep their edits (regeneration policy
  is the user's call, like xcodegen/tuist).
- **Fast, simple build path** — `build start` is just "route + run"; no I/O,
  parsing, or temp-dir management on the hot path.
- **Decoupled failure modes** — a generation bug surfaces at `generate` time with
  a clear artifact, not mid-build.

## Why the Rust CLI

The build engine already *is* the Rust CLI:

- `sweetpad-lib/src/cli/commands/build.rs` — `build start` resolves the project
  and routes to a backend; **`build generate`** (new) routes to a backend's
  generator. Backend selection lives in one helper, `select_backend`.
- `sweetpad-lib/src/cli/backend.rs` — the `BuildBackend` trait + registry +
  `select`. `Xcodebuild` and `SwiftPm` wrap the existing invocations.
- `sweetpad-lib/src/cli/xcodebuild.rs` / `swiftpm.rs` — the actual `xcodebuild` /
  `swift build` argv assembly (unchanged by the refactor).
- `sweetpad-lib/src/cli/scaffold.rs` — already does **pure, Mac-free file
  generation** (`pbxproj`/`xcscheme`/sources) for `project new`. The xtool/Bazel
  generators are the same shape and reuse this precedent.
- `sweetpad-lib/src/cli/commands/doctor.rs` — already probes the toolchain;
  per-backend availability reporting slots in here later.

## Trait (as implemented + planned)

```rust
pub trait BuildBackend {
    fn id(&self) -> &'static str;                 // "xcodebuild" | "swiftpm" | "xtool" | "bazel"

    /// Auto-selection: can this backend build `container`? An explicit
    /// `--backend` bypasses this (so xcodebuild can be forced onto a package).
    fn can_build(&self, container: &Container) -> bool;

    /// Compile the resolved project (routes; assumes any needed config exists).
    fn build(&self, ctx: &mut Context, resolved: &Resolved, opts: &BuildOptions) -> CliResult;

    /// Generate this backend's config files into `out_dir`, to be committed and
    /// then consumed by `build`. Native backends (xcodebuild, swiftpm) read the
    /// project directly and have nothing to generate — the DEFAULT impl reports
    /// that as a no-op. Config-generating backends OVERRIDE it.
    fn generate(&self, ctx: &mut Context, _resolved: &Resolved, _out_dir: &Path) -> CliResult {
        ctx.out.note(&format!(
            "the {} backend builds the project directly — no config to generate", self.id()
        ));
        Ok(())
    }
}
```

The default `generate` is the crux of the pivot: native backends inherit a clean
no-op, and a config-gen backend is "just" a `generate` override plus a `build`
that shells out to its tool — no core changes, no `match` on the tool anywhere.

### Backends

| Backend | `can_build` (auto) | `build` runs | `generate` writes |
|---------|---|---|---|
| `xcodebuild` | workspace / project | `xcodebuild` | — (native, no-op) |
| `swiftpm` | Swift package | `swift build` | — (native, no-op) |
| `xtool` | (explicit) | `xtool dev build` | `Package.swift` + `xtool.yml` |
| `bazel` | (explicit) | `bazel build //App:App` | `MODULE.bazel` + `BUILD.bazel` |

> tuist / xcodegen are the **reverse** direction (manifest → `.xcodeproj`), so
> they are *project generators*, not backends here.

### xtool generator

`generate` renders, from the Build Plan, a `Package.swift` (if the project isn't
already a package) plus an `xtool.yml` (app name, bundle id, deployment target,
icons), into the project dir. Pure functions `render_package` / `render_xtool_yml`
→ `String`, round-trip unit-tested like `scaffold.rs`. `xtool.yml` is plain YAML
(a *standard* format) so it is rendered with `serde_yaml`, not hand-rolled (per
the crate's dependency policy). `build` then runs `xtool dev build`. See the
[xtool doc](./xtool-linux-build.md) — this is the Linux build story.

### Bazel (`rules_apple`) generator

Same *shape* as xtool — a config-generating backend — but a different *value
proposition*: xtool buys **Linux/Windows** portability; Bazel buys
**hermeticity, remote caching, and scale** on a (still macOS-bound) Apple
toolchain. `generate` writes:

- `MODULE.bazel` (bzlmod) pulling in `rules_apple` + `rules_swift` +
  `apple_support`, version-pinned.
- `BUILD.bazel` with `swift_library` target(s) for the sources and an
  `ios_application` / `macos_application` top-level target (`bundle_id`,
  `minimum_os_version`, `families`, `infoplists`, `provisioning_profile` mapped
  from the Build Plan).
- a `.bazelrc` for the destination/arch.

`build` runs `bazel build //App:App`; the bundle lands under `bazel-bin/…` and is
located via `bazel cquery --output=files` (sandbox/symlink-safe). Bazel is the
**strictest consumer of the source graph** (every target's `srcs`/`deps`/`data`
and dependency edges must be explicit), which makes it the best forcing function
for getting source-graph extraction right.

## The hard part: reconstructing the source graph

Most config fields (bundle id, product name, deployment target, signing) come for
free — the crate already resolves the full build-settings map in-process
(`build_settings.rs` / `resolver.rs`). The **expensive** fields are
`sources` / `resources` / `dependencies`: build settings don't report file
membership, so a generator must read the `.pbxproj` (already parsed:
`pbxproj.rs`, `project.rs`) to reconstruct an equivalent package. This work lives
entirely in `generate` (run occasionally), **not** on the build hot path — a
direct benefit of the pivot.

## CLI surface

- `sweetpad build generate --backend <tool> [--output <dir>]` — write the tool's
  config. Default `--output` is the **project directory** (committed); `--backend`
  is required to be a config-generating tool (native backends report a no-op).
- `sweetpad build start --backend <tool>` — build with the tool against the
  committed config.
- Selection precedence (both commands): `--backend` flag (`SWEETPAD_BACKEND`) >
  per-project config (`backend = …`) > auto by container type.

## VS Code extension impact

Effectively none in the pipeline: the extension already invokes the `sweetpad`
CLI. A user/extension runs `sweetpad build generate --backend xtool` once
(committing the result), then builds with `--backend xtool`. The host-platform
gate in the extension (`src/build/commands.ts:377`) becomes "is a usable backend
available?" rather than a hard `darwin` check.

## Phased plan

1. ✅ **Backend trait + routing.** *Done* — `backend.rs` adds `BuildBackend` +
   `Xcodebuild`/`SwiftPm`; `build start` routes via `select_backend` (flag >
   config > auto). No behavior change; routing unit-tested.
2. ✅ **Generate command + seam.** *Done* — `build generate [--output]` plus the
   default-no-op `generate` trait method. Native backends report nothing to
   generate; the override point is ready for config-gen backends.
3. ✅ **`BuildPlan` IR + inspection.** *Done* — `sweetpad-lib/src/cli/plan.rs`
   extracts a normalized `BuildPlan` (scheme, configuration, app target, product
   name, bundle id, deployment target, supported platforms, **source graph**, and
   target dependencies) from the resolved project: identity via the in-process
   build-settings resolver, the source/dependency graph via the pbxproj reader
   (`project::target_source_files` / `target_dependencies`). Surfaced by
   `sweetpad build plan [--json]`, the tested consumer that config-gen backends
   will render from. Covers `.xcodeproj` and `.xcworkspace` (owning member via
   `workspace::project_for_scheme`); Swift packages are rejected (already SwiftPM).
4. ✅ **xtool generator.** *Done* — `sweetpad-lib/src/cli/xtool.rs`:
   `XtoolBackend` (registered, explicit-only via `--backend xtool`). `generate`
   consumes the `BuildPlan` and writes a `.library` `Package.swift` (sources from
   the graph) + a minimal `xtool.yml` into the project dir; `build` runs
   `xtool dev build`. Pure `render_*` functions with fixture-backed unit tests.
   End-to-end verified on Linux by `.github/workflows/xtool-linux-build.yaml`
   (the generate path is reliable; the full cross-build against a Mac-exported
   iOS SDK is the workflow's best-effort/experimental job). The rendered
   `Package.swift` template should be validated against a real `xtool new`.
5. **Bazel generator** (`render_module_bazel` / `render_build_bazel`) reusing the
   source-graph extraction; validates the trait against a second, very different
   tool.
6. **Wire availability into `doctor`** and `--backend` into the extension;
   generalize run/install to consume each backend's product location.

## Open questions

- Regeneration policy: overwrite vs merge when the generated config has hand
  edits? (Lean: overwrite with a warning if dirty; `--force` to skip the check.)
- How faithfully must a generated package reproduce the xcodeproj (build phases,
  run scripts, entitlements) before a build counts as "equivalent"?
- Backend selection scope: global flag, per-project config, or per-scheme?
