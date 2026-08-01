# SweetPad CLI — design

The `sweetpad` binary's standalone, headless command set: **"xcodebuild for
humans."** A pure native front-end to the `sweetpad-lib` Rust engine for
running, building, and exploring Xcode projects without an editor.

It lives in the **same `sweetpad` binary** as the existing `vscode` namespace
(a generic JSON-RPC client that controls the VS Code extension — see
`src/vscode_cli.rs`). `vscode` stands on its own; the new resources sit beside it.

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

**Verb shortcuts for the daily loop.** The hot-path commands also work without
their action token — `sweetpad build` (= `build start`), `sweetpad test`
(= `test run`), `sweetpad app` / the flagship `sweetpad run` (= `app run`),
`sweetpad format` (= `format run`) — plus `sweetpad clean` and the aggregated
`sweetpad devices` view. Bare `sweetpad` prints the status view inside a
project. Aliases: `dep`, `sim`, `dd`, `fmt`.

**Flag policy.** `--yes` skips a confirmation prompt (the action is unchanged);
`--force` overrides a safety check (e.g. `project new --force` scaffolds into a
non-empty directory; `merge`-family `--force` redoes work git considers done).
`--all` widens scope to "everything in this store" (`derived-data --all`,
`context remove --all`); commands that act on "every item" by default express
it by *omitting* a positional (`dependency update`), not with `--all`.

**Deletion verbs are domain-faithful, not uniform:** `simulator erase` (Apple's
term for factory-reset), `context remove`/`dependency remove` (take something
out of a collection), `derived-data purge` (delete stored data wholesale).

**`--target` glossary.** The word is overloaded by the domain itself: a build
target (`settings show --target`), a link target (`dependency add --target`),
the default *test* target (`context select target --testing`), and the
positional simulator argument named TARGET on `simulator` verbs. Each command's
help says which one it means.

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
sweetpad app launch                  launch an already-installed app (--mac: detached)
sweetpad app logs                    stream app logs (macOS: os_log + captured stdout; §9h)
sweetpad app stop                    kill the running app

sweetpad vscode <method> [--flag …]  control the VS Code extension (JSON-RPC)
```

`build` stays purely "compile"; the full run/install/launch/logs/stop lifecycle
groups under `app`, the noun it acts on.

Out of scope for v1 (later iterations): `test`, `format`, `device` (physical)
management, `bsp` (autocomplete config), `tools` (Homebrew).

### The surface as shipped (post-audit, July 2026)

The audit pass grew the tree beyond the v1 sketch:

```
sweetpad                          status view in a project; help outside one
sweetpad run [--on X] [--hot]     the flagship loop (= app run)
sweetpad build [--clean|--watch|--show-command] [-- XCODEBUILD_ARGS]
sweetpad build diagnostics        last build's errors/warnings, no rebuild
sweetpad test [--failed|--retry-flaky N|--coverage|--junit P|--watch]
sweetpad clean [--purge]          xcodebuild clean; --purge adds DerivedData
sweetpad archive [--export-method M] [--no-export]
sweetpad devices                  everything runnable, specifier-ready
sweetpad status / open / doctor / self-update / help <topic>
sweetpad simulator <boot|create|delete|clone|push|privacy|status-bar|
                    location|media-add|record|screenshot|…>   (alias: sim)
sweetpad app <run|install|launch|debug|diagnose|uninstall|logs|stop|open-url|
              screenshot|ui>           (screenshot: simulator or macOS window, §9h;
                                        ui: drive a macOS app's UI, §9i;
                                        debug --batch / diagnose: scriptable lldb, §9j;
                                        logs: os_log + captured stdout on macOS, --source/--last, §9h)
sweetpad merge <install|run>      semantic conflict resolution (pbxproj/spm
                                  are hidden aliases)
