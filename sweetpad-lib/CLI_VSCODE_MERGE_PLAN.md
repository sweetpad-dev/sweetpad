# Plan — one CLI core, the VS Code extension as a thin frontend

Status: **proposal for discussion.** No code changes yet; this documents a
target architecture and a phased path to it.

## 0. North star

> Ideally the VS Code extension is built on top of the CLI — or an efficient CLI
> core — so there is **one engine**, not two.

Today the same Xcode orchestration is implemented **twice**:

- in **TypeScript** inside the extension (`src/build`, `src/simulators`,
  `src/devices`, `src/common/cli/scripts.ts` — ~9k lines that shell out to
  `xcodebuild`/`simctl`/`devicectl`), and
- in **Rust** inside `sweetpad-lib` under the `cli` feature (`cli/commands/*`,
  `cli/{xcodebuild,simctl,devicectl,buildlog}.rs`) — reachable only through the
  `sweetpad` binary.

The extension already consumes the Rust core for the *resolver* (build settings,
schemes, targets, compiler args, BSP, scheme parsing) via the N-API addon
(`@sweetpad/lib`, `node.rs`). The goal is to extend that — make the core the
single source of truth for **orchestration too**, and reduce the extension to UI
+ editor glue.

## 1. Where logic lives today

```
                        ┌──────────────────────────────────────┐
                        │            sweetpad-lib (Rust)         │
   resolver (shared) →  │  resolver: build_settings, schemes,    │
                        │  pbxproj, compiler_args, bsp           │
                        │  ── cli feature ──                     │
   orchestration  →     │  cli/commands/*, simctl, devicectl,    │
   (binary-only)        │  xcodebuild, buildlog                  │
                        └──────────────┬───────────────┬─────────┘
                          N-API (sync) │               │ binary (sweetpad …)
                                       │               │
        ┌──────────────────────────────▼──┐         ┌──▼───────────────────┐
        │      VS Code extension (TS)      │         │  headless CLI user    │
        │  resolver  → via @sweetpad/lib   │         │  (terminal / CI)      │
        │  ORCHESTRATION → reimplemented    │         └───────────────────────┘
        │  in TS (build/sim/device ~9k LOC) │
        │  + UI: trees, status bar, editor  │
        │  + cli-server (RPC) ← sweetpad vscode
        └───────────────────────────────────┘
```

Two duplications fall out of this:

1. **Orchestration** — TS build/sim/device logic vs. the Rust `cli` module.
2. **The `vscode` RPC** — `sweetpad vscode <method>` is a *client of the
   extension* (`vscode_cli.rs` → cli-server). In the target it inverts: the
   extension becomes a client of the **core**.

## 2. Target architecture

**One core, two thin frontends, plus a small editor-control RPC.**

```
                ┌─────────────────────────────────────────────┐
                │     sweetpad-lib core (interface-agnostic)   │
                │  resolver  +  orchestration (build/run/test/ │
                │  sim/device/format/doctor/derived-data)      │
                │  command logic lives in LIBRARY modules,     │
                │  not in print paths (CLI_DESIGN.md §8)       │
                └───────┬───────────────────────┬──────────────┘
            sync, in-proc│                       │ streaming, out-of-proc
            (N-API addon)│                       │ (spawn `sweetpad … --json`)
                         │                       │
        ┌────────────────▼───────────────────────▼────────────┐
        │            sweetpad binary  (already thin)           │
        │            VS Code extension (BECOMES thin)          │
        │   = UI: trees, status bar, QuickPicks, debug,        │
        │     editor wiring; delegates work to the core        │
        └──────────────────────────────────────────────────────┘
                         ▲
                         │  editor-control only
        sweetpad vscode <method>  → cli-server (Bucket C below)
```

### 2a. How the extension consumes the core — the key decision

Two mechanisms, each suited to a different call shape:

- **N-API addon (`@sweetpad/lib`)** — in-process, typed, zero spawn cost. Right
  for **synchronous, pure queries** (already used: resolution, build settings,
  scheme parse). Awkward for long-running/streaming work (needs async + log
  callbacks across the boundary).
- **Spawn the `sweetpad` binary with `--json`** — out-of-process, streaming is
  natural (stdout lines), the boundary is a **stable text protocol** the CLI was
  *designed* to emit (`CLI_DESIGN.md` §4). Costs a process spawn per call and
  output parsing; can't share in-memory caches.

**Recommendation — hybrid, matched to call shape:**

| Call shape | Mechanism |
|---|---|
| Resolution, settings, scheme/target/config lists, paths | **N-API** (as today) |
| Build / run / test / app lifecycle (streaming logs, cancelable) | **spawn `sweetpad … --json`**, stream stdout |
| Simulator/device one-shots (boot, install, screenshot) | either; prefer N-API if cheap, else spawn |

