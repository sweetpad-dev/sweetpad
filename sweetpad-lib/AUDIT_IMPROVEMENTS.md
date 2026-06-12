# sweetpad-lib audit — June 2026

A full review of the Rust library (`sweetpad-lib/`, ~18k lines of source plus the
test harness) and the seams where the extension consumes it. Every finding below
is concrete and implementable; line references are against commit `54c40a1`.

**Health snapshot.** `cargo test` fully green (corpus oracle, BSP conformance,
round-trip suites). `cargo fmt --check` clean. `cargo clippy --all-targets`
(pedantic) emits only 4 minor warnings — but note CI runs `cargo clippy` on the
lib only, so test-code lints never gate. Corpus oracle baseline: **exact 88%,
canonical 96–99%, structural 99–100%** on Xcode 15.4 / 16.4 / 26.5. The
no-`unwrap` discipline on user project data holds throughout the resolution
engine; the crash routes found are all recursion/expansion bounds, not indexing.

Findings are grouped into priority tiers. Within a tier, roughly in
impact order.

---

## P0 — Broken in production or crashable on real input

### 0.1 BSP server cannot start from the extension's own `bsp.json`
`src/bsp/mod.rs:273-279` resolves the project as `flags.workspace → flags.project
→ pull("workspacePath") → pull("projectPath")`. The extension writes
`workspacePath` as the **VS Code workspace folder** (`src/bsp/config.ts:54-56`
in the extension), not an `.xcworkspace` — so the server opens a plain directory
as a project and exits 1 (`bsp: open project: I/O error`). Reproduced
empirically. Every conformance test passes `--project` flags, so the
`from_json` path — the only path the shipped extension uses — is untested.
**Fix:** drop `pull("workspacePath")` from the chain (or honor it only when it
ends in `.xcworkspace`), and add a conformance test that drives the server
through a `.sweetpad/bsp.json` written with the extension's actual schema.

### 0.2 `buildServer.json` dangles after every extension update
`src/common/cli/scripts.ts:721-737` (extension) writes an absolute `argv` path
inside the **versioned** extension install dir; `src/build/utils.ts:461-474`
skips regeneration whenever `name === "sweetpad"`. After any extension update
the old dir is gone and sourcekit-lsp silently fails to spawn the server. The
bsp doctor checks field presence, not that `argv[0]` exists.
**Fix:** regenerate when `argv[0]` is missing or differs from the current
launcher path; better, copy the launcher to a stable global-storage path. Add
the existence check to the doctor.

### 0.3 Extension activation crash on non-macOS hosts
`@sweetpad/lib` is imported at module top level (`src/common/cli/scripts.ts:4`,
`src/build/utils.ts:3`), reachable from `extension.ts`'s import graph. The VSIX
is platform-universal but ships only darwin `.node` binaries, so on
Linux/Windows (Remote-SSH included) the napi loader throws at `require` time and
the whole extension fails to activate.
**Fix:** publish darwin-targeted VSIXs (`vsce package --target darwin-arm64
darwin-x64`), or lazy-load the addon behind a friendly "macOS only" guard.

### 0.4 Parser crash class: unbounded recursion / expansion on adversarial input
All confirmed by reproduction; these files come from arbitrary repos and
DerivedData, and the host is a long-lived extension process.

- **bplist exponential expansion** — `src/bplist.rs:147-297`. `read_object`
  re-materializes every referenced object with no memo/visited set;
  `MAX_DEPTH=256` bounds depth, not width. A ~600-byte file with shared refs
  (`object i = [ref i-1, ref i-1]`) expands to 2^N nodes — unbounded CPU and
  memory. **Fix:** budget total materialized objects (e.g. against
  `num_objects`), or error on re-entry of an object index on the current path.
- **pbxproj parse stack overflow** — `src/pbxproj.rs:313-415`. `parse_value` →
  `parse_array`/`parse_dict` recursion has no depth limit; ~200k `(((…` aborts
  the process (SIGABRT). **Fix:** thread a depth counter, error past ~512.
