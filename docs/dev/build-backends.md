# Build Backends: a pluggable build pipeline (Rust CLI)

> Status: **Design / RFC.** Generalizes the [xtool exploration](./xtool-linux-build.md)
> into a build-tool-agnostic architecture. The goal: try different build tools
> (xcodebuild, swift build, xtool, …) against the **same `.xcodeproj` /
> `.xcworkspace` / `Package.swift`** without migrating the project — by
> normalizing the project into an intermediate Build Plan and letting each
> backend either consume it as CLI arguments or generate its own config on the
> fly.
>
> **This lives in the Rust CLI (`sweetpad-lib`, the `sweetpad` binary), not in the
> VS Code extension.** The extension is a thin caller that shells out to
> `sweetpad …`; it should need *no* changes to gain a new backend beyond passing
> `--backend <id>`.

## Why the Rust CLI

The build engine already *is* the Rust CLI:

- `sweetpad-lib/src/cli/commands/build.rs:25` — `build start` resolves the
  project and then **hard-`match`es on the container**: a `SwiftPackage` goes to
  `swiftpm::build` (runs `swift build`), everything else to
  `xcodebuild::BuildPlan{…}.run()`. That `match` is the proto-backend-dispatch we
  want to make pluggable.
- `sweetpad-lib/src/cli/xcodebuild.rs:15` — `BuildPlan` already assembles an
  xcodebuild argv and runs it. This is the **xcodebuild backend in all but name**
  (param mode: it passes scheme/configuration/destination/settings as flags).
- `sweetpad-lib/src/cli/swiftpm.rs:145` — `swiftpm::build` is the **swift-build
  backend in all but name** (runs from the package dir, no Xcode).
- `sweetpad-lib/src/cli/resolve.rs:225` — `BuildTarget { scheme, configuration,
  destination }` is the embryonic IR; `Container` (`resolve.rs:22`) is the
  project handle.
- `sweetpad-lib/src/cli/scaffold.rs` — already does **pure, Mac-free file
  generation** (`pbxproj`/`xcscheme`/sources) for `project new`. xtool config
  generation is the same shape and reuses this precedent.
- `sweetpad-lib/src/cli/commands/doctor.rs:88` — already probes the toolchain
  (`xcodebuild`, `swift`, …). Per-backend `detect()` reporting slots in here.

So this is mostly *naming and lifting* existing code behind a trait, not new
machinery.

## Concept

```
.xcodeproj / .xcworkspace / Package.swift
        │  (frontend: resolve once)
        ▼
   BuildPlan (IR)  ── backend-agnostic description of WHAT to build
        │  (select backend: --backend / config / auto by container+host)
        ▼
   BuildBackend::prepare(&plan)
        ├── arg mode:        map IR → CLI flags            (xcodebuild, swift build)
        └── config-gen mode: materialize IR → config file  (xtool: Package.swift+xtool.yml;
                                                            bazel: MODULE.bazel+BUILD.bazel)
        ▼
   PreparedBuild { program, args, cwd, env, run(), resolve_product() }
        │  (existing process runner / buildlog — unchanged)
        ▼
   run / install / launch  (app.rs, via resolve_product())
```

The key asymmetry: some backends accept the **whole config as parameters**
(xcodebuild: `-scheme`, `-configuration`, `KEY=VALUE` settings); others
**cannot** and need a config file generated for them (xtool: `Package.swift` +
`xtool.yml`). This is a per-backend **capability**, and config generation is an
**internal detail of that backend's `prepare()`**, never a concern of the core.

## Trait sketch (Rust)

```rust
/// WHAT to build — neutral, derived from the resolved project. A superset of
/// today's `resolve::BuildTarget`, carrying the extra fields a config-generating
/// backend needs. Source-graph fields are populated lazily (see "the hard part").
pub struct BuildPlan<'a> {
    pub container: &'a Container,          // Workspace | Project | SwiftPackage
    pub action: Action,                    // Build { clean } | Test { .. } | Clean
    pub scheme: &'a str,
    pub configuration: &'a str,            // Debug / Release
    pub destination: Option<&'a str>,      // raw -destination, None for SPM
    // product identity (from -showBuildSettings; free for xcode containers)
    pub product_name: Option<&'a str>,
    pub bundle_id: Option<&'a str>,
    pub deployment_target: Option<&'a str>,
    // signing
    pub development_team: Option<&'a str>,
    // pass-through
    pub setting_overrides: &'a [(String, String)],  // archs, debug symbols, hot-reload
    pub extra_args: &'a [String],
    pub env: &'a [(String, String)],
    // source graph — Some(..) ONLY when the chosen backend can't read .xcodeproj
    pub sources: Option<SourceGraph<'a>>,
}

/// What a backend can and can't do — drives selection and whether `prepare()`
/// must generate config. No closed enum the core matches on.
pub struct Capabilities {
    pub reads_xcode_project: bool,   // false ⇒ must generate config
    pub accepts_config_as_args: bool,// false ⇒ must generate config
    pub supports_simulator: bool,
    pub supports_device: bool,
    pub runs_off_mac: bool,          // true for xtool (Linux/Windows)
}

pub trait BuildBackend {
    fn id(&self) -> &'static str;                 // "xcodebuild" | "swiftpm" | "xtool"
    fn capabilities(&self) -> Capabilities;
    /// Is this backend usable on this host? (reported by `doctor`.)
    fn detect(&self) -> Availability;
    /// Turn the IR into something runnable. A config-gen backend writes its
    /// files to `ctx.scratch_dir(..)` HERE — the core never knows.
    fn prepare(&self, plan: &BuildPlan, ctx: &BackendContext) -> Result<PreparedBuild, CliError>;
}

pub struct PreparedBuild {
    pub program: String,                 // "xcodebuild" | "swift" | "xtool"
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
    /// The backend OWNS its log parsing — a closure/strategy, not an enum the
    /// core switches on. A new backend with a new log format ⇒ zero core changes.
    pub log: LogStyle,                   // Beautify(parser) | Raw | Quiet
    /// How to find the built product afterwards (differs per backend).
    pub resolve_product: Box<dyn Fn() -> Result<AppBundle, CliError>>,
}
```

