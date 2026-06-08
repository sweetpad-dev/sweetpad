# sweetpad-lib → BSP server roadmap

Where the compiler-argument work is heading: a **Build Server Protocol (BSP)**
server for Xcode projects, so `sourcekit-lsp` can drive editor intelligence
(completion, diagnostics, jump-to-definition) in VS Code / Neovim / etc. without
Xcode's editor.

## The chain it plugs into

```
editor ──LSP──► sourcekit-lsp ──BSP──► our build server ──► compiler args ──► SourceKit
```

- **LSP** (Language Server Protocol): editor ↔ `sourcekit-lsp`.
- **BSP** (Build Server Protocol): `sourcekit-lsp` (client) ↔ our build server.
  The server answers: *what targets exist*, *what files per target*, and — the
  core question — **the compiler arguments for this file**.
- **SourceKit**: the engine `sourcekit-lsp` feeds those args to so it can parse +
  type-check the file the same way the compiler would.

## Approach: derive, don't observe

The established tool, [`xcode-build-server`](https://github.com/SolaWing/xcode-build-server),
**observes**: it parses an `xcodebuild` build log to recover the real compile
commands. Accurate, but requires a prior build, goes stale on edits, is slow, and
broke when Xcode 26 stopped persisting the activity log.

We **derive**: compute the compiler args directly from the project + resolved
build settings (what Xcode does internally before it runs anything). No build
required, always fresh, fast, machine-independent — but we must reproduce Xcode's
settings resolution + flag generation exactly. That engine is **built and proven
against real builds** (see `PLAN_COMPILER_ARGS.md`): swift/clang/link arg vectors,
validated across product types, Xcode 15.4/16.4/26.5, and macOS/iOS/tvOS/watchOS/
visionOS, at 90–100 % precision on the semantic flags.

## What BSP needs that the build-oracle de-prioritized

The compiler-args oracle scored *semantic flags* and **excluded geometry**
(search paths, file maps) as machine-specific noise. For BSP the priorities
invert — the editor never links, so:

- **Search paths are load-bearing.** `-I` / `-F` / `-isystem` / module paths are
  how SourceKit finds imported modules; get them wrong and you get "no such
  module" + dead completion. The clang generator currently under-emits these
  (missing `-I`/`-F`/`-iframework`); completing them is the #1 BSP engine task.
- **Per-file, not per-target.** The editor asks file-by-file; a `.mm` needs the
  C++ dialect/flags, a `.m` must not.
- **Link / autolink is irrelevant** to the editor (no linking happens). The one
  "structural gap" from the compiler-args work (autolinked `-framework`s) does
  **not** matter for BSP.

## The cross-module "outputs must exist" problem

Intra-module completion is live (SourceKit type-checks all of a target's source
together — no build needed). **Cross-module** (`import MyOwnLib`) needs MyOwnLib's
compiled `.swiftmodule` to exist on disk. System frameworks resolve on-demand
from SDK headers (no build); **your own modules must have been built.** Xcode
hides this by building dependency modules automatically (full build + background
"prepare for indexing"). Our server must arrange the same.

`sourcekit-lsp` background indexing (default in **Swift 6.1**) handles this via
BSP, but **delegates the compiling back to the build server** — it calls
`buildTarget/prepare` and *we* must produce the modules (it does **not** compile
them itself for an external server; only its built-in SwiftPM path does that).

---

## Roadmap

### v1 — Working autocomplete (relies on a prior build) — ✅ functionally complete

**Engine** ✅
- Complete search-path / module-input emission: swift hand-codes its side; clang
  emits `HEADER_SEARCH_PATHS`/`USER_HEADER_SEARCH_PATHS`/`SYSTEM_HEADER_SEARCH_PATHS`/
  `FRAMEWORK_SEARCH_PATHS` + the products dir's generated-headers, de-duplicated
  (`emit_clang_search_paths`). DerivedData located via `xcode_hash`.
- Per-file argument API (`build_settings::resolve_file_arguments`): Swift = the
  whole module's swiftc invocation; clang = gated to the one file's language
  (`-x objective-c` for a `.m`, C++/ObjC++ for a `.mm`).
- Editor mode: strips `-explicit-module-build`/emit/`-c` (implicit modules);
  advertises the build's index store (`indexStorePath`/`indexDatabasePath`) for
  project-wide navigation.

**Validation** ✅ — the 3-layer loop (`tests/bsp_*`):
- Layer 0 (`swiftc -typecheck`/`clang -fsyntax-only`): **0** resolution errors,
  incl. cross-module `import` and ObjC `HEADER_SEARCH_PATHS`.
- Layer 1 (conformance): protocol round-trip + the full reply-shape / per-file
  `-x` dialect matrix (see the measurement-layers section).
- Layer 2 (real `sourcekit-lsp`): **0** diagnostics on `b.swift` and
  jump-to-definition `Greeter → ModuleA/a.swift`.

**Server** ✅ — `sweetpad-lib bsp`: `build/initialize`, `workspace/buildTargets`,
`buildTarget/sources` (+ `inverseSources`), `textDocument/sourceKitOptions`,
shutdown/exit; `sweetpad-lib config` writes `buildServer.json`.

**Cross-module strategy:** relies on a prior `xcodebuild` build (search paths +
index point at DerivedData), like `xcode-build-server`. Seamless prepare is v2.

**v1 hardening — real-project breadth** ✅ (validated against the OSS corpus +
synthetic fixtures; the synthetic-fixture harness alone missed these):
- Xcode-16 buildable folders (`PBXFileSystemSynchronizedRootGroup`): sources are
  walked from the folder, honoring `membershipExceptions`.
- Target dependency edges: `workspace/buildTargets` reports each target's
  `PBXTargetDependency` graph (also the v2 prepare-order input).
- Swift-package products: `-F …/PackageFrameworks` for package-consuming targets
  (`_synthetic-spm`, both harness layers).
- Corpus soundness check: every corpus target resolves its dependency/source/
  package queries, edges name real targets, graph is acyclic.
- `buildServer.json` carries the `version` field sourcekit-lsp's decoder requires
  (without it the server was silently skipped).

**v1 tail** ✅:
- `buildTarget/didChange` + `project.pbxproj` watching: a poll-based watcher
  pushes the notification so the client re-queries mid-session without an LSP
  restart (per-request resolution is already fresh — the parse cache is
  mtime-validated). Writes lock stdout per frame so the watcher and request loop
  don't interleave.
- VS Code extension wiring: `buildServer.provider: "sweetpad"` generates the
  config (Electron-as-Node launcher) and hands off to `sourcekit-lsp`.

### v2 — Seamless background indexing (no manual build), `xcodebuild`-driven prepare — ✅ done

- `build/initialize` advertises `data.prepareProvider: true`; the server handles
  `buildTarget/prepare` and `workspace/waitForBuildSystemUpdates`. sourcekit-lsp's
  background indexing (default-on, Swift 6.1+) calls prepare for the file's
  target.
- `prepare` runs an incremental `xcodebuild` **by scheme** (a bare `-target` build
  doesn't populate the products dir) into the DerivedData our search paths point
  at, so the target's dependency `.swiftmodule`s + generated inputs exist.
  `project::scheme_for_target` maps the prepared target → a scheme. It runs on a
  serialized worker thread and replies only when the build finishes (sourcekit-lsp
  blocks on the response); it's best-effort (replies even on build failure).
  **This is where we surpass `xcode-build-server`** — it has no prepare.
- Validated end-to-end: from a **clean DerivedData**, a real headless
  sourcekit-lsp resolves `ModuleB`'s `import ModuleA` with **0 diagnostics**
  (`tests/bsp_lsp_e2e.rs::prepare_resolves_cross_module_without_prior_build`),
  and prepare produces the dependency module
  (`tests/bsp_prepare.rs`). Both gated on `BSP_ORACLE=1` + Xcode 26.5.
- Known nuance: a file compiled before its first prepare keeps a stale "no such
  module" until re-pulled — a real editor re-pulls on `workspace/diagnostic/refresh`.
- Caveats: Xcode has **no** fast declarations-only prepare mode (SwiftPM has
  `--experimental-prepare-for-indexing`), so prepare = a real incremental build —
  heavier than the SwiftPM path. Cancellation (`$/cancelRequest`) of a superseded
  prepare isn't handled yet (the build runs to completion).

### v3 — Self-built prepare (custom executor) for the fast path — ✅ done (Swift fast path)

Emit dependency `.swiftmodule`s **ourselves** via direct `swiftc` — no
`xcodebuild` process, no link, single arch — the "custom build planner" fast path.

- **Decision from the project graph** (`prepare_target` in `bsp/mod.rs`): if the
  prepared target and its whole transitive closure are `project::is_self_buildable`
  (pure Swift; no Swift-package products, C-family sources, shell-script phases, or
  build rules), emit each transitive dependency (in `project::transitive_dependencies`
  topo order) with `swiftc -emit-module -emit-module-path` into the products dir,
  reusing the **editor arguments** we already feed sourcekit-lsp. The module name +
  products dir are read back out of those args, so output lands exactly where
  dependents' `-I` looks.
- **Fallback** to the v2 `xcodebuild`-by-scheme path for any non-self-buildable
  closure (packages, C-family, code-gen: asset catalogs / Core Data / string
  catalogs / custom rules) **and** for any self-build that unexpectedly fails — so
  classification only needs to be a fast-path gate, not perfect.
- **Validated:** the self-build path produces a usable module with `swiftc` alone
  (`tests/bsp_prepare.rs` asserts the swiftc path, no `xcodebuild`), and the
  no-prior-build cross-module e2e still resolves — now in **~1s vs ~5s** for the
  xcodebuild path. Classification + closure unit-tested in `tests/project.rs`.
- **Remaining for "full" v3** (not blocking the fast path): per-target *mixing*
  (self-build the simple deps of an otherwise-complex closure rather than
  xcodebuild the whole thing); code-gen-resource classification (today an asset
  catalog is caught only when its emit fails, or when other disqualifiers fire);
  owning more output-layout geometry (header maps / VFS) for mixed-language deps.
- **Existence proofs:** SwiftPM (a swiftc-driver build system; sourcekit-lsp's own
  prepare uses it) and Bazel `rules_swift`/`rules_apple`.

---

## Decisions (locked) + things to expand later

- **Approach:** walking skeleton first — stand up a minimal BSP server early to
  kill the integration risk, then iterate arg quality against an automated loop.
- **Server:** a `bsp` subcommand on the `sweetpad-lib` Rust binary (`sweetpad-lib
  bsp`), calling the engine directly. Started as the walking skeleton.
- **Toolchain:** Xcode **26.5 only** to start. ⚠️ *Expand later* to 15.4 / 16.4
  (key the harness by version like the compiler-args oracle).
- **Cross-module fixture:** a committed synthetic multi-module Xcode project (app
  + two framework targets + a real cross-module `import`), same approach as
  `_synthetic-staticlib` / `_synthetic-rich`.
- **"Modules must exist" handling:** the harness does a hermetic `xcodebuild
  -derivedDataPath <tmp>` build once per fixture; the engine computes search paths
  into that same path.
- **`sourceKitOptions` granularity:** **per-target first** (map file → target →
  the target's args). ⚠️ *Expand later* to true per-file args (per-file `-x`
  dialect + language-specific flags).
- **Harness language:** Rust (Layer 0/1 as integration tests like
  `compiler_args_oracle.rs`; Layer 2 a small scripted LSP client).

## Automated measurement layers (the loop) — all green

The whole stack is headless/scriptable, so the BSP analog of the xcodebuild
oracle is an automated, self-labeling loop (no human in the iteration). All three
layers are built and passing against the multi-module fixture:

- **Layer 0 — type-check oracle** (`tests/bsp_typecheck_oracle.rs`, no server/LSP):
  builds the fixture, runs `swiftc -typecheck` with our generated args; metric =
  module-resolution errors → **0**, including `ModuleB`'s cross-module
  `import ModuleA`. Opt-in `BSP_ORACLE=1` (builds + needs Xcode 26.5).
- **Layer 1 — BSP conformance** (`tests/bsp_conformance.rs`, server alone): drives
  `sweetpad-lib bsp` with scripted JSON-RPC and pins the structural reply surface
  sourcekit-lsp decodes. Beyond the smoke test (targets listed, sources returned,
  `sources` ↔ `inverseSources` round-trip, `-I` in / `-explicit-module-build`
  out): the full `initialize` capability set (data kind, both providers, the five
  language ids, index-store geometry from `--derived-data-path`), the
  `buildTargets`/`sources` item shapes, target resolution by membership when no
  target is given, the unowned-file (empty/`null`) and unknown-method (`-32601`)
  edges, build-only-flag stripping (verified non-vacuous against the real argv),
  and the per-file clang `-x` dialect matrix (`.m`/`.mm`/`.cpp`/`.c`) with ObjC vs
  C++ flag gating. Fast, hermetic, ungated.
- **Layer 2 — end-to-end** (`tests/bsp_lsp_e2e.rs`, real headless `sourcekit-lsp`):
  writes `buildServer.json` → our server → `sourcekit-lsp` opens `b.swift` →
  **0 module-resolution diagnostics**. Opt-in `BSP_ORACLE=1`.
- **Layer 2 at corpus scale** (`tests/bsp_corpus_completion.rs`, opt-in
  `BSP_CORPUS=1`): the same end-to-end loop, but over the real OSS corpus instead
  of one synthetic fixture — it turns "works on ~X% of projects" into a measured
  number. Per project it builds the main scheme once into a throwaway DerivedData
  (build-first, to isolate *arg/search-path correctness* from prepare), points
  `sourcekit-lsp` at our `buildServer.json`, opens a bounded sample of the
  project's own **non-test** source files, and classifies each into clean /
  *resolution-failure* (a missing-module/header arg bug — the headline) /
  *internal-error* (a `sourcekit-lsp` stdlib-load rough edge, retried once).
  Knobs: `BSP_CORPUS_ONLY`, `BSP_CORPUS_SAMPLE`, `BSP_CORPUS_BUILD_TIMEOUT`.
  Build-first means a project must build headlessly; failures are reported, not
  hidden. OSS-corpus finding: **0 resolution failures** on the clean Swift
  frameworks (Kingfisher, Alamofire) and the large ObjC+Swift macOS app
  (NetNewsWire); the iOS-simulator path surfaced the stdlib-load rough edge
  (sourcekit-lsp #2328) on some IceCubesApp files.

  **Generated-source + CocoaPods coverage** (committed synthetic fixtures
  `fixtures/_synthetic-{coredata,assetsym,strcat,cocoapods}`, each a forced
  `Probe*.swift` that references a build-time-generated / Pod symbol; a `strict`
  fixture treats *any* editor error as a failure and also gates on our BSP
  actually returning options, so a null reply can't masquerade as clean). The
  harness drove three real engine fixes the OSS frameworks never exercised:
  1. **Generated sources** — `resolve_file_arguments` now folds the `.swift`
     xcodebuild emits into `DERIVED_SOURCES_DIR` (Core Data subclasses,
     `GeneratedAssetSymbols.swift`, intent classes, string-catalog symbols) into
     the editor's input set; without it references to those intra-module symbols
     failed with "cannot find … in scope".
  2. **Nested-group xcconfig** — `resolve_file_ref_path` walks the parent
     `PBXGroup` chain instead of assuming the root group, so CocoaPods'
     `Pods`-nested `Pods-<App>.xcconfig` resolves (it was hard-failing the whole
     app-target resolve → our BSP served nothing → a *false* clean).
  3. **Quoted search paths** — a quote-aware `ws_paths` tokenizer strips the
     double quotes xcconfigs put around each path (`"${…}/Pod"`), which otherwise
     reached swiftc as a literal-quoted relative path (`-F "…"`), defeating
     framework module loading. Fixes CocoaPods and any spaced/quoted path.
  Measured before→after: all five generated-source/Pod fixtures (Core Data, asset
  symbols, string catalogs, SiriKit intents, CocoaPods) **fail → clean**
  end-to-end; the full unit+oracle suite stays green (the changes are additive /
  no-op on unquoted corpus paths). Also validated against a real Tuist Core Data
  example (`Model.swift` uses the generated `User` class) — `tuist generate` the
  gitignored `corpus/_tuist-src/examples/.../generated_ios_app_with_coredata`; the
  harness skips it when absent.

  **Known limitation — sourcekit-lsp #2328.** On the iOS-simulator path some files
  intermittently return "Loading the standard library failed" — a sourcekit-lsp
  internal error, **not** our args: our exact editor arguments type-check cleanly
  under `swiftc -typecheck` (the stdlib loads via the prebuilt modules). It
  persists past one retry and accounts for the entire iOS-corpus gap (IceCubesApp
  ~43% clean, **0** of which are resolution bugs). The harness buckets it
  separately (`is_internal_error`) so it can never read as our failure; we can
  only mitigate (the retry), not fix it in argument generation. ⚠️ Still
  single-`.xcodeproj`: multi-project workspaces beyond the CocoaPods app-target
  case remain out of scope.

Expectations are auto-derived (self-evident "zero false errors"; differential vs
the captured build args; source-derived "every `import` must resolve"), so an
agent can push the metric without human labeling. Next: a positive cross-module
check (completion/definition), the differential-vs-captured-args variant, and the
search-path / per-file engine work the loop now measures.

## Required BSP methods (reference)

Standard: `build/initialize`, `workspace/buildTargets`, `buildTarget/sources`,
`buildTarget/inverseSources`, `buildTarget/didChange`. SourceKit-LSP core:
`textDocument/sourceKitOptions`. Background-indexing extensions:
`buildTarget/prepare`, `workspace/waitForBuildSystemUpdates`.

## References

- sourcekit-lsp — Background Indexing: https://github.com/swiftlang/sourcekit-lsp/blob/main/Contributor%20Documentation/Background%20Indexing.md
- Swift Forums — Extending BSP with SourceKit-LSP (the prepare/indexing extensions, non-SwiftPM servers): https://forums.swift.org/t/extending-functionality-of-build-server-protocol-with-sourcekit-lsp/74400
- sourcekit-lsp — Enable Background Indexing: https://github.com/swiftlang/sourcekit-lsp/blob/main/Documentation/Enable%20Experimental%20Background%20Indexing.md
- xcode-build-server (the "observe build logs" approach we contrast with): https://github.com/SolaWing/xcode-build-server
- Bazel rules_swift / rules_apple (build Apple targets without xcodebuild): https://github.com/bazelbuild/rules_swift , https://github.com/bazelbuild/rules_apple
- sourcekit-lsp #2328 (external-BSP stdlib loading rough edge): https://github.com/swiftlang/sourcekit-lsp/issues/2328
