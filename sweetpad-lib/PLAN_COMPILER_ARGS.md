# sweetpad-lib — Compiler Argument Resolution

## Goal

Given a resolved build-settings context (the existing `BuildContext::resolve`
output) plus the target's inputs, produce the exact compiler/linker argument
vectors `xcodebuild` would invoke per target — `swiftc`, `clang`/`clang++`,
`ld`/`libtool`, and the asset/resource tools — and validate them against the
**literal commands captured from a real build** (the oracle). Same
fixture-driven, snapshot-oracle discipline as the settings resolver in
`PLAN.md`.

This is the layer directly above settings resolution:

```
pbxproj / xcconfig / xcspec
  → [settings resolver: DONE]   → resolved settings dict
  → [argument resolver: HERE]   → per-tool argv
```

The settings resolver answers "what is `SWIFT_OPTIMIZATION_LEVEL`?"; this layer
answers "what is the full `swiftc …` command line for this target, byte for
byte?"

## Why a new oracle (and not dry-run)

`xcodebuild -showBuildSettings` gives values, not the assembled command line.
The Phase-1 capture used `xcodebuild -dry-run` for command lines, but:

- **`-dry-run` was removed in Xcode 26.** 632 of the 654 committed
  `dry-run/*.txt` are just `error: option '-dry-run' is no longer supported` +
  usage. Only 22 (Xcode 15.4, a few 16.4 macOS) hold real compiler lines.
- The PATH-wrapped toolshim's `tool-invocations.jsonl` is **empty for the main
  compilers** — `xcodebuild` invokes `swiftc`/`clang`/`ld` by absolute toolchain
  path, bypassing the PATH shim.
- **Xcode 26's command-line `xcodebuild build` no longer persists a
  `.xcactivitylog`** — the DerivedData `Logs/Build/LogStoreManifest.plist` comes
  back with an empty `logs` dict, so XCLogParser has nothing to read. (Verified
  on the Alamofire macOS pilot, Xcode 26.5.)

So ground truth must come from a **real build**, and the source that survives is
`xcodebuild`'s **stdout**: it echoes every command it runs verbatim — one
command per line, under a `<Phase> … (in target 'T' from project 'P')` header
whose body is indented `cd` / `export` / the tool invocation (the Swift driver
appears as `builtin-SwiftDriver -- /…/swiftc …`). We parse those blocks,
shell-tokenize each command, and expand its response files. That is the literal
argv that executed — the most correct oracle possible, and the only source that
uniformly covers every tool (including the linker, which an index-settings query
omits). `-showBuildSettingsForIndex` was evaluated and dropped: it is the
index/AST args (a near-twin of build args, no linker, no asset tools), strictly
weaker than the build itself.

## Locked decisions

| Decision | Value |
|---|---|
| Oracle source | **Real build** → parse `xcodebuild` **stdout** (Xcode 26 drops the `.xcactivitylog`) → per-tool argv, response files expanded |
| Committed artifact | One JSON per `(scheme, config, dest)` with a `targets` array; per-target **per-tool argv only**, raw (un-canonicalized) values; clang stored as `{commonArguments, files}` to avoid per-file duplication |
| Raw build logs | **Gitignored** working dir during capture (`.derived/`); never committed |
| Generation approach | **Hybrid** — hand-code the obvious mappings to a green loop fast, then back them with xcspec `CommandLine*` parsing |
| Tool scope | **All tools, all flags** is the goal; oracle captures everything from day 1; generator grows tool-by-tool, order `swiftc → clang → ld/libtool → asset/resource tools` |
| Generation unit | Swift = one module invocation per target; clang = per-target flags + per-file I/O; link = per-target |
| Scoring | exact → canonical → structural tiers, per-version floors, per-flag systematic-mismatch tally; **judge by structural %**; geometry tokens out-of-scope-to-match; document irreducibles |
| Lives in | Rust `sweetpad-lib` (new module + new oracle test); consumes `BuildContext::resolve` output |
| Public API | `compilerArguments(options) → Array<TargetCompilerArguments>` (napi, mirrors `buildSettings`) |
| Pilot | Alamofire, `Alamofire macOS` scheme, Debug, **Xcode 26.5** (pure-Swift framework, no runtime, no signing) |
| Expansion order | Kingfisher target → NetNewsWire `RSDatabaseObjC` (clang/ObjC) → an app target (ld + actool) → full 5 × 3 matrix |
| Non-settings inputs | Permitted (source-file lists from pbxproj, etc.) |

