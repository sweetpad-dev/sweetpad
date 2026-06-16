# Plan — merging `sweetpad vscode` into the main CLI grammar

Status: **proposal for discussion.** No code changes yet; this documents the
target shape and a phased path to it.

## 1. The two surfaces today

The single `sweetpad` binary exposes two command surfaces (`src/bin/sweetpad.rs`
peels `vscode` off the front, everything else goes through clap):

- **`sweetpad vscode <method.name> [args] [--raw]`** — a one-shot JSON-RPC
  client (`vscode_cli.rs`) for a *running* SweetPad VS Code extension. It walks
  up to the project's control socket (`~/.local/state/sweetpad/projects.json`)
  and calls a dotted method (`scheme.list`, `build.start`, …). The full method
  set is the `METHOD_CATALOG` in `src/cli-server/method-catalog.ts`. Output and
  errors mirror the former bundled JS CLI exactly.

- **`sweetpad <resource> <action> [flags]`** — the standalone, headless
  "xcodebuild for humans" tree (`cli/mod.rs`, resources in
  `cli/commands/*`). It drives Xcode itself (`xcodebuild`/`simctl`/`devicectl`)
  through the resolver in this crate, with its **own** config/state stores
  (`~/.config/sweetpad`, `~/.local/state/sweetpad`). No editor required.

The two share noun names (`scheme`, `destination`, `simulator`, `device`,
`build`, build settings) but mean different things:

| | `vscode <method>` | headless `<resource> <action>` |
|---|---|---|
| Acts on | the **running extension** | **Xcode directly** |
| Source of truth | extension's live state / workspaceState | the pbxproj/xcconfig resolver |
| Selection store | VS Code `workspaceState` | `~/.config` + `~/.local/state` |
| Builds | async, tracked by the extension's build manager | synchronous, foreground |
| Needs an editor open | **yes** | no |

"Merging" means: wherever a `vscode` method is really just a **generic Xcode
operation**, the headless resource command should be the one obvious way to do
it — and the `vscode` surface shrinks to only what genuinely requires the live
editor.

## 2. Classifying every `vscode` method

### Bucket A — pure Xcode ops → absorb into headless resources

These need nothing from the editor; the headless command is (or should be) the
canonical entry point.

| `vscode` method | headless command | gap |
|---|---|---|
| `scheme.list` | `scheme list` | — exists |
| `destination.list` | `destination list` | — exists |
| `simulator.list` | `simulator list` | — exists |
| `simulator.start` | `simulator boot` | — exists |
| `simulator.stop` | `simulator shutdown` | — exists |
| `simulator.screenshot` | `simulator screenshot` | — exists |
| `simulator.openUrl` | `app open-url` | — exists (v3) |
| `simulator.install` | `simulator install <udid> <app>` | **add** |
| `simulator.uninstall` | `simulator uninstall <udid> <bundleId>` | **add** |
| `simulator.launchApp` | `simulator launch` / `app launch` | **add** |
| `simulator.terminateApp` | `simulator terminate` / `app stop` | **add** |
| `device.install` | `device install <id> <app>` | **add** |
| `device.launch` | `device launch` / `app run --device` | partial |
| `device.terminate` | `device terminate <id> <bundleId>` | **add** |
| `buildSettings.get` | `settings show [--keys …]` | mostly exists |
| `xcodebuild.list` | `project info` (+ `--json` raw) | mostly exists |
| `derivedData.path` | `derived-data path` | — exists |
| `appPath.find` | `app path` | **add** |
| `bundleId.get` | `app bundle-id` | **add** |
| `build.start` build/clean | `build start [--clean]` | — exists |
| `build.start` run/launch | `app run` / `app launch` | — exists |
| `build.start` test | `test run` | — exists |

After Bucket A, the only missing headless verbs are a handful of
simulator/device app-lifecycle actions and two `app` lookups. They reuse the
existing `simctl`/`devicectl` modules — small, mechanical additions.

### Bucket B — selection state → optional headless get/set

`scheme.get/set`, `destination.get/set`, `buildConfig.list/get/set` read and
**persist a selection**. The headless CLI already resolves selection by layered
precedence (flag > env > config > remembered state, see `cli/resolve.rs`) but
only *reads* it — `set` would write `~/.local/state/sweetpad/state.toml`.

The catch: the editor persists selection in VS Code `workspaceState`, the CLI in
its own state file. They are **different stores** and won't agree unless the
extension adopts the CLI's store (design `CLI_DESIGN.md` §7 — "adoptable later,
not committed"). So:

- Add `scheme use <name>` / `destination use <id>` / a `configuration`
  resource with `list`/`use`, writing the CLI's own state. Low priority.
- Keep `scheme get`/`set` over the *extension* state under `vscode` (Bucket C)
  until/unless the extension shares the CLI store.

### Bucket C — irreducibly editor-coupled → stay under `vscode`

