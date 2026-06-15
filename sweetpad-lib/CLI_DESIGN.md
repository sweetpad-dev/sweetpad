# SweetPad CLI — design

The `sweetpad` binary's standalone, headless command set: **"xcodebuild for
humans."** A pure native front-end to the `sweetpad-lib` Rust engine for
running, building, and exploring Xcode projects without an editor.

It lives in the **same `sweetpad` binary** as the existing `vscode` namespace
(the JSON-RPC client that controls the VS Code extension — see
`src/vscode_cli.rs`). `vscode` is unchanged; the new resources sit beside it.

> Status: committed design goals. Implementation tracked in §8.

---

## 1. Positioning

- **Standalone & headless.** Drive Xcode projects from a terminal or CI with no
  editor and no Node runtime. Contrast with the previous CLI iteration, which
  only *controlled* the VS Code extension (now the `vscode` namespace).
- **For humans, not just scripts.** Friendlier than raw `xcodebuild`/`xcrun`:
  sane discovery, readable output, interactive pickers — while staying fully
  scriptable.
- **Backed by `sweetpad-lib`.** Scheme/destination/build-setting resolution
  comes from the existing Rust engine; the CLI is a thin, well-factored layer
  on top.

## 2. Command grammar

**Noun-verb, resource-first:** `sweetpad <resource> <action> [flags]`.
Consistent and discoverable (like `kubectl`/`docker`/`gh`). Resources live at
the **top level**; `vscode` is just one more top-level entry.

## 3. Command surface (v1)

v1 scope is **explore + build/run** — the minimum to actually develop headless.

```
sweetpad scheme list                 list schemes
sweetpad destination list            list build destinations
sweetpad project info                targets, configurations, schemes
sweetpad project new <Name>          scaffold a new minimal SwiftUI iOS app
sweetpad settings show               resolved build settings (lib's specialty)
sweetpad simulator list              list simulators
sweetpad simulator boot              boot a simulator
sweetpad build start                 compile only

sweetpad app run                     build + install + launch
sweetpad app install                 build + install, no launch
sweetpad app launch                  launch an already-installed app
sweetpad app logs                    stream app logs
sweetpad app stop                    kill the running app

sweetpad vscode <method>             control the VS Code extension (unchanged)
```

`build` stays purely "compile"; the full run/install/launch/logs/stop lifecycle
groups under `app`, the noun it acts on.

Out of scope for v1 (later iterations): `test`, `format`, `device` (physical)
management, `bsp` (autocomplete config), `tools` (Homebrew).

## 3a. `project new` — scaffolding