## The oracle: capture

- Real `xcodebuild build`, one `(scheme, config, destination)` per capture,
  under the matching `DEVELOPER_DIR`, with `CODE_SIGNING_ALLOWED=NO` and a
  dedicated `-derivedDataPath` (removed first, so the log is a full build, not
  "up-to-date" skips).
- **Source of truth: `xcodebuild` stdout.** Split it into
  `<Phase> … (in target 'T' …)` blocks; from each block take the indented tool
  command (stripping the `builtin-SwiftDriver -- ` driver wrapper),
  shell-tokenize, and **expand**:
  - `@<file>` response files (`*-linker-args.resp`) spliced inline,
  - `@*.SwiftFileList` → the Swift `inputFiles` list (these exist in DerivedData
    post-build),
  - `-filelist <file>` (the object list) and `-output-file-map <file>` are left
    as raw geometry — recorded, not scored (see below).
- **Committed artifact:**
  `fixtures/<slug>/xcode-<ver>/compiler-args/<scheme>__<config>__<dest>.json` —
  a single object `{ slug, xcode_version, scheme, configuration, destination,
  sdk, arch, targets: [...] }`. One file per `(scheme, config, dest)`, every
  target the build emitted commands for in the `targets` array (mirrors the
  build-settings oracle, whose JSON is an array of per-target entries).
  - per-target argv, **verbatim/raw values** (real paths, real DerivedData
    hashes) — exactly like the committed `-showBuildSettings` JSON is raw and
    the test canonicalizes. No canonicalization at capture.
  - `swift`: one `{ arguments, inputFiles }` (`arguments` drops argv[0] and the
    input-file-list token).
  - `clang`: `{ commonArguments, files: [{ file, extraArguments }] }` — the
    shared flag set stored once, per-file deltas only (keeps app targets with
    hundreds of `.m` files compact).
  - `link`: `{ tool, arguments }` (`tool` = the driver basename, e.g. `clang`).
- The build's stdout/stderr and DerivedData stay in a **gitignored** working dir
  (`corpus/<slug>/.work/`); only the extracted argv JSON is committed.
- Script: `scripts/16_capture_compiler_args.py`.

## The oracle: scoring

- Normalize argv into a canonical model: standalone flags + `(flag, value)`
  pairs. Repeatable, order-insignificant families (`-I`, `-F`, `-D`, `-Xcc`,
  `-l`, `-L`, `-framework`) compared as **multisets**; the few order-significant
  tokens compared positionally.
- Three tiers per token, identical philosophy to the settings comparator:
  **exact** (byte-equal) → **canonical** (equal after `canonicalize_value`
  strips `$HOME`/DerivedData-hash/Xcode-dev/SDK-version/project-root drift —
  reused as-is; it already handles paths embedded in flags like
  `-fmodule-map-file=` and `-load-plugin-executable`) → **structural** (both
  absolute paths). Per-version floors; judged by **structural %**.
