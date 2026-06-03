# sweetpad-lib — Phase 1: Test Corpus Collection

## Goal

Build a high-quality test corpus of resolved Xcode build settings + build plans + raw inputs, using existing open-source projects and `xcodebuild`/Apple tools as the oracle. The corpus will later serve as the snapshot contract our Rust resolver must match.

**No Rust code is written in phase 1.** Python (one-off scripts) and shell only. Each script is idempotent and committed for reproducibility, but does not need to be production-grade.

## Locked decisions (from planning conversation)

| Decision | Value |
|---|---|
| Corpus size | 5 projects |
| Build depth | Full builds (simulator, unsigned) |
| Xcode matrix | Current + 2 previous majors |
| Signing | `CODE_SIGNING_ALLOWED=NO`, simulator destinations only |
| Fixture storage | In-repo, size-aware (Git LFS only if needed) |
| Plan document | This file |

## Corpus

| Slug | Repo | Pin strategy | Setup steps |
|---|---|---|---|
| `ice-cubes` | `github.com/Dimillian/IceCubesApp` | Latest stable release tag at capture time | None — SwiftPM resolves on first build |
| `alamofire` | `github.com/Alamofire/Alamofire` | Latest stable release tag | None |
| `netnewswire` | `github.com/Ranchero-Software/NetNewsWire` | Latest commit on default branch at capture time | Run any documented prebuild step from its `README` (none expected) |
| `tuist-fixtures` | `github.com/tuist/tuist` (subset of `fixtures/`) | Latest Tuist release tag | Install Tuist, run `tuist install && tuist generate` per fixture |
| `kingfisher` | `github.com/onevcat/Kingfisher` | Latest stable release tag | None |

For `tuist-fixtures`, the executing agent selects **5–8 fixtures** from `fixtures/` that exercise distinct shapes: basic app, framework dependency, multi-platform, static vs dynamic linkage, resources/assets. Record which fixtures were selected in `corpus/manifest.json`.

Record per-project: upstream URL, exact commit SHA, capture timestamp, tool versions (Tuist, etc.) in `corpus/manifest.json`.

## Xcode matrix

| Slot | Constraint |
|---|---|
| `current` | Latest stable Xcode installed via `xcodes` at capture time |
| `prev-major` | Latest minor of the major before `current` |
| `prev-major-2` | Latest minor of two majors before `current` |