This keeps the hot, synchronous path in-process and makes the streaming path a
clean, versionable CLI contract — which is exactly "the extension built on top of
the CLI." A single TS `runSweetpad(args)` helper (spawn + parse `--json` +
forward log lines to the build channel + map exit codes) becomes the one seam.

### 2b. Precondition: orchestration logic must be library-shaped

`cli/commands/*` today interleaves logic with human/`--json` printing. For the
extension (and unit tests) to drive the same code, each orchestration must be a
**library function returning structured results**, with the command layer only
formatting. The design already commits to this (`CLI_DESIGN.md` §8: "command
logic lives in testable library modules"); this plan makes it load-bearing.
Whichever consumption mechanism wins, the refactor is the same and is the real
work.

## 3. What stays in the `vscode` / cli-server RPC

With orchestration sourced from the core, the `vscode` surface shrinks to what
genuinely needs the **live editor**. Classifying every method in
`src/cli-server/method-catalog.ts`:

### Bucket A — generic Xcode ops → served by the core (headless command)

`scheme.list`, `destination.list`, `simulator.list/start/stop/screenshot/openUrl`,
`derivedData.path`, `buildSettings.get`, `xcodebuild.list`, and the
build/run/test/clean variants of `build.start`. Headless equivalents mostly
exist; the gaps to add are `simulator install/uninstall/launch/terminate`,
`device install/launch/terminate`, and `app path` / `app bundle-id` — thin
wrappers over the existing `simctl.rs`/`devicectl.rs`.

The extension calls these on the **core**, not over RPC.

### Bucket B — selection state → core state, optional

`scheme.get/set`, `destination.get/set`, `buildConfig.list/get/set` persist a
*selection*. The editor persists in VS Code `workspaceState`; the CLI in its own
`state.toml`. Unifying them is the deeper "shared store" question — once the
extension is a core frontend, the core's state becomes the single store and these
collapse into `scheme use` / `destination use` / a `configuration` resource.

### Bucket C — irreducibly editor-coupled → keep in the RPC

`meta.*`, `state.get`, the async **build-manager** queries
(`build.stop/wait/status/list/logs/diagnostics`), `simulator.refresh` (tree
view), `workspace.detect/use/recent`, `workspaceState.*`,
`vscode.executeCommand`, `vscodeSettings.*`, `logs.tail`. These exist only
because an editor is running and remain the legitimate purpose of `sweetpad
vscode`: an agent driving the *editor*. (Some, like the build-manager history,
shrink if the core gains a build-history store — out of scope.)

## 4. Phased migration

Incremental, command-by-command, each behind a flag with parity kept until the
TS path is deleted:

1. **Parity in the core (Bucket A gaps).** Add the missing simulator/device/app
   verbs; arg-vector snapshot-tested, no Mac needed (`CLI_DESIGN.md` §10).
2. **Library-shape one orchestration end-to-end** (suggest **build**): factor
   `cli/commands/build` + run into a library API returning structured
   events/results; the binary and a new `runSweetpad` TS helper both consume it.
3. **Migrate the extension's build path** to call the core (spawn `--json`,
   stream logs into the existing build channel/diagnostics), behind
   `sweetpad.experimental.coreBuild`. Keep `src/build/manager.ts` as the *UI*
   manager (status bar, tree, history) but delegate execution.
4. **Repeat** for run/test, then simulators, then devices — deleting the
   superseded TS in `scripts.ts`/`build`/`simulators`/`devices` as each lands.
5. **Shrink cli-server** to Bucket C once Bucket A is core-served; document the
   `vscode <method>` → core-command mapping for users/agents.
6. **(Optional) Unify selection state** (Bucket B) on the core store.

Each step is independently shippable and reversible; the extension keeps working
throughout because the UI layer is untouched — only the engine under it swaps.

## 5. Decisions to confirm

1. **Consumption mechanism** — hybrid (N-API for sync queries, spawn `sweetpad
   --json` for streaming orchestration) as recommended in §2a? Or push
   everything through one mechanism?
2. **First migration target** — build, or a lower-risk one-shot (e.g.
   simulators) to prove the `runSweetpad` seam first?
3. **Scope now** — implement Phase 1 (core parity) immediately, or keep this as
   an agreed plan and sequence the work behind feature flags?

## 6. Non-goals / compatibility

- `sweetpad vscode <method>` stays byte-compatible (stable agent contract); it
  is *narrowed* to editor-control, not removed.
- The headless CLI stays editor-free; the extension dependency is one-directional
  (extension → core), never core → editor.
- No big-bang rewrite: the extension's UI, commands, and tree/status surfaces are
  preserved; only the orchestration engine beneath them is replaced.