- **xcscheme/workspace XML stack overflow** — `src/xcscheme.rs:279-390`. Same
  class: `parse_element` (and recursive `write_element`) unbounded. **Fix:**
  depth-limit `parse_element`; depth-check serialization.
- **pbxproj_writer cycle overflow** — `src/pbxproj_writer.rs:260-277`.
  `comment_for` follows `fileRef`/`productRef` assuming "recursion is shallow by
  construction"; a self-referential `PBXBuildFile` recurses forever (confirmed).
  **Fix:** visited set or depth bound.
- **TestTargetID host recursion** — `src/build_context.rs:515-525` →
  `test_bundle_subpath` (`:619-632`) resolves the host via
  `test_target_id_host` (`src/project.rs:3202-3223`), which accepts **any**
  target without checking product type (unlike `find_app_host_target`). Two
  test bundles pointing `TestTargetID` at each other (or one at itself) recurse
  `resolve → build_layers → test_bundle_subpath → resolve` to stack overflow,
  under default trigger conditions. **Fix:** only accept hosts whose
  `productType` starts with `com.apple.product-type.application`, or thread a
  recursion guard.
- **Variable-expansion fan-out** — `src/resolver.rs:396-521`.
  `MAX_EXPAND_DEPTH=32` bounds depth but not output size: `A = $(B) $(B)`,
  `B = $(C) $(C)`, … doubles per level (~2³² bytes before the cap), compounded
  by the 16-pass outer loop. **Fix:** total-output budget (e.g. 1 MiB per
  value), returning the unexpanded text when exceeded.