Use [`xcodes`](https://github.com/RobotsAndPencils/xcodes) to install. Switch active Xcode between captures via `sudo xcode-select -s /Applications/Xcode-<ver>.app`. Record exact `Xcode-select -p`, `xcodebuild -version` output in every per-capture `meta.json`.

Expect ~15–20 GB per Xcode install. Total disk impact ~50 GB before captures.

## What we capture per (project × Xcode-version)

### A. Metadata (no build required)

For each project + Xcode version:

- `xcodebuild -list -json -workspace|-project <...>` → save as `metadata/list.json`
- `xcodebuild -showsdks -json` → `metadata/showsdks.json`

For each scheme listed:

- `xcodebuild -showdestinations -scheme <S>` → `metadata/schemes/<S>/destinations.json`
- For each (configuration × simulator destination) combination:
  - `xcodebuild -showBuildSettings -json -scheme <S> -configuration <C> -destination <D>` → `metadata/schemes/<S>/build-settings/<C>__<D_slug>.json`
  - `xcodebuild -dry-run -scheme <S> -configuration <C> -destination <D> 2>&1` → `metadata/schemes/<S>/dry-run/<C>__<D_slug>.txt`

`<D_slug>` is `platform=...,name=...` flattened with `_` for filesystem safety.

### B. Raw inputs

Copy from `corpus/<slug>/` into `fixtures/<slug>/xcode-<ver>/raw/`:

- Every `*.xcodeproj/project.pbxproj`
- Every `*.xcworkspace/contents.xcworkspacedata`
- Every `*.xcscheme` (both `xcshareddata/xcschemes/` and `xcuserdata/`)
- Every `*.xcconfig` transitively referenced
- Every `Info.plist`, `*.entitlements`

Preserve relative paths within `raw/`.

### C. Build artifacts (one build per scheme × config × simulator destination)

Per project, the executing agent should:

1. Pick one or two **representative** simulator destinations (e.g., `platform=iOS Simulator,name=iPhone 15` for iOS; `platform=macOS` for Mac targets). Do not exhaustively build every destination — that explodes effort.
2. For each (scheme × config × chosen destination):
   - Clean: `xcodebuild clean -scheme <S> -configuration <C>`
   - Build with PATH-wrapped toolchain (see Capture techniques):
     `xcodebuild build -scheme <S> -configuration <C> -destination <D> -resultBundlePath <fixtures>/build/<...>/result.xcresult CODE_SIGNING_ALLOWED=NO`
3. After build:
   - Locate `.xcactivitylog` in `<DerivedData>/<...>/Logs/Build/`, gzip-copy to `build/<...>/xcactivitylog.gz`
   - Parse with [`XCLogParser`](https://github.com/MobileNativeFoundation/XCLogParser) (install once via `brew install xclogparser`), output JSON to `build/<...>/xcactivitylog.parsed.json`
   - Extract `result.xcresult` with `xcrun xcresulttool get --format json --legacy --path result.xcresult > build/<...>/xcresult.json`
   - Snapshot the index store: tar+gzip `<DerivedData>/<...>/Index.noindex/DataStore/v5/` → `build/<...>/index-store.tgz`
   - Save the tool-invocations JSONL produced by the PATH shim → `build/<...>/tool-invocations.jsonl`
   - Save stdout, stderr, exit code → `build/<...>/{stdout.txt,stderr.txt,exit_code}`

### D. Apple-side spec data (per Xcode version, once)

Copy to `xcspec-cache/xcode-<ver>/`:

- All `*.xcspec` files under `/Applications/Xcode-<ver>.app/Contents/` (find recursively; there are hundreds, mostly small)
- All `SDKSettings.plist` files under `Contents/Developer/Platforms/*/Developer/SDKs/*.sdk/`
- The output of `xcrun --show-sdk-path --sdk <each available sdk>`

## Capture techniques

### PATH-wrapped toolchain shim

Goal: capture the exact argv every Apple tool was called with during the build.

- Create `scripts/toolshim/` directory with executable scripts named: `swiftc`, `clang`, `clang++`, `ld`, `ld64.ld`, `actool`, `ibtool`, `momc`, `mapc`, `lipo`, `codesign`, `dsymutil`, `strip`, `bitcode_strip`, `plutil`.
- Each script is identical logic:
  1. Read `$SWEETPAD_SHIM_LOG` (path to a JSONL file).
  2. Append `{"tool": "<name>", "argv": [...], "cwd": "...", "env": {... allowlisted vars ...}, "timestamp": ...}` to that file (atomic-append; lock if needed).
  3. Resolve the real tool by looking ahead in `PATH` beyond `scripts/toolshim/` (or via `xcrun --find <name>` with the shim dir removed from `PATH`).
  4. `exec` the real tool with the same argv.
- Before invoking `xcodebuild`, the build script prepends `scripts/toolshim/` to `PATH` and sets `SWEETPAD_SHIM_LOG=<fixture_path>/tool-invocations.jsonl`.

Env allowlist for the JSONL (to keep size sane and avoid leaking host secrets): `SDKROOT`, `BUILT_PRODUCTS_DIR`, `CONFIGURATION`, `CONFIGURATION_BUILD_DIR`, `DERIVED_FILE_DIR`, `PROJECT_DIR`, `PROJECT_NAME`, `SRCROOT`, `TARGET_NAME`, `EFFECTIVE_PLATFORM_NAME`, `ARCHS`, `CURRENT_ARCH`, `SWIFT_VERSION`, plus any `*_SEARCH_PATHS`, `OTHER_*`, `GCC_*`, `SWIFT_*`, `WARNING_*`, `CLANG_*`, `LD_*` keys. Drop everything else.

### Xcode version switching

A helper in `common.py` exposes `with_xcode(version)` context manager:
- Saves current `xcode-select -p`.
- `sudo xcode-select -s /Applications/Xcode-<ver>.app/Contents/Developer`.
- Restores on exit.

Agent must run `sudo` once at script start (cache credentials) and inform user this is required.

## Python orchestration

Layout of `scripts/`:

```
scripts/
  common.py                # shared: paths, xcode switching, fixture path helpers, dest slugging
  00_bootstrap.py          # install xcodes + the 3 Xcodes; verify CLT; install Tuist, XCLogParser
  01_clone_corpus.py       # clone all corpus repos at pinned refs; per-project setup; write manifest.json
  02_capture_metadata.py   # capture A (metadata) and B (raw inputs) for every (project, xcode)
  03_run_builds.py         # capture C (build artifacts) for every (project, xcode, chosen scheme/config/dest)
  04_snapshot_xcspecs.py   # capture D (Apple xcspec/SDKSettings) per Xcode version
  05_validate.py           # walk fixtures/, write fixtures/REPORT.md coverage matrix
  toolshim/
    swiftc
    clang
    ...
```

Each numbered script:
- Takes `--force` to redo already-captured outputs.
- Takes `--project <slug>` and `--xcode <ver>` to scope.
- Skips work whose output already exists unless `--force`.
- Writes a `errors/<step>.txt` (non-empty) inside the relevant fixture directory if it fails — does not abort the whole run.

## Output layout

```
sweetpad-lib/
  PLAN.md
  README.md                                # one-paragraph pointer to PLAN.md
  .gitignore                               # corpus/* (clones), DerivedData/, .venv/, __pycache__/
  corpus/
    manifest.json                          # { slug: {repo, sha, captured_at, extras...} }
    ice-cubes/                             # git clone (gitignored)
    alamofire/
    netnewswire/
    tuist-fixtures/
    kingfisher/
  fixtures/
    REPORT.md                              # generated by 05_validate.py
    ice-cubes/
      xcode-16.2/
        meta.json                          # { xcode_version, host_macos, captured_at, source_sha, status }
        metadata/
          list.json
          showsdks.json
          schemes/
            <scheme>/
              destinations.json
              build-settings/
                <config>__<dest_slug>.json
              dry-run/
                <config>__<dest_slug>.txt
        raw/
          <relative paths preserved>
        build/
          <scheme>__<config>__<dest_slug>/
            xcactivitylog.gz
            xcactivitylog.parsed.json
            xcresult.json
            index-store.tgz
            tool-invocations.jsonl
            stdout.txt
            stderr.txt
            exit_code
        errors/                            # may be empty
      xcode-15.x/
      xcode-14.x/
    alamofire/
    netnewswire/
    tuist-fixtures/
    kingfisher/
  xcspec-cache/
    xcode-16.2/
      <copied xcspec tree>
      sdksettings/
    xcode-15.x/
    xcode-14.x/
  scripts/
    common.py
    00_bootstrap.py
    01_clone_corpus.py
    02_capture_metadata.py
    03_run_builds.py
    04_snapshot_xcspecs.py
    05_validate.py
    toolshim/
      swiftc
      clang
      ...
```

## Step-by-step execution order

The executing agent runs these in order. Each step has a validation gate.

1. **Init repo.** `git init`, write `.gitignore` and `README.md` (one-paragraph stub). _Validation:_ `git status` clean except new files.
2. **Bootstrap.** Write and run `00_bootstrap.py`. Install `xcodes`, `tuist`, `xclogparser` (via Homebrew). Install the 3 Xcode versions. _Validation:_ `xcodes installed` lists all three, each has a runnable `xcodebuild -version`.
3. **Clone corpus.** Write and run `01_clone_corpus.py`. Per-project setup (Tuist generate where needed). _Validation:_ `corpus/<slug>/` exists for each; `corpus/manifest.json` has 5 entries with SHAs; for `tuist-fixtures`, generated `.xcodeproj` files exist in the chosen fixtures.
4. **Snapshot xcspecs.** Write and run `04_snapshot_xcspecs.py` for each Xcode version. _Validation:_ `xcspec-cache/xcode-<ver>/` has >100 `.xcspec` files and the `sdksettings/` subdirectory.
5. **Capture metadata + raw.** Write and run `02_capture_metadata.py`. _Validation:_ for each (project, Xcode) the `metadata/list.json` exists and parses; `metadata/schemes/<S>/build-settings/` non-empty for every scheme.
6. **Run builds + collect artifacts.** Write and run `03_run_builds.py`. _Validation:_ per project, in the current-Xcode slot, at least one `build/<...>/` directory has `xcactivitylog.parsed.json`, `xcresult.json`, `tool-invocations.jsonl` all non-empty, `exit_code == 0`. Older Xcode versions may legitimately fail to build newer projects; document failures in `errors/`.
7. **Validate + report.** Run `05_validate.py --report`. Writes `fixtures/REPORT.md` with a coverage matrix (rows: projects, columns: Xcode versions, cells: capture completeness %).
8. **Size check + LFS decision.** `du -sh fixtures/ xcspec-cache/`. If any individual file > 50 MB or total > 1.5 GB, initialize Git LFS for those patterns; otherwise commit normally.
9. **Commit.** Show the proposed commit message to the user for approval, then commit.

## Validation criteria (definition of done)

- [ ] All 5 projects cloned at pinned SHAs; `corpus/manifest.json` complete.
- [ ] All 3 Xcode versions installed; each has its own `xcspec-cache/xcode-<ver>/` snapshot.
- [ ] Per project: metadata captured for **every** scheme × config × at least one simulator destination, in **at least the current Xcode**. Older Xcodes are best-effort.
- [ ] Per project: build artifacts captured for at least one (scheme, config, destination) in the current Xcode.
- [ ] `fixtures/REPORT.md` exists, lists the coverage matrix, and explains every blocked tuple.
- [ ] Repo size on disk < 2 GB without LFS, or LFS set up if larger.
- [ ] All scripts re-runnable: a second pass produces no diffs.

## Out of scope for phase 1

- Any Rust code.
- Resolver / CLI design.
- Diffing or analyzing fixtures across Xcode versions (we just collect; analyze later).
- CocoaPods, Carthage projects (deferred to phase 2 corpus expansion).
- Archive / Release-signed builds.
- Multi-host (CI) capture; everything runs on the dev machine.
- Public API or schema design.

## Phase 2: Rust scaffold + first isolated parser (pbxproj)

Status: ready to start. The corpus produced in phase 1 is the test oracle for phase 2.

### Goal

Stand up one Rust crate inside this repo and build the first isolated unit — a `project.pbxproj` parser — driven by fixtures already under `fixtures/<slug>/xcode-<ver>/raw/`. No resolver, no CLI subcommands, no Apple xcspec ingest yet. The parser stands alone, returns a typed AST, and round-trips a representative subset of the corpus in tests.

### Locked decisions

| Decision | Value |
|---|---|
| Shape | Single Cargo package (no workspace) |
| Location | Repo root: `Cargo.toml` next to `PLAN.md` |
| Package name | `sweetpad` |
| Outputs | Library (`src/lib.rs`) + binary `sweetpad` (`src/main.rs`) in one crate |
| Rust edition | `2024` |
| Toolchain | Pinned via `rust-toolchain.toml` to `1.94.0` |
| License | `MIT OR Apache-2.0` (both `LICENSE-MIT` and `LICENSE-APACHE`) |
| Initial dependencies | None — add via `cargo add` only when concrete code needs them |
| Lints | `[lints]` in `Cargo.toml` enables `clippy::pedantic = "warn"` |
| `Cargo.lock` | Committed (crate ships a binary) |
| CI | None for now |

### Development style (applies to all Rust work in this repo)

- **Build in isolated parts.** First isolated unit: pbxproj parsing. Get it working end-to-end against real fixtures before introducing the resolver, xcconfig parser, scheme parser, or CLI subcommands.
- **Tests driven by fixtures.** Integration tests read inputs directly from `fixtures/<slug>/xcode-<ver>/...`. No fabricated test data unless required to isolate a bug.
- **Minimum abstraction.** Concrete types and plain functions. No trait / generic / lifetime layer until concrete duplication forces it. Three repeated lines beat a premature `trait`.
- **Readability beats performance.** `.clone()` and `String` are fine; optimize only when a benchmark says so.
- **Sparse comments.** Code self-documents via names. Comments only for non-obvious *why* — a workaround for an `xcodebuild` quirk, a surprising invariant in the corpus data.

### Output layout (additions to the repo)

```
sweetpad-lib/
  Cargo.toml              # package metadata, [lints], no [dependencies] yet
  Cargo.lock              # committed
  rust-toolchain.toml     # channel = "1.94.0"
  rustfmt.toml            # edition + max_width + unix newlines
  LICENSE-MIT
  LICENSE-APACHE
  src/
    lib.rs                # near-empty; modules added one at a time as work begins
    main.rs               # minimal fn main(); subcommands added later
  tests/                  # created when the first integration test lands
  target/                 # gitignored
```

`.gitignore` gains a single new entry: `/target`.

### Step-by-step execution order

1. **Scaffold.** Write `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `LICENSE-MIT`, `LICENSE-APACHE`, near-empty `src/lib.rs`, minimal `src/main.rs`. Add `/target` to `.gitignore`. _Validation:_ `cargo build`, `cargo test`, `cargo fmt --check`, `cargo clippy` all clean against the empty crate.
2. **First isolated unit: pbxproj parser.** Add a parser module under `src/`. Parse the OpenStep plist format that `project.pbxproj` uses, returning a typed AST. Drive development with integration tests pointing at `fixtures/*/xcode-*/raw/**/project.pbxproj`. Add dependencies via `cargo add` only when needed; prefer hand-rolled simple code initially. _Validation:_ at least one `project.pbxproj` from each of the 5 corpus projects parses without errors and produces a non-empty AST.
3. **Next isolated units** (deferred — decide in a follow-up planning round before starting): xcconfig parser, scheme parser, build-settings resolver, CLI subcommand surface. Do not pre-stub any of these modules now.

### Validation criteria (phase 2 done = first increment complete)

- [ ] `cargo build && cargo test && cargo fmt --check && cargo clippy` all clean.
- [ ] pbxproj parser handles at least one `project.pbxproj` from each of the 5 corpus projects under the current-Xcode slot.
- [ ] Tests live in `tests/` and read inputs directly from `fixtures/`.
- [ ] No abstractions in the codebase without at least one concrete caller justifying them.

### Out of scope for phase 2

- xcsettings resolver (precedence rules, xcconfig inheritance, default fill-in).
- xcconfig and scheme parsers.
- CLI subcommand surface beyond a placeholder `main`.
- Apple xcspec / SDKSettings ingest.
- Index store as a second oracle source.
- crates.io publishing.
- CI.

## Phase 3+ preview (do not start)

- xcsettings resolver: precedence rules, xcconfig inheritance, default fill-in from xcspec/SDKSettings. Snapshot tests against `fixtures/<slug>/xcode-<ver>/metadata/schemes/*/build-settings/*.json` — must match byte-equivalent (or with documented expected diffs).
- CLI subcommand surface (`sweetpad <subcommand>`): expose parser + resolver via subcommands roughly equivalent to `xcodebuild -list`, `xcodebuild -showBuildSettings`, etc.
- Apple xcspec / SDKSettings ingest → spec-driven resolution.
- Index store ingest as a second oracle source.
- Expand corpus with a CocoaPods project, an app with extensions (Bitwarden iOS or similar), and one large project (WordPress or Firefox iOS) once phase 2 methodology is proven.

## Settings resolution — scope & known residuals (updated 2026-05-29)

Phase 3 is underway: the resolver exists and is validated by
`tests/corpus_oracle.rs` against every captured `-showBuildSettings`. This
section is the authoritative scope record — read it instead of re-deriving the
same in/out-of-scope and "why doesn't this match" questions each session.

**How to read the metric.** The oracle reports three tiers: exact (byte-equal),
canonical (after stripping `$HOME` / DerivedData-hash / Xcode-build / SDK-version
/ project-root drift), structural (both sides absolute paths). The current
baseline is ~88% exact / ~97% canonical / ~99.96% structural. The exact% is
**capped by test geometry** — we resolve against `fixtures/<slug>/.../raw/` while
the oracle was captured at the original checkout, so `PROJECT_DIR` / `SRCROOT` /
`BUILD_DIR` / absolute search-paths can never byte-match. Judge resolver
correctness by the **structural %** and the per-key "systematic mismatches"
tally (keys that fail even structural — the genuine value bugs), not by exact%.

**In scope** — anything derivable from project inputs (pbxproj/xcconfig) plus the
Apple xcspec/SDKSettings defaults. This now **includes signing settings that are
pass-through or per-SDK/per-platform defaults**: `DEVELOPMENT_TEAM` (resolved via
self-reference inheritance — `KEY = $(KEY)` inherits the lower layer), the literal
`CODE_SIGN_IDENTITY` per-SDK default (`-` on simulators, `Apple Development` on
macOS), `CODE_SIGN_STYLE`, the `ENABLE_HARDENED_RUNTIME` per-platform default, and
the `maccatalyst.` `PRODUCT_BUNDLE_IDENTIFIER` prefix.

**Out of scope — the "real signing" (environment-derived, not in project inputs):**
`EXPANDED_CODE_SIGN_IDENTITY` and the expanded identity string
(`Apple Development: Name (TEAMID)`), `PROVISIONING_PROFILE_SPECIFIER` and the
resolved profile UUID, and anything that requires the Mac keychain,
`~/Library/MobileDevice/Provisioning Profiles/`, or the Xcode account. These are
not reproducible from project files; matching them would require a
caller-provided "signing environment" input. (Also still out, per phase 1:
archive / release-signed builds, CocoaPods, Carthage.)

**Known irreducible residuals — documented in code, do NOT keep re-investigating:**
- `ENABLE_DEBUG_DYLIB` for `application` in Release, and the coupled
  `DEBUG_INFORMATION_FORMAT = dwarf-with-dsym` in Debug for those same targets.
  This is an xcodebuild internal heuristic not a function of any observable input
  (proven: not objectVersion, deployment target, `ONLY_ACTIVE_ARCH`, Swift-package
  dependencies, or any declared setting — NetNewsWire=YES vs ice-cubes=NO with
  identical inputs). We emit the majority-correct default and accept the residual.

**Known data gaps — need corpus expansion, not resolver work:**
- `CLANG_COVERAGE_MAPPING` for the Alamofire visionOS scheme: its `TestAction`
  uses a `.xctestplan` not captured under `fixtures/.../raw/`.
- `SWIFT_INCLUDE_PATHS` and similar tuist values anchored at the tuist DerivedData
  build directory.
- `IPHONEOS_DEPLOYMENT_TARGET` 13.0-vs-13.1 minor drift — a capture-time artifact.

## Notes for the executing agent

- Do not assume Homebrew location. Resolve via `command -v brew` or fail clearly.
- Some `xcodebuild` invocations on macOS targets need `-destination "platform=macOS"` and not a simulator destination; detect target platform from the scheme's supported destinations.
- Index store paths under DerivedData are deterministic but Xcode-version-dependent; resolve dynamically per build, do not hard-code.
- `xcrun xcresulttool get` semantics changed in Xcode 16 — the script must detect and use `--legacy` only when supported. Test on each Xcode version during bootstrap.
- Do not delete DerivedData between builds of the same project in the same Xcode version unless explicitly cleaning — incremental state can be useful and reproducible.
- If a build fails for reasons outside our control (missing API key, expired team, etc.), write the failure cause to `errors/build.txt` and continue. Do not modify the source project to make it build.

## Multi-version capture — plan (added 2026-05-30)

Goal: capture a **second Xcode major (16.x)** alongside the existing 26.0.1
corpus, so the resolver is validated against more than one version and
version-conditional defaults surface instead of overfitting 26.0.1. The pipeline
is parameterized by the **full** version string (`xcode-16.4.0`), so adding a
minor/patch later is just another iteration of the same loop — not a redesign.
(Per-major churn is large; minor churn is small and dominated by the bundled-SDK
bump, most of which the canonicalizer already strips — so a different *major* is
the high-value capture and 16.x is the whole near-term target.)

**What already exists (reuse, do not rebuild).** The fixture layout
(`fixtures/<slug>/xcode-<ver>/{raw,metadata,build,errors}`), the per-version
`xcspec-cache/xcode-<ver>/`, `with_xcode()` (switch + auto-restore active Xcode),
`discover_installed_xcodes()` (slots installs current/prev-major), the
~50 GB/Xcode disk budget in `00_bootstrap.py`, dynamic destination selection
(`02_capture_metadata.py` reads `-showdestinations` and picks representative
destinations, so it adapts to 16.x's iOS 18.x simulators on its own), and the
per-`(slug,ver)` `errors/` + continue-on-failure rule. The gap is only: (1) a
bounded acquire→capture→teardown loop, (2) simulator-runtime provisioning, and
(3) untying the oracle tests from the hardcoded 26.0.1 catalog.

**Decisions (locked).**
- Target **16.x** as the second major; parameterize by full version string.
- **Acquire:** one interactive `xcodes` sign-in up front (a single 2FA code);
  `xcodes` caches the session, so the orchestrator then drives `xcodes install`
  unattended. `sudo -v` (with keepalive) up front so the `xcode-select` switches
  don't re-prompt. A `--check-auth` preflight surfaces both.
- **Scope:** smoke builds end-to-end, so the matching **iOS 18.x simulator
  runtime** (~7–10 GB) is provisioned for 16.x.
- **Disk:** strict **one-at-a-time** — never hold two Xcodes; reclaim eagerly.
- **Smoke subset:** `alamofire` (fast framework), `kingfisher` (framework +
  SwiftPM), `ice-cubes` (app + SPM). `tuist-fixtures` is **best-effort** (needs
  `tuist install && tuist generate` against a 16.x-compatible tuist; on failure
  write an error and skip, do not block). netnewswire is excluded from the smoke
  tier (default-branch, large, slow). Overridable via `--subset`.
- **Committed fixtures:** commit the smoke-subset 16.x fixtures **including
  `raw/`** — the resolver tests resolve directly against `raw/` (pbxproj /
  xcconfig), and reproducibility outweighs the modest tracked-size bump. Revisit
  git-lfs only if total fixture size becomes a real problem.

**New orchestrator — `scripts/13_capture_version.py`.** Runs the existing
numbered steps for one version inside an acquire/teardown envelope:

```
for ver in args.versions:
    if fixtures_complete(ver) and not force: continue   # resumable / idempotent
    preflight_disk(need ≈ 70GB)                         # 50 Xcode + 10 runtime + headroom
    xip = acquire(ver); delete(xip)                     # reclaim ~10GB immediately
    with with_xcode(install(ver)):
        provision_runtime(ver)                          # iOS 18.x sim runtime
        snapshot_xcspecs(ver)                           # step 04 → xcspec-cache/xcode-<ver>/
        for slug in SMOKE_SUBSET:
            capture_metadata(slug, ver)                 # step 02 (no-dest + per-target + project-defaults + scheme)
            run_builds(slug, ver)                       # step 03 (smoke build, toolshim per-file settings)
            purge_derived_data(slug)                    # reclaim between projects
        run_settings_steps(ver)                         # steps 07–12 (cheap, no builds)
    cargo_test(version=ver)                             # version-aware oracle
    teardown(install(ver), runtime(ver))               # delete Xcode.app + runtime → back to baseline
```

CLI: `--versions 16.4.0`, `--subset …`, `--keep` (skip teardown — use on the
first wet run to inspect before trusting it), `--force`, `--no-runtime`
(settings-only escape hatch), `--dry-run`, `--check-auth`.

**Disk-bounding (the teardown order is the point).** `.xip` deleted immediately
after install; DerivedData purged after each project; `corpus/<slug>/` clones are
**shared across versions** (clone once via step 01, keep ~1.2 GB, never re-clone);
after capture **and** a green validation, delete `Xcode-<ver>.app` and (unless
`--keep`) the runtime. Kept permanently: `fixtures/<slug>/xcode-<ver>/` +
`xcspec-cache/xcode-<ver>/`. Peak footprint ≈ one Xcode + one runtime + fixtures
+ corpus — flat across N versions instead of N×50 GB. Preflight aborts if free
space < budget so a run never wedges the disk mid-way.

**Version-aware validation (prerequisite refactor, do this first).** Today
`tests/common/mod.rs` hardcodes `xcspec_root()` / `sdksettings_root()` to
`xcspec-cache/xcode-26.0.1`, while `find_capture_files` already globs
`fixtures/*/xcode-*/`. So 16.x captures would be discovered but scored against the
**wrong** catalog. Fix: derive `<ver>` from each capture's path (the
`/xcode-<ver>/` segment the canonicalizer already parses) and load the matching
`xcspec-cache/xcode-<ver>/` catalog, **memoized per version**. The canonicalizer
already strips SDK-version / toolchain-build / Xcode-app-path drift, so a 16.x SDK
string is not a mismatch. Add a per-version line to the score summary and set
16.x coverage floors data-driven from its first clean run (same pattern as the
per-target floors). This refactor is verifiable against the current corpus alone
(must stay green) and should land before any 16.x capture.

**Work sequence.** (1) version-aware test refactor; (2) `13_capture_version.py`
with `--dry-run`; (3) runtime provisioning + `--check-auth`; (4) wet run on
16.4.0, smoke subset, `--keep` first; (5) set 16.x floors, commit fixtures +
update COVERAGE.md / this file.

### 16.4.0 capture — outcomes (2026-05-30)

Captured Xcode **16.4.0 (16F6)** for the smoke subset (alamofire, kingfisher;
ice-cubes blocked — its pinned release ships Swift-tools-6.2 package manifests
that Xcode 16.4's Swift 6.1 rejects) via a **sudo-free** run: `with_xcode` now
selects Xcode through `DEVELOPER_DIR` instead of `sudo xcode-select`, so the
whole capture (and the orchestrator) runs unattended. Scheme/synthetic capture
for 16.4 is limited to macOS because Xcode 16.4 won't offer iOS destinations
without its own iOS 18.5 platform (`xcodebuild -downloadPlatform iOS`, a
user-gated download) — per-target + project-defaults are complete and are the
cleanest resolver oracle.

The capture surfaced and we fixed these resolver/canonicalizer gaps (every 16.4
per-target & project-defaults mismatch is now resolved except the irreducible
`ENABLE_DEBUG_DYLIB`):

- **`XCODE_VERSION_{MAJOR,MINOR,ACTUAL}`** (keystone): the resolver didn't emit
  them, so nested project recipes like
  `$(SWIFT_STRICT_CONCURRENCY_XCODE_$(XCODE_VERSION_MAJOR))` mis-resolved. Now
  injected in `built_in_settings` from the **catalog** version (`Catalog`
  reads `xcode_version` from `meta.json`). Fixed `SWIFT_STRICT_CONCURRENCY`
  (16.4: 20→0) and added +450 exact matches on 26.0.1 — a fix only the second
  major exposed.
- **`PRODUCT_TYPE_SWIFT_STDLIB_TOOL_FLAGS`** (12→0): test-canonicalizer now
  collapses an Xcode-dev path embedded in a *quoted* flag token.
- **`TARGETED_DEVICE_FAMILY`** (2→0): iOS test bundles default to `1`, not `1,2`.
- **`SYSTEM_FRAMEWORK_SEARCH_PATHS`** (16.4: 4→0): Catalyst recipe appends the
  `SubFrameworks` segment only on Xcode ≥26 (16.4 SDKSettings is Frameworks-only).
- **`CODE_SIGN_IDENTITY`** (4→0): macOS test bundles with no team/identity → `-`.
- **`DEBUG_INFORMATION_FORMAT`** (6→0): non-macOS Debug test bundles force
  `dwarf-with-dsym`; iphoneos apps with **no destination** default to it too
  (destination-gated, so destination-bound builds are untouched).
- **`ENABLE_DEBUG_DYLIB`** (residual, left as-is): opaque per-project Release
  heuristic — any flip regresses more captures than it fixes. Documented.

Tooling added: per-version oracle diagnostic (`ORACLE_ONLY_VERSION=<ver>` isolates
one version's systematic-mismatch tally and skips the floors).

### Per-version floors + baseline-green (2026-05-30)

Bringing up `cargo test` against the now-two-version corpus exposed that **every
oracle's coverage floor was stale** — the committed `HEAD` itself failed them
(corpus 84% exact / 95% canonical vs the asserted 87% / 96%). The single
**blended** floor across all versions is the wrong design once there's more than
one major: tuist-fixtures is ~76% of all keys at ~84% exact (geometry-capped — we
resolve against `raw/`, the oracle was captured at the original checkout), so the
blend drifts as majors are added and masks per-version regressions. Fixes:

- **Per-version data-driven floors.** New shared helper
  `common::assert_version_floors(label, per_version, version_floor)` asserts each
  Xcode version against its own codified `(exact, canonical, structural)` floor
  (set from the first clean run minus a ~1pt margin) and prints the observed line
  every run. `structural` (the geometry-independent correctness signal, ~99%
  everywhere) is floored at 98 across the board; `exact`/`canonical` are
  per-version. A freshly captured major with no codified floor gets only the
  `structural ≥ 98` safety guard plus a printed `NO CODIFIED FLOOR` line — so
  adding a version's captures never hard-fails before its floor is calibrated.
  Applied to all five oracles (corpus, per-target, project-defaults,
  synthetic-override, real-xcconfig) and the `scratch` xcspec coverage test.
- **`DEBUG_INFORMATION_FORMAT` destination gating.** The non-macOS Debug
  test-bundle `dwarf-with-dsym` override (added with the 16.4 capture) was firing
  on destination-bound scheme captures too, where xcodebuild actually emits plain
  `dwarf` (it's a no-destination "default-target" behaviour, same split as
  `ENABLE_DEBUG_DYLIB`). Gated it on `destination.is_none()`: keeps the per-target
  / project-defaults wins, removes 10 tuist-fixtures over-fires on 26.0.1.

Net on 26.0.1 this session: `PRODUCT_TYPE_SWIFT_STDLIB_TOOL_FLAGS` 62→0,
`DEBUG_INFORMATION_FORMAT` back to the irreducible baseline, exact 84→85%. The
full suite is green; correctness is judged by structural% + the per-key
systematic-mismatch tally, not the geometry-capped exact%.

### 15.4.0 capture — third major + two undomained-xcspec parser fixes (2026-05-30)

Landed **Xcode 15.4 (15F31d)** as the third corpus major (Strategy #1), captured
sudo-free from the already-installed `/Applications/Xcode-15.4.0.app` via the
orchestrator (`--versions 15.4.0 --subset kingfisher,tuist-fixtures --no-runtime
--keep --min-disk-gb 5`). `kingfisher` (objectVersion 54) and the
`tuist-fixtures` generated projects (objectVersion 55) open in 15.4 with no
re-clone; `alamofire`/`netnewswire`/`ice-cubes` stay walled off (objectVersion
76/77, Swift-tools 6.2) and still need era-appropriate refs for a fuller 15.x.

15.4 came in at **structural 90%** (vs ~99% on 16+) — the safety guard caught it.
Triage found two genuine, **cross-cutting xcspec-parser robustness bugs** that
only surfaced because Xcode ≤15 leaves specs *undomained* where 16+ adds an
explicit `_Domain` (so 16/26 were accidentally correct and never exposed them):

- **`PACKAGE_TYPE` undomained clobber** (`src/xcspec.rs`): the base
  `com.apple.product-type.application` is declared in three 15.4 files, all
  `_Domain`-less; our last-wins `insert` let a PackageTypes-less watchOS shim
  overwrite the real "Darwin Product Types.xcspec" definition, leaving
  `PACKAGE_TYPE` empty and collapsing the whole bundle chain (`WRAPPER_NAME`,
  `CONTENTS_FOLDER_PATH`, `EXECUTABLE_FOLDER_PATH`, …). Fix: a definition carrying
  `PackageTypes` is authoritative — never overwrite a `Some` package_type with a
  `None`. Structural 90→96%.
- **`BUNDLE_FORMAT` macOS-domain pollution** (`src/xcspec.rs`): 15.4's "MacOSX
  Core Build System.xcspec" has no `_Domain`, so its macOS-only `BUNDLE_FORMAT =
  deep` (and the `Contents/` folder-path cascade) landed in `universal` and
  applied to iOS/tvOS/watchOS apps. Fix: infer the domain from the spec
  *filename* (`MacOSX …` → `macosx`) when `_Domain` is absent. Bundle-layout
  family fully resolved.

Both fixes are **verified no-ops on 16.4/26.0.1** (those versions are already
domained, so the new branches never fire — their per-version numbers are
byte-identical pre/post). Remaining 15.4 residuals are **irreducible host/version
arch reporting** the resolver can't derive from project inputs and that newer
Xcodes normalize away: `NATIVE_ARCH`/`HOST_ARCH = arm64e` (15.x labels Apple
Silicon literally), concrete `CURRENT_ARCH` on the no-destination path (16+ →
`undefined_arch`), and `VALID_ARCHS` ordering. Documented; 15.4 floors codified
at the achieved level (after the keystone fix below: per-target 84/91/95,
project-defaults 84/92/95, corpus 84/94/95 exact/canon/struct).

### Project-shape sweep (#2) — DEVELOPER_DIR keystone fix (2026-05-30)

Sweeping the per-target oracle (the workhorse that isolates the resolver, all
shapes) surfaced the single biggest systematic mismatch corpus-wide:
**`OTHER_LDFLAGS` ×300** on 26.0.1, which turned out NOT to be a tuist artifact
but a **keystone bug** in the same family as `XCODE_VERSION_MAJOR`. The values
differed only by `Xcode-16.4.0.app` vs `Xcode-26.0.1.app`: the resolver built
`DEVELOPER_DIR` (and everything derived — `DEVELOPER_*_DIR`, `TOOLCHAIN_DIR`,
`DT_TOOLCHAIN_DIR`, and the `-L$(DT_TOOLCHAIN_DIR)/usr/lib/swift/<platform>`
flag in `OTHER_LDFLAGS`) from the **host's `xcode-select`ed Xcode** (16.4)
instead of the **catalog's** (the Xcode each capture was taken with). It only
showed as a *structural* miss for `OTHER_LDFLAGS` because the path is embedded in
a flag (so the canonicalizer's path-token logic didn't absorb it); the
`DEVELOPER_*_DIR` family was silently a *canonical* miss the whole time.

Fix: `Catalog` now reads `developer_dir` from `meta.json` and `built_in_settings`
prefers it over `detect_developer_dir()` (CLI with no catalog still falls back to
the host). Impact — exact% jumped on **every** non-host version at once
(per-target 26.0.1 84→87, 15.4 81→85; same on corpus/project-defaults/synthetic/
xcconfig), `OTHER_LDFLAGS` ×300→0, and the `DEVELOPER_*_DIR` family now
byte-matches per version. 16.4 is byte-identical pre/post (host-active already
16.4 — confirms no regression). After this, 26.0.1 per-target's only systematic
mismatches are documented irreducibles (`ENABLE_DEBUG_DYLIB`), a known tuist data
gap (`SWIFT_INCLUDE_PATHS`), and a niche NetNewsWire iOS-test rpath
(`LD_RUNPATH_SEARCH_PATHS`, left as data-specific). All per-version floors
re-codified upward to the new baseline.

The diverse shapes the plan targeted are already captured and scored cleanly by
the per-target oracle (tuist: watchapp2, frameworks, SPM static/dynamic/auto,
coredata, multiplatform; kingfisher: watch/tvOS/macOS demos; netnewswire: mac app
+ extensions), so #2's value came from this cross-cutting resolver fix rather
than adding more fixtures.

### Cross-version delta tool (#3) (2026-05-30)

Built `scripts/14_compare_versions.py` — the `compare_versions(a, b)` core of the
delta design: it diffs two captured versions' per-target build settings,
canonicalizing the volatile Xcode-app / SDK-version path drift and dropping
version-echo keys, then splits the result into **behavioural** diffs
(value/flag/list changes, keys added/removed) and a collapsed **path-geometry**
bucket (different build-output root / capture methodology — not Xcode behaviour,
the same separation the oracle's structural tier makes). Standalone value: it
answers "what actually changes across an Xcode major" and surfaces
version-conditional behaviour worth modelling. E.g. **16.4 → 26.0.1** cleanly
shows `AVAILABLE_PLATFORMS` +`webassembly`, `*.icon` added to the excluded-search
list, bitcode keys (`ENABLE_BITCODE`, …) *removed*, new security defaults
(`ENABLE_C_BOUNDS_SAFETY`, `ENABLE_POINTER_AUTHENTICATION`, `RUNTIME_EXCEPTION_*`),
and `-needed_framework` XCTest linking; **15.4 → 16.4** isolates exactly the
irreducible arch-reporting family (`NATIVE_ARCH=arm64e`, concrete `CURRENT_ARCH`)
plus `EXPANDED_CODE_SIGN_IDENTITY` (15.x) vs `ENABLE_DEBUG_DYLIB` (16.x).

The `--delta` **auto-capture-and-commit** mode (stage a fresh per-target capture,
diff vs the last-kept version, commit only on change, write a `fixtures/
versions.json` ledger) is **intentionally not built**: the version-selection
policy captures the latest non-beta minor per major with no minor sweeps, so it
has no caller — building it would be speculative infrastructure against this
repo's minimum-abstraction rule. `compare_versions` is the reusable core if that
workflow is ever revived; the human-readable "ledger" is COVERAGE.md's
"Xcode versions captured" table.

### 26.x refresh: 26.0.1 → 26.5, full multi-platform capture (2026-05-30)

Refreshed the 26.x representative from **26.0.1 to 26.5 (17F42)** — the latest
non-beta — and **dropped 26.0.1 entirely** (fixtures + xcspec-cache + all the
unit/integration tests' hardcoded `xcode-26.0.1` paths repointed to `26.5.0`).
The full corpus was re-captured against 26.5: all 5 projects (ice-cubes now opens
— its Swift-tools 6.2 needs Xcode 26), per-target + project-defaults + **568
scheme captures across iOS/tvOS/watchOS/visionOS simulators + macOS** + synthetic
+ xcconfig + `_synthetic-xcconfigs` + `_global`. Resolver is clean on 26.5
(structural 99%, same documented residuals as 26.0.1).

The capture was driven autonomously and surfaced several real gaps, all fixed:
- **Sudo-only first-run steps for a *newer-than-system* Xcode.** `--no-superuser`
  skips license + first-launch, which is fine for Xcode *older* than the system
  one (15.4/16.4) but not 26.5 (> system 26.0.1): `xcodebuild` then refuses with a
  license error, then an `IDESimulatorFoundation`/`CoreSimulator` plugin-load
  error. Both need one `sudo` each (`xcodebuild -license accept`,
  `xcodebuild -runFirstLaunch`) — the only non-autonomous steps. Documented as the
  newer-than-system caveat.
- **`SWIFT_EMIT_CONST_VALUE_PROTOCOLS`** (the AppIntents const-extractable list):
  not in any xcspec (xcodebuild injects it from the SDK), so it's a hardcoded
  per-SDK snapshot — updated to 26.5's (sorted, +`AppUnionValue`/
  `AppUnionValueCasesProviding`). 26.x-only key.
- **02 simulator enumeration race:** under an orchestrated run `-showdestinations`
  intermittently omits concrete simulators (lazy CoreSimulator device creation),
  so `02_capture_metadata.py` now derives a representative simulator per supported
  platform from `simctl` (reliable) and injects it (`augment_with_simulators`).
- **Orchestrator `--force` wasn't forwarded** to the numbered sub-scripts, so
  re-captures silently skipped already-present outputs — now forwarded through
  `capture_projects` / `capture_settings_steps`.

Runtimes were cycled within disk (delete old → `-downloadPlatform` per platform,
all sudo-free) and removed after use; the `compare_versions 26.0.1 26.5` delta is
recorded above (deployment-target bumps + ~15 new 26.5 settings, no resolver work
needed). Build settings don't depend on runtime version, so any installed runtime
satisfies a platform's simulator captures.

## Resolution-quality strategy (recommended, added 2026-05-30)

Ordered by impact, for driving build-settings resolution quality going forward:

1. **Maximize oracle breadth across Xcode majors (highest ROI).** Capture more
   majors (15, 14, …), not more projects at one version. Each major surfaces
   version-conditional keystone bugs that fix all versions at once — like the
   `XCODE_VERSION_MAJOR` nested-expansion bug the 16.4 capture exposed but 26.0.1
   alone hid. Keep captures committed (`fixtures/<slug>/xcode-<ver>/` +
   `xcspec-cache/`) so each version is validated forever without reinstalling it.
2. **Diversify project shape, not count.** More frameworks teach little.
   Prioritize projects exercising distinct machinery: multiplatform apps
   (`SDKROOT=auto`), Mac Catalyst, app extensions, watch companion targets,
   SwiftPM-heavy graphs, tuist-generated projects, version-conditional xcconfigs.
   The per-target oracle isolates the resolver cleanly, so coverage of distinct
   constructs beats raw project count.
3. **Enrich destination/scheme coverage per version.** Install the matching
   simulator platform (`xcodebuild -downloadPlatform iOS`) so you capture
   iOS-Simulator + device + multi-platform scheme builds, not just macOS — this
   validates destination-aware defaults (`ARCHS`, `ONLY_ACTIVE_ARCH`, code
   coverage) under each toolchain.
4. **Keep the three-tier scoring honest.** Track structural ≈ 99% (correctness),
   canonical (cross-machine exact), and exact separately. Watch the
   systematic-mismatch tally per key — that distinguishes real value bugs from
   path-geometry noise. Set data-driven floors per version so regressions catch.
5. **Ground every rule, never over-fit.** For each mismatch: confirm against the
   xcspec (authoritative defaults), correlate across the corpus, then the web —
   in that order. Encode a rule only when it's a function of inputs you can see;
   when a value is an irreducible build-system heuristic (e.g. a no-destination
   default) or out-of-scope (code-signing identity depends on the local
   keychain), document it in code rather than forcing it. Over-fitting one
   version regresses another.
6. **Adversarially verify fixes.** Use the fan-out/verify pattern — independent
   agents investigate each candidate fix — and always re-run the full oracle on
   **all** versions after a change to confirm a 16.x fix doesn't regress another
   version.

## Coverage execution plan — majors + shapes (chosen 2026-05-30)

**Decision.** Pursue the two highest-ROI axes together: **#1 breadth across Xcode
majors** and **#2 project-shape diversity**. Explicitly NOT doing now: routine
per-minor sweeps (low ROI — behavior is major-dominated), simulator/destination
depth for its own sake (settings don't depend on the runtime), or "harden &
stop". The user will compact context, then ask to execute BOTH.

**Foundation facts (established this session — the basis for the plan).**
- Build settings depend on the **Xcode major** (xcspec defaults + version-keyed
  settings like `XCODE_VERSION_MAJOR`) and the **SDK/platform** (`iphoneos`,
  `iphonesimulator`, `macosx`, `tvos`, `watchos`, `xros`). Coverage matrix =
  **majors × platforms (N×P)**.
- They do NOT depend on the simulator **runtime version** (557/563 keys identical
  across iOS 18.1 vs 26.0 — the only diffs are `TARGET_DEVICE_*` /
  `ASSETCATALOG_FILTER_FOR_DEVICE_*` echoes) nor much on **minor** versions.
- Apple jumped **16 → 26** (no 17–25). On macOS 26 the realistically capturable
  majors are **26, 16, 15** (15 hits a corpus-format wall; older won't run).
- The pipeline is **fully sudo-free** (verified end-to-end): `xcodes install
  --no-superuser`, Xcode selection via `DEVELOPER_DIR` (not `xcode-select`),
  install into project-local `.xcodes/` (gitignored), fresh runtime download via
  `xcodebuild -downloadPlatform`, real builds, and teardown via `rm -rf` +
  `xcrun simctl runtime delete` — none need sudo (the machine is already
  license-bootstrapped by the system Xcode).
- The **per-target (no-destination)** capture is the richest resolver oracle AND
  needs **no runtime download** (SDKs are bundled in the `.app`). Only
  `iphonesimulator` cells need a runtime, and one per major is shared across all
  its minors (any compatible version works).

**Enablers already built (do not rebuild).**
- Version-aware oracle tests: `CatalogCache` + `capture_xcode_version` pair each
  fixture with its `xcspec-cache/xcode-<ver>/` catalog; `ORACLE_ONLY_VERSION=<ver>`
  prints one version's systematic-mismatch tally and skips floors (diagnostic).
- `Catalog.xcode_version` (read from `meta.json`) feeds `XCODE_VERSION_{MAJOR,
  MINOR,ACTUAL}` in `built_in_settings` — the keystone fix.
- `scripts/13_capture_version.py`: sudo-free, project-local (`--xcodes-dir`,
  default `.xcodes/`), `--no-superuser` install, `--no-runtime`/`--keep`/
  `--dry-run`/`--check-auth`, per-version validate.
- 6 resolver fixes landed (XCODE_VERSION_*, quoted-Xcode-path canon,
  TARGETED_DEVICE_FAMILY, Catalyst SubFrameworks≥26, macOS-test CODE_SIGN_IDENTITY,
  DEBUG_INFORMATION_FORMAT). COVERAGE.md reconciled to **111 ✅ / 21 ❌**.
- Delta/dedup capture mechanism is **designed, not built** (see below).

**Strategy #1 — breadth across majors.**
- Goal: latest-minor-per-major across capturable majors; each surfaces
  version-conditional resolver bugs (the way 16.4 exposed `XCODE_VERSION_MAJOR`).
- **Version-selection policy (which minor per major).** Capture the **latest
  non-beta minor** of each major — never a beta. Behaviour is major-dominated, so
  the newest *released* minor is the single best representative of that major
  (26 → 26.5, 16 → 16.4, 15 → 15.4, etc.). This applies to 26 too: the 26
  representative was **refreshed from 26.0.1 to 26.5 (17F42)** — the latest
  non-beta — and 26.0.1 was dropped entirely (fixtures + xcspec-cache). The full
  multi-platform corpus (all 5 projects, per-target + project-defaults +
  iOS/tvOS/watchOS/visionOS-simulator + macOS schemes + synthetic + xcconfig) was
  re-captured against 26.5; `compare_versions 26.0.1 26.5` confirmed the delta is
  just deployment-target bumps + a handful of new 26.5 settings, with the
  resolver clean (structural 99%, same documented residuals as 26.0.1).
- Status: `26.5` (full corpus, all 5 projects, all simulator platforms — the 26.x
  representative, refreshed from 26.0.1 which is dropped), `16.4` (alamofire+
  kingfisher per-target/project-defaults + macOS scheme), and `15.4` (kingfisher+
  tuist) are captured.
- **THE CORPUS WALL (main new work):** older Xcodes can't open latest-release
  projects — `alamofire` is `objectVersion 77` (needs Xcode 16+), `netnewswire`
  76, `ice-cubes` uses Swift-tools 6.2 (needs Xcode 26). `kingfisher` is
  `objectVersion 54` and DOES open in 15.4. So capturing 15.x (and older) needs
  **era-appropriate project refs** — pin each corpus project to a tag whose
  pbxproj objectVersion / Swift-tools the target Xcode supports. The shared-clone
  model breaks here; may need per-version checkouts for older majors.
- Per-major loop: install (sudo-free, project-local) → snapshot xcspec (04) →
  per-target + project-defaults capture (no runtime) → `cargo test` (version-aware)
  → triage via `ORACLE_ONLY_VERSION=<ver>` → fix version-conditional resolver gaps
  → document irreducibles → teardown. Optionally provision ONE iOS platform per
  major for the simulator scheme captures.
- Next concrete majors: (a) get **15.x** opening more than kingfisher via
  era-appropriate refs, capture, triage, fix. (b) 26.x stays at **26.0.1** —
  decided, see the version-selection policy above; no refresh.

**Strategy #2 — project-shape diversity.**
- Goal: expand the corpus to exercise distinct build-system machinery, captured
  under the EXISTING Xcodes (no new downloads). "Diversify shape, not count."
- High-value shapes to add or verify-and-strengthen: **Mac Catalyst** (iOS app on
  macOS), **multiplatform `SDKROOT=auto`** apps, **app extensions** (share/widget/
  action/notification/intent), **watch companion** targets, **SwiftPM-heavy
  graphs** (static/dynamic/auto linkage), **version-conditional xcconfigs** (the
  `$(FOO_XCODE_$(XCODE_VERSION_MAJOR))` pattern). Many already exist in
  ice-cubes/netnewswire/tuist-fixtures — verify they're actually captured + scored,
  then strengthen.
- Worthwhile ❌ gaps to close (judgment): custom configurations (e.g. `Profile`),
  recursive `$(prefix-$(VAR)-suffix)` substitution, mergeable libraries. LEAVE the
  niche ones (XPC, DriverKit, Metal, Core ML, privacy manifest) unless a real
  resolver need appears.
- Approach per shape: ensure a fixture exercises it → capture → run the per-target
  oracle (the workhorse; isolates the resolver, no runtime) → triage new
  systematic mismatches → fix resolver gaps grounded in xcspec+corpus.

**Execution order when resumed.**
1. (#1) Land a third major: pin era-appropriate corpus refs so 15.x opens beyond
   kingfisher; capture 15.x; triage + fix version-conditional gaps.
2. (#2) Sweep project shapes under existing 26.0.1/16.4: verify+strengthen
   Catalyst, multiplatform-auto, extensions, watch-companion, SPM graphs,
   version-conditional xcconfigs; triage per-target mismatches per shape; fix.
3. Build the **delta/dedup mechanism** if doing any minor/version dedup:
   `compare_versions(a,b)` = canonicalize + drop echo keys (`XCODE_VERSION_MINOR/
   ACTUAL`, `XCODE_PRODUCT_BUILD_VERSION`, `SDK_NAME`, `SDK_VERSION`), diff;
   `--delta` mode stages a per-target capture, diffs vs last-kept, commits only on
   change, writes a `fixtures/versions.json` ledger.
4. After EVERY capture/fix: run full `cargo test` (all versions must stay green —
   the cross-version regression guard), then update COVERAGE.md + this file.

**Method (apply throughout).** Ground every rule xcspec → corpus → web; document
irreducible heuristics rather than over-fit; use the fan-out/adversarial-verify
workflow for candidate fixes; re-run the full oracle on ALL versions after each
change so a fix for one version can't silently regress another.

**Uncommitted surface to land (show messages first, per the user's flow).** This
session's work is NOT yet committed: resolver fixes (`src/`), version-aware +
diagnostic test harness (`tests/`), the capture orchestrator + sudo-free
`with_xcode` (`scripts/`), the `16.4.0` fixtures + `xcspec-cache`, and the
PLAN.md/COVERAGE.md docs. Group sensibly and get message approval before committing.