- **Precision + recall:** score both **missing** (oracle has, we don't) and
  **extra** (we emit, oracle doesn't) tokens — both are real defects — in a
  per-flag systematic-mismatch tally split by direction.
- **Out-of-scope-to-match** (recorded, not scored — pure build geometry):
  `-o` and other output paths (`-emit-module-path`, `-emit-objc-header-path`,
  `-output-file-map`), `-index-store-path`, `-serialize-diagnostics`, dependency
  files (`-MF`/`-MT`/`-MD`), and the filelist path itself. Analogous to the
  settings layer's path-anchored keys.
- Lives in `tests/common/` (extend) + `tests/compiler_args_oracle.rs`.

## Phases

Each phase has a validation gate; later phases do **not** start until the pilot
(0–4) is proven green, per the repo's "build in isolated parts" rule.

### Phase 0 — Capture the pilot oracle (no Rust)

Build `Alamofire macOS` Debug under Xcode 26.5; extract + expand the `swiftc`
(and incidental `ld`/`libtool`) invocations; commit the raw argv oracle.

- Scoped clone: `corpus/alamofire` at the pinned SHA from `corpus/manifest.json`.
- Write `scripts/16_capture_compiler_args.py`; produce
  `fixtures/alamofire/xcode-26.5.0/compiler-args/Alamofire-macOS__Debug__macOS.json`.

_Validation:_ the committed JSON contains a complete, response-file-expanded
`swiftc` invocation for the Alamofire framework target (module name, sdk,
search paths, `-D`s, the full `.swift` input list) plus the link invocation. No
Rust yet.

### Phase 1 — argv comparator (Rust, no generator)

Build the scoring core in `tests/common`: tokenization, flag-family
normalization, multiset + positional comparison, three tiers reusing
`canonicalize_value`, geometry classification, precision/recall tally, `Stats`.

_Validation:_ comparator unit-tested (identity → 100%; injected
missing/extra/geometry tokens classify correctly); the Phase-0 oracle loads and
self-scores 100%.

### Phase 2 — per-target source files (`project.rs`)

Extend `project.rs` to resolve a native target's `PBXSourcesBuildPhase` →
ordered absolute source paths (handle `PBXGroup` nesting, `sourceTree` variants,
variant groups). This is new — today `project.rs` only resolves xcconfig file
refs.

_Validation:_ Alamofire's target yields exactly its `.swift` files; the set
equals the oracle's expanded `SwiftFileList`.

### Phase 3 — Swift generator (hand-coded core)

Generate the `swiftc` module argv from `Resolved.settings` + the source list:
`-module-name`, optimization (`-Onone`/`-O`/`-Osize`), `-swift-version`, `-sdk`,
`-target`, compilation mode (incremental vs `-wmo`), `-D` from
`SWIFT_ACTIVE_COMPILATION_CONDITIONS`, `-I`/`-F`/search paths,
`OTHER_SWIFT_FLAGS` passthrough, the input list. Score against the Phase-0
oracle and iterate.

_Validation:_ Alamofire macOS `swiftc` hits a codified structural floor; the
systematic-mismatch tally (both directions) is documented — every miss is a
known irreducible (geometry) or a tracked gap.

### Phase 4 — xcspec command-line ingest (`xcspec.rs`)

Extend `xcspec.rs` to parse the compiler specs' `CommandLineArgs` /
`CommandLineFlag` / `CommandLinePrefixFlag` / `Values` for `swiftc`, and route
generation through the spec where it is the cleaner source (replacing
hand-coded mappings from Phase 3). The spec files are already cached under
`xcspec-cache/xcode-<ver>/`.

_Validation:_ swiftc generation is spec-driven for the parsed option set;
Alamofire stays at/above its Phase-3 floor (no regression).

### Phase 5 — Expand corpus + clang + linker (gated on a green pilot)

In order, capturing each oracle and growing the generator:

1. **Kingfisher** framework target (SwiftPM in the graph).
2. **NetNewsWire `RSDatabaseObjC`** — brings in `clang` (per-file ObjC,
   modulemaps, PCH).
3. **An app target** — brings in `ld` linking + `actool`/resource tools.
4. The full **5 projects × 3 Xcode versions** matrix (provision simulator
   runtimes via `xcodebuild -downloadPlatform` for non-macOS destinations).

_Validation:_ each new shape scored against its oracle with codified floors; the
per-flag tally always shows which tools/flags are not yet generated (no silent
gaps).

### Phase 6 — Public API

Add `#[napi] compiler_arguments(options) -> Vec<TargetCompilerArguments>` in
`node.rs` (mirrors `build_settings`), regenerate `index.d.ts`.

_Validation:_ callable from TS; returns per-target `swift`/`clang`/`link` argv.

## Out of scope (initially)

- Per-primary-file `swiftc` **frontend** jobs (`-primary-file …`) — driver-
  internal, never issued by Xcode, no consumer needs them.
- Matching pure-geometry tokens (output paths, index-store, diagnostics, dep
  files) — recorded, not scored.
- Real signing / device builds; archive/Release-signed builds.
- SPM-package-internal target compiles beyond what the app/framework build emits
  (revisit during Phase 5).

## Methodology (inherited from `PLAN.md` / `CLAUDE.md`)

- Ground every mapping **xcspec → corpus → web**, in that order.
- Minimum abstraction; concrete types and plain functions first.
- Document irreducible build-system heuristics in code rather than over-fitting.
- After every change, re-run the full oracle on **all** captured versions — a
  fix for one version must not regress another.
- Per-version, data-driven floors; correctness judged by structural % + the
  systematic-mismatch tally, never the geometry-capped exact %.

## Status (as built)

Phases 0–4 and 6 are complete. Phase 5 (corpus + clang + link) covers the macOS
oracles — a real ObjC target, a Release app, and the framework dylibs — across
Xcode 15.4 / 16.4 / 26.5, with the non-macOS destination matrix the main
remaining capture work.

- **Phase 0–1:** capture (`scripts/16_capture_compiler_args.py`, stdout-sourced)
  + the argv comparator (`tests/common/argv.rs`): flag-family multiset, three
  tiers reusing `canonicalize_value`, geometry classification, precision/recall.
- **Phase 2:** `project::target_source_files` (PBXSourcesBuildPhase → absolute
  paths through the group tree). Alamofire yields its 43 `.swift` exactly.
- **Phase 3–4:** `compiler_args::swift_arguments`, routing the optimization
  level, active-compilation-conditions, the `SWIFT_UPCOMING_FEATURE_*` /
  `SWIFT_EXPERIMENTAL_FEATURE_*` families, strict-concurrency, and coverage
  through the xcspec command-line encodings (`xcspec.rs` `CompilerOption` /
  `CliArgs`, cached in the catalog), with the computed/build-system flags
  hand-coded. **Alamofire macOS and Kingfisher both score 100 % structural /
  100 % precision** (every semantic flag matches; geometry excluded).
- **Phase 5:** `clang_arguments` and `link_arguments`, scored against real ObjC
  (KingfisherTests' vendored Nocilla `.m`, reached via `--action
  build-for-testing`), a Release app (executable link + whole-module Swift), and
  the framework dylibs. The capture script expands clang `@*.resp` response files
  (like swift/link) and keeps one compile per source, so the oracle carries
  literal flag vectors.
  - clang is **language-gated** against the xcspec `FileTypes` and
    `Architectures` (a C++ `-std`/warning never reaches an ObjC `.m`; an
    Intel-only `-fasm-blocks` never reaches arm64) and emits the per-file `-x
    <dialect>`. **clang: 97 % precision / 95 % structural; the real-ObjC
    KingfisherTests target is 100 % precision.**
  - swift additionally handles whole-module Release builds
    (`-whole-module-optimization` / `-no-emit-module-separately-wmo`) and the
    `-import-objc-header` bridging header. **swift: 100 % precision / 97 %
    structural** (every framework/app target 100 %).
  - link adds the executable/bundle shapes (`-bundle`; the dylib identity +
    version stamps gated to `mh_dylib`) and the modern link-driver defaults
    grounded across every capture (`-Xlinker -reproducible` / `-dead_strip`,
    Debug `-no_deduplicate`, `-fobjc-link-runtime`). **link: 93 % precision /
    72 % structural;** the autolinked `-framework`, `-rdynamic`, the swift-runtime
    toolchain `-L`, and coverage `-fprofile-instr-generate` are the tracked tally
    gaps.
  - **Static library:** a synthetic static-library oracle
    (`scripts/17_static_library.py` → `fixtures/_synthetic-staticlib/`) validates
    the `libtool -static` link — `-static`, `-arch_only`, `-D`, `-syslibroot`, the
    `-L` search paths — at 100 % structural recall. It's a separate generator from
    the clang-driver link, selected by product type / `MACH_O_TYPE`.
  - **Version coverage:** the macOS oracles are captured and scored across
    **Xcode 15.4 / 16.4 / 26.5**, each guarded at its own per-version floor (15.4
    is Kingfisher-only — Alamofire's `.xcodeproj` is a newer format than Xcode
    15.4 will open). The
    Swift driver defaults that turned over at the Xcode 26 explicit-modules cutover
    are gated on the toolchain major (`-enforce-exclusivity=checked` for < 26, the
    libc++ `_LIBCPP_HARDENING_MODE` for ≥ 26), so every version scores swift
    ≥ 97 % structural, clang ≥ 93 %, link ≥ 70 %.
  - _Remaining (mechanical capture + iteration):_ the non-macOS destinations
    (iOS/tvOS/watchOS/visionOS) need simulator runtimes (`xcodebuild
    -downloadPlatform`); and more ObjC/C++ breadth (e.g. NetNewsWire).
- **Phase 6:** `#[napi] compiler_arguments` (`node.rs`) →
  `compiler_args::target_arguments` via `build_settings::resolve_compiler_arguments`;
  the generated `index.d.ts` exposes `compilerArguments(...)` returning per-target
  `swift`/`clang`/`link`. Verified callable from node against the Alamofire fixture.