### Backends

| Backend | `reads_xcode_project` | `accepts_config_as_args` | `prepare()` does |
|---------|:---:|:---:|---|
| `xcodebuild` | ✅ | ✅ | Today's `xcodebuild::BuildPlan` argv. No files written. |
| `swiftpm` | SPM only | partial | Today's `swiftpm::build` invocation. No manifest gen. |
| `xtool` | ❌ | ❌ | Generate `xtool.yml` (+ `Package.swift` if absent) into a scratch dir; run `xtool dev build` there. |
| `bazel` | ❌ | ❌ | Generate `MODULE.bazel` + `BUILD.bazel` (rules_apple/rules_swift) into a scratch dir; run `bazel build //App:App` there. |
| `buck` | ❌ | ❌ | Generate `BUCK` files (future). |

> tuist / xcodegen are the **reverse** direction (manifest → `.xcodeproj`), so
> they are *project generators*, not backends here.

### Bazel (`rules_apple`) backend

Same *shape* as xtool — a **config-generating** backend that materializes its
input from the Build Plan — but a different *value proposition*. xtool buys
**Linux/Windows** portability; Bazel buys **hermeticity, remote caching, and
scale** on a (still macOS-bound) Apple toolchain. So it's positioned as an
alternative to xcodebuild on Mac, **not** a Linux story.

- **What `prepare()` generates** (into `ctx.scratch_dir`):
  - `MODULE.bazel` (bzlmod) pulling in `rules_apple` + `rules_swift` +
    `apple_support`, pinned to a known-good version.
  - One or more `BUILD.bazel` files declaring `swift_library` targets for the
    sources and an `ios_application` / `macos_application` top-level target —
    `bundle_id`, `minimum_os_version`, `families`, `infoplists`, `provisioning_profile`
    all mapped from the Build Plan.
  - A `.bazelrc` for the destination/arch (`--ios_multi_cpus`,
    `--ios_simulator_device`, `--apple_platform_type`).
- **Hard dependency on the source graph.** Like xtool, Bazel doesn't read
  `.pbxproj`, so this backend needs `BuildPlan.sources` populated — Bazel is
  actually *stricter*: every target's `srcs`, `deps`, `data` (resources), and
  inter-target dependency edges must be enumerated explicitly. That makes it the
  most demanding consumer of "the hard part" below and the best forcing function
  for getting the source-graph extraction right.
- **`run()` / `resolve_product()`.** `bazel build //App:App` writes the bundle
  under `bazel-bin/…/App.ipa` (or `.app`); `resolve_product()` locates it via
  `bazel cquery --output=files` rather than a fixed path (sandbox/symlink-safe).
  `bazel run //App:App` can launch in the simulator via rules_apple's runner.
- **Capabilities:** `reads_xcode_project: false`, `accepts_config_as_args: false`,
  `runs_off_mac: false` (needs an Apple/Xcode toolchain; hermetic but Mac-bound),
  `supports_simulator: true`, `supports_device: true`.
- **Diagnostics:** Bazel wraps clang/swiftc output with its own action banners;
  `PreparedBuild.log` carries a Bazel-aware parser — no core change, per the
  no-closed-enums rule.

This is why the contract pays off: xtool and Bazel are wildly different tools
(YAML vs Starlark, Linux vs hermetic-Mac, `dev build` vs `bazel build`), yet both
fit the *identical* `BuildBackend` trait as config-gen backends — the core and
the VS Code extension don't grow a single branch for either.

## Pluggability contract (the actual goal)

**Adding a backend requires no changes to the core, and none to the VS Code
extension.** You add one Rust module implementing `BuildBackend`, register it,
and `--backend <id>` works. Two rules keep this true:

