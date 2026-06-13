# sweetpad-lib — Documentation

The single consolidated reference for `sweetpad-lib`. It replaces the former
`PLAN.md`, `PLAN_COMPILER_ARGS.md`, `PLAN_BSP.md`, `COVERAGE.md`,
`AUDIT_IMPROVEMENTS.md`, and `UPDATING_XCODE_VERSIONS.md` (their content is
merged, de-duplicated, and updated here). `CLAUDE.md` is a short pointer into
this file; `README.md` is the external-facing package stub.

**Contents**

1. [Overview](#1-overview)
2. [Repository layout](#2-repository-layout)
3. [Development principles](#3-development-principles)
4. [The test corpus](#4-the-test-corpus)
5. [Xcode version coverage](#5-xcode-version-coverage)
6. [Settings resolution](#6-settings-resolution)
7. [Compiler-argument resolution](#7-compiler-argument-resolution)
8. [The BSP server](#8-the-bsp-server)
9. [Feature coverage matrix](#9-feature-coverage-matrix)
10. [Runbook: updating Xcode versions](#10-runbook-updating-xcode-versions)
11. [Roadmap & open work](#11-roadmap--open-work)
12. [Project history](#12-project-history)

---

## 1. Overview

> ⚠️ **Internal — do not use externally.** This crate is an internal
> implementation detail of the [SweetPad](https://github.com/sweetpad-dev/sweetpad)
> VS Code extension, shipped to it as an N-API native addon (`@sweetpad/lib`).
> No stable API, no semver guarantees.

`sweetpad-lib` is a Rust resolver for Xcode `pbxproj` / `xcconfig` build
settings, snapshot-tested against real `xcodebuild` captures. It is built in
three layers, each validated against its own oracle:

```
pbxproj / xcconfig / xcspec
  → [settings resolver]    → resolved build-settings dict      (vs -showBuildSettings captures)
  → [argument resolver]    → per-tool argv (swiftc/clang/link) (vs real-build stdout captures)
  → [BSP server]           → editor intelligence via sourcekit-lsp
```

The BSP server plugs into:

```
editor ──LSP──► sourcekit-lsp ──BSP──► sweetpad BSP server ──► compiler args ──► SourceKit
```

**Approach: derive, don't observe.** The established tool,
[`xcode-build-server`](https://github.com/SolaWing/xcode-build-server),
*observes* — it parses an `xcodebuild` build log to recover real compile
commands. Accurate, but requires a prior build, goes stale on edits, and broke
when Xcode 26 stopped persisting the activity log. We *derive*: compute the
compiler args directly from the project + resolved build settings (what Xcode
does internally before running anything). No build required, always fresh, fast
— at the price of having to reproduce Xcode's settings resolution and flag
generation exactly. That is what the corpus oracles prove.

**Health snapshot (HEAD, 2026-06-12, commit `51bb938`).** `cargo test` fully
green (corpus oracles, BSP conformance, adversarial-input and round-trip
suites); `cargo fmt --check` clean. Settings-resolution scores
(exact/canonical/structural): corpus oracle **89/100/100** on Xcode 15.4,
**88/100/100** on 16.4, **88/97/99** on 26.5. The only remaining systematic
mismatch corpus-wide is `CLANG_COVERAGE_MAPPING` ×2 — a capture gap, not a
resolver bug (see [§11](#11-roadmap--open-work)). The June 2026 audit's P0/P1
findings were fixed in `51bb938`, except the extension-side packaging item
P0.3 (see [§11.2](#112-audit-follow-ups-june-2026)).

## 2. Repository layout

```
sweetpad-lib/
  Cargo.toml / Cargo.lock      # single crate `sweetpad`; lib + `sweetpad` CLI + `bsp-server` binary (BSP tests/debugging); lock committed
  rust-toolchain.toml          # pinned toolchain (edition 2024)
  rustfmt.toml
  package.json                 # @sweetpad/lib napi packaging (darwin targets)
  LICENSE-MIT / LICENSE-APACHE # MIT OR Apache-2.0
  src/                         # parsers (pbxproj, xcconfig, xcscheme, bplist, xcspec),
                               # resolver, build_context, compiler_args, bsp/, node.rs (napi),
                               # catalog_embedded.bin (embedded defaults catalog)
  tests/                       # oracle + conformance integration tests (read fixtures/ directly)
  examples/gen_embedded_catalog.rs  # regenerates src/catalog_embedded.bin
  scripts/                     # Python capture/validation pipeline (see §4.4)
  corpus/                      # git clones of the corpus projects (gitignored) + manifest.json
  fixtures/                    # committed captures: fixtures/<slug>/xcode-<ver>/{raw,metadata,build,pif,errors}
                               # + synthetic fixtures (fixtures/_synthetic-*) + _global/
  xcspec-cache/                # per-version Apple xcspec + SDKSettings snapshots
  .xcodes/                     # orchestrator-local Xcode installs (gitignored)
```

Fixture sizing: ~76 MB in the working tree (42 MB tuist-fixtures) but packs to
~8 MiB — checkout size and editor indexing are the only costs; acceptable.

## 3. Development principles

### 3.1 Dependencies

Keep them minimal — but *minimal*, not *zero for its own sake*. Hand-roll the
**project-domain** formats Apple invented and no crate handles well (OpenStep
`pbxproj`, `xcconfig`, the DerivedData path hash, the binary catalog cache):
that parsing *is* the library's value, and owning it keeps us exact. Do **not**
reinvent well-known standardized formats — JSON, XML, and the like — where a
mature crate is effectively part of the ecosystem's std (e.g. `serde_json` for
the BSP server's JSON-RPC). The Node runtime is feature-gated (`node`) because
it's heavy and platform-specific; a small pure-Rust utility crate for a
standard format does not need that ceremony.

### 3.2 Code style

- **Build in isolated parts.** Get each unit working end-to-end against real
  fixtures before introducing the next layer.
- **Tests driven by fixtures.** Integration tests read inputs directly from
  `fixtures/<slug>/xcode-<ver>/…`. No fabricated test data unless required to
  isolate a bug.
- **Minimum abstraction.** Concrete types and plain functions. No trait /
  generic / lifetime layer until concrete duplication forces it. Three repeated
  lines beat a premature `trait`. Don't build speculative infrastructure with
  no caller.
- **Readability beats performance.** `.clone()` and `String` are fine;
  optimize only when a benchmark says so.
- **Sparse comments.** Code self-documents via names. Comments only for
  non-obvious *why* — a workaround for an `xcodebuild` quirk, a surprising
  invariant in the corpus data.
- `[lints]` enables `clippy::pedantic = "warn"`.

### 3.3 Grounding rules: investigating how a build setting behaves

When you need to understand how a setting resolves — its default, what gates
it, how it couples to other settings, or why `xcodebuild` emits a value we
don't — use every source available, in this order:

1. **Apple's cached xcspecs** under `xcspec-cache/xcode-<ver>/` — the
   authoritative local source for documented defaults and per-product-type
   rules (e.g. `DarwinProductTypes.xcspec`, `macOSProductTypes.xcspec`).
2. **The corpus oracles** — `fixtures/<slug>/…/build-settings/*.json` are real
   captured outputs; correlate values across product types, configs, and
   platforms to derive a rule empirically.
3. **The internet** — Apple developer documentation, the Developer Forums,
   release notes, WWDC notes. Confirm whatever you read back against the
   xcspecs and the corpus before encoding it.

Prefer a rule grounded in the xcspec + verified against the corpus over a
guess. Encode a rule only when it's a function of inputs we can see; when a
value is an irreducible build-system heuristic (e.g. a no-destination default)
or out of scope (signing identity depends on the local keychain), **document it
in code rather than over-fitting** — over-fitting one Xcode version regresses
another.

### 3.4 Validation discipline

- `cargo test` runs the unit tests plus every oracle, which scores the full
  pipeline against every committed capture.
- After **every** capture or resolver change, re-run the full oracle suite on
  **all** captured versions — a fix for one version must not silently regress
  another.
- Judge correctness by the **structural %** and the per-key
  **systematic-mismatch tally** (keys that fail even structurally — the genuine
  value bugs), never by the geometry-capped exact %. See
  [§5.3](#53-scoring-three-tiers--per-version-floors).
- Adversarially verify fixes: use the fan-out/verify pattern — independent
  investigations of each candidate fix — before encoding rules.
- Ratchet per-version floors after every fix so progress is permanent.

## 4. The test corpus

The corpus is a set of real open-source projects whose Xcode outputs
(`-showBuildSettings`, `-list`, real-build compiler invocations) were captured
as oracles. Each project is cloned (gitignored) under `corpus/<slug>/` at a
pinned ref recorded in `corpus/manifest.json`; the captures are committed under
`fixtures/<slug>/xcode-<ver>/`.

### 4.1 Corpus projects

| Slug | Repo / pin | What it brings |
|---|---|---|
| `alamofire` | `Alamofire/Alamofire`, release tag | Pure-Swift library framework × iOS/macOS/tvOS/watchOS/visionOS variants; iOS example app; xcworkspace; nested sub-xcodeproj |
| `ice-cubes` | `Dimillian/IceCubesApp`, release tag | Real-world iOS app: many SPM deps, app extensions (share/widget/action/notifications), multi-target, multiplatform `SDKROOT = auto` |
| `netnewswire` | `Ranchero-Software/NetNewsWire`, default-branch commit | Multi-platform (macOS + iOS), ObjC interop, Core Data, many internal Swift frameworks, share/widget/intent extensions, xcconfig-heavy |
| `kingfisher` | `onevcat/Kingfisher`, release tag | Image library with iOS/macOS/tvOS/watchOS demo apps; xcworkspace; oldest pbxproj format (opens in Xcode 15) |
| `tuist-fixtures` | `tuist/tuist` fixtures subset, release tag | 16 generated Tuist projects: buildable folders, framework+tests, ios+extensions, static frameworks/libraries, command-line tools with dynamic framework/library, xcstrings resources, local package with traits, custom schemes/configurations, Core Data, watchapp2 |

### 4.2 What is captured per (project × Xcode version)

- **Metadata** (no build): `xcodebuild -list -json` → `metadata/list.json`;
  `-showsdks` → `metadata/showsdks.json`; per scheme `-showdestinations` →
  `metadata/schemes/<S>/destinations.json` and per (configuration ×
  destination) `-showBuildSettings -json` →
  `metadata/schemes/<S>/build-settings/<C>__<D_slug>.json`.
- **Raw inputs** copied into `raw/` preserving relative paths: every
  `project.pbxproj`, `contents.xcworkspacedata`, `*.xcscheme`, transitively
  referenced `*.xcconfig`, `Info.plist`, `*.entitlements` (and, per roadmap
  item A1, scheme-referenced `*.xctestplan`).
- **Build artifacts** (smoke builds, simulator/unsigned,
  `CODE_SIGNING_ALLOWED=NO`): result bundles, parsed activity logs (pre-26),
  index-store snapshots, PATH-shim `tool-invocations.jsonl`,
  stdout/stderr/exit code under `build/<scheme>__<config>__<dest>/`.
- **Per-version Apple spec data** in `xcspec-cache/xcode-<ver>/`: all
  `*.xcspec` under the Xcode app, all `SDKSettings.plist`, SDK paths.
- **Compiler-args oracles** under `compiler-args/` — see
  [§7.2](#72-the-oracle-capture-and-scoring).

Capture technique notes that remain relevant:

- The **PATH-wrapped toolshim** (`scripts/toolshim/`) logs argv/cwd/allowlisted
  env per tool to a JSONL. Caveat discovered later: `xcodebuild` invokes the
  main compilers by absolute toolchain path, so the shim log is empty for
  `swiftc`/`clang`/`ld` — that's why the compiler-args oracle parses build
  stdout instead ([§7.2](#72-the-oracle-capture-and-scoring)).
- Xcode selection is **sudo-free** via `DEVELOPER_DIR` (not `xcode-select`);
  `with_xcode()` in `scripts/common.py` switches and restores.
- Scripts are idempotent: `--force` to redo, `--project <slug>` / `--xcode
  <ver>` to scope; failures are written to the fixture's `errors/` and the run
  continues. Do not modify a source project to make it build.

### 4.3 Synthetic fixtures

Hand-built fixtures cover paths no real corpus project exercises:

| Fixture | Exercises |
|---|---|
| `_synthetic-xcconfigs` | xcconfig probes: `[arch=…]`, `[config=…]`, `[sdk=…]`, `${VAR:default=…}`/`:lower`/`:upper` modifiers, multi-line continuation, `#include`, `$(inherited)` — captured with and without the xcconfig layered on |
| `_synthetic-custom-config` | A third configuration `Profile`: config-name-driven selection + a `[config=Profile]` xcconfig override (`scripts/15_custom_configuration.py`) |
| `metadata/_synthetic/<override>` (under alamofire) | `KEY=VALUE` xcodebuild overrides for flags no real project enables: library evolution, LTO, arm64e, Swift 6, mergeable libraries… (`scripts/07_synthetic_overrides.py`) |
| `_synthetic-staticlib` | `libtool -static` link + ObjC++ (`.mm`) language gate (`scripts/17_static_library.py`) |
| `_synthetic-rich` | Rich settings: UBSan (+ sub-checks), exceptions, hidden visibility, warnings, `SWIFT_STRICT_CONCURRENCY = complete` (`scripts/18_rich_settings.py`) |
| `_synthetic-multimodule` | App + two framework targets with a real cross-module `import` — the BSP pilot fixture (`scripts/19_multimodule.py`) |
| `_synthetic-objc-headers` | ObjC header search paths for the BSP loop (`scripts/20_objc_headers.py`) |
| `_synthetic-multiplatform` | One `SDKROOT = auto` target with `SUPPORTED_PLATFORMS = iphoneos iphonesimulator macosx` — the IceCubesApp shape that keeps the SDK-binding regression in CI |
| `_synthetic-{coredata,assetsym,strcat,intents,cocoapods,macro,tests}` | BSP generated-source / CocoaPods / Swift-macro / XCTest coverage — each a forced `Probe*.swift` referencing a build-time-generated / Pod / macro-expanded symbol |
| `_synthetic-spm`, `_synthetic-workspace` | SwiftPM package products (`-F …/PackageFrameworks`); multi-project `.xcworkspace` resolution |
| `_global` | Per-SDK metadata (`sdks/<sdk>.json`), xcodebuild version banner |
| `_tuist-src` | Generated tuist examples adding a command-line tool (`mh_execute`) and a standalone dynamic library (`mh_dylib`) to the compiler-args oracle |

### 4.4 Capture scripts index

| Script | Purpose |
|---|---|
| `00_bootstrap.py` | Host prerequisites (xcodes, tuist, xclogparser; Xcode installs) |
| `01_clone_corpus.py` | Clone corpus repos at pinned refs; write `corpus/manifest.json` |
| `02_capture_metadata.py` | Metadata + raw inputs per (project, Xcode); `augment_with_simulators()` derives a representative simulator per platform from `simctl` (reliable against the `-showdestinations` race); self-sets `DEVELOPER_DIR` via `--xcode` |
| `03_run_builds.py` | Smoke builds + artifacts |
| `04_snapshot_xcspecs.py` | xcspec + SDKSettings snapshot per Xcode version |
| `05_validate.py` | Walks `fixtures/`, writes `fixtures/REPORT.json` + the capture-completeness half of the generated `fixtures/FIXTURES.md` |
| `06_audit_coverage.py` | Probe-based audit → `fixtures/AUDIT.json` + the feature-probe half of `fixtures/FIXTURES.md`; corpus-tree probes are tri-state (carried forward, marked stale, when the gitignored `corpus/` clones are absent) |
| `07_synthetic_overrides.py` | Synthetic `KEY=VALUE` override captures (does **not** self-set `DEVELOPER_DIR` — export it) |
| `08_global_defaults.py` | `_global` per-SDK metadata |
| `09_per_project_settings.py` | Per-target + project-defaults captures |
| `10_xcconfig_resolution.py` | `xcodebuild -xcconfig FILE -showBuildSettings` per real `.xcconfig` |
| `11_synthetic_xcconfigs.py` | The `_synthetic-xcconfigs` probes |
| `12_pif_dumps.py` | PIF cache dumps from DerivedData (dormant source) |
| `13_capture_version.py` | The per-version orchestrator: acquire → capture (04/02/03/07–12) → validate → teardown, disk-bounded, sudo-free, `--check-auth`/`--dry-run`/`--keep`/`--no-runtime`/`--force` (forwards to sub-scripts) |
| `14_compare_versions.py` | Cross-version delta: diffs two versions' per-target settings, canonicalizing path drift and dropping version-echo keys, split into behavioural vs path-geometry buckets |
| `15_custom_configuration.py` | `_synthetic-custom-config` |
| `16_capture_compiler_args.py` | Compiler-args oracle capture (build stdout → per-tool argv JSON) |
| `17_static_library.py` / `18_rich_settings.py` / `19_multimodule.py` / `20_objc_headers.py` | Synthetic fixtures above |
| `21_mutation_audit.py` | Injects plausible resolver bugs and checks a fast net goes red — measures the coverage of the coverage (7/7 caught by the fast tier; `--e2e` proves the de-exoneration reclassifies the SDKROOT=auto class) |

### 4.5 Oracle data sources and the tests that consume them

| Source | Path | Test |
|---|---|---|
| Per-scheme build-settings | `metadata/schemes/<S>/build-settings/*.json` | `tests/corpus_oracle.rs` — the headline oracle: full scheme-aggregated resolution |
| Per-target settings | `metadata/<sub>/_per_target/<proj>/<target>__<config>.json` | `tests/per_target_oracle.rs` — isolated single-target layer stack, no scheme aggregation/destination; the cleanest resolver oracle |
| Project-default settings | `metadata/<sub>/_project_defaults/<proj>/project-only*.json` | `tests/project_defaults_oracle.rs` — xcodebuild's default-target view |
| Synthetic overrides | `metadata/_synthetic/<override>/build-settings/*.json` | `tests/synthetic_override_oracle.rs` |
| xcconfig resolution view | `metadata/_xcconfig_resolution/*.json` | `tests/xcconfig_resolution_oracle.rs` — `flatten_xcconfig` on real `.xcconfig`s |
| Synthetic xcconfig probes | `fixtures/_synthetic-xcconfigs/…` | `tests/resolver.rs`, `tests/xcconfig.rs` |
| Custom configuration | `fixtures/_synthetic-custom-config/…` | `tests/custom_configuration_oracle.rs` |
| `-list` discovery | `metadata/**/list.json` (30 captures) | `tests/discovery_oracle.rs` — targets/configurations/schemes per container, sets **and** case-insensitive ordering, 100% exact |
| Compiler args | `compiler-args/*.json` | `tests/compiler_args_oracle.rs` |
| xcspec snapshots | `xcspec-cache/xcode-<ver>/` | `tests/xcspec.rs` (catalog + scratch oracle) |
| **Dormant** (committed, no test consumes them yet) | PIF dumps (`pif/`), dry-run captures, toolshim invocation logs, `_global` SDK metadata, version banner | — (see roadmap E21) |

Every oracle test shares `tests/common/mod.rs` (JSON reader,
`canonicalize_value` + `canon_*` helpers, corpus walk, `Stats`, `compare`,
`print_summary`) and prints a per-key systematic-mismatch tally plus a
canonical-only (path-root drift) tally, so each is a diagnostic, not just
pass/fail. Documented per-source skips are fixture-capture gaps, never
fabricated as passes. Diagnostics: `ORACLE_ONLY_VERSION=<ver>` isolates one
version's tally and skips floors; `DEBUG_DIFF_KEY=<KEY>` dumps one key's
mismatches.

### 4.6 Generated report

`fixtures/FIXTURES.md` is **generated — do not hand-edit**: the
capture-completeness section comes from `05_validate.py` (`REPORT.json`), the
feature-probe section from `06_audit_coverage.py` (`AUDIT.json`); either
script rebuilds the combined file from both JSONs. It is current against the
26.5/16.4/15.4 corpus. Corpus-tree probes (file presence in the gitignored
`corpus/<slug>/` clones) can only be re-evaluated on a host with the clones;
elsewhere the last corpus-present results are carried forward and marked
stale (`*`).

## 5. Xcode version coverage

### 5.1 Version policy

The corpus tracks the **latest non-beta minor of each Xcode major** — never a
beta. Behaviour is major-dominated (minor churn is small and dominated by the
bundled-SDK bump, which the canonicalizer strips), so the newest released minor
is the single best representative of a major; per-minor sweeps are explicitly
not done. A refresh = capture the new minor, then drop the old one entirely
([§10](#10-runbook-updating-xcode-versions)). Apple jumped 16 → 26 (no 17–25);
on macOS 26 the realistically capturable majors are 26, 16, 15.

Capturing a **new major is the highest-ROI coverage move**: version-conditional
keystone bugs (e.g. `XCODE_VERSION_MAJOR` nested expansion, the `DEVELOPER_DIR`
catalog bug) surface there and fix all versions at once. Build settings depend
on the Xcode major and the SDK/platform — **not** on the simulator runtime
version (557/563 keys identical across iOS 18.1 vs 26.0; the diffs are
`TARGET_DEVICE_*` echoes), so runtimes are disposable and any installed runtime
satisfies a platform's captures.

### 5.2 Versions captured

Each version is a directory under `fixtures/<slug>/xcode-<ver>/` plus a
matching `xcspec-cache/xcode-<ver>/`, committed so it is validated forever
without reinstalling the Xcode.

| Xcode | Captured | Notes |
|---|---|---|
| `26.5.0` | Full corpus (all 5 projects) | Latest non-beta 26.x — refreshed from 26.0.1 (dropped); per-target + project-defaults + 568 scheme captures across iOS/tvOS/watchOS/visionOS simulators + macOS + synthetic + xcconfig; all oracle sources |
| `16.4.0` | alamofire, kingfisher (per-target + project-defaults + macOS scheme) | Second major; ice-cubes incompatible (Swift-tools 6.2 manifests); iOS scheme captures need the user-gated `xcodebuild -downloadPlatform iOS` |
| `15.4.0` | kingfisher, tuist-fixtures (per-target + project-defaults + macOS scheme) | Third major; exposed two undomained-xcspec parser bugs (`PACKAGE_TYPE`/`BUNDLE_FORMAT` clobber, fixed) and a family of 16+-calibrated built-in rules now version-gated (see `tests/version_and_optimization_gates.rs` and the `legacy_xcode15` gates in `src/project.rs`); alamofire/netnewswire/ice-cubes walled off (objectVersion 76/77, Swift-tools 6.2) |

### 5.3 Scoring: three tiers + per-version floors

Every settings oracle scores each shared key in three tiers:

- **exact** — byte-equal.
- **canonical** — equal after `canonicalize_value` strips `$HOME` /
  DerivedData-hash / Xcode-build / SDK-version / project-root drift. The
  cross-machine ceiling.
- **structural** — both sides are absolute paths. Geometry-independent; this
  is the correctness signal.

The exact % is **capped by test geometry** — we resolve against
`fixtures/<slug>/…/raw/` while the oracle was captured at the original
checkout, so `PROJECT_DIR`/`SRCROOT`/`BUILD_DIR`-anchored values can never
byte-match. Judge resolver correctness by **structural %** plus the per-key
systematic-mismatch tally, never exact %.

Floors are **per Xcode version** (`common::assert_version_floors`), data-driven
from the first clean run minus ~1 pt; `structural` is floored at 98 across the
board. A freshly captured version with no codified floor gets only the
`structural ≥ 98` safety guard plus a printed `NO CODIFIED FLOOR` line, so
adding captures never hard-fails before calibration. A single blended floor
across versions is the wrong design — tuist-fixtures is ~76% of all keys, so a
blend drifts as majors are added and masks per-version regressions.

### 5.4 The corpus wall (older majors)

Latest-release projects only open in recent Xcode: `alamofire` is
`objectVersion 77` (needs Xcode 16+), `netnewswire` 76, `ice-cubes` uses
Swift-tools 6.2 (needs Xcode 26). `kingfisher` (`objectVersion 54`) and the
tuist-generated projects (55) open in 15.4. Capturing an older major therefore
needs **era-appropriate refs** — pin each corpus project to a tag whose
pbxproj objectVersion / Swift-tools the target Xcode supports. The shared
single-clone model breaks here; older majors may need per-version checkouts.

## 6. Settings resolution

### 6.1 Scope

**In scope** — anything derivable from project inputs (pbxproj/xcconfig) plus
the Apple xcspec/SDKSettings defaults. This includes signing settings that are
pass-through or per-SDK/per-platform defaults: `DEVELOPMENT_TEAM` (resolved via
self-reference inheritance — `KEY = $(KEY)` inherits the lower layer), the
literal `CODE_SIGN_IDENTITY` per-SDK default (`-` on simulators,
`Apple Development` on macOS), `CODE_SIGN_STYLE`, the
`ENABLE_HARDENED_RUNTIME` per-platform default, and the `maccatalyst.`
`PRODUCT_BUNDLE_IDENTIFIER` prefix.

**Out of scope** — the "real signing" (environment-derived, not in project
inputs): `EXPANDED_CODE_SIGN_IDENTITY`, the expanded identity string,
`PROVISIONING_PROFILE_SPECIFIER`, the resolved profile UUID — anything needing
the Mac keychain, `~/Library/MobileDevice/Provisioning Profiles/`, or the
Xcode account. Also out: archive / release-signed builds, CocoaPods and
Carthage integration, multi-host CI capture, public API/schema design.

### 6.2 Known irreducibles & data gaps

Documented in code — **do not keep re-investigating**:

- **`ENABLE_DEBUG_DYLIB`** for `application` targets in Release (and the
  coupled `DEBUG_INFORMATION_FORMAT = dwarf-with-dsym` in Debug for the same
  targets): an xcodebuild-internal heuristic that is not a function of any
  observable input (proven: not objectVersion, deployment target,
  `ONLY_ACTIVE_ARCH`, package deps, or any declared setting — NetNewsWire=YES
  vs ice-cubes=NO with identical inputs). We emit the majority-correct default
  and accept the residual. (Roadmap E20: one bounded PIF-dump investigation
  before accepting it as irreducible forever.)
- **15.x host/arch reporting family**, version-gated as `legacy_xcode15` rules:
  arm64e `NATIVE_ARCH`/`HOST_ARCH` on Apple Silicon, concrete
  `CURRENT_ARCH`/`arch` (= last of resolved `ARCHS`) on the no-destination
  path, legacy per-SDK `VALID_ARCHS` lists, empty `LOCROOT`/`LOCSYMROOT`, no
  `BUILD_ACTIVE_RESOURCES_ONLY` flip, no Catalyst `SUPPORTED_PLATFORMS` append
  / 13.1 deployment floor, watch UI-test XCTRunner wrapping.

Known data gaps (need corpus work, not resolver work):

- `CLANG_COVERAGE_MAPPING` ×2 on the Alamofire visionOS scheme: its
  `TestAction` uses a `.xctestplan` never copied into `raw/` (roadmap A1).
- `SWIFT_INCLUDE_PATHS` and similar tuist values anchored at the tuist
  DerivedData build directory.
- `IPHONEOS_DEPLOYMENT_TARGET` 13.0-vs-13.1 minor drift — a capture-time
  artifact.

Version-specific hardcoded value to watch on Xcode bumps:
`SWIFT_EMIT_CONST_VALUE_PROTOCOLS` (`src/project.rs`) — the AppIntents
const-extractable list is SDK-injected (in no xcspec), 26.x-only (gated on
major ≥ 26), and grows/re-sorts per SDK.

### 6.3 Measured baseline (2026-06-12, post-`51bb938`)

| Oracle | Exact / Canon / Struct | Systematic mismatches |
|---|---|---|
| corpus 15.4 | 89 / 100 / 100 | none |
| corpus 16.4 | 88 / 100 / 100 | none |
| corpus 26.5 | 88 / 97 / 99 | `CLANG_COVERAGE_MAPPING` ×2 (capture gap, roadmap A1) — the **only** one |
| per-target, project-defaults, synthetic-override, real-xcconfig, custom-config | all raised by the `51bb938` geometry fixes; structural 100% | none |
| discovery (`-list`, 30 containers) | 100% exact (sets + ordering) | none |

The `51bb938` geometry closures: `CCHROOT`/`CACHE_ROOT` now composed from the
**catalog's** Xcode build version (read from `meta.json`, with
`HostOverride::darwin_user_cache` pinning the capture host in tests; also emits
`XCODE_PRODUCT_BUILD_VERSION` and `XCODE_APP_SUPPORT_DIR`); per-variant object
dirs gained Swift Build's sanitizer suffixes from authored `ENABLE_*_SANITIZER`
values and scheme `LaunchAction` toggles (`Scheme::launch_sanitizers`); the
tuist canonicalizer maps the oracle's `examples/xcode/<dir>` capture layout to
the imported `examples_xcode_<dir>` spelling.

Schemes `xcodebuild` synthesizes from Swift *package* manifests (41 across
ice-cubes/netnewswire) are outside the pbxproj surface and are tallied by the
discovery oracle, not failed (scope decision pending — roadmap D15).

## 7. Compiler-argument resolution

### 7.1 Goal

Given a resolved build-settings context plus the target's inputs, produce the
argument vectors `xcodebuild` would invoke per target — `swiftc`,
`clang`/`clang++`, `ld`/`libtool` — validated against literal commands captured
from a real build. The settings resolver answers "what is
`SWIFT_OPTIMIZATION_LEVEL`?"; this layer answers "what is the full `swiftc …`
command line for this target?". Exposed as `#[napi] compiler_arguments` →
`compilerArguments(...)` returning per-target `swift`/`clang`/`link` argv.

Generation units: Swift = one module invocation per target; clang = per-target
common flags + per-file I/O (`{ commonArguments, files }`); link = per-target.
Approach is hybrid: hand-code the computed/build-system flags, back the option
encodings with xcspec `CommandLineArgs`/`CommandLineFlag` parsing
(`CompilerOption`/`CliArgs`, cached in the catalog).

### 7.2 The oracle: capture and scoring

**Why stdout, not dry-run or shims:** `-dry-run` was removed in Xcode 26 (632
of 654 committed `dry-run/*.txt` are just the unsupported error); the PATH
toolshim never sees the main compilers (invoked by absolute path); and Xcode
26's CLI `xcodebuild build` no longer persists a `.xcactivitylog`. The source
that survives is `xcodebuild`'s **stdout**, which echoes every command verbatim
under `<Phase> … (in target 'T' from project 'P')` headers. The capture
(`scripts/16_capture_compiler_args.py`) runs a real build (dedicated
`-derivedDataPath`, `CODE_SIGNING_ALLOWED=NO`), splits stdout into phase
blocks, strips the `builtin-SwiftDriver -- ` wrapper, shell-tokenizes, and
expands `@<file>` response files and `@*.SwiftFileList`. Committed artifact:
`fixtures/<slug>/xcode-<ver>/compiler-args/<scheme>__<config>__<dest>.json`
with raw (un-canonicalized) per-target argv; build logs/DerivedData stay
gitignored.

**Scoring** (`tests/common/argv.rs` + `tests/compiler_args_oracle.rs`):
argv normalized into standalone flags + `(flag, value)` pairs; repeatable
order-insignificant families (`-I`, `-F`, `-D`, `-Xcc`, `-l`, `-L`,
`-framework`) compared as multisets. Same three tiers as settings (reusing
`canonicalize_value`). **Precision and recall** both scored — missing *and*
extra tokens are defects — in a per-flag tally split by direction. Pure build
geometry is recorded but not scored: `-o`/output paths, `-index-store-path`,
`-serialize-diagnostics`, dependency files, filelist paths. Floors keyed by
`(version, platform)`. Out of scope: per-primary-file swift *frontend* jobs
(driver-internal; no consumer needs them).

### 7.3 Status (as built)

All phases complete (capture → comparator → source-file resolution
(`project::target_source_files`) → swift generator → xcspec ingest → clang +
link + corpus expansion → napi API). Coverage spans **Xcode 15.4 / 16.4 /
26.5** on macOS and, at 26.5, **macOS / iOS (device + simulator) / tvOS /
watchOS / visionOS** (the generator is platform-agnostic — driven by the
resolved triple + settings, no platform-specific gating).

- **swift**: optimization, active-compilation-conditions, upcoming/experimental
  feature families, strict concurrency, coverage, whole-module Release
  (`-whole-module-optimization` / `-no-emit-module-separately-wmo`), bridging
  headers, test-target framework paths. Driver defaults that turned over at the
  Xcode 26 explicit-modules cutover are gated on the toolchain major
  (`-enforce-exclusivity=checked` < 26; libc++ `_LIBCPP_HARDENING_MODE` ≥ 26).
  97–100% structural everywhere except xros 94%.
- **clang**: language-gated against xcspec `FileTypes`/`Architectures` (a C++
  `-std` never reaches an ObjC `.m`; an Intel-only `-fasm-blocks` never reaches
  arm64), per-file `-x <dialect>`; xcspec `Condition` predicates evaluated
  against resolved settings so gated options don't leak (e.g.
  `-fsanitize=integer` only when the parent sanitizer is on — and the
  `_synthetic-rich` fixture proves the gate also *passes* when it should).
  95–98% structural.
- **link**: executable/bundle/dylib shapes, modern driver defaults
  (`-Xlinker -reproducible`/`-dead_strip`, Debug
  `-no_deduplicate`/`-rdynamic`, `-fobjc-link-runtime`, coverage), Swift
  runtime rpath via the Concurrency/Span back-deployment gates, version-gated
  `-dead_strip`/`-export_dynamic` spellings, `LD_DEBUG_VARIANT`, dependency
  `-add_ast_path` + `-l<lib>` registration, test-bundle `-iframework` /
  XCTestSwiftSupport, explicitly linked frameworks from
  `PBXFrameworksBuildPhase`. 99–100% structural on 25 of 27 dynamic links;
  `libtool -static` at 100% structural recall (separate generator selected by
  product type / `MACH_O_TYPE`).
- Precision 94–100% per `(version, platform)` cell.

### 7.4 Remaining gaps

Itemized with fixes in roadmap Track C ([§11.1](#111-correctness-roadmap-to-100%)):
the visionOS coverage family (same root cause as the `.xctestplan` capture
gap), `.mm` enumeration in two synthetic fixtures, `<Product>_vers.o`,
test-bundle clang `-iframework`, a few version gates, the confident-wrong
extras tail (`-fexceptions`, `-fvisibility=hidden`, one
`-W(no-)shorten-64-to-32` flip), two `-target` triple mismatches, and the
autolinked `-framework` recall question (imports are encoded in object files,
not the project graph — likely out of scope since BSP never links).

## 8. The BSP server

### 8.1 What BSP needs (vs the build oracle)

The compiler-args oracle scored semantic flags and excluded geometry; for BSP
the priorities invert — the editor never links:

- **Search paths are load-bearing**: `-I`/`-F`/`-isystem`/module paths are how
  SourceKit finds imported modules.
- **Per-file, not per-target**: a `.mm` needs the C++ dialect/flags, a `.m`
  must not.
- **Link/autolink is irrelevant** to the editor.

**The cross-module problem:** intra-module completion is live, but
`import MyOwnLib` needs MyOwnLib's compiled `.swiftmodule` on disk.
sourcekit-lsp background indexing (default since Swift 6.1) delegates that to
the build server via `buildTarget/prepare` — *we* must produce the modules.

### 8.2 Versions (all shipped)

- **v1 — working autocomplete** (relies on a prior build): full search-path /
  module-input emission, per-file argument API
  (`build_settings::resolve_file_arguments`; Swift = the module's swiftc
  invocation, clang = gated to the file's language), editor mode (strips
  `-explicit-module-build`/emit/`-c`, advertises the build's index store).
  Server: `bsp-server bsp` — `build/initialize`, `workspace/buildTargets`,
  `buildTarget/sources` (+ `inverseSources`), `textDocument/sourceKitOptions`,
  `buildTarget/didChange` with a poll-based pbxproj watcher, shutdown/exit;
  `bsp-server config` writes `buildServer.json` (the `version` field is
  required — without it sourcekit-lsp silently skips the server). Hardening:
  Xcode-16 buildable folders (`PBXFileSystemSynchronizedRootGroup` with
  `membershipExceptions`), target dependency edges, Swift-package products
  (`-F …/PackageFrameworks`), corpus soundness checks.
- **v2 — seamless background indexing**: advertises `prepareProvider: true`;
  `buildTarget/prepare` runs an incremental `xcodebuild` **by scheme** (a bare
  `-target` build doesn't populate the products dir;
  `project::scheme_for_target` maps target → scheme) on a serialized worker,
  best-effort. This is where we surpass `xcode-build-server` (no prepare
  there). Validated from a clean DerivedData: real headless sourcekit-lsp
  resolves cross-module imports with 0 diagnostics. Caveat: Xcode has no fast
  declarations-only prepare, so prepare = a real incremental build.
- **v3 — self-built prepare (Swift fast path)**: if the prepared target's
  transitive closure is `project::is_self_buildable` (pure Swift; no package
  products, C-family sources, script phases, or build rules), emit each
  dependency with `swiftc -emit-module` directly (topo order, reusing the
  editor args; ~1s vs ~5s), falling back to the v2 xcodebuild path for any
  non-self-buildable closure or failed self-build. Remaining for "full" v3:
  per-target mixing, code-gen-resource classification, owning more
  output-layout geometry for mixed-language deps.

### 8.3 The measurement loop

All layers are headless and self-labeling (no human in the iteration):

- **Layer 0 — type-check oracle** (`tests/bsp_typecheck_oracle.rs`, opt-in
  `BSP_ORACLE=1`): `swiftc -typecheck` with our generated args → 0
  module-resolution errors, including cross-module imports.
- **Layer 1 — conformance** (`tests/bsp_conformance.rs`, fast/hermetic/
  ungated): scripted JSON-RPC pinning the full reply surface — capability set,
  item shapes, membership fallback, unowned-file and unknown-method edges,
  build-only-flag stripping, the per-file clang `-x` dialect matrix, plus the
  robustness behaviors (`-32700` on malformed JSON, framing limits).
- **Layer 2 — end-to-end** (`tests/bsp_lsp_e2e.rs`, `BSP_ORACLE=1`): real
  headless `sourcekit-lsp` → 0 diagnostics; prepare-from-clean-DerivedData
  validated in `tests/bsp_prepare.rs`.
- **Corpus scale** (`tests/bsp_corpus_completion.rs`, `BSP_CORPUS=1`): the same
  loop over the real OSS corpus, classifying each sampled file clean /
  resolution-failure / internal-error. **Full-corpus run: 138/138 sampled
  files clean (100%)** across kingfisher, alamofire, ice-cubes, netnewswire,
  and every synthetic fixture. Internal errors are **de-exonerated**: the
  file's args are re-run through standalone `swiftc -typecheck`, and if the
  compiler also fails, the file is reclassified as our failure (so the "it's
  all upstream #2328" misattribution cannot recur).
- **Arg-invariant linter** (`tests/bsp_arg_invariants.rs`, ungated): for every
  committed fixture target × `(platform, configuration)`, asserts properties
  any correct invocation has — exactly one real `-sdk` (never `auto`/`$(…)`),
  a `-target` agreeing with the `-sdk`, no leaked build variables, a valid
  `-module-name`.
- **Live differential** (`tests/showbuildsettings_live_diff.rs`,
  `BSP_LIVE_DIFF=1`): diffs the resolver against a fresh xcodebuild run bound
  to each `-sdk` — ground truth the pre-captured oracle (which keeps a literal
  `SDKROOT=auto`) never had.
- **Mutation audit** (`scripts/21_mutation_audit.py`): see §4.4.

CI (`.github/workflows/sweetpad-lib.yaml`) runs the fast tier — fmt, clippy
`-D warnings`, `cargo test` — on every push/PR; the build-gated tiers run
locally/nightly against the corpus.

**Expand later:** the build-gated BSP oracles (Layers 0/2, corpus run) are
pinned to **Xcode 26.5 only**; expand to 15.4/16.4 by keying the harness by
version like the compiler-args oracle. The fast hermetic tiers are
version-agnostic.

### 8.4 Engine fixes the harness drove (knowledge catalog)

Each of these was a real-world failure mode found by the loop; the fixtures
keep them fixed:

1. **Generated sources** — `resolve_file_arguments` folds the `.swift` files
   xcodebuild emits into `DERIVED_SOURCES_DIR` (Core Data subclasses,
   `GeneratedAssetSymbols.swift`, intent classes, string-catalog symbols) into
   the editor's input set.
2. **Nested-group xcconfig** — `resolve_file_ref_path` walks the parent
   `PBXGroup` chain, so CocoaPods' `Pods`-nested `Pods-<App>.xcconfig`
   resolves.
3. **Quoted search paths** — quote-aware `ws_paths`/`ws_unquoted` tokenizers
   strip the quotes xcconfigs put around paths (`"${…}/Pod"`); also applied to
   `OTHER_SWIFT_FLAGS`/`OTHER_LDFLAGS` passthrough splitting.
4. **Swift macro plugins** — per-plugin `-Xfrontend -load-plugin-executable
   -Xfrontend <plugin>#<module>` for each macro plugin built into the host
   products dir (a `-plugin-path` does **not** discover executable plugins —
   verified empirically); plugins enumerated as extension-less Mach-O
   executables, lazily loaded so spurious entries are harmless.
5. **Unit-test files** — add `-I $(PLATFORM_DIR)/Developer/usr/lib` (XCTest's
   Swift overlay; the platform-Frameworks `-F` only finds the ObjC API).
6. **Multiplatform `SDKROOT = auto`** — `editor_platform` derives the platform
   from `SUPPORTED_PLATFORMS` when `SDKROOT` is `auto`, and `build_context`
   binds `auto` to the chosen SDK's real path whenever the requested sdk is
   supported (the unbound `-showBuildSettings` oracle still reports the
   literal `auto` — no corpus regression). Took IceCubesApp from 30/30
   stdlib-load failures to 0. The genuine upstream sourcekit-lsp #2328 is a
   separate cluster our self-consistent args don't trigger.
7. **Multi-project workspaces** — a `.xcworkspace` resolves to member
   projects, targets unioned, each file resolved against the member declaring
   its target; prepare builds with `-workspace` (`tests/bsp_workspace.rs`).

### 8.5 Protocol surface & references

Methods: `build/initialize`, `workspace/buildTargets`, `buildTarget/sources`,
`buildTarget/inverseSources`, `buildTarget/didChange`,
`textDocument/sourceKitOptions`, `buildTarget/prepare`,
`workspace/waitForBuildSystemUpdates`.

- sourcekit-lsp Background Indexing: <https://github.com/swiftlang/sourcekit-lsp/blob/main/Contributor%20Documentation/Background%20Indexing.md>
- Swift Forums — extending BSP with SourceKit-LSP: <https://forums.swift.org/t/extending-functionality-of-build-server-protocol-with-sourcekit-lsp/74400>
- xcode-build-server (the "observe" approach): <https://github.com/SolaWing/xcode-build-server>
- Bazel rules_swift / rules_apple (building Apple targets without xcodebuild): <https://github.com/bazelbuild/rules_swift>, <https://github.com/bazelbuild/rules_apple>
- sourcekit-lsp #2328 (external-BSP stdlib rough edge): <https://github.com/swiftlang/sourcekit-lsp/issues/2328>

## 9. Feature coverage matrix

Tracks which Xcode build-system features have a real example in the corpus.
Hand-maintained; re-verify against the generated `fixtures/FIXTURES.md`
after each capture run. Current tally: **115 ✅ / 18 ❌**
(reconciled 2026-05-30 against the captured corpus with concrete evidence per
row; scheme-discovery rows updated 2026-06-10).

Legend: ✅ at least one fixture exercises it (rows marked *hermetic, not
corpus* are covered by tests building the layout in temp dirs — covered, but
with no oracle capture behind them) · ❌ known gap · 🚫 explicitly out of scope
(see §6.1).

**Where pointers:** the per-scheme capture layout repeats per version; paths
below point at the current `xcode-26.5.0` capture (repointed from the dropped
`26.0.1` — re-verify per §9.1 when in doubt).

### Project shapes

| Test case | Status | Where |
|---|---|---|
| Single `.xcodeproj`, no workspace | ✅ | fixtures/ice-cubes/xcode-26.5.0/metadata/list.json, fixtures/netnewswire/xcode-26.5.0/metadata/list.json |
| `.xcworkspace` wrapping one project | ✅ | fixtures/alamofire/xcode-26.5.0/metadata/list.json (workspace=Alamofire), kingfisher |
| `.xcworkspace` wrapping multiple projects | ✅ | fixtures/alamofire/xcode-26.5.0/raw/Alamofire.xcworkspace/contents.xcworkspacedata |
| Nested sub-`.xcodeproj` referenced from a parent project | ✅ | fixtures/alamofire/xcode-26.5.0/raw/Example/iOS Example.xcodeproj/project.pbxproj |
| Swift package as root project (Package.swift only) | ❌ | — (scope decision pending, roadmap D15) |
| Buildable Folders (Xcode 16+ groupless folders) | ✅ | fixtures/tuist-fixtures/xcode-26.5.0/raw/examples_xcode_generated_app_with_buildable_folders/App.xcodeproj |

### Target / product types

| Test case | Status | Where |
|---|---|---|
| iOS app | ✅ | fixtures/ice-cubes/.../schemes/IceCubesApp/build-settings/*iOS-Simulator*.json |
| macOS app | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire/build-settings/Release__macOS.json |
| watchOS WatchKit app | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_watchapp2/schemes/WatchApp |
| tvOS app | ✅ | fixtures/kingfisher/.../schemes/Kingfisher-tvOS-Demo |
| visionOS app | ✅ | fixtures/kingfisher/.../schemes/Kingfisher-Demo (visionOS-Simulator captures) |
| Dynamic framework | ✅ | fixtures/alamofire/.../schemes/Alamofire iOS |
| Static framework | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_static_frameworks |
| Static library (`.a`) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_static_libraries |
| Dynamic library (`.dylib`) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_command_line_tool_with_dynamic_library/schemes/DynamicLib |
| Resource bundle | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_extensions/schemes/Bundle |
| Command-line tool (macOS) | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_command_line_tool_with_dynamic_framework |
| Unit test target | ✅ | fixtures/alamofire/.../schemes/Alamofire iOS Tests |
| UI test target | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_watchapp2/schemes/App-Workspace |
| App extension — share | ✅ | fixtures/ice-cubes/.../IceCubesShareExtension |
| App extension — widget | ✅ | fixtures/ice-cubes/.../IceCubesAppWidgetsExtensionExtension |
| App extension — action | ✅ | fixtures/ice-cubes/.../IceCubesActionExtension |
| App extension — notification service | ✅ | fixtures/ice-cubes/.../IceCubesNotifications |
| App extension — intent | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire iOS Intents Extension |
| XPC service | ❌ | — (roadmap D17) |
| DriverKit driver | ❌ | — |
| Mac Catalyst (iOS app on macOS) | ✅ | fixtures/ice-cubes/xcode-26.5.0/metadata/schemes/IceCubesApp/build-settings/Release__macOS.json |

### Configurations & xcconfig

| Test case | Status | Where |
|---|---|---|
| `Debug` / `Release` configurations | ✅ | fixtures/*/xcode-26.5.0/metadata/schemes/*/build-settings/{Debug,Release}__*.json (hundreds) |
| Custom configuration (e.g. `Profile`) | ✅ | fixtures/_synthetic-custom-config/xcode-*/captures/Scratch__Profile.json (`tests/custom_configuration_oracle.rs`) |
| `.xcconfig` referenced from project | ✅ | fixtures/netnewswire/xcode-26.5.0/raw/NetNewsWire.xcodeproj/project.pbxproj |
| `.xcconfig` includes another (`#include`) | ✅ | fixtures/netnewswire/xcode-26.5.0/raw/xcconfig/*.xcconfig |
| Per-target overrides on top of shared xcconfig | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire-iOS/build-settings |
| Conditional `setting[sdk=*]` | ✅ | fixtures/netnewswire/.../raw/xcconfig/common/NetNewsWire_codesigning_common.xcconfig |
| Conditional `setting[arch=arm64]` | ✅ | fixtures/_synthetic-xcconfigs/.../xcconfigs/conditional-arch.xcconfig |
| Per-configuration override on a single target | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire/build-settings/{Debug,Release}__macOS.json |

### Settings inheritance & substitution

| Test case | Status | Where |
|---|---|---|
| `$(inherited)` propagation | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire/build-settings/Debug__macOS.json |
| `$(SRCROOT)` / `$(PROJECT_DIR)` substitution | ✅ | same |
| `$(TARGET_NAME)`, `$(PRODUCT_NAME)` substitution | ✅ | same |
| `$(BUILT_PRODUCTS_DIR)` cross-target reference | ✅ | corpus-wide (present in 600+ capture files) |
| Recursive substitution (variable referencing variable) | ✅ | fixtures/netnewswire/.../schemes/NetNewsWire/build-settings |
| `${VAR:default=…}` modifier syntax | ✅ | fixtures/_synthetic-xcconfigs/.../modifier-syntax.xcconfig |
| Lower/upper-case `${VAR:lower}` modifiers | ✅ | same |
| Settings with whitespace in values | ✅ | fixtures/netnewswire captures |
| Settings with quotes | ✅ | fixtures/alamofire/.../schemes/Alamofire iOS/build-settings/Release__macOS.json |
| Multi-line settings (backslash continuation) | ✅ | fixtures/_synthetic-xcconfigs/.../multi-line-continuation.xcconfig |

### Schemes

| Test case | Status | Where |
|---|---|---|
| Shared scheme under `xcshareddata/xcschemes/` | ✅ | fixtures/alamofire/.../xcshareddata/xcschemes/Alamofire iOS.xcscheme (and every project) |
| User scheme under `xcuserdata/` | ✅ | *hermetic, not corpus*: `tests/build_settings.rs` (`user_scheme_in_xcuserdata_resolves`) + `src/scheme.rs` discovery tests |
| Autocreated per-target schemes (no `.xcscheme` on disk) | ✅ | *hermetic, not corpus*: `src/workspace.rs` + `src/project.rs` schemeless tests. Autocreation fires even when other scheme files exist; tests, WatchKit extensions, watchapp2 containers, and Safari legacy extensions are excluded |
| Scheme with multiple build entries | ✅ | fixtures/alamofire/.../Alamofire iOS.xcscheme |
| Scheme with pre-action script | ✅ | fixtures/netnewswire/.../NetNewsWire-iOS.xcscheme |
| Scheme with post-action script | ❌ | — (roadmap D18: hermetic) |
| Scheme with environment variables | ✅ | fixtures/alamofire/.../Example/iOS Example.xcodeproj/.../iOS Example.xcscheme |
| Scheme with launch arguments | ❌ | — (roadmap D18: hermetic) |
| Scheme with custom test plan (`.xctestplan`) | ✅ | fixtures/alamofire/.../Alamofire iOS.xcscheme |
| Scheme using parallel testing config | ❌ | — (roadmap D18: hermetic) |
| Scheme overriding `buildImplicitDependencies` | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_app_with_custom_scheme |

### SDKs / platforms

| Test case | Status | Where |
|---|---|---|
| `iphoneos` | ✅ | fixtures/alamofire/.../schemes/iOS Example (no-destination captures) |
| `iphonesimulator` | ✅ | fixtures/alamofire/.../schemes/Alamofire iOS (simulator captures) |
| `macosx` | ✅ | fixtures/alamofire/.../schemes/Alamofire macOS |
| `watchos` / `watchsimulator` | ✅ | fixtures/alamofire/.../schemes/Alamofire watchOS |
| `appletvos` / `appletvsimulator` | ✅ | fixtures/alamofire/.../schemes/Alamofire tvOS |
| `xros` / `xrsimulator` (visionOS) | ✅ | fixtures/ice-cubes/.../schemes/IceCubesApp (visionOS-Simulator captures) |
| Mac Catalyst variant | ✅ | fixtures/ice-cubes/.../schemes/IceCubesApp/build-settings/Debug__macOS.json |
| DriverKit | ❌ | — |

### Architectures

| Test case | Status | Where |
|---|---|---|
| `arm64` (iOS device / Apple Silicon Mac) | ✅ | corpus-wide |
| `x86_64` (older sims) | ✅ | fixtures/ice-cubes/.../IceCubesApp simulator captures |
| `ARCHS_STANDARD` resolution | ✅ | same |
| `EXCLUDED_ARCHS` per platform | ✅ | fixtures/alamofire/.../watchOS Example WatchKit captures |
| `arm64e` | ✅ | fixtures/alamofire/xcode-*/metadata/_synthetic/archs-arm64e (`tests/synthetic_override_oracle.rs`) |
| Universal binary (macOS arm64 + x86_64) | ✅ | fixtures/alamofire/.../Alamofire macOS/build-settings/Release__macOS.json |

### Linking

| Test case | Status | Where |
|---|---|---|
| Embed dynamic framework into app | ✅ | fixtures/tuist-fixtures/.../examples_xcode_generated_ios_app_with_static_frameworks |
| Link static framework (no embed) | ✅ | same fixture, schemes/A |
| Dynamic library link (`.dylib`) | ✅ | fixtures/tuist-fixtures/.../command_line_tool_with_dynamic_library |
| Static library link (`.a`) | ✅ | fixtures/tuist-fixtures/.../ios_app_with_static_libraries |
| `OTHER_LDFLAGS` extra args | ✅ | fixtures/alamofire/.../Alamofire iOS (incl. quoted whitespace: `-framework "My Framework"`) |
| `LD_RUNPATH_SEARCH_PATHS` defaults | ✅ | corpus-wide |
| Mergeable libraries (`MERGEABLE_LIBRARY=YES`) | ✅ | fixtures/alamofire/.../metadata/_synthetic/mergeable-library |
| Link-time optimization (`LLVM_LTO`) | ✅ | fixtures/alamofire/.../metadata/_synthetic/llvm-lto |
| Optional / weak framework link | ❌ | — (roadmap D16) |
| `-framework` vs `-l` flag styles | ❌ | — (roadmap D16) |

### Resources

| Test case | Status | Where |
|---|---|---|
| Asset catalog (`.xcassets`) | ✅ | fixtures/netnewswire raw pbxproj |
| AppIcon set | ✅ | fixtures/ice-cubes per-target captures (`ASSETCATALOG_COMPILER_APPICON_NAME`) |
| `xcstrings` localization (Xcode 15+) | ✅ | fixtures/tuist-fixtures/.../static_framework_with_xcstrings |
| Legacy `.strings` files | ✅ | fixtures/netnewswire raw pbxproj (Intents.strings) |
| Storyboard / XIB | ✅ | fixtures/netnewswire raw pbxproj |
| Core Data `.xcdatamodeld` | ✅ | fixtures/tuist-fixtures/.../ios_app_with_coredata |
| CloudKit schema in `.xcdatamodeld` | ❌ | — |
| Core ML `.mlmodel` | ❌ | — (stays ❌ by design — compile rule, not settings) |
| Metal shader `.metal` | ❌ | — (same) |
| Embedded `.bundle` resource | ❌ | — (same) |
| Privacy manifest `PrivacyInfo.xcprivacy` | ✅ | fixtures/alamofire + kingfisher raw pbxproj |
| Loose files copied via Build Phase | ✅ | fixtures/kingfisher (PBXCopyFilesBuildPhase) |

### Swift specifics

| Test case | Status | Where |
|---|---|---|
| `SWIFT_VERSION` declaration | ✅ | corpus-wide |
| Mixed Swift + ObjC target | ✅ | fixtures/netnewswire per-target captures + corpus/netnewswire |
| ObjC bridging header | ✅ | fixtures/netnewswire (`SWIFT_OBJC_BRIDGING_HEADER`) |
| `BUILD_LIBRARY_FOR_DISTRIBUTION = YES` | ✅ | fixtures/alamofire/.../metadata/_synthetic/library-evolution |
| Strict concurrency (`complete`) | ✅ | fixtures/ice-cubes captures |
| Swift macros | ✅ | fixtures/tuist-fixtures/.../dynamic_frameworks_linking_static_frameworks + `_synthetic-macro` |
| Swift package traits | ✅ | fixtures/tuist-fixtures/.../app_with_local_package_with_traits |
| `@testable import` | ✅ | fixtures/ice-cubes/.../schemes/ModelsTests |
| Custom Swift compiler flags | ✅ | fixtures/netnewswire/.../NetNewsWire Share Extension |

### Build phases / scripts

| Test case | Status | Where |
|---|---|---|
| Compile Sources phase | ✅ | fixtures/alamofire raw pbxproj |
| Copy Bundle Resources phase | ✅ | fixtures/alamofire example app |
| Embed Frameworks phase | ✅ | fixtures/kingfisher + alamofire watchOS example |
| Headers phase (public/project/private) | ❌ | — (roadmap D18: hermetic) |
| Run Script phase | ✅ | fixtures/netnewswire raw pbxproj |
| Run Script with input/output files | ❌ | — (roadmap D18: hermetic) |
| Build rule (custom file extension → command) | ❌ | — (stays ❌ by design) |
| Generated source files (`.intentdefinition` etc.) | ✅ | fixtures/netnewswire raw pbxproj |

### Dependencies

| Test case | Status | Where |
|---|---|---|
| Same-target dependency | ✅ | fixtures/ice-cubes raw pbxproj |
| Cross-project dependency (sub-xcodeproj) | ✅ | fixtures/alamofire example app |
| Workspace cross-project dependency | ✅ | fixtures/alamofire workspace |
| SPM remote dependency | ✅ | fixtures/ice-cubes raw pbxproj |
| SPM local dependency | ✅ | fixtures/tuist-fixtures/.../app_with_local_package_with_traits |
| SPM target `static` / `dynamic` / `auto` linkage | ✅ | corpus/netnewswire RSCore + corpus/ice-cubes Env |
| Binary XCFramework dependency | ✅ | fixtures/tuist-fixtures/.../dynamic_frameworks_linking_static_frameworks |
| System framework (e.g. `UIKit.framework`) | ✅ | fixtures/ice-cubes raw pbxproj |
| Optional framework (weak link) | ❌ | — (roadmap D16) |

### Info.plist & entitlements

| Test case | Status | Where |
|---|---|---|
| Info.plist explicitly listed (`INFOPLIST_FILE`) | ✅ | fixtures/netnewswire captures |
| Info.plist generated from build settings | ✅ | fixtures/ice-cubes/.../schemes/IceCubesApp |
| `.entitlements` file referenced | ✅ | fixtures/netnewswire (`CODE_SIGN_ENTITLEMENTS`) |
| App Groups / iCloud / Push / Keychain entitlements | ✅ | corpus/ice-cubes + corpus/netnewswire entitlements files |

### Tuist-specific shapes

| Test case | Status | Where |
|---|---|---|
| `Project.swift` → `.xcodeproj` generation parity | ✅ | fixtures/tuist-fixtures (16 generated fixtures) |
| `Workspace.swift` workspace generation | ✅ | fixtures/tuist-fixtures/.../ios_app_with_custom_configuration |
| Tuist plugins | ❌ | — (stays ❌ by design) |
| Tuist + remote SPM | ✅ | fixtures/tuist-fixtures/.../ios_app_with_spm_dependencies |

### Phase-2 corpus expansion queue (reconciled)

- 🚫 CocoaPods project — out of scope for the settings oracle (the BSP loop
  has its own `_synthetic-cocoapods` fixture).
- ✅ Mergeable libraries, PrivacyInfo, SPM traits, command-line tool — already
  satisfied (see tables above).
- ◐ Large app with many extensions — every extension product type is already
  covered; another large app would be incremental, not new coverage.
- ❌ Custom build rule, `.metal`/`.mlmodel` resources — structural pbxproj
  features that don't change the resolved settings dictionary (low resolver
  value); stay ❌ by design.

### 9.1 Verification workflow

For any row you doubt:

1. Open the relevant `fixtures/<slug>/xcode-26.5.0/metadata/schemes/<S>/build-settings/<C>__<D>.json`.
2. Grep for the setting the row claims (e.g. `BUILD_LIBRARY_FOR_DISTRIBUTION`).
3. If present with a non-default value, keep ✅ and record the exact path in
   **Where**; if absent, pick a different fixture or downgrade to ❌.

This matrix is the source of truth for "do we have a snapshot of *thing*?" —
update it whenever a fixture is added or dropped.

## 10. Runbook: updating Xcode versions

For the agent asked to **refresh a major to its latest minor** (e.g. `26.0.1`
→ `26.5`) or **add a new major**. A refresh = capture the new minor, then drop
the old one entirely. Last done for the 26.x refresh (commit `ea02daf`) — read
that diff for a worked example. Most of this is autonomous; the only steps a
human must do are flagged **[HUMAN]** — surface them early and wait.

### 10.0 Decide the target version

- Latest non-beta of the major: `xcodes list | grep '^<major>\.' | grep -viE
  'beta|rc'`, take the highest.
- The canonical version string is the **`/Applications/Xcode-<ver>.app` folder
  name** (e.g. `26.5.0`); the numbered scripts resolve `--xcode <ver>` against
  it.

### 10.1 Install — into `/Applications`, NOT `.xcodes/`

```
xcodes install <ver> --no-superuser --experimental-unxip --empty-trash
```

- **[HUMAN, one-time]** `xcodes signin` (one 2FA code) if the session isn't
  cached. Probe first: `scripts/13_capture_version.py --check-auth`.
- **Install to `/Applications`** (the default): `discover_installed_xcodes()`
  only scans `/Applications`. If an app lands elsewhere, `mv` it there (the
  license is version-keyed, not path-keyed, so moving is safe).
- Free disk first if needed (a capture wants ~50–60 GB with runtimes).
  Reclaim: old runtimes (`xcrun simctl runtime delete all`, ~8 GB each) and the
  Xcode app you're about to drop. Settings don't depend on runtime versions.

### 10.2 License + first launch — [HUMAN, sudo] only if newer than the system Xcode

If `<ver>` is **newer** than the system-licensed Xcode (`defaults read
/Library/Preferences/com.apple.dt.Xcode IDEXcodeVersionForAgreedToGMLicense`),
the `--no-superuser` install skipped license + first-launch, and `xcodebuild`
will refuse everything (first a license error, then a
`CoreSimulator`/`IDESimulatorFoundation` plugin-load error). These writes go to
root-owned `/Library` — ask the user to run:

```
sudo DEVELOPER_DIR="/Applications/Xcode-<ver>.app/Contents/Developer" xcodebuild -license accept
sudo DEVELOPER_DIR="/Applications/Xcode-<ver>.app/Contents/Developer" xcodebuild -runFirstLaunch
```

Verify with `DEVELOPER_DIR=… xcodebuild -showsdks`. If `<ver>` is
equal-or-older than the system Xcode, skip — the capture is fully sudo-free.

### 10.3 Provision simulator runtimes (sudo-free)

Per platform the corpus uses (iOS, tvOS, watchOS, visionOS):

```
DEVELOPER_DIR=/Applications/Xcode-<ver>.app/Contents/Developer xcodebuild -downloadPlatform iOS   # ~8 GB each
```

Cycle them (download → capture → `simctl runtime delete`) if disk is tight.
**Boot one device to warm CoreSimulator** before capturing, else
`-showdestinations` races:

```
DEVELOPER_DIR=…/Developer xcrun simctl boot "iPad (A16)"
```

### 10.4 Capture the full corpus

```
python3 scripts/13_capture_version.py --versions <ver> \
  --subset alamofire,kingfisher,ice-cubes,netnewswire,tuist-fixtures \
  --no-runtime --keep --force --min-disk-gb 5
```

- `--no-runtime` skips only the smoke *builds* (03); metadata (02) still
  captures whatever destinations the installed runtimes offer.
- **Verify simulator destinations landed:** `find fixtures -path
  '*xcode-<ver>*build-settings*' -name '*Simulator*' | wc -l` should be large.
  If 0 (CoreSimulator race), re-run 02 directly with the runtimes warm:
  `python3 scripts/02_capture_metadata.py --xcode <ver> --project <slug>
  --force` per project.
- If synthetic overrides (07) produced nothing, run it directly **with
  `DEVELOPER_DIR` exported** (07 does not self-set it).

Confirm every oracle source exists for `<ver>` before dropping the old one:
per-target, project-defaults, scheme build-settings (incl. simulators),
`_synthetic/` (07), `_synthetic-xcconfigs/` (11), `_global/` (08),
`_xcconfig_resolution` (10), and `xcspec-cache/xcode-<ver>/`.

### 10.5 Triage & fix

```
cargo test                                                                   # new version gets the structural>=98 guard
ORACLE_ONLY_VERSION=<ver> cargo test --test per_target_oracle -- --nocapture # systematic tally
DEBUG_DIFF_KEY=<KEY> ORACLE_ONLY_VERSION=<ver> cargo test --test per_target_oracle -- --nocapture
python3 scripts/14_compare_versions.py <old> <ver>                           # what changed across the bump
```

Per-target is the cleanest oracle. Ground every fix per §3.3. Expect mostly
version-echo deltas (deployment targets, new setting keys). Check the
version-specific hardcodes (§6.2, e.g. `SWIFT_EMIT_CONST_VALUE_PROTOCOLS`).

### 10.6 Codify per-version floors

Each oracle has a `version_floor(version)`. Add/rename the arm for `<ver>`
from the observed numbers minus ~1 pt (structural floored at 98; lower only
with a documented irreducible). Harvest the numbers from the printed
`[<oracle> <ver>] exact=.. canon=.. struct=..` lines.

### 10.7 Drop the old version (when refreshing a major)

```
git rm -r fixtures/*/xcode-<old> xcspec-cache/xcode-<old>
rm -rf fixtures/*/xcode-<old> xcspec-cache/xcode-<old>     # untracked artifacts
```

Then repoint every hardcoded `<old>` reference — `cargo test` failures
pinpoint each:

- Unit tests: `src/{project,build_context,xcspec,workspace,bplist,scheme}.rs`.
- Integration tests: `tests/*.rs` (NOT `tests/common/mod.rs` — its version refs
  are canonicalizer *test data*, version-agnostic).
- Floor tables in all oracles (§10.6).
- `src/bplist.rs`: `CanonicalName == "macosx<NN.N>"` → new macOS SDK version.
- `tests/xcspec.rs`: `macosx<NN.N>` (twice) + the xcspec file-count comment.
- `tests/scheme_planner.rs`: the oracle filename's destination slug
  (`OS<NN.N>_iPad-A16` → the new simulator OS).

`grep -rn 'xcode-<old>\|macosx<old SDK>\|OS<old sim OS>' src/ tests/` should
come back clean except `tests/common/mod.rs` test data. Also re-run
`scripts/05_validate.py` / `06_audit_coverage.py` so the generated
`fixtures/FIXTURES.md` matches the corpus (run them on the capture host —
the corpus-tree probes need the `corpus/` clones), and update §9's Where
pointers.

### 10.7b Refresh the embedded defaults catalog

`build-settings` with no `--xcspec-root` resolves against a catalog baked into
the binary (`src/catalog_embedded.bin`) tracking the **newest** captured
Xcode. When adding a new major or bumping the latest minor:

```
# point DEFAULT_VERSION in examples/gen_embedded_catalog.rs at the new version, then:
cargo run --release --example gen_embedded_catalog
git add src/catalog_embedded.bin
```

(No need when only refreshing an *older* major.)

### 10.8 Green, docs, commit

- `cargo test` (all versions green), `cargo fmt`, `cargo clippy --tests`.
- Update §5.2's table and §12's history in this file.
- **[HUMAN approval]** Show the commit message and wait (per the user's commit
  rules: terse, no co-author, no test plan, don't push unless asked).

### 10.9 Cleanup

```
xcrun simctl shutdown all; xcrun simctl runtime delete all   # this run's runtimes
rm -rf /Applications/Xcode-<old>.app                         # the replaced Xcode
```

Keep the new Xcode app + the other majors' apps if still capturing.

### Gotchas (all hit during the 26.5 refresh; fixes are in the repo)

- **Newer-than-system Xcode ⇒ two sudo steps** (license + runFirstLaunch) —
  the only non-autonomous blocker. Equal-or-older ⇒ fully sudo-free.
- **Install to `/Applications`** — sub-scripts don't see `.xcodes/`.
- **`-showdestinations` simulator race** — intermittently omits concrete
  simulators (lazy CoreSimulator device creation). Mitigated by
  `augment_with_simulators()` in 02; still boot a device first and run 02 with
  `--force`.
- **Orchestrator `--force`** forwards to the numbered sub-scripts (was a
  silent no-op before `ea02daf`).
- **07 doesn't self-set `DEVELOPER_DIR`** — export it when running directly.
- **`--no-runtime`** only skips the builds; 02 still captures simulator
  destinations if runtimes are installed.

## 11. Roadmap & open work

### 11.1 Correctness roadmap to 100%

The prioritized queue for closing the remaining gap to `xcodebuild`.
Re-measure (`cargo test --release --test <oracle> -- --nocapture`) before
acting and re-rank if numbers moved. **Definition of done:** structural 100% +
an empty systematic-mismatch tally on every oracle, every version.
Exact/canonical are geometry-capped (§5.3) — canonical is the cross-machine
ceiling; byte-exact 100% across machines is not a meaningful goal.

**Track A — settings: the last 2 keys (highest value, small)**

1. **Close `CLANG_COVERAGE_MAPPING` ×2.** Extend the raw-input copy list in
   `scripts/02_capture_metadata.py` to include scheme-referenced `.xctestplan`
   files (the 5 plans exist under `corpus/alamofire/`, never copied to
   `raw/`), re-capture alamofire 26.5 raw inputs, then derive
   `CLANG_COVERAGE_MAPPING` from the plan's `codeCoverage` flag during scheme
   resolution. *Acceptance:* corpus tally empty; 26.5 structural floor → 100.
2. **Audit the skipped corpus captures** ("target/project lookup failed" in
   `tests/corpus_oracle.rs` + per-target skips). Every silent skip is unscored
   surface. Fix the lookup or move to an explicit documented-skip list printed
   in test output. *Acceptance:* 0 skips, or every skip named + justified.
3. **Capture `IceCubesApp.xcconfig`** — the baseConfiguration ice-cubes
   references that was never copied into `raw/` (the lone documented
   per-source skip). Same capture-script family as A1.

**Track B — geometry closure (canonical ceiling)** — ~~B4 `CCHROOT`~~ and
~~B5 tuist capture-root drift~~ **done in `51bb938`** (§6.3). Remaining:

6. Re-measure and ratchet every canonical floor; document whatever residue
   remains (should be only true per-machine paths).

**Track C — compiler-args: itemized structural gaps**
(visible in `tests/compiler_args_oracle.rs -- --nocapture`; fix → re-run all
versions → raise that `(version, platform)` floor)

7. **visionOS coverage family** (xros swift 94 / clang 95 / link 92): missing
   `-profile-generate`, `-profile-coverage-mapping`, `-fcoverage-mapping`,
   `-fprofile-instr-generate`, `-Xlinker`. Same root cause as A1 — the
   scheme's test plan enables coverage and we never see it.
8. **`.mm` sources dropped from clang compiles** (`util.mm` missing in
   `_synthetic-rich`/`_synthetic-staticlib`): enumeration or language-gating
   bug — investigate `project::target_source_files` / the `FileTypes` gate.
9. **`<Product>_vers.o` missing from links** (~8 cells):
   `VERSIONING_SYSTEM = apple-generic` generates a `<Product>_vers.c` compile
   + link object — model it (pure function of `CURRENT_PROJECT_VERSION` +
   product name) or classify as geometry; decide once, apply everywhere.
10. **Test-bundle clang `-iframework <Platform>/Developer/Library/Frameworks`**
    (×1 per version): port the existing swift rule to `clang_arguments`.
11. **Version gates:** `-explicit-module-build` emitted on 15.4/16.4 (gate to
    ≥26); 15.4 clang missing `-g`/`-gmodules`; 15.4 swift missing `-F` ×4.
12. **Confident-wrong extras tail:** `-fexceptions`
    (`GCC_ENABLE_EXCEPTIONS`), `-fvisibility=hidden`
    (`GCC_SYMBOLS_PRIVATE_EXTERN`), the one-target `-W(no-)shorten-64-to-32`
    flip. Ground the true defaults in the xcspec; if genuinely underivable,
    document per-flag and exclude from precision.
13. **`-target` triple mismatches** (26.5 macosx link ×1, 26.5 watchos clang
    ×1): diff the literal triples and fix derivation.
14. **Autolinked `-framework` recall**: decide formally — out-of-scope
    (recommended: BSP never links) and document, or model an import-scan
    heuristic. Stop carrying it as an open item either way.

**Track D — corpus expansion for the remaining ❌ rows (ranked)**

15. **Swift package as root** (`Package.swift`, no xcodeproj) — the biggest
    genuinely uncovered real-world surface (41 synthesized package schemes
    tallied by the discovery oracle). A *scope decision* first: if the BSP
    server should serve SPM-root repos, add a synthetic fixture + extend
    discovery/resolution to manifests; if not, record 🚫 with rationale.
16. **Weak/optional framework link + `-framework` vs `-l` styles** (3 ❌ rows,
    one fixture): a synthetic project with a `Weak` ATTRIBUTES entry + a
    `-l`-style link (`scripts/17_static_library.py` is the template).
17. **XPC service** (and DriverKit if cheap): distinct product-type xcspec
    domains; a synthetic macOS XPC target needs no runtime.
18. **Scheme post-actions / launch args / parallel testing + headers phase +
    run-script I/O** — structure-only; cover hermetically in
    `src/scheme.rs`/`tests/xcscheme.rs` parse tests; flip to ✅ *(hermetic)*.
19. Core ML / Metal / `.bundle` / custom build rules / CloudKit / Tuist
    plugins — stay ❌/🚫 (compile rules, not settings).

**Track E — harness hardening**

20. **Interrogate PIF dumps for `ENABLE_DEBUG_DYLIB`** before accepting it as
    irreducible forever — one bounded investigation, then model it or close
    the question permanently.
21. **Wake cheap dormant sources:** `_global/.../sdks/*.json` → a small SDK
    discovery test; the version banner → trivial assert. Retire the dead
    `dry-run/` captures (superseded by the compiler-args oracle).
22. **Extend the mutation audit** with one row per rule added in Tracks A/C so
    every new rule has a net that goes red.
23. **Ratchet floors after every fix.**
24. ~~Refresh the generated reports~~ — done: consolidated into the
    regenerated `fixtures/FIXTURES.md` (current against 26.5/16.4/15.4).
    Remaining sliver: the corpus-tree probes are carried forward as stale;
    re-run `06_audit_coverage.py` on a host with the `corpus/` clones to
    refresh them.

**Suggested execution order:** A1 (+C7, same root cause) → A2/A3 → C8–C13 →
B6 → D16 → D15 scope decision → E20–E24 interleaved. Mac-host capture steps
(A1, A3, D16, D17) need a macOS machine with the corpus Xcodes; everything
else runs anywhere against committed fixtures.

### 11.2 Audit follow-ups (June 2026)

A full library audit (line references against `54c40a1`) landed with commit
`51bb938`, which **fixed all P0 and P1 findings except P0.3**:

- ~~P0.1~~ BSP startup from the extension's `bsp.json` (workspacePath only
  counts when it names an `.xcworkspace`).
- ~~P0.2~~ `buildServer.json` regeneration when the launcher path goes stale +
  doctor check + build-time version constant.
- **P0.3 — STILL OPEN:** extension activation crash on non-macOS hosts.
  `@sweetpad/lib` is imported at module top level
  (`src/common/cli/scripts.ts`, `src/build/utils.ts`), reachable from
  `extension.ts`'s import graph; the VSIX is platform-universal but ships only
  darwin `.node` binaries, so on Linux/Windows (Remote-SSH included) the napi
  loader throws at `require` time and the extension fails to activate.
  **Fix:** publish darwin-targeted VSIXs (`vsce package --target darwin-arm64
  darwin-x64`), or lazy-load the addon behind a "macOS only" guard.
- ~~P0.4~~ Parser/resolver crash hardening: bplist materialization budget +
  size-int rejection, pbxproj/xcscheme depth limits, pbxproj_writer cycle
  bound, TestTargetID host product-type check, variable-expansion output
  budget, `resolve_group_paths` visited set, condition-parser depth cap, BSP
  16 MiB frame cap, telemetry write timeouts. Pinned by
  `tests/adversarial_inputs.rs` (a `cargo-fuzz` target over the parsers would
  lock this in long-term — still open as an idea).
- ~~P1.1–P1.10~~ Correctness divergences: mismatch clusters
  (CCHROOT/sanitizer object dirs/tuist roots), the `AuthoredProbe` unification
  of the three shadow resolvers, probes honoring `-xcconfig`/CLI overlays
  (with two capture-proven carve-outs documented in place),
  `arch=undefined_arch` binding on the showBuildSettings-emulation path (KASAN
  special case deleted), quote-aware passthrough splitting, `links()`
  denylist, host-arch editor default, fresh target snapshots after
  `didChange`, JSON-RPC robustness, ASSETCATALOG filter suppression,
  `SWIFT_EMIT_CONST_VALUE_PROTOCOLS` version gate, distinct workspace error
  variants, serde_json for meta.json, bplist size-int, DerivedData hashing
  as-opened.

**P2 — CI, packaging, release pipeline (open):**

- The `node` feature is never compiled before release (CI runs default
  features; `src/node.rs` first compiles on the tag-triggered release build).
  Add `cargo clippy --no-default-features --features node -- -D warnings` or a
  `napi build` smoke step; switch CI clippy to `--all-targets`.
- No PR/push CI for the extension (`ci.yaml` triggers only on tags) — add a PR
  workflow on `macos-latest`: `npm ci && npm run check:all && npm test && npm
  run build`.
- Embedded-catalog staleness unguarded: no test calls
  `catalog_cache::embedded()`; bumping `FORMAT_VERSION` or refreshing
  `xcspec-cache/` without regenerating ships stale defaults with green CI. Add
  a byte-equality test against a fresh serialize; default
  `examples/gen_embedded_catalog.rs` to the newest `xcspec-cache/xcode-*`
  instead of a hardcoded version.
- Stale universal `.node` shadows fresh debug builds
  (`rolldown.config.mjs` prefers any lingering `*universal*.node`): delete
  `sweetpad-lib/*.node` before debug builds or pick by newest mtime.
- Version single-sourcing: Cargo.toml `0.1.0` vs napi package.json `0.1.1`;
  have the bsp doctor compare extension↔addon versions.

**P3 — architecture & maintainability (open):**

- **Split `project.rs` (~4,500 lines)** along its measured seams:
  `project/mod.rs` (model/open/scheme autocreation),
  `project/settings_layers.rs`, `project/graph.rs`, `project/builtins.rs`
  (`built_in_settings` ~1,070-line fn + `built_in_overrides` ~400),
  `project/platform.rs` (platform/arch/version tables, host detection,
  Catalyst — the Xcode-version-rot surface, with an "update on Xcode bump"
  note pointing at §10), `scheme_for_target` → `scheme.rs`. Known layering
  wrinkle: discovery calls down into settings resolution via
  `is_safari_extension_target`.
- **Parameter-struct the builtin entry points:** `built_in_overrides` takes 22
  positional parameters, `built_in_settings` 16 — transposing two adjacent
  bools compiles silently. A `struct BuiltinInputs` with named fields; a named
  `UserLayers` struct for `BuildSettingsContext.layers`.
- **Deduplicate parser scaffolding:** the `Parser { input, pos }` byte-cursor
  is copy-pasted between `pbxproj.rs` and `xcscheme.rs`;
  `split_conditional_key` re-implements xcconfig's bracket-condition parser
  with different failure behavior; `parse_flags` exists twice with divergent
  trailing-flag semantics; `STRIP_FLAGS` vs `MODERN_DRIVER_DEFAULTS` are
  parallel hand-maintained tables (add a test asserting every driver default
  is stripped or explicitly allowlisted).
- **Structured errors at the public boundary:**
  `resolve_build_settings`/`resolve_compiler_arguments`/`resolve_file_arguments`
  return `Result<_, String>`; callers can't distinguish "unknown target" from
  IO failure. An error enum wrapping inner errors; an `ErrorKind` on parser
  errors (matters more now that resource-limit errors exist).

**P4 — performance (long-lived process, per-keystroke BSP queries; open):**

- `Catalog::layer_for` clones `self.universal` per query — memoize per
  `(product_type, sdk)` as `Arc<Vec<Assignment>>`.
- `expand_variables` clones the full merged map per fixed-point pass (up to 16
  × ~1.4k entries), and conditional assignments double the pipeline via the
  two-pass resolve — track a dirty-key set; reuse pass 1's reduced layers.
- `parent_group_of` is O(all objects) per group level — build a child→parent
  index once per parsed pbxproj.
- Filesystem rescans per resolve (`fs::canonicalize`,
  `find_derived_data_container` double `read_dir`, `darwin_user_cache_dir`) —
  cache at `BuildContext::open`. (`find_derived_data_container` also picks the
  lexicographically first container on collision — document or fix.)
- `source_kit_options` resolves twice per request (probe + real) — cache the
  probe per target.
- `decode_entities` is O(n²) on entity-dense text — cap the `;` window.
- N-API calls are sync on the extension-host event loop; the first call also
  builds the catalog — expose async variants (`AsyncTask`).

**P5 — caching & BSP lifecycle robustness (open):**

- Catalog disk-cache writes are non-atomic on a path shared by two processes —
  write temp + `rename`.
- `file_cache` stamp is `(len, mtime)` — fold in inode/ctime.
- Process-lifetime caches never evict; disk `catalog-*.bin` never GC'd — wire
  into `xcode::flush_caches()`; prune on write.
- Server exit mid-prepare orphans the spawned `xcodebuild`; `$/cancelRequest`
  is ignored — keep the child handle, kill on shutdown.
- No lifecycle gating: requests served before `build/initialize` / after
  `build/shutdown` — BSP expects `-32002`-style errors.
- The change watcher polls only member `project.pbxproj` files — watch
  `contents.xcworkspacedata` too; `LiveConfig.scheme` is diffed but never used
  by `options_for` (refresh storms with no behavioral change) — use it or stop
  diffing it.
- `xcode::locate_uncached` probes a relative path when `parent()` is empty.
- pbxproj `\U` escapes don't combine surrogate pairs; xcconfig block comments
  spanning lines drop surrounding same-line text — verify against Apple's
  whitespace-collapse rule.

**Suggested order of attack:** P2 CI items (cheap, prevents release breakage)
→ P0.3 packaging → §11.1 Track A/C → P3.1/3.2 project.rs split + parameter
structs (best before more corpus-derived rules accrete; protected by the green
oracle) → P4/P5 as independently shippable background tasks.

## 12. Project history

Condensed log of how the library got here; each entry's full technical detail
lives in the sections above and in the named commits.

- **Phase 1 — corpus collection (2026-05).** Python/shell capture pipeline
  (scripts 00–05) over 5 OSS projects × Xcode versions: metadata, raw inputs,
  smoke builds, xcspec snapshots. Locked: simulator-only unsigned builds,
  in-repo fixtures, no Rust.
- **Phase 2 — Rust scaffold + pbxproj parser.** Single crate `sweetpad`
  (lib + binary), edition 2024, pinned toolchain, `MIT OR Apache-2.0`,
  clippy-pedantic, no initial dependencies. First isolated unit: the OpenStep
  pbxproj parser, fixture-driven.
- **Phase 3 — settings resolver.** Precedence layers, xcconfig inheritance,
  xcspec/SDKSettings defaults; `tests/corpus_oracle.rs` scoring every capture
  in three tiers (§5.3). Scope record in §6.1–6.2.
- **2026-05-30 — multi-version capture.** Sudo-free `DEVELOPER_DIR`-based
  orchestrator (`13_capture_version.py`), disk-bounded one-Xcode-at-a-time.
  **16.4** captured → exposed the `XCODE_VERSION_MAJOR` keystone bug (nested
  `$(…_XCODE_$(XCODE_VERSION_MAJOR))` recipes; fixed +450 exact matches on
  26.x too) and five more version-gated rules. Per-version data-driven floors
  replaced the blended floor. **15.4** captured → two cross-cutting
  undomained-xcspec parser fixes (`PACKAGE_TYPE` clobber: a definition
  carrying `PackageTypes` is authoritative; `BUNDLE_FORMAT` domain inferred
  from spec filename when `_Domain` is absent), both no-ops on 16+/26.
  **DEVELOPER_DIR keystone fix**: the resolver had built `DEVELOPER_DIR` (and
  `TOOLCHAIN_DIR`, `OTHER_LDFLAGS` ×300…) from the host's Xcode instead of the
  catalog's — `Catalog` now reads `developer_dir` from `meta.json`.
  Cross-version delta tool (`14_compare_versions.py`); its auto-capture
  `--delta` mode intentionally not built (no caller under the
  latest-minor-per-major policy). **26.x refreshed 26.0.1 → 26.5** (full
  corpus, all simulator platforms, 568 scheme captures; 26.0.1 dropped);
  surfaced the newer-than-system sudo steps, the `-showdestinations` race fix,
  and the orchestrator `--force` forwarding (commit `ea02daf`).
- **2026-06 — compiler-args layer.** Stdout-sourced oracle (Xcode 26 killed
  `-dry-run` and the activity log), argv comparator with precision+recall,
  swift/clang/link generators routed through xcspec `CommandLine*` encodings
  with `Condition` gating, synthetic staticlib/rich fixtures, napi
  `compilerArguments`. §7.
- **2026-06 — BSP server v1→v3.** Walking skeleton → per-file args →
  background-indexing prepare (xcodebuild-by-scheme) → self-built Swift fast
  path (~1s vs ~5s). Four-net measurement loop (invariant linter,
  de-exonerated corpus run, live differential, mutation audit); the
  `SDKROOT = auto` editor-binding bug — which had slipped all three original
  layers — drove the nets' design. Full-corpus run: 138/138 sampled files
  clean. §8.
- **2026-06-10/11 — discovery + roadmap.** Scheme discovery (user schemes,
  autocreated per-target schemes — hermetic tests); `tests/discovery_oracle.rs`
  at 100% exact against 30 `-list` captures (deriving the autocreation
  exclusion rules); link oracle to 100% structural+precision on 25/27 dynamic
  links; the measured "roadmap to 100%" written (§11.1).
- **2026-06-12 — audit + P0/P1 fixes (`51bb938`).** Full library audit; all
  P0/P1 findings fixed except P0.3 (extension packaging): BSP startup/config
  bugs, parser/resolver crash hardening (+ `tests/adversarial_inputs.rs`),
  authored-probe gating unification (KASAN case deleted), quote-aware flag
  splitting, geometry closures (CCHROOT, sanitizer object dirs, tuist capture
  roots). Corpus oracle now 89–88 exact / 97–100 canonical / 99–100 structural
  per version with a single remaining systematic mismatch (§6.3). Open items
  consolidated into §11.2.