`project new` creates a fresh, buildable **minimal SwiftUI iOS app** with no
external tools. The `.xcodeproj` is generated **natively**: a
[`crate::pbxproj`] object graph assembled in [`cli::scaffold`] and serialized by
the crate's own [`crate::pbxproj_writer`], with the shared `.xcscheme` built as
a [`crate::xcscheme::Element`]. This keeps the CLI standalone — no XcodeGen
dependency — and on-policy with DOCS §3 (hand-roll Apple's project formats).

```
sweetpad project new <Name> [flags]
```

- **One command, new directory by default.** Creates `./<Name>/`; `--current-dir`
  scaffolds into the working directory instead (name then defaults to its
  basename).
- **Interactive wizard.** On a TTY, any value not supplied as a flag is prompted
  for — location (current dir?), name, platform, bundle id, deployment target,
  and git init — each with a default that **Enter accepts** (the name has no
  universal default, so new-directory mode requires typing it). A non-empty
  target additionally prompts to continue (the `--force` question). Non-TTY /
  `--json` runs stay strict: flags and defaults only, and a missing name is an
  error.
- **Inline "use defaults" escape.** Every step after the name carries its own
  way to accept the remaining defaults without more questions: the platform
  picker has a trailing *"Use defaults for everything else"* entry, and the text
  steps accept a lone `*`. Choosing it fills that field and all later ones from
  defaults and finishes — no separate "proceed?" question.
- **Back-navigation.** The wizard is a step machine over the un-flagged fields,
  so any step past the first can go back to change an earlier answer — a `← Back`
  entry on the `Select` steps and a lone `<` on the text steps. A revisited step
  is pre-filled with the prior answer, and dependent defaults (bundle id from the
  name, deployment target from the platform) recompute when their input changes.
- **Platform.** `--platform ios|macos` (default `ios`); the wizard offers a
  picker. Switching platform swaps `SDKROOT`, the deployment-target key and its
  default (`17.0` iOS / `14.0` macOS), the framework runpath, and the iOS-only
  Info.plist keys (launch screen, orientations, device family).
- **Flags:** `--bundle-id` (default `com.example.<Name>`), `--deployment-target`
  (platform default), `--platform`, `--no-git` (git init runs by default),
  `--force` (allow a non-empty target), `--json` (emits the created paths).
- **Generated tree:** `<Name>.xcodeproj` (pbxproj + inner `.xcworkspace` + shared
  scheme), `<Name>/<Name>App.swift`, `<Name>/ContentView.swift`, `.gitignore`.
- **Names** must be plain identifiers (letters/digits/underscore) so they're safe
  as a Swift type, target, and product name in one.

Generation is a **pure** function (spec → list of files), unit-tested by
round-tripping the pbxproj through the parser and resolving it with
`project::open_from_value`; the `cli-smoke` job then scaffolds a project and
builds it with real `xcodebuild`.

## 4. Output model

- **Human/colored by default** — tables, spinners, formatted build logs.
- **`--json`** on any command emits stable, machine-readable JSON for
  scripting/CI.
- Color **auto-disables** when stdout is not a TTY or when `NO_COLOR` is set;
  `--no-color` forces it off.
- Errors: human messages on **stderr** by default; **structured error objects**
  under `--json`. Meaningful exit codes.

## 5. Target resolution

What the command acts on (workspace/project, scheme, configuration,
destination) resolves by **layered precedence**:

```
explicit flag  >  env var  >  config file  >  auto-discovery
```

- **Auto-discovery:** find the `.xcworkspace` / `.xcodeproj` / `Package.swift`
  in the working directory.
- **Env vars:** `SWEETPAD_SCHEME`, `SWEETPAD_DESTINATION`,
  `SWEETPAD_CONFIGURATION`, `SWEETPAD_PROJECT` / `SWEETPAD_WORKSPACE`.
- **Interactive fallback:** when something is ambiguous/unset **and stdout is a
  TTY**, drop to a fuzzy picker (choose a scheme/destination from a menu).
  **Non-TTY/CI stays strict** and errors instead of prompting.

## 6. Configuration & state

**No files are written to the project root.** Two distinct stores, kept apart so
the tool never clobbers hand-authored config:

### Config — hand-authored only
- `~/.config/sweetpad/config.toml` (honoring `XDG_CONFIG_HOME`).
- **Global settings** plus optional **per-project overrides** keyed by project
  path: `[projects."/abs/path/to/Proj"]`.
- The tool **reads** this and **never rewrites** it (preserves comments/format).

```toml
# ~/.config/sweetpad/config.toml
[defaults]
configuration = "Debug"

[projects."/Users/me/code/MyApp"]
scheme = "MyApp"
destination = "platform=iOS Simulator,name=iPhone 15"
```

### State — machine-managed
- `~/.local/state/sweetpad/state.toml` (honoring `XDG_STATE_HOME`).
- Holds **remembered interactive selections** (last scheme, destination, …),
  keyed by **project identity = canonicalized workspace/project path**.
- Churns freely; safe for the tool to rewrite.

Precedence note: an authored per-project override in `config.toml` outranks
remembered `state.toml` selections (config > auto, and remembered state feeds
the auto/last-used layer).

## 7. Relationship to the VS Code extension

**Standalone now, adoptable later.** Build the CLI cleanly factored, with a
clear internal API, so the extension *could* later shell out to / drive the
`sweetpad` binary instead of its own TS build logic — but **no migration is
committed**. For now the CLI and extension share only `sweetpad-lib` (the
resolver). Build/simulator orchestration is implemented fresh in Rust for the
CLI.

## 8. Implementation notes

- **Crate layout:** command logic lives in **testable library modules** (a new
  `cli` module tree in `sweetpad-lib`); `src/bin/sweetpad.rs` stays a thin
  entry point dispatching to it. Gated behind the existing default `cli`
  feature.
- **Arg parsing:** `clap` (derive) — auto `--help`/usage, nested subcommands,
  "did you mean", shell completions, env-var binding. This is the first
  substantial dependency beyond `serde_json`; justified under the DOCS §3.1
  policy (don't reinvent *standard* things — a CLI parser is standard).
- **TOML:** a `toml` crate (read-only for config) plus `serde` for
  config/state (de)serialization.
- **Global flags:** `--json`, `--no-color`, `-v/--verbose`, plus the resolution
  flags (`--scheme`, `--configuration`, `--destination`,
  `--project`/`--workspace`).
- **Process orchestration:** spawn and stream `xcodebuild` / `xcrun simctl`;
  parse output for human and `--json` render paths.

## 9. v2 — completing the headless dev loop

Shipped on top of v1, same grammar and plumbing:

```
sweetpad test run [--only-testing ID]… [--skip-testing ID]…
                                     xcodebuild test; --json emits a pass/fail
                                     summary parsed from the .xcresult bundle
sweetpad format run [paths…] [--tool swift-format|swiftlint] [--check]
                                     formats in place (or lints with --check);
                                     each tool reads its own project config
sweetpad device list                 connected physical devices (xcrun devicectl)
sweetpad bsp init [--output PATH]     write buildServer.json for sourcekit-lsp
                                     (reuses the crate's bsp::write_config)
sweetpad completions <shell>          clap_complete-generated scripts
```

`app run` gains the full session experience:

- **`--device` / `--device-id <id>`** — build + install + launch on a physical
  device via `devicectl` (destination becomes `platform=iOS,id=<udid>`).
- **`--mac`** — build and run as a native macOS app: no install step, launch the
  built executable directly (`TARGET_BUILD_DIR/EXECUTABLE_PATH`).
- **inline logs by default** — after launching, follow the app's output:
  `simctl spawn … log stream` on a simulator, `devicectl … launch --console` on
  a device, the executable's own stdout/stderr for macOS. Disable with
  **`--no-logs`**.
- **interactive rebuild session** — at an interactive terminal, `app run` keeps
  the loop under the developer's control instead of auto-watching files: the app's
  output streams from a background child (sim `log stream`, device `--console`, or
  the macOS executable itself) while a single-key reader sits in front. **`r`**
  rebuilds + relaunches on demand; **`q`**, Ctrl-C, or Ctrl-D quit. On each `r`
  the running app is **terminated first** (`simctl`/`devicectl terminate`, or
  killing the macOS process) so the relaunch is always a fresh process picking up
  the new binary — `simctl launch` alone would just foreground the stale one — and
  the app is likewise terminated on quit. A failed rebuild keeps the session alive;
  fix and press `r` again.

  The reader uses a hand-rolled raw mode (`libc`, unix-only) that flips only
  stdin's line discipline (`ICANON`/`ECHO`/`ISIG`/`IEXTEN`), leaving the terminal's
  output post-processing on so streamed logs still render cleanly; clearing `ISIG`
  routes Ctrl-C in as a byte we handle, so the RAII guard always restores the
  terminal on exit. Reads are a non-blocking `VTIME` poll, which lets a watcher
  thread keep reading stdin **during** a build: Ctrl-C there is forwarded as
  `SIGINT` to xcodebuild's process group (so a long build stays abortable without
  leaving raw mode), and any other key pressed mid-build is swallowed so it can't
  queue a spurious rebuild. The build runs `xcodebuild` in its own process group
  with piped stdout fed through the [`buildlog`] beautifier. Non-interactive /
  piped runs (and `--no-logs`) fall back to a one-shot launch + inline follow.

`destination list` aggregates **macOS + simulators + connected devices**, each
with a ready `-destination` specifier. SPM containers are supported for
`scheme`/`build`/`test`/`run`: schemes come from `xcodebuild -list -json` (Xcode
synthesizes them from the manifest, which the pbxproj resolver can't).

Notes / heuristics:
- `test run` exits non-zero on failures; the `--json` summary lands on stdout
  and the failure error on stderr, so both are independently consumable.
- simulator inline logs use a best-effort `processImagePath CONTAINS` log
  predicate; may need refinement per app.
- New deps (under the `cli` feature only): `clap_complete`, `dialoguer`, `libc`
  (the last just for the `app run` raw-mode key reader, unix-only).

A **`cli-smoke` GitHub Actions job** (macOS) generates a real iOS app with
XcodeGen (`ci/fixture-app/`) and runs the actual dev loop — `scheme/project/
settings/destination/simulator/bsp/completions`, then `build start`,
`test run`, `app run` — against live `xcodebuild`/`simctl`. This is the runtime
counterpart to the unit tests below.

## 9b. v3 — toolchain & maintenance commands

Quality-of-life commands on the same grammar and plumbing, aimed at the
everyday frictions raw `xcodebuild`/`xcrun` leave to the user:

```
sweetpad doctor                      diagnose the toolchain (flutter-doctor style):
                                     Xcode/xcodebuild/swift, simulator runtimes,
                                     devicectl, swift-format/swiftlint — each ok/
                                     warning/problem with a fix hint. A missing
                                     required tool is a non-zero exit.
sweetpad derived-data path [--all]   this project's DerivedData folder(s), or the
sweetpad derived-data size [--all]   whole store with --all (size is human + bytes)
sweetpad derived-data purge [--all] [--yes]
                                     delete DerivedData — this project by default
                                     (the safe default), or --all; confirms on a
                                     TTY unless --yes
sweetpad simulator shutdown [NAME]   shut down a sim (defaults to the booted one)
sweetpad simulator erase [NAME]      erase contents & settings (must be shut down)
sweetpad simulator open              open the Simulator.app GUI
sweetpad simulator screenshot [NAME] [--output PATH]
                                     PNG of a booted sim (timestamped by default)
sweetpad simulator appearance <light|dark> [NAME]
                                     toggle a booted sim's UI appearance
sweetpad app open-url <URL> [--simulator NAME]
                                     drive deep / universal links in via
                                     `simctl openurl` (boots the sim if needed)
```

Notes / heuristics:
- `doctor` probes each tool with both stdio streams captured (so the report
  stays clean) and reports the first version line; the runtime-count, summary,
  status-glyph, and `first_line` helpers are pure and unit-tested.
- DerivedData scoping matches Xcode's `<Name>-<hash>` folders by the
  container's file-stem (exact name or `<Name>-` prefix), tested against
  prefix-collision cases (`MyApp` must not match `MyAppHelper-…`).
- the side-effecting `simulator`/`app open-url` actions share one
  simulator picker (`resolve::select_simulator`): explicit name/UDID wins, else
  the lone booted sim, else prompt (booted set, or the full list) / strict
  error off a TTY.

## 9c. v4 — git conflict resolution (.pbxproj + Package.resolved)

`project.pbxproj` is the canonical git merge-conflict nightmare: a flat,
UUID-keyed plist where a line-based merge drops `<<<<<<<` markers in arbitrary
spots and usually yields an unparseable file. This crate already owns both ends
of the fix — a faithful parser ([`pbxproj`]) and a **byte-exact** writer
([`pbxproj_writer`], verified against the whole fixture corpus) — so a *semantic*
three-way merge is a thin layer between them.

Two file kinds are covered: Xcode's `project.pbxproj` (object-graph merge via
[`pbxproj_merge`] + the byte-exact [`pbxproj_writer`]) and SwiftPM's
`Package.resolved` (JSON pin merge via [`spm_resolved`]). Both run on demand
*and* automatically as git merge drivers; the shared plumbing lives in
[`cli::merge`].

```
sweetpad pbxproj resolve [PATHS…] [--force]
sweetpad spm resolve     [PATHS…] [--force]
                                     resolve conflicted .pbxproj / Package.resolved
                                     files mid-conflict. Defaults to every matching
                                     conflicted file in the repo; reads the three
                                     clean inputs from git's index stages (:1: base,
                                     :2: ours, :3: theirs), merges, writes the
                                     result, and `git add`s it. --force recovers the
                                     inputs from HEAD/MERGE_HEAD when git already
                                     auto-merged the file textually. Non-zero exit if
                                     anything is left unresolved.

sweetpad merge install [--global]    register both as git merge drivers
                                     (.gitattributes + `git config`) so plain
                                     `git merge` resolves them automatically.
sweetpad merge driver <KIND> %O %A %B %P
                                     the driver git itself invokes (hidden); reads
                                     git's three temp files and writes the merge over
                                     %A, exiting non-zero on a real conflict so git
                                     leaves the path unmerged (then `<kind> resolve`
                                     shows the structured report).
```

The pbxproj engine ([`pbxproj_merge`]) is pure (no git, no I/O, no Xcode) and
runs the standard three-way rule per UUID-keyed object and per field: identical
edits and one-sided changes resolve silently, disjoint object/array additions
union (reference lists like `children`/`files` are ordered sets, honoring
deletions), and only genuine contradictions — both sides setting the same scalar
differently, or modify-vs-delete — are reported. On any conflict the file is left
untouched, with a graph-path report (`objects/<UUID> (<isa>)/<field>`) of what
collided. The SPM engine ([`spm_resolved`]) is the same shape over `serde_json`:
the `pins` array merges by `identity` (union disjoint pins, take one-sided version
bumps, conflict only on both-sides-bumped-differently), re-rendered to Xcode's
exact `Package.resolved` style (2-space indent, `" : "`, sorted keys, pins sorted
by identity). `originHash` is a derived digest Xcode regenerates, so it is never
treated as a conflict.

Notes / heuristics:
- Reads pristine blobs from git, never the marker-riddled working copy, so the
  textual conflict's placement is irrelevant. The same engines back both the
  on-demand `resolve` commands (index stages) and the `merge driver` (git's temp
  files), so behavior is identical either way.
- The merged pbxproj dict preserves base key order (then ours-only, then
  theirs-only additions) and the parser's single-line layout hint, keeping output
  Xcode-stable and low-churn.
- `merge install` writes the driver to `git config` (per-clone; collaborators run
  it once) and the attribute lines to the repo `.gitattributes` (commit it) — or,
  with `--global`, to global git config + `core.attributesFile`.
- Engines are unit-tested without a Mac (pbxproj: disjoint adds, one-sided delete,
  modify-delete, same-field conflict, array union+delete, layout-hint; spm:
  byte-exact serialize, pin union+sort, version bump, both-bump conflict, add/remove,
  originHash divergence); the end-to-end git driver path is exercised by real
  synthetic merges.
- Later: a `Package.resolved`-style driver for other regenerated lockfiles is the
  same pattern; a built-in `git merge`-driver self-test could pin the integration.

## 9d. v5 — built-in hot reload (`app run --hot`)

`app run --hot` adds **live code injection** to the interactive session: save a
Swift file and the running app picks up the change in-place, with state
preserved — no relaunch, no `r`. **iOS Simulator only** for v5 (codesigning
strips `DYLD_INSERT_LIBRARIES` on devices; watchOS ships no injection dylib).
The full-rebuild `r` path (§9c) stays as the always-available fallback.

> Status: committed design; implementation tracked in the milestones below.

### Architecture — the CLI *is* the injection server

Hot reload (John Holdsworth's InjectionNext/InjectionLite lineage) is always two
halves: a small, stable **client** loaded into the running app, and a **server**
that watches sources, recompiles the changed file to a `.dylib`, and hands it
over. The injected app is the **TCP client** — its `+load` hook
(`ClientBoot.mm`) connects *out* to `127.0.0.1:8887`; whatever is listening
there is the server. `InjectionNext.app` is just one such listener.

**So `sweetpad` becomes the listener.** It binds `:8887` before launch and
serves the same prebuilt client the VS Code extension already injects
(`libiphonesimulatorInjection.dylib` via `DYLD_INSERT_LIBRARIES`) — no new
in-app code, and **`InjectionNext.app` is not required**. This is "Option Y":
the CLI owns the watch + recompile + serve loop itself, rather than delegating
to the menu-bar app or to the in-app standalone watcher.

### Wire protocol (grounded in the upstream `InjectionNextC` source)

- **Transport:** TCP, localhost, port `8887`. Framing is native little-endian:
  `int` = 4-byte `int32`; `string`/`data` = `int32` length then bytes; the EOF
  sentinel is `-1`. A command is an `int32` code then its optional payload
  (`SimpleSocket.mm`).
- **Handshake** — on connect the app pushes, and the server reads in order:
  `int` `INJECTION_VERSION` (4001, validate) · `string` home dir · then an
  `InjectionResponse` stream: `.platform`+string then a bare `string` arch ·
  `.projectRoot`+string (when `INJECTION_PROJECT_ROOT` is set) · `.tmpPath`+string
  · optionally `.executable`+string. These tell the server the platform/arch/sdk
  context to compile for.
- **Server → app** (`InjectionCommand`): the two that matter for v5 are
  `.load`+`string dylibPath` (app `dlopen`s that host path directly — works on
  the simulator, which shares the host filesystem) and `.inject`+`name`+`data`
  (ship the bytes; for devices, out of scope now). Optionally `.xcodePath`+string
  up front so the client's reloader knows the toolchain.
- **App → server** after a load: `.injected` / `.failed` / `.unhide` — surfaced
  as a session status line.

### Build & launch wiring

Two hooks, mirroring the extension's proven `hot-reload.ts` path:

- **Build flags** — `[`crate::cli::xcodebuild::BuildPlan`]` gains, under `--hot`
  and gated to simulator SDKs: `OTHER_LDFLAGS=$(inherited) -Xlinker -interposable`
  (lets dyld swap symbols at runtime) and `EMIT_FRONTEND_COMMAND_LINES=YES`
  (needed to recover compile commands on Xcode 16.3+; see the recompiler below).
  Both are gated to `--hot` so ordinary `build`/`run` never pay for them.
- **Launch env** — `[`crate::cli::simctl`]` gains an env-passing `launch`
  variant; `--hot` sets `SIMCTL_CHILD_DYLD_INSERT_LIBRARIES=<client dylib>`,
  `SIMCTL_CHILD_INJECTION_PROJECT_ROOT=<workspace root>`, and the XCTest
  `DYLD_FRAMEWORK_PATH`/`DYLD_LIBRARY_PATH` the client dylib's deps need
  (`simctl` forwards any `SIMCTL_CHILD_*` var into the launched process).

**Beautifier interaction (`EMIT_FRONTEND_COMMAND_LINES` × §11).** The setting
prints the `swift-frontend` invocations into xcodebuild's *raw* transcript, but
those lines start with a tool path, not a task verb, so `[`buildlog::parse_line`]`
classifies them as `Event::Other`, which `[`buildlog::render`]` suppresses unless
`-v` — the same path that already swallows xcodebuild's per-task command echoes.
So the **beautified default stream is unchanged** (no extra verbosity, nothing
broken; they can't reach the diagnostic matcher, which requires `: error:`/
`: warning:`/`: note:` markers a command line never carries). The only cost is a
larger raw transcript, paid only under `--hot`. Because parsing is decoupled from
rendering, path A captures the **raw** frontend lines for the recompiler index in
parallel with (not instead of) beautification — both consume the same stream, so
there is no double-printing and no leakage into the pretty output.

The server must be listening on `:8887` before the app launches so the client's
`+load` connect succeeds.

### The recompiler — resolver-first (F), live-capture fallback (A)

Turning a saved `Foo.swift` into a loadable `.dylib` is the load-bearing risk.
The upstream approach (InjectionLite's `LogParser`/`Recompiler`) **scrapes the
build logs**: `gunzip` the newest `*.xcactivitylog` in DerivedData, `grep` for
the ` -primary-file Foo.swift ` frontend invocation, regex-rewrite it down to a
single-primary `-c -o eval.o`, then regex out `-sdk` to assemble a fixed
`clang -dylib -interposable …` link line. It works, but it rides an undocumented
log format that shifts every Xcode release and breaks under log pruning, Whole-
Module mode, and `COMPILATION_CACHE_ENABLE_CACHING`. We do **not** take that as
the primary path.

Both implemented strategies instead converge on running **one
`swift-frontend -primary-file` job** for the changed file (single-file speed) and
linking it into a dylib; they differ only in where that frontend command comes
from. Recovered commands are **cached per source** (stable until the file
set/settings change), so the per-save cost is just compile + link.

**(F) Default — resolver → frontend via `swiftc -###`.**
`[`crate::compiler_args`]` produces, from the resolved pbxproj/xcspec settings
(snapshot-tested against real `xcodebuild`), the target's **driver** `swift_arguments`.
But single-file compilation is a **frontend** (`-primary-file`) operation, and the
two flag vocabularies differ — so the recompiler asks the *user's own toolchain
driver* to translate: `xcrun swiftc -### -disable-batch-mode <driver args>
<module files>` is a **dry run** that prints the `swift-frontend` jobs it *would*
spawn (one `-primary-file` per file). We parse those, cache each by source, and
on a save run the changed file's job (rewritten to a single `-o eval.o`) then a
`clang -dynamiclib -interposable -undefined dynamic_lookup` link. No build-log
dependency, no Xcode-version log-format drift, and because `-###` uses the
*active* toolchain the driver/frontend/version all match by construction. If
`-###` recovery ever fails, it falls back to whole-module `swiftc -emit-library`.
(We deliberately do **not** link `swift-driver` as a library: a vendored driver
wouldn't match the user's Xcode — the same skew we avoid everywhere — and the
cached one-shot spawn makes per-save cost ~0 anyway.)

**(A) Switchable — capture frontend command lines from our own build.**
Because the CLI *is* the builder, the `--hot` build tees the `swift-frontend`
invocations straight out of `xcodebuild`'s stdout (`EMIT_FRONTEND_COMMAND_LINES`)
— so the exact per-file command is a **free byproduct**, no `-###` spawn at all.
Same single-file/link path as (F), sourced from the transcript and cached per
source. Selected with `--hot-recompiler buildlog`.

### Module layout & session integration

A new `cli/inject/` tree, kept off the existing tool-spawning modules:
`protocol.rs` (the two enums + framing primitives), `socket.rs` (the `:8887`
TCP listener), `server.rs` (accept + handshake + command loop), `recompiler.rs`
(F + A), `watcher.rs` (debounced FS watch of the workspace root, ignoring build
output dirs). The server runs as a sidecar thread alongside the existing
`Running` struct in `[`crate::cli::commands::app`]`; the watcher becomes a third
event source next to the keypress reader and the log stream. `r` still does a
full rebuild+relaunch; `q`/Ctrl-C/Ctrl-D quit and tear the server down.

### Milestones

> **Milestone 1: ✅ validated** — run #5 of `hot-reload-spike.yaml` on a real
> arm64 simulator: the Rust server completed the `:8887` handshake (`version 4001`,
> `iPhoneSimulator arm64`, projectRoot/tmpPath/executable), recompiled the changed
> file, linked a dylib, sent `.load`, and the in-app client confirmed `.injected`.
> The novel socket protocol and the build→load→patch chain are proven.

1. **Socket spike — ✅ done.** Validated transport + a recompile/`.load`/`.injected`
   round-trip using the **(A)** live build-log command.
2. **Build-flag + launch-env plumbing — ✅ done.** `BuildPlan.hot` appends
   `-interposable` + `EMIT_FRONTEND_COMMAND_LINES`; `simctl::launch_with_env`
   forwards the `SIMCTL_CHILD_*` injection vars (`app run --hot`, simulator-gated).
3. **Recompiler — ✅ done.** Both strategies in `cli/inject/recompiler.rs`
   converge on a cached single-file frontend command: **F** (default) recovers it
   from the resolver via `xcrun swiftc -###` (whole-module `-emit-library`
   fallback); **A** (`--hot-recompiler buildlog`) recovers it from the captured
   transcript. (F's `-###` path wants the macOS CI's confirmation; A is proven.)
4. **Watcher + session integration — ✅ done.** Polling watcher → `server.inject`;
   `run_hot_session` builds + serves + launches + watches; key loop keeps `r`
   (full rebuild, client reconnects) / `q`; `.injected`/`.failed` status lines.
5. **Client build from source — ✅ done & validated.** `client.rs::build_and_cache`
   clones the pinned InjectionNext (with submodules) and `xcodebuild`s it against
   the **active Xcode**, caching the whole built **`InjectionNext.app`** per Xcode
   build id; `resolve_dylib` order is override → per-Xcode cache →
   build-from-source → `InjectionNext.app` fallback. We cache the *entire* `.app`
   (not just the dylib): `lib<sdk>Injection.dylib` is a symlink into a companion
   `*.bundle` whose Swift/XCTest deps resolve at load time via `@loader_path`
   (`build_bundles.sh`), so a lone copied Mach-O loads + connects but fails to
   inject — keeping the bundle intact mirrors the proven installed-app /
   prebuilt-release layouts. Validated green by the `hot-reload-src` CI job, which
   runs the real `app run --hot` (no dylib override) and injects on **both Xcode
   16 and 26** — the prebuilt-binary version skew is gone.
6. **Polish — ✅ mostly done.** "Inject package missing" advisory ported;
   teardown (watcher/server/app/cleanup) wired. (Config-level default for the
   recompiler mode — beyond the `--hot-recompiler` flag — is the remaining nicety.)

> **Implementation status:** the `cli/inject/` module + `app run --hot` are
> implemented and **validated end-to-end on real simulators** (Xcode 16 + 26),
> both recompilers, with the client built from source per Xcode. `clippy -D
> warnings`/`fmt` clean, unit tests on Linux, live e2e on the macOS matrix.

### Client distribution — vendor full source, compile per Xcode (decided)

The client is the **full InjectionNext source, vendored unmodified** (pinned
revision; MIT — InjectionNext + InjectionImpl + SwiftTrace + DLKit + SwiftRegex),
compiled **on-demand and cached per Xcode build id**, then
`DYLD_INSERT_LIBRARIES`-injected at launch. The key move: building *from source
against the user's active Xcode* makes the XCTest ABI match automatically — so
the per-Xcode skew that broke a *prebuilt* binary under Xcode 16.4 (Milestone 1)
never arises. Given that, there is **no reason to strip XCTest or write a minimal
client** (the analysis — ~4 jobs; vendor-and-strip ≈ ½ week, from-scratch v0 ≈ a
week and weeks for parity — concluded the effort buys nothing here). Test
hot-reload therefore comes along as a **latent capability**; the promoted feature
is still app UI/code reload (SwiftUI/UIKit).

- **No per-Xcode binary matrix.** Cache key = Xcode build id (`xcodebuild
  -version`); a miss recompiles the vendored client once (first `--hot` after an
  Xcode change), hits reuse it. Cache at e.g.
  `~/.cache/sweetpad/hot-reload/<xcode-build>/`.
- **Drop-in UX preserved** — no project edit, no `InjectionNext.app`. The SwiftUI
  `@ObserveInjection`/`.enableInjection()` annotations remain the user's to add
  (UIKit reloads without them).
- **Build mechanism — `xcodebuild` on the vendored InjectionNext project
  (decided).** The package set has C/asm and multi-package deps, so the client is
  built with `xcodebuild` against InjectionNext's own Xcode project targeting the
  simulator SDK — **not** raw `swiftc` or `swift build`. Rationale: first-class
  iOS-simulator targeting, it produces the `iOSInjection.bundle`/dylib artifact
  directly, and it reproduces (and inherits the per-Xcode maintenance of)
  upstream's intended build. `swift build` was rejected — its iOS-sim support is
  finicky, it doesn't naturally emit the bundle, and the recipe would become our
  burden (the Milestone-1 `swift build` probe also tripped on the upstream repo's
  dev symlinks to sibling SwiftTrace/DLKit/InjectionImpl checkouts).

### Open decisions

- **ABI match — A proven, F pending.** Path A (exact build-log command) injects
  cleanly (Milestone 1), so it is primary. The (F) resolver path's ABI match is
  still to confirm; until then F is an optimization, not the default.
- _(Resolved: keep the full vendored client — no XCTest strip / no minimal
  rewrite; build it with `xcodebuild` for the simulator, cached per Xcode build
  id. See Client distribution above.)_

### macOS test harness (permanent)

Hot reload needs macOS + Xcode + a simulator, so it's validated in CI by the
permanent **`xcode-tests.yaml`** workflow — a reusable matrix harness for any
Xcode/simulator-requiring test, across Xcode versions (16.x, 26.x; weekly + on
push/PR). Two jobs:

- **`cli`** — the full standalone-CLI e2e (`ci/smoke.sh`) on each Xcode.
- **`hot-reload`** — the injection e2e (`ci/hot-reload-e2e.sh`): it generates the
  fixture app, downloads the InjectionNext client dylib, and runs the *real*
  `sweetpad app run --hot --hot-selfcheck` (hidden flag) for **both** recompilers
  (resolver + build-log). The self-check builds with the interposable/frontend
  flags, starts the `:8887` server, launches with the client injected, edits a
  Swift file once, and asserts `.injected` — exiting non-zero otherwise. It runs
  on the Xcode matching the prebuilt client; once the per-Xcode client build
  lands (Milestone 5) it joins the full matrix.

This supersedes the original throwaway spike (`hot-reload-spike.yaml`), whose
run #5 first proved the socket + recompile→load→inject chain end-to-end.

## 10. Testing

The CLI modules carry inline `#[cfg(test)]` units that need no Xcode, so the
tool-spawning code is pinned without a Mac:

- **Arg-vector snapshots** — `BuildPlan`/`TestPlan` produce exact `xcodebuild`
  argument vectors (the main guard against silent flag drift).
- **Parser fixtures** — `simctl list`, `devicectl list`, `xcresulttool`
  summary, and `-showBuildSettings` JSON parsed from captured-shape payloads
  (this caught a missing `rename_all` on the devicectl device struct).
- **Pure logic** — resolution precedence, config/state TOML round-trips,
  `choose` fallback branches, destination/`udid` parsing, and the session
  key → action mapping (`r` rebuild / `q`·Ctrl-C·EOF quit / else ignore).
- **Inject protocol** (§9d) — the little-endian `int`/`string`/`data` framing
  and the handshake parse (version + platform/arch/projectRoot/tmpPath) round-
  trip against captured byte sequences; the resolver→single-file→dylib argv
  transform is an arg-vector snapshot, like `BuildPlan`. The live-injection
  truth (a save actually swaps in the running sim) lands in the `cli-smoke` job.

The *runtime* truth (does xcodebuild actually build, does the log
predicate/console attach behave) is exercised by the `cli-smoke` macOS job.

## 11. Build-log beautifier

`build`/`test` output is beautified natively (no `xcbeautify` dependency):
[`buildlog`] parses each raw `xcodebuild` line into a structured [`buildlog::Event`]
(compile/link/sign/diagnostic/test/result), then renders a concise, colorized
stream. Parsing is decoupled from rendering so the events can also feed CI
summaries or diagnostics later. `-v` passes raw output through; `--json` stays
quiet. `parse_line` is pure and unit-tested without Xcode.

## 12. Open / later

- SPM `app run` runs executable products on the host via `swift run <product>`
  (`--device`/`--mac` don't apply; library packages have nothing to run).
- Whether the extension actually adopts the CLI as its engine.
- (Declined for now: `tools` resource, `config`/`state` subcommands.)