1. **No core branching on backend identity.** `build start` becomes:

   ```rust
   // build.rs — after this refactor
   fn start(ctx: &mut Context, clean: bool) -> CliResult {
       let plan    = resolve::build_plan(ctx, Action::Build { clean })?; // frontend
       let backend = backend::select(ctx, &plan)?;   // --backend / config / auto
       let prepared = backend.prepare(&plan, &ctx.backend_ctx())?;
       prepared.run(&ctx.out)
   }
   ```

   No `match` on the tool anywhere in the core. Backends self-register:

   ```rust
   // backend/mod.rs — core
   pub fn registry() -> &'static [&'static dyn BuildBackend] {
       &[&Xcodebuild, &SwiftPm, &Xtool, &Bazel]   // add a line; that's the only edit
   }
   pub fn select(ctx: &Context, plan: &BuildPlan) -> Result<&'static dyn BuildBackend, CliError> {
       // explicit --backend wins; else first registered backend that is
       // `detect()`-available and whose capabilities fit the container + host.
   }
   ```

2. **No closed enums on the contract.** Anything that varies per tool is carried
   *by the backend*: the log parser is a value on `PreparedBuild` (not a
   `mode: &str` the core switches on), supported destinations come from
   `Capabilities`, and the product location comes from `resolve_product()`.

**Temp config is a backend-internal fallback, not a core concept.** A backend
that can pass everything as args (xcodebuild) writes nothing. A backend that
can't (xtool) writes config to a scratch dir *inside its own `prepare()`*, reusing
the pure-generation style of `scaffold.rs`. The framework only offers
`ctx.scratch_dir(key)` so backends don't reinvent temp handling; whether to use
it is the backend's choice. The core never learns a file was written.

```rust
// XtoolBackend::prepare — config-gen fully self-contained, unit-testable (no Mac)
fn prepare(&self, plan: &BuildPlan, ctx: &BackendContext) -> Result<PreparedBuild, CliError> {
    let dir = ctx.scratch_dir(plan.container.key())?;     // optional helper
    write(dir.join("xtool.yml"), render_xtool_yml(plan))?; // pure fn -> String (serde_yaml)
    if !has_manifest(plan) {
        write(dir.join("Package.swift"), render_package(plan)?)?;
    }
    Ok(PreparedBuild {
        program: "xtool".into(),
        args: vec!["dev".into(), "build".into()],
        cwd: Some(dir.clone()),
        env: plan.env.to_vec(),
        log: LogStyle::Beautify(swiftpm_parser()),
        resolve_product: Box::new(move || read_xtool_product(&dir)),
    })
}
```

## The hard part: reconstructing the source graph

Most IR fields (bundle id, product name, deployment target, signing,
build-setting overrides) come for free — the crate already resolves the full
build-settings map in-process (`build_settings.rs` / `resolver.rs`). The
**expensive** fields are `sources` / `resources` / `dependencies`: build settings
don't report file membership, so a config-generating backend must read the
`.pbxproj` (the crate already parses it: `pbxproj.rs`, `project.rs`) to
reconstruct an equivalent package. Keep IR generation **lazy** — only populate
`BuildPlan.sources` when the selected backend's capabilities require it.

Per the crate's dependency policy (CLAUDE.md / DOCS.md §3): hand-roll Apple's
project-domain formats, but `xtool.yml` is plain YAML — a *standard* format — so
render it with `serde_yaml`, not a hand-rolled writer.

## VS Code extension impact

Effectively none in the build pipeline: the extension already invokes the
`sweetpad` CLI. Gaining the xtool backend means passing `--backend xtool` (and
surfacing backend availability from `sweetpad doctor`). The host-platform gate in
the extension (`src/build/commands.ts:377`) becomes "is a usable backend
available?" answered by the CLI, rather than a hard `darwin` check.

## Phased plan

1. **Lift the trait, no behavior change.** Introduce `BuildBackend` +
   `Xcodebuild`/`SwiftPm` wrapping today's `xcodebuild.rs` / `swiftpm.rs`; replace
   the `match` in `build.rs:25` with `backend::select` defaulting to current
   behavior. Existing `cargo test` arg-vector tests carry over unchanged.
2. **Promote `BuildTarget` → `BuildPlan`** with the extra identity/signing fields
   (still only consumed by xcodebuild at first).
3. **Add `XtoolBackend`** (config-gen mode), build-only on Linux first — see the
   [xtool doc](./xtool-linux-build.md). Pure `render_xtool_yml` / `render_package`
   with round-trip unit tests, mirroring `scaffold.rs`. This is also where the
   lazy **source-graph extraction** from `.pbxproj` lands (the first config-gen
   backend forces it).
4. **Add `BazelBackend`** (config-gen mode) reusing that source-graph extraction:
   pure `render_module_bazel` / `render_build_bazel` emitting Starlark, with
   round-trip-ish unit tests (generate → assert structure). Validates that the
   trait holds for a second, very different generated-config tool.
5. **Wire `detect()` into `doctor`** and `--backend` into the extension.
6. **Generalize run/install** (`app.rs`) to consume `resolve_product()`.

## Open questions

- Backend selection scope: global flag, per-project config, or per-scheme?
- How faithfully must a generated package reproduce the xcodeproj (build phases,
  run scripts, entitlements) before a build counts as "equivalent"?
- Scratch-dir location & lifecycle: under derived data, ignored, with an opt-in
  dump for inspection; regenerate only when the project fingerprint changes.
