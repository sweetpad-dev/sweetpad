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
- **`--watch`** — poll the project's `.swift` sources (std-only, no extra deps)
  and rebuild + reinstall + relaunch on change; a failed rebuild keeps watching.

`destination list` aggregates **macOS + simulators + connected devices**, each
with a ready `-destination` specifier. SPM containers are supported for
`scheme`/`build`/`test`/`run`: schemes come from `xcodebuild -list -json` (Xcode
synthesizes them from the manifest, which the pbxproj resolver can't).

Notes / heuristics:
- `test run` exits non-zero on failures; the `--json` summary lands on stdout
  and the failure error on stderr, so both are independently consumable.
- simulator inline logs use a best-effort `processImagePath CONTAINS` log
  predicate; may need refinement per app.
- New deps (under the `cli` feature only): `clap_complete`, `dialoguer`.

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

## 10. Testing

The CLI modules carry inline `#[cfg(test)]` units that need no Xcode, so the
tool-spawning code is pinned without a Mac:

- **Arg-vector snapshots** — `BuildPlan`/`TestPlan` produce exact `xcodebuild`
  argument vectors (the main guard against silent flag drift).
- **Parser fixtures** — `simctl list`, `devicectl list`, `xcresulttool`
  summary, and `-showBuildSettings` JSON parsed from captured-shape payloads
  (this caught a missing `rename_all` on the devicectl device struct).
- **Pure logic** — resolution precedence, config/state TOML round-trips,
  `choose` fallback branches, destination/`udid` parsing, watch snapshotting.

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