Once fixed, add a small adversarial test suite ("rejects pathological input
without panicking") — currently nothing exercises deep nesting, cyclic graphs,
or malformed bplists. A `cargo-fuzz` target over the four parsers would lock
this in long-term.

---

## P1 — Correctness divergences from xcodebuild

### 1.1 Corpus mismatch clusters (measurable, from the oracle's own tally)
The systematic-mismatch report points at a handful of keys carrying nearly all
remaining error: `CCHROOT` (every fixture); the `PROJECT_DIR` /
`PROJECT_FILE_PATH` / `SOURCE_ROOT` / `SRCROOT` family plus `LOCROOT` /
`LOCSYMROOT` and the `BUILD_DIR`-derived family on **tuist-fixtures** (270
captures each — likely one root-path rule); `LD_DEPENDENCY_INFO_FILE` /
`OBJECT_FILE_DIR_normal` / per-arch dirs / `STRINGSDATA_DIR` on alamofire;
`LIBRARY_SEARCH_PATHS` / `FRAMEWORK_SEARCH_PATHS` / `HEADER_SEARCH_PATHS`
broadly. Fixing the tuist project-root rule and `CCHROOT` alone should move the
exact tier several points.

### 1.2 Shadow resolvers disagree with the real engine
Three separate re-implementations of precedence are used for gating decisions:
- `last_unconditional_setting` (`src/project.rs:1026-1035`) skips *all*
  conditional assignments — `SUPPORTS_MACCATALYST[sdk=macosx*] = YES` is
  invisible to the Catalyst gate even when the condition matches.
- `is_unoptimized_build` (`src/project.rs:3157-3172`) matches conditions but
  does **no** `$(inherited)`/variable expansion (`GCC_OPTIMIZATION_LEVEL =
  $(MY_OPT_LEVEL)` misclassifies), and is fed the unversioned `query.sdk`
  (`src/build_context.rs:543`) while the main resolve binds the canonical
  versioned SDK (`:242-247`).

**Fix:** derive gates from a cheap pre-resolve via `resolver::resolve`, or a
single shared "effective value" helper that folds inherited and uses the
canonical SDK.

### 1.3 Authored-value probes inconsistently ignore `-xcconfig`/CLI overrides
`src/build_context.rs:445-460` builds `layers_with_extra` for three probes, but
`is_unoptimized_build`, `user_only_active_arch`, `user_ios_deployment`,
`supports_maccatalyst`, `natural_sdk`, `user_product_bundle_identifier`,
`derive_maccatalyst_bundle_id` (`:380-465`) read bare `bundle.layers`. An
override xcconfig setting `GCC_OPTIMIZATION_LEVEL=0` changes xcodebuild's
output but not these gates. **Fix:** compute every probe from
`layers_with_extra`; document intentional exclusions.

### 1.4 `[arch=…]` conditionals fire when xcodebuild's aggregated view wouldn't
`src/build_context.rs:248` binds the real `query.arch`, but xcodebuild's
`-showBuildSettings` resolves with `arch=undefined_arch` — a fact the code
itself relies on for the KASAN workaround (`src/project.rs:2002-2017`).
**Fix:** bind `arch: "undefined_arch"` on the showBuildSettings-emulation path
(keep the real arch for per-arch compiler args), then delete the KASAN special
case.

### 1.5 `OTHER_SWIFT_FLAGS`/`OTHER_LDFLAGS` split without quote handling
`src/compiler_args.rs:326` and `:821` use a plain whitespace split while
`ws_paths` (`:1254`) exists precisely because CocoaPods quotes values. A typical
Pods `-Xcc -fmodule-map-file="${PODS_…}/module.modulemap"` yields argv tokens
with literal `"`, so sourcekitd never finds the module map. **Fix:** use a
quote-aware splitter for passthrough flag settings.

### 1.6 `links()` misclassifies loadable bundles and UI-test bundles
`src/compiler_args.rs:162-164` treats product types containing `"bundle"`
without `"unit-test"` as non-linking — but `com.apple.product-type.bundle` and
`…bundle.ui-testing` both link (`mh_bundle`); UI-test targets get no link
invocation. **Fix:** restrict the non-linking set to the actual non-linking
types (aggregate/legacy/in-app-purchase) instead of substring matching.

### 1.7 Editor arch hardcoded to `arm64`
`src/bsp/mod.rs:1061-1063` requires **both** `--sdk` and `--arch` for an
override; otherwise `editor_platform` returns `"arm64"` unconditionally
(`:1090`) — every `-target` triple is wrong on Intel Macs. **Fix:** honor each
override independently and default from the host arch.

### 1.8 BSP handlers serve a stale startup snapshot after `didChange`
`src/bsp/mod.rs:954-957` (`inverse_sources`), `:982-987` (membership fallback),
and `:1011` (default target list) iterate the startup-cached `self.targets`
while `build_targets()` re-reads. Files in a newly added target resolve to no
owner until restart. **Fix:** use `self.current_targets()` at all three sites.

### 1.9 JSON-RPC robustness gaps
- Malformed JSON frame is silently dropped (`src/bsp/mod.rs:114-116`) — a
  client that sent an id blocks forever. **Fix:** reply `-32700` (id null).
- A malformed `Content-Length` kills the whole server with no error frame and
  no trace log (`src/bsp/framing.rs:19-23` swallows the parse, `mod.rs:113`
  propagates). Headers are also matched case-sensitively. **Fix:** log + resync
  (or at least error-frame) instead of hard exit; case-insensitive headers.

### 1.10 Smaller divergences
- `ASSETCATALOG_FILTER_FOR_DEVICE_MODEL`: `device_model_for`
  (`src/project.rs:2986-2995`) only matches 5 oracle-filename-shaped labels, and
  the caller (`:1896-1918`) pushes the three filter settings unconditionally —
  real CLI destinations (`name=iPhone 16`) emit empty/garbage values. **Fix:**
  skip the pushes when the model is empty; key on normalized names.
- `SWIFT_EMIT_CONST_VALUE_PROTOCOLS` (`src/project.rs:2158-2174`) is a
  26.x-only key per its own comment but emitted for every version. **Fix:**
  gate on major ≥ 26.
- Workspace member-project loop swallows *all* resolve errors
  (`src/build_settings.rs:188-199`, `:283-284`) — a malformed member xcconfig
  surfaces as the misleading "no target matched". **Fix:** distinct error
  variants for "target not here" vs IO/parse; propagate the latter.
- `xcspec.rs` scrapes `meta.json` with substring matching
  (`src/xcspec.rs:312-320`) despite `serde_json` being a hard dependency — and
  CLAUDE.md's own rule says don't reinvent JSON. **Fix:** parse with
  `serde_json`.
- bplist `read_size` accepts a 16-byte extended size and silently truncates to
  `usize` (`src/bplist.rs:309-327`). **Fix:** reject `nbytes > 8`.
- DerivedData hash input: `src/bsp/mod.rs:631-633` / `src/project.rs:1132-1161`
  hash the `fs::canonicalize`d path, but Xcode hashes the path *as opened* —
  symlinked roots (`/tmp` → `/private/tmp`) or NFD unicode yield the wrong
  container. All pinned vectors (`src/xcode_hash.rs:302-324`) are symlink-free
  ASCII. **Fix:** hash without resolving symlinks (or probe both); add a
  symlink + unicode capture to the pins.

---

## P2 — CI, packaging, release pipeline

- **The `node` feature is never compiled before release.**
  `.github/workflows/sweetpad-lib.yaml` runs fmt/clippy/test with default
  features; `src/node.rs` is first compiled by the tag-triggered release build.
  Any error there blocks a release at publish time. **Fix:** add
  `cargo clippy --no-default-features --features node -- -D warnings` (or a
  `napi build` smoke step). Also switch CI clippy to `--all-targets` so test
  code is linted (it currently has 4 warnings CI never sees).
- **No PR/push CI for the extension.** `ci.yaml` triggers only on tags —
  vitest, `check:all`, the rolldown bundle, and the universal `.node` build run
  zero times pre-release. **Fix:** a PR workflow on `macos-latest` running
  `npm ci && npm run check:all && npm test && npm run build`.
- **Embedded catalog staleness is unguarded.** No test calls
  `catalog_cache::embedded()`; bumping `FORMAT_VERSION`
  (`src/catalog_cache.rs:46`) or refreshing `xcspec-cache/` without regenerating
  `src/catalog_embedded.bin` ships corrupt/stale defaults and CI stays green.
  **Fix:** test that `embedded()` parses and byte-equals a fresh
  `serialize(load_catalog(<newest xcspec-cache>), 0)` (the walk is sorted, so
  it's deterministic). Also default `examples/gen_embedded_catalog.rs:23` to
  the newest `xcspec-cache/xcode-*` dir instead of a hardcoded `"26.5.0"`.
- **Stale universal `.node` shadows fresh debug builds.**
  `rolldown.config.mjs:38-39` prefers any lingering `*universal*.node` over the
  addon `build:debug` just produced. **Fix:** delete `sweetpad-lib/*.node`
  before debug builds, or pick by newest mtime.
- **Version single-sourcing.** Cargo.toml says `0.1.0`, the napi package.json
  `0.1.1`, and the extension hardcodes `version: "0.1.0"` into buildServer.json
  (`scripts.ts:731`) while the Rust side uses `CARGO_PKG_VERSION`. Nothing
  detects an extension↔addon mismatch. **Fix:** single-source (napi version
  export) and have the bsp doctor compare.

---

## P3 — Architecture & maintainability

### 3.1 Split `project.rs` (4,467 lines)
Measured seams suggest this breakdown:

| Lines (today) | Responsibility | Proposed module |
|---|---|---|
| 15–371 | `Project`/`Target` model, `open`, scheme autocreation | `project/mod.rs` |
| 373–498, 3174–3496 | settings layers, xcconfig loading, `split_conditional_key` | `project/settings_layers.rs` |
| 500–1000 | target-graph queries (sources, frameworks, deps) | `project/graph.rs` |
| 1002–2585 | `built_in_settings` (~1,070-line fn) + `built_in_overrides` (~400) | `project/builtins.rs` |
| 2587–3172 | platform/arch/version tables, host detection, Catalyst | `project/platform.rs` |
| 3498–3553 | `scheme_for_target` | move to `scheme.rs` |

This also isolates the Xcode-version-rot surface (`platform_metadata`,
`archs_standard_*`, KASAN strings, Catalyst recipes, `legacy_xcode15` computed
independently in two places at `:1192` and `:2234`) into one module with an
"update on Xcode bump" note referencing `UPDATING_XCODE_VERSIONS.md`.
Known layering wrinkle to untangle while splitting: discovery
(`open_from_value`) calls down into settings resolution via
`is_safari_extension_target` (`:200`).

### 3.2 Parameter-struct the builtin entry points
`built_in_overrides` takes **22 positional parameters** (`:2210-2233`),
`built_in_settings` 16 (`:1111-1128`) — call sites are walls of
`false, false, None, None, …` where transposing two adjacent bools compiles
silently and flips behavior. **Fix:** `struct BuiltinInputs { … }` with named
fields; `build_context::build_layers` already computes everything in one place.
Similarly, `BuildSettingsContext.layers: Vec<Vec<Assignment>>`
(`src/project.rs:376-380`) carries positional meaning by comment only — a named
`UserLayers` struct with an `ordered()` iterator removes a silent-reorder
hazard.

### 3.3 Deduplicate parser scaffolding
- `Parser { input, pos }` + `peek`/`advance`/`line_column` is copy-pasted
  between `pbxproj.rs:248-311` and `xcscheme.rs:175-204` — extract a shared
  byte-cursor util.
- `split_conditional_key` (`src/project.rs:3460-3484`) re-implements xcconfig's
  bracket-condition parser with different failure behavior (silent drop vs hard
  error) — expose the xcconfig one with a lenient flag.
- `parse_flags` exists twice with divergent trailing-flag semantics
  (`src/bsp/mod.rs:1144-1163` drops, `src/bin/sweetpad_lib.rs:63-83` inserts
  empty) — share one that rejects a missing value.
- `STRIP_FLAGS` vs `MODERN_DRIVER_DEFAULTS` are parallel hand-maintained tables
  (`src/bsp/mod.rs:1118-1134`, `src/compiler_args.rs:851-860`) — add a unit
  test asserting every driver default is stripped or explicitly allowlisted.

### 3.4 Structured errors at the public boundary
`resolve_build_settings` / `resolve_compiler_arguments` /
`resolve_file_arguments` (`src/build_settings.rs:155, 217, 305`) return
`Result<_, String>` while every inner module has typed errors; callers
(node.rs, BSP) can't distinguish "unknown target" from IO failure. The parsers'
`ParseError.message: String` has the same issue and matters more once the P0
resource-limit errors exist (callers may treat "limit exceeded" differently
from "malformed"). **Fix:** a `build_settings::Error` enum wrapping inner
errors; an `ErrorKind` on parser errors.

### 3.5 Repo hygiene
`PLAN.md` (848 lines, opens with a long-false premise), `PLAN_BSP.md`,
`PLAN_COMPILER_ARGS.md` are stale relative to the shipped code — refresh or
move to an archive dir, since CLAUDE.md points readers at them. Fixtures are
76 MB in the working tree (42 MB tuist-fixtures) but compress fine (8 MiB pack)
— checkout size and editor indexing are the only costs; acceptable, just worth
knowing.

---

## P4 — Performance (long-lived process, per-keystroke BSP queries)

- **Catalog layer cloned per query.** `Catalog::layer_for` starts with
  `self.universal.clone()` (`src/xcspec.rs:189`, called at
  `src/build_context.rs:359`) — hundreds of `Assignment`s per resolve. Memoize
  per `(product_type, sdk)` as `Arc<Vec<Assignment>>`.
- **`expand_variables` clones the full merged map per fixed-point pass**
  (`src/resolver.rs:370-385`, up to 16 passes × ~1.4k entries), and any
  conditional assignment (always, with a catalog) doubles the whole pipeline
  via the two-pass resolve (`:202-226`) plus a third clone in
  `with_context_aliases` (`:250`). Track a dirty-key set; reuse pass 1's
  reduced layers.
- **`parent_group_of` is O(all objects) per group level**
  (`src/project.rs:3429-3441`), run per `baseConfigurationReference` per query —
  O(n·depth) on CocoaPods-scale pbxprojs. Build a `child → parent` index once
  per parsed pbxproj.
- **Filesystem rescans per resolve:** `fs::canonicalize`, the
  `find_derived_data_container` double `read_dir` (`src/project.rs:2694-2717`),
  and `darwin_user_cache_dir` run inside `built_in_settings` on every call —
  cache at `BuildContext::open`. (`find_derived_data_container` also picks the
  lexicographically first workspace dir on collision — document or fix the
  tiebreak.)
- **`source_kit_options` resolves twice per request** (`src/bsp/mod.rs:1068`
  probe + `:1035` real resolve) — cache the probe per target.
- **`decode_entities` is O(n²)** on entity-dense text (`src/xcscheme.rs:469`) —
  cap the `;` search window (entity names are short).
- **N-API calls are sync on the extension-host event loop**
  (`src/node.rs:234`, `:271`); the first call also builds the catalog. Expose
  async variants (`AsyncTask`) so resolution runs off the JS thread.

---

## P5 — Caching & BSP lifecycle robustness

- Catalog disk-cache writes are non-atomic on a path shared by two processes
  (`src/catalog_cache.rs:221-226`) — write temp + `rename`.
- `file_cache` stamp is `(len, mtime)` (`src/file_cache.rs:18-29`) — fold in
  inode/ctime to avoid serving stale parses after same-length, same-mtime
  rewrites.
- Process-lifetime caches never evict (`pbxproj.rs:236`, `xcconfig.rs:282`,
  `catalog_cache.rs:147`); disk `catalog-*.bin` files are never GC'd
  (`catalog_cache.rs:198-204`). Wire eviction into `xcode::flush_caches()` /
  the reset command; prune old cache files on write.
- Telemetry broadcast does blocking socket writes under the clients mutex on
  the request path (`src/bsp/control.rs:133-138` via `push_log`,
  `src/bsp/mod.rs:538-548`) — a stalled extension socket wedges the whole BSP
  server. Use write timeouts or an mpsc → writer thread.
- Server exit mid-prepare orphans the spawned `xcodebuild`
  (`src/bsp/mod.rs:912`); `$/cancelRequest` is ignored. Keep the child handle,
  kill on shutdown.
- No lifecycle gating: requests served before `build/initialize` / after
  `build/shutdown` (`src/bsp/mod.rs:121-162`) — BSP expects `-32002`-style
  errors. Track a three-state lifecycle.
- The change watcher polls only member `project.pbxproj` files
  (`src/bsp/mod.rs:706-710`) — workspace-membership and scheme edits never fire
  `didChange`; meanwhile `LiveConfig.scheme` is diffed but never used by
  `options_for` (`:1093-1111`), causing refresh storms with no behavioral
  change. Watch `contents.xcworkspacedata`; use or stop diffing the scheme.
- `xcode::locate_uncached` probes a relative path when `parent()` is empty
  (`src/xcode.rs:199`) — filter the empty candidate.
- pbxproj `\U` escapes don't combine surrogate pairs
  (`src/pbxproj.rs:460-478`) — astral chars round-trip as errors; combine pairs
  or document. xcconfig block comments spanning lines drop surrounding
  same-line text (`src/xcconfig.rs:303-332`) — verify against Apple's
  whitespace-collapse rule.

---

## Suggested order of attack

1. **P0.1 + P0.2** — the BSP integration is currently broken end-to-end for
   extension users; both are small, high-leverage fixes with a conformance test
   to lock them in.
2. **P0.4 as one "parser hardening" PR** — depth/budget guards + an adversarial
   test suite; mechanical and self-contained.
3. **P2 CI items** — cheap one-day win that prevents whole classes of release
   breakage (node-feature clippy, PR workflow, embedded-catalog guard).
4. **P1.1 via the oracle** — the tuist root-path family and `CCHROOT` are the
   measured next steps toward the COVERAGE.md 100% roadmap.
5. **P1.2–1.4 gating unification** — fixes a family of subtle divergences and
   deletes the KASAN special case.
6. **P3.1/3.2 project.rs split + parameter structs** — best done before more
   corpus-derived rules accrete; purely mechanical, protected by the green
   corpus oracle.
7. P4/P5 as background tasks, each independently shippable.