sweetpad context <show|select|set|alias|remove>
sweetpad settings show            resolved build settings (porcelain; §9f/§9g)
sweetpad pbxproj <resolve|settings|folder|membership|fileref|group>  plumbing (§9g)
```

Destination selection is `--on <ref>` (fuzzy name / `booted` / `mac` /
`device` / platform word / UDID / a `context alias` name), with
`--destination` as the raw escape hatch. `-o json|ndjson` is the machine
surface (§4).

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
- **Sources are a synchronized root group** (`objectVersion = 77`, the fresh
  Xcode 16 template shape, §9f): the pbxproj carries no per-file objects, so
  adding a source file is creating it on disk — the project file never changes
  as the app grows. `sweetpad pbxproj folder`/`membership` (§9g) manage
  folders and exceptions.
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
- Color **auto-disables** when stdout is not a TTY or when `NO_COLOR` is set
  (non-empty, per the no-color.org spec); `--no-color` forces it off;
  `CLICOLOR_FORCE`/`FORCE_COLOR` force it back on when piped (an explicit
  `--no-color`/`NO_COLOR` still wins).
- Errors: human messages on **stderr** by default; **structured error objects**
  under `--json`. Meaningful exit codes.

### The JSON envelope

- Success: `{"schema": 1, "ok": true, "data": …}` on **stdout**,
  pretty-printed. Errors: `{"schema": 1, "ok": false, "error": {code,
  message}}` on **stderr**, compact single-line by design (robust to scrape
  even when child-process stderr interleaves).
- **`ok` means "the command executed"**, not "the outcome was good": a red
  test suite is `ok: true` with `data.passed: false` (exit 3), `doctor` with
  problems is `ok: true` with per-check statuses (exit 1), `format --check`
  reports findings in `data` (exit 3). Every such payload carries its own
  status field — read that, not `ok`.
- **`schema` bump policy:** additive fields never bump it; a removed or
  re-typed field bumps it. Consumers should tolerate unknown fields.
- **Exceptions:** the live session `app run` rejects `--json` (build and
  launch as separate steps instead); `app logs --json` emits a *stream* of raw
  `log stream` NDJSON events (one JSON object per line, no envelope; on macOS,
  captured stdout/stderr lines are `{"source":"stdout",…}`);
  `completions` ignores it. Clap usage errors (exit 2) print clap's human
  text on stderr regardless of `--json`.

### The NDJSON stream (`-o ndjson`)

- **stdout carries only events**: one compact JSON object per line, each with
  an `event` discriminator (`task`, `diagnostic`, `test`, `suite`), and the
  stream ends with exactly **one** terminal line —
  `{"event":"result","ok":true,"data":…}` on success, or
  `{"event":"result","ok":false,"error":{code,message}}` on failure (the
  compact stderr envelope is also emitted, as the machine-parsed error
  surface). Non-streaming commands degenerate to just the result line.
- Human chatter stays on stderr; child tools are run captured/quiet so their
  raw stdout never interleaves with the events.
- **Exceptions:** `app run` rejects ndjson like it rejects `--json`;
  `--watch` is refused under both machine modes (a rerun-forever loop has no
  terminal result); `app logs` passes through the raw `log stream` events
  (plus `{"source":"stdout",…}` lines for a macOS app's captured console).
- `--gh-annotations` conflicts with both machine modes (workflow commands and
  the envelope/event stream both claim stdout) and is rejected up front.

### Exit codes

```
0  success                      4  target resolution failed (no/unknown
1  generic failure                 scheme, destination, simulator, …)
2  usage error (owned by clap)  5  required tool missing (xcodebuild, …)
3  build or test failure        6  cancelled by the user (a declined or
                                   Esc'd prompt, Ctrl-C in a session)
```

A SIGINT/SIGTERM that kills the process exits `128 + signo` (130/143), after
the handler restores the terminal and reaps children.

## 5. Target resolution

What the command acts on (workspace/project, scheme, configuration,
destination) resolves by **layered precedence**:

```
explicit flag  >  env var  >  config file  >  remembered state  >  auto-discovery
```

- **Auto-discovery:** find the `.xcworkspace` / `.xcodeproj` / `Package.swift`
  in the working directory (deterministic: same-kind siblings resolve to the
  alphabetically first, with a warning; pass `--workspace`/`--project` to
  disambiguate). The search walks *up* toward the git root, so a command works
  from a nested source directory.
- **Container resolution** runs on its own shorter ladder — `--workspace` /
  `--project` > a `sweetpad.toml` `workspace`/`project` key > auto-discovery —
  because every layer below it (per-project config, remembered state) is
  *keyed by* the container and so can't take part in finding it.
- **Downward scan**, once the upward walk finds nothing: look below the working
  directory, then below the repository root, at most two levels down. This is
  what makes the common nested layouts work with no configuration at all —
  `ios/App.xcodeproj`, `Sources/App.xcodeproj`, `apps/ios/App.xcodeproj` — none
  of which the upward walk can reach from the repository root. Build output and
  vendored trees (`Pods`, `node_modules`, `Carthage`, `vendor`, `DerivedData`,
  `build`, dotfiles) are never entered, so `Pods/Pods.xcodeproj` is never a
  candidate, and symlinks are never followed.
  - The **shallowest** level holding anything wins outright, and within a
    single directory the usual workspace > project > package ordering applies
    — so a `ios/` holding both a workspace and a project resolves silently to
    the workspace, as CocoaPods and React Native layouts want.
  - Directories **tied at that depth** (Flutter's `ios/` and `macos/`, a
    monorepo's `apps/*`) are a real choice, not a tiebreak: the command errors
    with every candidate listed and names both ways to settle it. This is a
    deliberate split from same-kind siblings *in* the working directory, which
    warn and take the alphabetically first — standing in a directory is a
    signal of intent, a hit two levels down is not.
  - A scan-resolved container is **announced** (`using Sources/App.xcodeproj
    (found below the current directory)`), since the user never named it.
  - Bare `sweetpad` treats a tie as "no container" and prints the help wall:
    the status-or-help probe has nowhere to put a question, and `sweetpad
    status` reports the ambiguity properly a keystroke later.
- **Env vars:** `SWEETPAD_SCHEME`, `SWEETPAD_DESTINATION`,
  `SWEETPAD_CONFIGURATION`, `SWEETPAD_SDK`, `SWEETPAD_PROJECT` /
  `SWEETPAD_WORKSPACE` (value-carrying, folded into the flag layer — but an
  explicitly *typed* flag still beats an exported env var, so
  `SWEETPAD_WORKSPACE` can't override a typed `--project`), and
  `SWEETPAD_NONINTERACTIVE` (boolean). Boolean `SWEETPAD_*` vars parse
  truthiness: `0`/`false`/`no`/`off`/empty mean **off**.
- **Remembered state:** the last interactive picks, saved per project, feed the
  layer just above auto-discovery so the daily loop doesn't re-prompt (§6).
  Only picker-settled values are remembered — a one-off flag/env/config
  override never rewrites the stored context, and `app run --mac`/`--device`
  destinations are never remembered.
- **Interactive fallback:** when something is ambiguous/unset **and the
  terminal is interactive** (stderr is a TTY, no `--json`, no
  `--non-interactive`/`SWEETPAD_NONINTERACTIVE`), drop to a fuzzy picker
  (choose a scheme/destination from a menu). **Non-interactive/CI stays
  strict** and errors instead of prompting.
- **Validation:** an explicitly-requested scheme/configuration is checked
  against the project's candidates up front, with a did-you-mean hint — and
  the `Debug` configuration default applies only when the project actually has
  a `Debug`; otherwise the configuration picker runs.
- **Testing:** the `test` action resolves a *separate* testing context — testing
  config/state layered over the build context — so tests can pin their own
  scheme/configuration/destination (§6, "Context").

## 6. Configuration & state

**No files are written to the project root.** Two distinct stores, kept apart so
the tool never clobbers hand-authored config:

### Config — hand-authored only
- `~/.config/sweetpad/config.toml` (honoring `XDG_CONFIG_HOME`).
- **Global settings** plus optional **per-project overrides** keyed by the
  canonicalized **container** path — the `.xcworkspace`/`.xcodeproj`/
  `Package.swift` itself, *not* the directory holding it:
  `[projects."/abs/path/to/Proj.xcodeproj"]`.
- The tool **reads** this and **never rewrites** it (preserves comments/format).
  Unknown keys and `[projects."…"]` keys that can't match a real container are
  **warned about** on load (with a did-you-mean where possible), never
  silently ignored.

```toml
# ~/.config/sweetpad/config.toml
[defaults]
configuration = "Debug"

[projects."/Users/me/code/MyApp/MyApp.xcodeproj"]
scheme = "MyApp"
destination = "platform=iOS Simulator,name=iPhone 15"

# Test-action overrides, layered over the build values for `test` only.
[projects."/Users/me/code/MyApp/MyApp.xcodeproj".testing]
configuration = "Test"
```

- Keys per table: `scheme`, `configuration`, `destination`, `sdk`. Each table
  also accepts a `[….testing]` sub-table — `scheme`/`configuration`/
  `destination`/`target` overrides used by `test`, falling back to the build
  values when unset (mirrors the extension's `sweetpad.testing.*` settings);
  `target` narrows the run to `-only-testing:<target>` when no explicit
  selector is given.

### Project file — committed, team-shared
- An *optional, hand-authored* `sweetpad.toml` at the project root. This is how
  a team standardizes: it travels with the clone, needs no absolute paths, and
  sweetpad still **never writes** to the project root — the file is read-only
  to the tool, like the user config.
- Precedence slot: **user config > sweetpad.toml > remembered state**.
- **Located by walking up** from the working directory to the git root, so one
  file serves the whole checkout. It may sit beside the container (the common
  case) or above it, where `workspace`/`project` names a container nested
  below — the layout `xcodebuild` can't be pointed at without `-C`:

```
App/
  sweetpad.toml          project = "Sources/App.xcodeproj"
  Scripts/               `sweetpad build` works from here too
  Sources/App.xcodeproj
```

- Keys: `workspace`/`project` (which container this file belongs to, relative
  to the file — `workspace` wins if both are set), `scheme`, `configuration`,
  `destination`, `sdk`, a `[testing]` sub-table
  (`scheme`/`configuration`/`destination`/`target`), plus the tool defaults
  that previously had flags but no home:

```toml
# sweetpad.toml (committed)
project = "Sources/MyApp.xcodeproj"   # only when it isn't a sibling
scheme = "MyApp"
configuration = "Debug"

[testing]
configuration = "Test"

[run]
hot = true                 # `app run` defaults to hot reload (--no-hot opts out)
hot_recompiler = "resolver"

[format]
tool = "swiftlint"
```

- Unknown keys are warned about; a malformed file is warned about and ignored
  (a broken committed file must not brick every teammate's CLI). An absolute
  `workspace`/`project` warns too — it resolves to nothing on every other
  machine, the one mistake that makes a committed file worse than none.
- A declared container that doesn't exist is an **error**, never a fallback to
  discovery: the file said which project this is, and quietly building a
  different one is worse than stopping. When `generator` is set, the error
  names the tool to run — the usual cause is a project that hasn't been
  generated yet.

### State — machine-managed
- `~/.local/state/sweetpad/state.toml` (honoring `XDG_STATE_HOME`).
- Holds the **remembered build context** (scheme, configuration, sdk,
  destination), a separate **testing context**, the **recent/most-used
  destinations**, and the **last launched app** — keyed by **project identity =
  canonicalized workspace/project path**.
- Churns freely; safe for the tool to rewrite. Manage it with `context` (below),
  not by hand.

Precedence note: an authored per-project override in `config.toml` outranks
remembered `state.toml` selections (config > auto, and remembered state feeds
the auto/last-used layer).

### Context — inspect & manage remembered state

`sweetpad context` is the first-class way to view and edit the remembered
selections, instead of hand-editing `state.toml` or relying on a build command's
prompt-and-remember side effect. It mirrors the richer context the VS Code
extension keeps in its workspace state (§7).

```
sweetpad context show                  print the build + testing context, recent
                                       destinations, and last launched app (--json)
sweetpad context select [VAR]          set a variable interactively (no VAR → the
                                       core: scheme, configuration, destination)
sweetpad context remove [VAR] [--all]  clear a variable, or the whole context
```

- **Variables:** `scheme`, `configuration`, `destination` (both contexts),
  `sdk` (build-only), `target` (testing-only). `scheme`/`configuration` reuse the
  project's pickers; `destination` uses the simulator picker below.
- **`--testing`** on `select`/`remove` acts on the testing context; otherwise the
  build context. `--all` clears the whole project entry (or just the testing
  sub-context with `--testing`); emptied entries are pruned.
- **Adaptive destination picker.** Picking a destination records it into the
  project's recents and usage counts; the picker then orders **most-used first,
  then booted (marked `●`), then a deterministic platform → newest-OS → family →
  natural-name sort** — so your habitual target sits on top.
- **Testing precedence.** `test` resolves each field as `flag > testing config >
  testing state > build config > build state`, so a pinned testing override wins
  and `test` otherwise follows the build selection.

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
- **Universal flags:** `--json`, `--no-color`, `-v/--verbose`, `-q/--quiet`,
  and `--non-interactive` are global — accepted on every command and its
  actions. `--quiet` mutes progress chatter (notes, spinners, step labels, the
  beautified build stream apart from diagnostics and failure banners) while
  errors and primary data/JSON still emit; it wins over `--verbose`.
  `--non-interactive` (or `SWEETPAD_NONINTERACTIVE`) forces the strict no-
  prompt behavior at a TTY. The **targeting flags**
  (`--workspace`/`--project`, `--scheme`, `--configuration`, `--destination`,
  `--sdk`) are scoped to the commands that consume them, in three tiers —
  container-only (`project`, `bsp`, `derived-data`), container plus `--scheme`
  (`scheme`), and the full build target (`build`, `test`, `settings`, `app`).
  Within a resource they are global, so they parse on either side of the
  action token: both `sweetpad build --scheme App run` and
  `sweetpad build run --scheme App` work. A resource that doesn't consume a
  tier never advertises its flags (e.g. `project info` rejects
  `--destination`).
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
sweetpad bsp init [--output-file PATH]  write buildServer.json for sourcekit-lsp
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
`scheme`/`build`/`test`/`run`: schemes are read straight from the manifest via
`swift package dump-package` (the product names xcodebuild would synthesize —
no xcodebuild spawn, no pbxproj needed).

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
sweetpad simulator screenshot [NAME] [--output-file PATH]
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
preserved — no relaunch, no `r`. Targets: the **iOS Simulator** and **native
macOS apps** (`--mac --hot`; see the macOS subsection below). Physical devices
are out (codesigning strips `DYLD_INSERT_LIBRARIES`); watchOS ships no
injection dylib. The full-rebuild `r` path (§9c) stays as the always-available
fallback.

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

- **Build flags** — `[`crate::cli::xcodebuild::BuildPlan`]` gains, under `--hot`:
  `OTHER_LDFLAGS=$(inherited) -Xlinker -interposable`
  (lets dyld swap symbols at runtime) and `EMIT_FRONTEND_COMMAND_LINES=YES`
  (needed to recover compile commands on Xcode 16.3+; see the recompiler below).
  Both are gated to `--hot` so ordinary `build`/`run` never pay for them.
- **Launch env** — `[`crate::cli::simctl`]` gains an env-passing `launch`
  variant; `--hot` sets `SIMCTL_CHILD_DYLD_INSERT_LIBRARIES=<client dylib>`,
  `SIMCTL_CHILD_INJECTION_PROJECT_ROOT=<workspace root>`, and the XCTest
  `DYLD_FRAMEWORK_PATH`/`DYLD_LIBRARY_PATH` the client dylib's deps need
  (`simctl` forwards any `SIMCTL_CHILD_*` var into the launched process).

### macOS (`--mac --hot`)

The same server/recompiler/watcher drive a native mac app; only the launch and
the signing posture differ:

- **Injectability is settled at build time.** A macOS `--hot` build adds
  `ENABLE_HARDENED_RUNTIME=NO` and `ENABLE_APP_SANDBOX=NO` — command-line
  settings outrank project ones, so the hot Debug product is built without the
  two protections that break injection (the hardened runtime makes dyld strip
  `DYLD_INSERT_LIBRARIES` and library validation reject the ad-hoc recompiled
  dylibs; the sandbox blocks the client's socket and dlopen from outside the
  container). No post-build re-signing, no project mutation. Xcode 14+ mac
  templates declare both protections via exactly these settings, so a template
  app is injectable with zero setup.
- **Preflight.** A sandbox declared in an explicit `.entitlements` file (App
  Store projects) is beyond build settings — it is auto-stripped for the hot
  build (see *Zero-config sandbox stripping* below), and
  `[`crate::cli::inject::mac_preflight`]` stays as the safety net: it inspects
  the built product (`codesign -d`) and refuses with the exact fix when the
  sandbox survived (stripping disabled or failed) instead of launching a dead
  session. A hardened product (re-signed by a run-script phase) is refused the
  same way, unless it carries the `allow-dyld-environment-variables` +
  `disable-library-validation` entitlements pair that makes it injectable anyway.
- **Direct spawn, raw env.** The mac app is our own child process — the same
  injection env as the simulator's but unprefixed (no `SIMCTL_CHILD_`, no
  install step), stdout/stderr piped through the session console. `r` kills and
  respawns the child; `d` detaches leaving it running.
- **Bundled mac client.** `vendor/injection-client/build.sh` produces a second
  prebuilt (`SweetpadInjectionClientMac.dylib`, the upstream SPM product built
  for `generic/platform=macOS`), embedded alongside the simulator client and
  selected by SDK. `InjectionNext.app` stays the fallback.
- **Connect watchdog.** A mac app that hasn't dialed back within 15s is running
  uninjected (something undid the insert env); the session says so, with the
  `codesign` command that shows what the product carries.
- **Validated end-to-end** by `ci/hot-reload-e2e.sh`: the `--hot-selfcheck`
  nonce round-trip (edit → `.injected` → the new code's marker observed in the
  host unified log) passes for both recompilers against the fixture's
  `SweetpadCIMac` scheme.

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

### Zero-config sandbox stripping (`--hot` on macOS)

You cannot inject into a sandboxed app — dyld sanitizes the environment, the
container blocks the CLI's socket and dylib paths — so "hot reload on a
sandboxed project" can only ever mean *automating the un-sandboxing*; there
is no keep-the-sandbox variant to chase. The build-setting overrides above
already handle the common case, but when the sandbox comes from an explicit
`CODE_SIGN_ENTITLEMENTS` plist (the normal shape for App Store projects),
the plist wins at signing. Until v8 the only fixes were user-side and
permanent (edit the project, or hand-pass an override every run); now `--hot`
is zero-config:

1. **Resolve the effective entitlements** for the (scheme → app target,
   configuration, macOS) being hot-built, via the in-process build-settings
   resolver — the engine behind `settings show`, so `$(SRCROOT)` interpolation
   and conditional `CODE_SIGN_ENTITLEMENTS[sdk=macosx*]` spellings come for
   free. No explicit file ⇒ nothing to do (the `ENABLE_APP_SANDBOX=NO`
   override suffices).
2. **Strip ephemerally.** If the plist asserts `com.apple.security.app-sandbox`,
   copy it to the hot-reload cache
   (`~/.cache/sweetpad/hot-reload/entitlements/<projhash>/<config>-nosandbox.entitlements`),
   delete the sandbox key (everything else stays — network entitlements etc.
   become no-ops, keeping behavior close to sandboxed-minus-container), and
   ensure `com.apple.security.get-task-allow` for attach/injection. The edit
   runs through `plutil`/`PlistBuddy`, so binary plists work; any failure
   falls back to the preflight's guidance rather than guessing. The file is
   regenerated from the real plist on every hot run, so edits propagate.
3. **Override for the hot build only**: `CODE_SIGN_ENTITLEMENTS=<cache path>`
   rides next to the sandbox/hardened-runtime overrides. Nothing in the
   user's project is written — crash-safe by construction (`kill -9` leaves
   the repo pristine, and generated projects show no spec/`.xcodeproj` diff).
4. **Announce, don't ask.** One honest line
   (`hot reload: running un-sandboxed for injection …`) instead of a blocking
   prompt — zero-config by default, CI/headless friendly, and the Keychain/
   container behavior change is named. Un-sandboxed Debug means data lands in
   `~/Library` rather than the app container and sandbox-only bugs hide until
   a sandboxed build — which is why it's hot-Debug-only and announced.

Knobs: `--keep-sandbox` skips the strip (reproducing the preflight refusal —
the honest opt-out), `--hot-entitlements FILE` signs the hot build with a
caller-supplied plist instead of auto-deriving (apps needing specific
non-sandbox entitlements while injected), and a committed
`[run] auto_unsandbox = false` opts a whole project out. Mechanism
alternatives considered and rejected: a temp `-xcconfig` (equivalent, plus a
file), post-build re-signing (redundant second step when we already own the
build), and editing the real `.entitlements` with restore-on-exit (the
crash-footgun the ephemeral design exists to avoid).

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
5. **Bundled client — ✅ done & validated.** The client is built once from the
   pinned InjectionNext **SPM product** (XCTest-free — see Client distribution
   below; `vendor/injection-client`) and embedded into the binary via `build.rs`;
   `resolve_dylib` order is override → bundled (materialized under a content key) →
   `InjectionNext.app` fallback. Validated green by the `hot-reload-src` CI job,
   which runs the real `app run --hot` (no dylib override) and injects on **both
   Xcode 16 and 26** from one prebuilt — no clone, no per-Xcode build.
6. **Polish — ✅ mostly done.** "Inject package missing" advisory ported;
   teardown (watcher/server/app/cleanup) wired. (Config-level default for the
   recompiler mode — beyond the `--hot-recompiler` flag — is the remaining nicety.)

> **Implementation status:** the `cli/inject/` module + `app run --hot` are
> implemented and **validated end-to-end on real simulators** (Xcode 16 + 26),
> both recompilers, with the client bundled from the pinned InjectionNext SPM
> product. `clippy -D warnings`/`fmt` clean, unit tests on Linux, live e2e on the
> macOS matrix.

### Client distribution — bundle one prebuilt, built from the upstream SPM product (decided)

The client is **compiled once and bundled into the `sweetpad` binary**, not built
on the user's machine. `vendor/injection-client/build.sh` builds it from the
**pinned upstream InjectionNext SPM product** (MIT) for the iOS simulator, `build.rs`
embeds the result via `include_bytes!`, and on the first `--hot` the CLI writes it
to a content-addressed cache and `DYLD_INSERT_LIBRARIES`-injects it. No git clone,
no per-Xcode `xcodebuild`, no runtime network.

The key move is **dropping XCTest** — the one Xcode-versioned dependency in the
client. InjectionNext's *Xcode* `InjectionBundle` target links XCTest + Quick +
Nimble for its test-reload feature, and that ABI skew is what broke a *prebuilt*
binary under Xcode 16.4 (Milestone 1). But its *SPM* product references none of
them, and an SPM build defines `SWIFT_PACKAGE` — exactly the flag the engine's
`canImport(Nimble)` build sentinel keys on — so the product compiles the full
engine **without** Quick/Nimble/XCTest. The resulting dylib depends only on
ABI-stable OS/runtime libraries (`/usr/lib/swift/*`, `/System/...`), so **one
prebuilt is portable across Xcode versions** and can be shipped. (Verified: `otool
-L` shows zero XCTest, and the e2e injects on Xcode 16 + 26.) Test hot-reload is
dropped along with XCTest; the promoted feature is app UI/code reload (SwiftUI/UIKit).

- **Zero-edit wrapper, not a fork.** `vendor/injection-client/Package.swift` is a
  thin SPM package that depends on pinned InjectionNext and re-exposes its product
  as a `.dynamic` library (upstream ships only static ones) so it's loadable via
  `DYLD_INSERT_LIBRARIES`. Upstream is unpatched; bumping the client = bumping one
  `revision` pin. `-all_load` keeps the client's ObjC `+load` connect hook from
  being dead-stripped, and the extracted Mach-O is ad-hoc re-signed (a `.framework`
  signature doesn't survive extraction, and the simulator won't load a mismatched
  insert).
- **`xcodebuild` drives the SPM build**, targeting `generic/platform=iOS
  Simulator`. This sidesteps raw `swift build`'s finicky iOS-sim support and the
  dev-symlink snag from Milestone 1, while SPM still resolves InjectionNext's
  submodules automatically.
- **Not committed.** The ~4.7 MB dylib is gitignored; CI (`hot-reload-src`) and the
  release CLI scripts (`build:cli` / `build:cli:universal`) run `build.sh` before
  the cargo build, and `build.rs` embeds whatever is present. Builds without it
  compile fine (empty embed) and fall back to `InjectionNext.app` at runtime.
- **Drop-in UX preserved** — no project edit, no `InjectionNext.app` required. The
  SwiftUI `@ObserveInjection`/`.enableInjection()` annotations remain the user's to
  add (UIKit reloads without them).

## 9e. `dependency` — Swift Package Manager dependencies (`dep`)

View, add, remove, and resolve a project's SPM dependencies without opening
Xcode — the one package operation Xcode otherwise gates behind its GUI.

```
sweetpad dependency list [--transitive]      declared packages + locked versions
sweetpad dependency add <url> <requirement>  add a package, link a product
sweetpad dependency remove <pkg>             remove a package (or unlink a product)
sweetpad dependency update [<pkg>] [req]      bump pins, or change a requirement
sweetpad dependency resolve                  refresh Package.resolved
```

Works on all three containers. For an `.xcodeproj`/`.xcworkspace` there is no
Apple CLI for this, so the object graph is edited directly via
`crate::spm_pbxproj` (parse → mutate → `pbxproj_writer::serialize` → write,
byte-for-byte, like the scaffold/merge paths); for a `Package.swift` it drives
the Swift 6 `swift package add-dependency`/`add-target-dependency`/`resolve`.

- **`list`** shows each directly-declared package's requested requirement next to
  its locked version, correlated by SwiftPM identity against `Package.resolved`,
  plus its `product → target` links. `--transitive` adds the resolved-only pins.
- **`add`** takes one SPM-style requirement flag (`--from`/`--exact`/
  `--up-to-next-minor-from`/`--branch`/`--revision`, plus `--to` for a range),
  resolves the package to read its real products, then prompts for the
  product(s)/target(s) to link (or takes `--product`/`--target`; strict-errors
  off a TTY). Auto-resolves afterward unless `--no-resolve`. Supports remote git
  URLs and local paths (`XCLocalSwiftPackageReference`); the product is linked via
  a Frameworks `PBXBuildFile`, or a `PBXTargetDependency` for static-library
  targets.
- **`remove`** drops the whole package (reference, product dependencies, target
  links, build files, and its `Package.resolved` pin) by name/URL/identity, or
  narrows to unlinking one product from one target with `--product`/`--target`.
  Removing a *local* package also matches products Xcode wrote without a
  `package` back-ref (by the names its manifest declares). Naming a transitive
  pin yields a hint to change/remove the direct package that pulls it in.
- **`update`** with no requirement re-pins to the latest the current
  requirements allow — `swift package update [name]` for a package, or dropping
  the pin(s) (one, or the whole lockfile) and re-resolving for an xcodeproj.
  With a requirement (`dep update <pkg> --exact 6.0.0`) it rewrites that
  package's `requirement` in place — a bump, pin, or **downgrade** — then drops
  the stale pin and re-resolves.
- `add`/`update` discovery resolves **once**: it reads the package's products
  from the resolved checkout located precisely via SourcePackages'
  `workspace-state.json` (robust to monorepo sub-paths and case), so there's no
  second resolve and no checkout-name guessing.
- A workspace `add`/`remove`/`update` targets the member project that declares
  the package (for remove/update), else its sole member, else an interactive
  pick (strict `--project` error off a TTY). All `xcodebuild
  -resolvePackageDependencies` calls pass a `-scheme` (required for a workspace).

> Supersedes the earlier "vendor full source, compile per Xcode" plan: the
> from-source per-Xcode build (and its `~/.cache/.../<xcode-build>/` cache) existed
> only to keep XCTest's ABI matched against the active Xcode. Building the
> XCTest-free SPM product removes that need, so one bundled prebuilt suffices. The
> earlier strip-the-bundle analysis (≈ ½ week) is moot — the SPM product is already
> XCTest-free with no patching.

### Open decisions

- **ABI match — A proven, F pending.** Path A (exact build-log command) injects
  cleanly (Milestone 1), so it is primary. The (F) resolver path's ABI match is
  still to confirm; until then F is an optimization, not the default.

### macOS test harness (permanent)

Hot reload needs macOS + Xcode + a simulator, so it's validated in CI by the
permanent **`xcode-tests.yaml`** workflow — a reusable matrix harness for any
Xcode/simulator-requiring test, across Xcode versions (16.x, 26.x; weekly + on
push/PR). Two jobs:

- **`cli`** — the full standalone-CLI e2e (`ci/smoke.sh`) on each Xcode.
- **`hot-reload-src`** — the injection e2e (`ci/hot-reload-e2e.sh`) on **both
  Xcode 16 and 26**: it builds the bundled client (`vendor/injection-client/build.sh`),
  generates the fixture app, and runs the *real* `sweetpad app run --hot
  --hot-selfcheck` (hidden flag) with **no** dylib override, so it exercises the
  client **bundled into the binary** (Milestone 5), for **both** recompilers
  (resolver + build-log). The self-check builds with the interposable/frontend
  flags, starts the `:8887` server, launches with the client injected, edits a
  Swift file once, and asserts `.injected` — exiting non-zero otherwise. (An
  earlier `hot-reload` job ran the same e2e against a *prebuilt-download* client
  that still linked XCTest, so it carried the per-Xcode ABI skew — flaky, and
  removed once the bundled XCTest-free client made one prebuilt portable.)

This supersedes the original throwaway spike (`hot-reload-spike.yaml`), whose
run #5 first proved the socket + recompile→load→inject chain end-to-end.

## 9f. v6 — project mutation: build settings & sync-group sources

Make the pbxproj itself directly drivable — the settings half of an XcodeGen
`project.yml` becomes `settings set` calls, and the `sources:` half becomes
Xcode 16 **synchronized root groups**, so per-file membership stops being a
problem anyone has to manage. Both ride the proven mutation pipeline
(parse → mutate → `pbxproj_writer::serialize`, byte-for-byte, GUIDs via
`fresh_guid` — the same path `dependency add` ships on). *Declined:* a
declarative spec file / `project sync` (that's re-implementing XcodeGen and
creates a second source of truth), XcodeGen interop, and an xcconfig write
mode. Idempotent imperative commands in a committed script *are* the spec.

**Mutations never guess.** Unlike the run/build resolution flow (TTY pickers),
`settings set`/`unset` and the `source` verbs hard-error on any ambiguity —
interactive or not — with the flag that disambiguates named in the message.
A mutation either applies exactly what was asked or changes nothing.

### `settings set` / `unset` / `show --raw`

```
sweetpad settings set KEY=VALUE [KEY=VALUE …] [--target T]… [--configuration C]…
sweetpad settings set KEY+=VALUE …                    append to a list setting
sweetpad settings unset KEY [KEY …] [--target T]… [--configuration C]…
sweetpad settings show --raw [--target T]             the stored pbxproj layer
```

- **Scope.** Project-level `XCBuildConfiguration`s by default; `--target`
  (repeatable) switches to those targets' configurations. No `--all-targets` —
  project-level *is* "all targets"; that's what inheritance is for. All
  configurations by default (XcodeGen `settings.base` semantics);
  `--configuration` (repeatable) narrows. Unknown target/configuration names
  are errors.
- **Multiple assignments, one write.** All pairs apply in a single
  parse → mutate → serialize pass (temp + rename): one diff, atomic.
- **Arrays.** Repeating a key builds an array in argument order
  (`set LD_RUNPATH_SEARCH_PATHS='$(inherited)' LD_RUNPATH_SEARCH_PATHS=…`).
  `KEY+=VALUE` appends: the prior value normalizes to its element list (arrays
  as-is; strings whitespace-split, matching how xcodebuild resolves list
  settings) and the new element lands at the end. Canonical on-disk form:
  pbxproj array for >1 element, plain string for 1.
- **Conditional keys** (`CODE_SIGN_IDENTITY[sdk=iphoneos*]`) pass through
  verbatim as part of the key. `set`/`unset` match the exact key only —
  conditional variants are separate keys, never implicitly swept.
- **`unset`** removes the key (true inheritance), not `$(inherited)`. Absent
  key → no-op with a note: re-runnable scripts stay green.
- **Validation, xcspec-backed.** A known key set to a value outside its xcspec
  domain (enum/boolean) gets a *warning*, never an error; unknown keys are
  accepted silently (user-defined settings are legal, xcspec coverage isn't
  total).
- **xcconfig interplay.** When a touched configuration has a
  `baseConfigurationReference` whose xcconfig also assigns the key, warn that
  the pbxproj value now shadows it. Writing xcconfig files is out of scope.
- **Workspaces.** `--target` maps the target to its owning member project and
  edits that pbxproj; the same target name in two members is an error naming
  `--project`. A project-level set resolves to the sole member, else requires
  `--project`. Swift packages: clean error (no pbxproj).
- **`show --raw`** prints what the pbxproj layer actually stores per
  target/configuration — the verification companion, and the answer to "why
  does `show` still have a value after `unset`" (inheritance).
- **Report** (JSON envelope): per (target, configuration): key, old raw value,
  new raw value, plus the re-resolved value (the *effect*, via the in-process
  resolver) and the file written.

### Sync-group sources

- **`project new` scaffolds a `PBXFileSystemSynchronizedRootGroup`** for
  `<Name>/` — no per-file `PBXFileReference`/`PBXBuildFile` objects,
  `objectVersion = 77` (Xcode 16+ floor, the fresh-template shape:
  `preferredProjectObjectVersion`, no `compatibilityVersion`). Adding a file to
  the app is `touch`; the pbxproj never changes as the project grows. The
  classic per-file scaffold shape is deleted, not flagged — one graph shape,
  one test suite. (Corpus already round-trips both sync-group generations:
  converted objectVersion-70 projects like ice-cubes and fresh
  objectVersion-77 templates.)
- **`source`** — the sync-group-era replacement for XcodeGen's `sources:` list:

  ```
  sweetpad source list [--target T]           roots + membership exceptions
  sweetpad source add <dir> --target T        attach a synchronized root
  sweetpad source remove <dir> --target T     detach a root
  sweetpad source exclude <path> --target T   membership exception (opt a file out)
  sweetpad source include <path> --target T   drop the exception
  ```

  `exclude`/`include` edit the root's
  `PBXFileSystemSynchronizedBuildFileExceptionSet`; `add` inserts one group
  object and lists it in the target's `fileSystemSynchronizedGroups`.
- **`settings set` auto-exception.** Setting `INFOPLIST_FILE` to a path inside
  a target's sync root also adds the membership exception. Investigated live
  (Xcode 26.5 / 17F42, scratch sync-root app, macOS + iphonesimulator) and
  against the corpus:
  - *Without* the exception, an in-root Info.plist is treated as an ordinary
    resource **and** processed as the Info.plist: on iOS the two outputs
    collide at the flat bundle root — `error: Multiple commands produce
    '….app/Info.plist'`, **build failure**; on macOS they don't (resources go
    to `Contents/Resources/`), so it's the "Copy Bundle Resources … contains
    this target's Info.plist" warning plus a stray duplicate plist shipped in
    the bundle. The auto-exception is correctness on iOS, hygiene on macOS.
  - *With* `membershipExceptions = (<path>)` both platforms build clean, the
    custom plist is the one processed, and no resource copy happens — exactly
    the objects Xcode itself persists (ice-cubes and NetNewsWire both carry
    `membershipExceptions = (Info.plist)` sets for every target whose
    `INFOPLIST_FILE` points into a sync root; Xcode does *not* special-case
    the filename at build time — any un-excepted `.plist` in a root is copied
    to Resources).
  - `CODE_SIGN_ENTITLEMENTS` needs **no** exception: a `.entitlements` file in
    a sync root joins no build phase (not copied, no warning) and is consumed
    via `ProcessProductPackaging` — so the auto-exception applies to
    `INFOPLIST_FILE` only, matching the corpus (no entitlements entries in any
    exception set).

> Superseded spellings: §9g moves this surface under the `pbxproj` plumbing
> namespace — `settings set/unset` → `pbxproj settings set/unset`,
> `show --raw` → `pbxproj settings show`, `source` → `pbxproj folder` (with
> `exclude`/`include` relocated to `pbxproj membership`). Everything else in
> this section — semantics, scoping, the auto-exception — is unchanged.

## 9g. v7 — the `pbxproj` plumbing namespace & explicit membership

Two decisions in one section: project-graph mutation is **plumbing**, visibly
separated from the everyday porcelain (git's plumbing/porcelain split); and
classic-project conversion ships as **explicit primitives, not a converter**.
These commands are for scripts and agents — it is better to run 200 explicit,
reviewable commands than one that decides everything silently. A monolithic
`project convert` is *declined*: its only real intelligence is a set
subtraction (folder contents − project membership), and the caller can do
that subtraction itself once the primitives expose the data. Every hard
conversion case (stray files, cross-target borrowing, per-file flags) becomes
a visible line in a script instead of a converter heuristic.

**The namespace.** `sweetpad pbxproj <resource> <verb>` — the CLI's first
three-level command path, justified by the boundary it draws: inside the
namespace you are thinking about `project.pbxproj` *objects*; outside it,
about tasks. The namespace already existed (hidden) for `pbxproj resolve`;
it becomes visible. `dependency` stays porcelain despite editing the pbxproj —
it's a GUI-equivalent daily task, not graph surgery. All namespace mutations
follow §9f law: never guess, hard-error on ambiguity, idempotent no-ops,
one atomic write per invocation.

```
sweetpad pbxproj resolve                            merge plumbing (§9c, unchanged)

sweetpad pbxproj settings show [--target T] [--key K]     the STORED layer
sweetpad pbxproj settings set KEY=VALUE … [--target T]… [--configuration C]…
sweetpad pbxproj settings unset KEY … [--target T]… [--configuration C]…

sweetpad pbxproj folder list [--target T]           synchronized folders + exceptions
sweetpad pbxproj folder add <dir> --target T
sweetpad pbxproj folder remove <dir> --target T

sweetpad pbxproj membership list [--target T]       everything a target builds
sweetpad pbxproj membership add <path>… [--fileref ID]… --target T --phase P
sweetpad pbxproj membership remove <path>… --target T     classic build-file entries
sweetpad pbxproj membership exclude <path> --target T     sync-folder exception
sweetpad pbxproj membership include <path> --target T     drop the exception

sweetpad pbxproj fileref list [--under PREFIX]      the reference objects
sweetpad pbxproj fileref add <path>… [--type T] [--source-tree ST] [--group ID|DIR]
sweetpad pbxproj fileref remove <id> [--dangling]

sweetpad pbxproj group list                         the navigator tree
sweetpad pbxproj group add <name> --parent ID|DIR [--path P] [--source-tree ST]
sweetpad pbxproj group remove <id> [--orphan-children]
sweetpad pbxproj group attach <id> --group ID|DIR   list a child
sweetpad pbxproj group detach <id> --group ID|DIR   unlist a child
```

- **`settings` splits by layer, not by flag.** Top-level `sweetpad settings
  show` stays the porcelain question ("what will the build use" — resolved).
  `pbxproj settings show` answers "what does the file say" — the raw stored
  layer, per configuration; §9f's `--raw` flag dissolves into the namespace.
  `set`/`unset` live only here (semantics exactly as §9f).
- **`folder`** is §9f's `source` renamed to Xcode's own term (Xcode 16 UI:
  "New Folder", "Convert to Folder"). Same list/add/remove semantics.
- **`membership`** is the new resource, named for Xcode's File Inspector
  panel ("Target Membership" — the checkbox UI these verbs script). It spans
  both representations:
  - **`list`** reports everything a target builds with *provenance*: the
    classic build-file entries (resolved path, build phase — sources/
    resources/headers/frameworks/copy-with-name — plus per-file
    `COMPILER_FLAGS`, `ATTRIBUTES`, platform filters), and the synchronized
    folders with their exceptions. It does **not** enumerate the disk under
    sync folders — the folder + exceptions *is* the membership statement,
    and `ls` is the primitive for expanding it.
  - **`remove`** (batched paths, one write) deletes a target's classic
    build-file entries for the named files. When the last build file
    referencing a file reference goes, the reference is deleted and emptied
    ancestor groups are pruned — the same orphan-cleanup contract
    `folder remove` set. A file that isn't a member is a recorded no-op.
  - **`exclude`/`include`** are §9f's exception verbs, relocated: excluding
    a file *is* a membership edit (unchecking the box in Xcode writes
    exactly these exception sets).
  - **`add`** (batched, one write) gives a target a classic build-file entry
    for each file, in the phase `--phase` names. The phase is never derived
    from the extension, and the file reference has to exist already
    (`fileref add` makes one). The two things a smart `add` would have had to
    guess are the two things the caller states instead. A file a sync folder
    already builds is refused with the same cross-hint discipline as `remove`.
    Files are named by path or by `--fileref <ID>`, and the two mix in one
    invocation — see "Two ways to name a thing" below.
  - **Verbs are mechanism-specific and cross-hint.** `remove` on a file
    that's built via a sync folder errors with "use membership exclude";
    `exclude` on a file with a classic build-file entry errors with "use
    membership remove". The wrong verb never silently does the other thing.
- **`fileref` and `group` are the classic representation's other two axes**,
  kept apart because they answer different questions: a `PBXFileReference`
  says a file *exists* in the project, a `PBXGroup` entry says where it
  *appears* in the navigator, and membership says what *builds* it. Wiring a
  new file into a target is three explicit commands — `fileref add`, then
  `group attach` if the reference wasn't created under a group, then
  `membership add` — the way `git hash-object` and `git update-index` are two.
  - **No verb cascades into a neighbouring axis.** `fileref remove` refuses
    while a build file still points at the reference (`--dangling` overrides);
    `group remove` refuses while the group still lists children
    (`--orphan-children` overrides); `group detach` unlists a child without
    deleting the object. The single exception is referential integrity —
    deleting an object also drops it from its parent's `children`, since a
    group naming a missing object is a corrupt file rather than a valid
    intermediate state — and every outcome reports what it took with it.
  - **Paths are anchored, not guessed.** A reference's `path` resolves through
    its `sourceTree` (`<group>`, `SOURCE_ROOT`, `<absolute>`), and each
    outcome returns the resolved on-disk path, so a wrong path/anchor pairing
    surfaces at the command rather than at the next build.

**Two ways to name a thing.** Separate axes mean two commands, and the cost of
two commands is naming the same file twice. Three rules keep that from being
friction, without collapsing the axes:

- **A group is named by id *or* by its resolved directory** (`--group
  Sources/App`), everywhere a group is selected: `fileref add --group`,
  `group add --parent`, `group attach/detach --group`. Ids are unambiguous by
  construction, so an id that exists wins outright; otherwise the directory
  must match exactly one group. Naming none, or two, is an error that lists
  the candidates — organizational groups (a `name` with no `path`) resolve to
  their parent's directory, so collisions are normal and only the caller knows
  which it meant. This removes the `group list` lookup that otherwise preceded
  every add; it is a rule that errors, not a guess that picks.
- **`fileref add` is batched**, like every other mutating verb here.
  `--type`/`--source-tree`/`--group` apply to the whole batch, which is the
  case that actually recurs (a directory of new sources); files that disagree
  are separate calls. A bad path refuses the batch rather than half-applying
  it.
- **Membership is addressable by file-reference id** (`--fileref <ID>`), which
  is what `fileref add` returns. This is the spelling that *composes* — the id
  flows from one command to the next and nothing is spelled twice, the way
  `git hash-object` feeds `git update-index`. It is also the only way to name
  a reference no group lists (no navigator path exists) or to disambiguate a
  path two references share, which is why the ambiguity error points at it.
  Paths remain the spelling for files that already exist.

The pair stays two commands, because a reference without membership is a real
state (`Info.plist` and `*.entitlements` are referenced by build settings and
compiled by nothing) and one verb cannot express "yes it exists, no it isn't
built". What these rules remove is the *bookkeeping* between the two, not the
distinction.

**Conversion as a recipe.** With these primitives, classic → sync-folder
conversion is a script the caller owns, one decision per line:

```
sweetpad pbxproj membership list --target App -o json   # the explicit truth
sweetpad pbxproj membership remove App/… --target App   # dismantle the list (batched)
sweetpad pbxproj folder add App --target App            # attach the folder
sweetpad pbxproj membership exclude App/Old.swift --target App  # strays stay out
sweetpad pbxproj settings show --target App             # verify the stored layer
```

Intermediate states are inconsistent (after `remove`, before `folder add`,
the file builds nowhere) — fine for scripts, nothing builds mid-sequence.
Constructs that don't map to sync folders (localization variant groups,
Core Data version groups) are simply visible in `list` and left classic —
a converter would have had to refuse; a script just doesn't touch them.

**Generated-project guard.** A `.xcodeproj` produced by XcodeGen or Tuist is
an *output*: any pbxproj edit is silently clobbered by the next
`xcodegen`/`tuist generate` (verified live — a probe setting vanished on
regenerate). So every pbxproj-mutating verb — `pbxproj settings set/unset`,
`folder add/remove`, `membership remove/exclude/include`, and `dependency
add/remove/update` when they edit a project — hard-errors when a generator
is detected, naming the spec to edit instead and the regenerate command that
would eat the change. `--force` says the ephemeral edit is deliberate
(what CI harnesses do); reads (`list`/`show`) are never guarded. Detection:
a `generator = "xcodegen" | "tuist" | <tool>` declaration in `sweetpad.toml`
wins, else a spec file next to the `.xcodeproj`
(`project.yml`/`project.yaml`/`project.json` → XcodeGen, `Project.swift` →
Tuist). Erroring (not warning) is deliberate: these commands are run by
scripts and agents that read exit codes, not stderr prose — a warning above
a success is exactly how the footgun fired in the first place.

The same detection drives a **staleness warning** on the resolution path: when
the spec is newer than the project's `project.pbxproj`, every command that
resolves a container says so once, naming the regenerate command. A file added
to the spec is invisible to the build until the project is regenerated, and the
build then fails with an ordinary `cannot find 'X' in scope` — a compile error
naming a symbol when the real cause is a stale project, which is the most
expensive kind of wrong answer to hand an agent. `project.pbxproj` is the
comparison target rather than the `.xcodeproj` directory because Xcode writes
`xcuserdata` and workspace state inside the bundle constantly, and any of that
would otherwise read as "freshly generated". This one warns rather than errors —
the opposite of the mutation guard above, and for the reason that distinguishes
them: the guard sits over a command that *succeeds* while doing the wrong thing,
where stderr prose gets missed, while staleness rides alongside a build that is
already failing and a caller already reading. Erroring would also be wrong on
its face, since a spec edited in a way that changes no file (a comment, a
setting) still builds correctly.

**Decision: the guard and the staleness warning are the whole generator story
(for now).** The CLI neither *regenerates* a project from an XcodeGen/Tuist spec
(no `project generate` passthrough — run `xcodegen`/`tuist generate` yourself)
nor *edits* those spec files on the user's behalf (no writing a
`settings set` through into `project.yml`). Detecting the spec and refusing
to fight it is the full extent of generator awareness. Rationale: the CLI's
own primitives (§3a scaffolding, §9f/§9g plumbing) make the `.xcodeproj`
itself a perfectly good source of truth, so the forward-looking answer to
"my project is generated" is migrating off the generator (the §9g recipe),
not deepening the CLI's entanglement with third-party spec formats and
their release cycles. Can be revisited if real demand shows up.

*Deliberately not built:* `project convert` (the recipe above, owned by the
caller), disk expansion in `membership list` (`ls` exists), and — per the
decision above — `project generate` / spec-file editing for XcodeGen/Tuist
projects.

**Amendment: a classic `membership add` ships, because the objection was to a
*smart* one.** The case against it was that creating file references and build
files by hand means guessing a file type, an anchor, a group, and a build
phase — inference Xcode does better. The plumbing verbs guess none of those:
`--type`, `--source-tree`, `--group`, and `--phase` are all stated by the
caller, and a path with no reference is an error naming `fileref add` rather
than an invented reference. What was declined was the inference, not the
capability, and refusing to infer removes the objection entirely. `folder add`
remains the forward-looking answer for a project that can adopt synchronized
folders; `fileref`/`group`/`membership add` are for the ones that cannot.

## 9h. v8 — `app screenshot` for native macOS apps

`simulator screenshot` covers simulators; nothing covered a **running macOS
app**, so an agent driving the headless loop (`app run --mac --no-logs`) had
no way to visually verify the UI without leaving the CLI. `app screenshot`
closes that loop, and `app stop` learns macOS so the whole cycle —
launch → capture → stop — stays inside sweetpad:

```
sweetpad app screenshot [--output-file PATH] [--window N] [--pid N] [--clipboard]
sweetpad app stop                      # now also terminates a macOS app
```

**Target resolution** follows the `logs`/`stop` ladder:

1. `--pid N` captures that process's window directly — no project context,
   no bundle resolution (for windows sweetpad didn't launch).
2. Otherwise the **last-launched app** recorded for this project (unless
   explicit targeting flags opt out): a `macos` record captures the app
   window; a `simulator` record delegates to the `simctl io screenshot`
   path (same capture `simulator screenshot` does — so one verb serves
   the agent loop on either destination); a `device` record errors
   (devicectl has no capture).
3. Otherwise the resolved build target: `platform=macOS` destinations
   resolve the built `.app` via the in-process build-settings resolver (no
   build, no xcodebuild spawn), simulators delegate, devices error.

**Window discovery** is the CGWindowList path: the app's pids come from
matching `ps` command paths against the bundle's executable
(`…/Contents/MacOS/<name>` — full-path matching, so same-named binaries
elsewhere never collide), and `CGWindowListCopyWindowInfo` (on-screen,
front-to-back) filtered to those pids at layer 0 with nonzero alpha yields
the capturable windows. The default is the frontmost; `--window N` picks
the Nth (1-based, front-to-back), and an out-of-range index errors listing
what's there. A freshly-`open`ed app gets a short grace poll (~5s) for its
first window instead of a racy instant failure; a pid with no on-screen
window after that errors (minimized windows are off-screen by definition).

**Capture** is `screencapture -o -x -l<windowid>` — the window is captured
wherever it is on screen (no focus steal, no shadow, no sound). Preflight is
`CGPreflightScreenCaptureAccess()`: without the Screen Recording permission
`screencapture` silently produces wallpaper instead of failing, so the
missing permission is a hard, actionable error naming System Settings →
Privacy & Security → Screen Recording (and, on an interactive terminal
only, `CGRequestScreenCaptureAccess()` triggers the one-time OS prompt; a
headless run never pops UI). Window *enumeration* needs no permission —
only capture does.

`--output-file PATH` overrides the destination (default
`./sweetpad-shots/<app>-<epoch>.png`, the `simulator screenshot`
convention); `--clipboard` additionally copies the PNG to the pasteboard.
`--json` emits `{path, pid, windowId, bundleId, windows}`. The interactive
session's `s` key captures macOS targets through the same path (it was
simulator-only).

`app stop` on a macOS target terminates by pid — the recorded last launch's
executable path (fast path, no resolution), else the resolved bundle's —
with SIGTERM, and reports `{action: "terminated", pid}` (the `udid` field
is null for mac).

`app launch --mac` is its symmetric counterpart: it starts an already-built
macOS app and returns, leaving it running. The process is deliberately not
ours — `setsid` gives it its own session so a Ctrl-C in the launching
terminal can't reach it, and stdout/stderr are redirected to
`<state>/logs/<bundle-id>.log` rather than a pipe. The pipe is the reason
the interactive session's `d` (detach) key carries a caveat: a detached
child whose console pipes died with the CLI is killed by its next `print`.
A launch that never had pipes has no such failure mode. Spawning the
executable directly (rather than `open`) is also what lets `--env` reach the
process.

`app run --mac --detach` is the build-first form of the same thing: build,
launch detached, return. On a simulator or device the app already outlives
the CLI, so `--detach` there is `--no-logs`. It is rejected with `--hot`,
which has to stay attached to recompile and inject.

A macOS app logs to two disjoint places — plain stdout/stderr (`print`,
`NSLog`'s stderr leg, C `printf`) and the unified log (`os_log`/`Logger`) —
so `app logs` on macOS follows *both*: the captured
`<state>/logs/<bundle-id>.log` and `log stream`, interleaved by arrival.
`--source oslog|stdout|both` narrows it (default `both`); `stdout` is
macOS-only, since only a detached launch captures a file. The captured file
is truncated per launch and stamped with a one-line run header, so reading it
from the top shows only the current run — never `print` output a previous run
left behind (the append-mode file used to carry stale lines that read as the
live run). Simulator and device logs stay `os_log`-only. In `--json`/`-o
ndjson`, `os_log` entries pass through as the raw `log stream` objects and
captured lines are tagged `{"source":"stdout",…}`, so the two are
distinguishable on one stream. `--last <dur>` (e.g. `2m`, `90s`, `1h`) swaps
follow for a one-shot backfill — `log show --last` for the `os_log` history
`log stream` can't replay, plus the captured file — for an app that has gone
quiet or already exited; it is refused for a physical device, whose syslog has
no history query. `app status` prints the `detached log` path when the last
launch was macOS, so the file is discoverable without catching the one launch
line that first named it.

`install`/`uninstall` stay simulator/device verbs: a macOS app is built in
place, so there is nothing to install.

The CoreGraphics/CoreFoundation FFI is a small hand-rolled block private to
the `app` command (DOCS §3: no binding-crate dependency for four calls);
everything above it — `ps` parsing, window filtering/pick — is pure and
unit-tested.

*Deliberately not built:* a `--screenshot PATH` auto-capture flag on
`app run` (the agent loop composes explicit commands — `app run --mac
--no-logs`, then `app screenshot`, then `app stop` — and owns its own
timing, per §9g's explicit-primitives philosophy), and device screenshots
(devicectl exposes no capture; the error says so).

## 9i. v8 — `app ui` — reading and driving a macOS app's UI

§9h closed the *observe* half of the agent loop and `app open-url` covers
*stimulate* at arm's length, but "click this, assert that" had no spelling: a
PNG is not something a script can assert on, and a deep link only reaches
states the app chose to expose as URLs. `app ui` adds the missing half
through the Accessibility API, where one interface serves both — the element
tree is the assertion surface and the same elements take the actions:

```
sweetpad app ui tree  [--depth N] [--pid N]           # what the app exposes
sweetpad app ui click <--label TEXT|--role ROLE> [--nth N] [--pid N]
sweetpad app ui type  <TEXT> --label TEXT [--role ROLE] [--nth N] [--pid N]
```

A bare `app ui` runs `ui tree`, the one verb that only observes.

**Target resolution** is §9h's ladder exactly — `--pid`, then the recorded
last launch, then the resolved build target, never a build. It is
**macOS-only**, and the non-mac error says what does work there instead
(`app screenshot` + `app open-url`, or a UI test target through
`sweetpad test`): `simctl` has no tap or type verb at all, so the honest
answer for a simulator is XCUITest, which is a different model — write a
Swift test, build it, run it — not a command an agent issues between edits.
Where §9h picks the frontmost window when an app has several, `ui` *refuses*
a multi-process app: there is no "frontmost" element tree, so it names the
pids and asks for `--pid`.

That scope is a property of the API, not a gap left to fill. Accessibility
is host-side and addresses processes on the Mac by pid; a simulated app runs
inside the simulator's own OS and is not in that namespace. The tempting
workaround does not exist either, and it fails in the way most likely to be
mistaken for progress: `Simulator.app` is itself a macOS app, so
`app ui --pid <Simulator pid>` *succeeds* and prints a 368-element tree —
which is **entirely its own menu bar** (`File`, `Device`, `I/O`). There is
no `AXWindow` in it at all; the device window is not bridged, so nothing
about the simulated app's UI is reachable. Anyone reaching for `--pid` as
an escape hatch gets real-looking output and no way to act on the app, which
is why the destination-level refusal is a hard error rather than a
best-effort attempt.

**The tree** is `AXUIElementCreateApplication(pid)` walked through
`AXChildren`, each node carrying its role, label, identifier, enabled state
and the actions it advertises. The label is `AXTitle`, else
`AXDescription`, else a string-valued `AXValue`. An `AXIdentifier` is
preferred over the label when a developer assigned one — it survives copy
changes, so it is the thing to write in a script — but AppKit hands *every*
view an auto-generated identifier of the form `_NS:945`, an internal serial
number that changes between runs and would otherwise mask every real title
(`AXWindow "_NS:34"` for a window plainly called `uitest.txt`). Those are
dropped at read time and never reach the model.

**Matching** follows §9g's resolver rule — naming nothing, or two things, is
an error rather than a pick. `--label` matches identifier or label,
case-insensitively, with **exact matches tried before substring ones** so
`--label Save` prefers a "Save" button over "Save As…" instead of calling
the pair ambiguous. `--role` accepts `button` or `AXButton`. A genuine tie
lists its candidates and asks for `--nth` (1-based, front-to-back), and the
suggestion to narrow names the axis the caller hasn't already used. An empty
query is refused outright: it would match the application element and press
something arbitrary.

**Acting** is `AXUIElementPerformAction(…, "AXPress")` for `click` and
setting `AXValue` for `type`. Because a snapshot is pure data holding no
live element refs, `act` re-descends by index path and **re-checks the role
on arrival** — if the UI restructured between snapshot and act, that is a
clear "the UI changed under us" error rather than a press landing on
whatever now occupies the slot. `type` is a value assignment, not keystroke
synthesis; the help says so, because an app watching for individual key
events won't see any.

**Permission** is `AXIsProcessTrustedWithOptions`, mirroring §9h's
Screen Recording preflight: a missing Accessibility grant is a hard error
naming System Settings → Privacy & Security → Accessibility, and only an
interactive terminal triggers the one-time OS prompt. The grant attaches to
the *hosting* app (Terminal, iTerm, the editor), not to the sweetpad binary.

**Occlusion does not gate reads**, which is what makes this usable
unattended. §9h's capture path and the AppKit notes behind it fail when a
window is occluded or the display is asleep — the display pipeline is gated,
`cacheDisplay` reads empty backing stores, and lazy `NSTableView` row views
never materialize. The accessibility hierarchy is derived from the view
tree instead, and a tree walked while the app sat fully behind other windows
was byte-identical to the same walk with it frontmost. What an app
*exposes* is still its own choice: an unlabeled SwiftUI view is a bare
`AXGroup` with nothing to match on, and no amount of CLI can invent a label
the app never set.

The `ApplicationServices`/CoreFoundation FFI is the same shape as §9h's — a
small hand-rolled block, no binding crate, no Objective-C runtime, since
`AXUIElement` is plain C — with the tree model, matching and rendering above
it pure and unit-tested.

*Deliberately not built:* coordinate-based clicking via `CGEvent` (brittle,
and it can assert nothing — the tree is the point), keystroke synthesis,
element waiting/polling (`ui tree` is cheap; a caller that needs to wait owns
its own timing, per §9g), and any simulator path short of XCUITest.

## 9j. v8 — `app diagnose` and scriptable `app debug --batch`

`app debug` handed the terminal to lldb and waited at its prompt — no way to
pass commands, no batch mode, so it was unusable from an agent or CI. Chasing a
swallowed Objective-C exception (which imitates a hang, App Nap, and an executor
bug at once — see the wedge-hunt notes below) meant dropping to raw lldb by hand
and resolving the binary out of DerivedData yourself. The consumer of a debugger
on this platform is almost always an *agent* hunting a defect, rarely a human at
a prompt, so the surface leads with a structured preset and keeps the raw
passthrough as the escape hatch.

```
sweetpad app diagnose [--mac|--device] [--arg A] [--env K=V] [--timeout SECS]
sweetpad app debug --batch [--cmd LLDB_CMD]… [--on-crash LLDB_CMD]… [--timeout SECS]
```

**`app diagnose`** is the agent-facing verb: build, launch under `lldb -b` with a
breakpoint on `objc_exception_throw`, run bounded by `--timeout`, and on the
first stop print a structured report — `stopReason`, `signal`, `exitStatus`,
`exception { name, reason }`, `backtrace`, and the full lldb `transcript` — then
kill the app and quit. `-o json` is the point of the verb: the freeform lldb text
becomes fields an agent acts on, with the raw transcript alongside for whatever
parsing can't reach. Human mode prints a one-line verdict and the backtrace. lldb
recognizes an ObjC throw natively (`stop reason = hit Objective-C exception`);
`$arg1` at that breakpoint is the `NSException`, read ABI-neutrally so it works on
arm64 and x86_64 sims. The chain prints `script print('@@…@@')` sentinels between
sections and runs under `-Q` (no command echo), so a captured transcript splits
cleanly even though lldb interleaves prompts, app `os_log` lines, and its own
diagnostics.

**`app debug --batch`** is the raw escape hatch: `--cmd` forwards to lldb's
`-o/--one-line` verbatim (sweetpad's own `-o` already selects the output format,
so the flag is `--cmd`, not `-o`), `--on-crash` to `-k`. You write your own
`run`/`continue` and `quit`; the output streams. It rejects `--json` like `app
run` does — a live lldb session has no coherent one-shot envelope; that is
exactly what `diagnose` is for.

**The exit code reflects the launch, not the finding.** `lldb -b` returns `0`
whether the debuggee crashed, an attach was denied, or nothing happened, so
neither verb derives success from it. `diagnose`'s answer is the report (an agent
reads `stopped`/`stopReason`/`exception`); `--batch`'s answer is the streamed
transcript. The help says so on both.

**Timeout is mandatory, not optional.** `lldb -b`'s `run` blocks until the
process stops or exits, so an app that launches and stays up (the common GUI
case) would hang the session forever — the precise anti-pattern for an
unattended agent. `--timeout` (30s for `diagnose`, 300s for `--batch`, `0`
disables) spawns lldb, waits, and on expiry kills the *inferior first* (resolved
by executable path on macOS, by the launched pid on a simulator) then lldb, so a
timed-out `diagnose` reports `timedOut: true` and leaves nothing running. The
kill targets only what this run launched: a diagnose against a DerivedData build
never touches a copy the user opened from `/Applications`.

**Target coverage** mirrors interactive `app debug`. On macOS lldb owns the
launch (`run`, breakpoints armed before the process exists); on a simulator the
app is launched suspended via `simctl --wait-for-debugger` and lldb attaches to
the pid and `continue`s — the one asymmetry, hidden inside `diagnose` and left
verbatim in `--batch`. Physical devices and Swift packages are refused with a
pointer (`lldb -b … -- <binary>` for SPM), matching where the interactive verb
already draws the line.

*Deliberately not built:* parsed backtrace *frames* (SB/Python API — the
transcript plus a best-effort frame-line list is enough for v1), a device path
(needs `debugserver` plumbing, like the interactive verb), and any preset beyond
exception-catching (a `--sample`/stack-snapshot sibling for "wedged or merely
idle?" is a natural follow-on, noted in the jiraffe field log, but it inspects a
*running* pid rather than owning a launch, so it belongs with `screenshot`/`ui`
under the observe verbs, not here).

## 9k. v8 — bounded follows, listener recovery, honest compile counts

Three papercuts from one family: the CLI already held the answer and had no way
to say it or act on it. Each showed up in an agent session as lost time rather
than as an error, which is why none of them had been filed as a bug.

### `app logs --until` / `--timeout`

`app logs` followed until killed — the human half of the verb. The agent-shaped
operation is "start it, poke it, tell me what it said", and expressing that
against a follow-forever stream costs a background job, two guessed `sleep`s and
a `kill` per cycle, where the guess either wastes time or truncates the answer.

```
sweetpad app logs --until TEXT [--timeout DUR]
sweetpad app logs --timeout DUR
```

`--until` ends the follow on the first line containing TEXT and exits 0; missing
it exits non-zero, so the caller branches on the exit code instead of parsing.
`--timeout` alone bounds a follow and exits 0 — a tail with a deadline asks no
question, so reaching the end is not a failure. Together, the deadline is the
answer's deadline.

**Substring, not a regex.** The stop condition is nearly always a literal marker
line, and `regex` would add four crates to a nine-entry dependency list to serve
it. The help says "plain substring" outright rather than leaving the caller to
discover which metacharacters quietly do nothing.

**The match runs against the rendered line**, not the raw ndjson, so what you
match is what you see. On macOS both sources feed it, since a `--detach`ed app's
marker may arrive on captured stdout rather than through `os_log`; either source
matching ends the follow, by SIGTERM-ing the `log stream` child whose EOF
unblocks the reader, so every exit leaves through one path. A deadline ends the
same child, and stands down when the stream finishes first so it can never signal
a pid it no longer owns. `--last` conflicts with both flags at parse time —
history is already finite.

### `hot status` / `hot reset`

A `--hot` session that dies without unwinding leaves `:8887` bound, and every
later run fails to bind. The error already named the holding pid — `port_holder()`
shelled out to `lsof` for exactly that — but nothing could act on it, so recovery
lived outside the CLI (`lsof`, then `kill`) and the standing workaround became
"always pass `--no-hot`": silently giving up the feature to avoid the papercut.

```
sweetpad hot status
sweetpad hot reset [--force]
```

**`reset` is guarded by ownership rather than by prompting.** The port can
legitimately belong to InjectionNext.app or an unrelated listener, so the holder's
executable is resolved through `ps -o comm=` and only a `sweetpad` process is
ended by default; anything else is named in the refusal and needs `--force`.
**The result reports the port, not the signal** — it polls for the release and
says so when a holder outlives its SIGTERM, because "a signal was sent" is not
the question being asked.

### One `Compiling` line per file

Xcode emits two shapes for the same Swift work: a batch header (`SwiftCompile …
Compiling\ A.swift,\ B.swift <paths>`) and then a line per file. Rendering both
announced most files twice, which reads as duplicated work — and a header took
whichever member `source_name` happened to match first, so a group of twelve was
labelled with one arbitrary file.

A one-file header now defers to the per-file line behind it, and a wider header
renders its count (`Compiling 12 files`). Entries are counted by their separators,
so an escaped space inside a filename stays one entry. A suppressed header becomes
`Other`, which keeps it visible under `-v` and out of ndjson, where it was never a
distinct unit of work.

*Deliberately not built:* `--until` as a regex; a `--signal` sibling on `app run`
for apps that expose Darwin-notification debug hooks (a narrower need than the
stop condition); and dedupe of a wider batch header against the per-file lines
behind it, which would need lookahead over a stream to save a line that is honest
as it stands.

## 9l. Direction — the run session as a server

`app run`'s only door is a tty. The session owns everything an iterating loop
needs — the settled `RunPlan`, the live process, the hot-reload channel (§9d),
the log stream — and the sole way to ask it for anything is a keystroke it
reads in raw mode ([`rawmode`]). A second window, whether a human's or an
agent's, cannot reach it.

> Status: direction, not a committed version. Nothing below is scheduled.

That gap forces every other client to re-derive the world, and the copies
disagree. A detached `app run` from another window calls `plan` again,
independently: remembered scheme/configuration/destination live in the same
state file that window's own commands write, `refresh_stale_destination` can
recover a vanished simulator pin to a *different* destination mid-session, and
resolution takes structurally different paths interactively (a picker) versus
not (an error, per `--non-interactive`). Worse, the run-shaping options never
persist at all — `hot`, `hot_entitlements`, `launch` args/env and `passthrough`
come from the command line of the session that started it, and
`LastLaunchedApp` records none of them. A session started as `run --hot --env
API=staging` is answered by a detached run that builds a non-hot binary without
the variable. That isn't drift; it's a different build, arrived at silently.

The answer is to stop treating the tty as the interface. The session becomes
the owner and exposes a control channel; the terminal attaches as a client, an
agent attaches as a client, and the extension (§7) could attach as a third. `r`
and an `app session rebuild` from another window resolve to the same command on
the same channel — one produced locally by a keystroke, the other arriving over
a socket. The divergence above then has nothing left to diverge: one
resolution and one app, because there is one owner — structurally, rather than
by keeping copies in sync. This is the shape dev loops with more than one
observer converge on (Flutter's daemon mode, Metro, Vite's HMR clients).

The dispatch half already exists: the session loops match on a `SessionKey`
command enum rather than on raw bytes, so the keystroke reader is one
*producer* of commands, not the control flow itself. What's missing is a second
producer (a socket listener feeding the same channel), a response path (build
results reach the client that asked, not only stdout), and the surface —
`app session <rebuild|relaunch|status|stop>`, under a `session` noun rather
than a bare `app rebuild` that would read as a sibling of `build` and blur that
it addresses a *running* thing. Per §9g's rule that a caller owns its own
timing, the request is non-blocking: `rebuild` returns a job id and `status`
polls, so a long build never freezes an agent's loop.

*Considered and rejected as an interim:* persisting the settled `RunPlan` into
state for a detached run to adopt. It removes the silent-wrong-build hazard
with no IPC at all, but it cannot carry `--hot` (which needs the live process),
it makes one process depend on another's pid-namespaced temp directory for
`hot_entitlements`, and it is discarded wholesale once the channel exists.
Signals and watched control files are cheaper still and strictly worse: they
trigger a rebuild but return nothing, leaving the caller blind to the result
that was the reason it asked.

## 10. Testing

The CLI modules carry inline `#[cfg(test)]` units that need no Xcode, so the
tool-spawning code is pinned without a Mac:

- **Arg-vector snapshots** — `BuildPlan`/`TestPlan` produce exact `xcodebuild`
  argument vectors (the main guard against silent flag drift).
- **Parser fixtures** — `simctl list`, `devicectl list`, `xcresulttool`
  summary, and `-showBuildSettings` JSON parsed from captured-shape payloads
  (this caught a missing `rename_all` on the devicectl device struct), and the
  `app diagnose` lldb transcript → outcome parse (§9j) against real `lldb -b -Q`
  output for an ObjC throw, a signal crash, and a clean exit.
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