These have no headless meaning; they only exist because an editor is running.

- `meta.usage` / `meta.schema` / `meta.version` / `meta.workspacePath` — describe
  the *server*.
- `state.get` — snapshot of the extension's live selection + build.
- `build.stop` / `build.wait` / `build.status` / `build.list` / `build.logs` /
  `build.diagnostics` — read the **extension's async build manager** (history,
  in-flight builds, persisted logs/diagnostics). Headless builds are synchronous
  and foreground; there is no build registry to query. Keeping these editor-side
  is correct unless the headless CLI grows its own build-history store (out of
  scope).
- `simulator.refresh` — re-scan **and refresh the extension's tree view**.
- `workspace.detect` / `workspace.use` / `workspace.recent` — manipulate the
  editor's active-workspace selection.
- `workspaceState.*` — raw `sweetpad.*` keys in the extension's workspaceState.
- `vscode.executeCommand` — drive the editor / other extensions.
- `vscodeSettings.*` — the VS Code settings space.
- `logs.tail` — the extension's output channel.
- `scheme.reveal` — disk-based (path + XML); could go headless as
  `scheme reveal`, but it's minor — leave for later.

## 3. Target grammar after the merge

```
# Headless — the primary, editor-free surface (Buckets A + optional B)
sweetpad scheme list | use <name>
sweetpad destination list | use <id>
sweetpad simulator list | boot | shutdown | erase | open | screenshot
                        | appearance | install | uninstall | launch | terminate
sweetpad device list | install | launch | terminate
sweetpad app run | install | launch | stop | logs | open-url | path | bundle-id
sweetpad build start [--clean]
sweetpad test run
sweetpad settings show [--keys …]
sweetpad project info | new
sweetpad derived-data path | size | purge
sweetpad configuration list | use <name>          # (Bucket B, optional)

# Editor control — the residual, deliberately small `vscode` surface (Bucket C)
sweetpad vscode <method.name> [args]   # meta/state/build-manager/workspace/
                                       # workspaceState/executeCommand/settings/logs
```

The `vscode` RPC protocol **stays byte-compatible** — agents and scripts that
call `sweetpad vscode build.start` keep working. We are removing *conceptual
duplication and the need to reach for `vscode` for generic Xcode tasks*, not
deleting the protocol.

## 4. How headless and editor-backed coexist (the one real decision)

When VS Code *is* running, should `sweetpad scheme list` reflect the editor's
state or resolve fresh? Two models:

- **Model 1 — headless-first (recommended).** The merged commands always run
  headless against Xcode. `vscode` remains the explicit escape hatch for live
  editor state. Simplest, single code path, matches the standalone-first design.
- **Model 2 — auto-proxy.** The shared nouns detect a running extension (via the
  projects.json index) and proxy to it, else fall back to headless. One command
  that "does the right thing," but two code paths and surprising behavior.

**Recommendation: Model 1 now**, with Model 2 available later as an *opt-in*
`--remote` / `SWEETPAD_REMOTE=1` flag on the shared nouns (proxy to the
extension when asked, never implicitly). This keeps the headless CLI's core
promise — it never needs the editor — while leaving a clean door to live data.

## 5. Phased execution

1. **Close the Bucket A gaps** (headless parity). Add `simulator
   install/uninstall/launch/terminate`, `device install/launch/terminate`, and
   `app path` / `app bundle-id`, reusing `simctl.rs` / `devicectl.rs`. Snapshot
   the arg-vectors (per `CLI_DESIGN.md` §10) — no Mac needed for those tests.
2. **Document the split.** Update `CLI_DESIGN.md`: the headless tree is the
   default surface; `vscode` is the editor-control protocol (Bucket C). Add a
   "coming from `vscode <method>`?" mapping table (Bucket A) so existing users
   find the new home of each command.
3. **(Optional) Bucket B selection state.** `scheme use` / `destination use` /
   `configuration` writing the CLI's own state.
4. **(Optional) Model 2 `--remote`.** A shared flag that proxies a headless noun
   to the running extension, reusing `vscode_cli.rs`'s socket transport.
5. **(Optional) Reshape the residual `vscode` surface** from dotted methods to a
   clap subtree (`sweetpad vscode build status` ≈ `build.status`) for grammar
   consistency — purely cosmetic, do last, keep the dotted form as an alias.

## 6. Compatibility & non-goals

- **Keep** `sweetpad vscode <method>` working unchanged (the RPC protocol and
  its JS-compatible output are a stable contract used by agents).
- **No shared selection store** between editor and CLI is committed here; that
  is the larger "extension adopts the CLI as its engine" question (`CLI_DESIGN.md`
  §7) and stays out of scope.
- The headless build/test commands stay **synchronous**; the async
  build-manager queries (`build.status/list/logs/diagnostics/wait/stop`) remain
  editor-only by design.
