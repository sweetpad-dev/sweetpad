# SweetPad Agent CLI — Design Doc

Status: **design** (no code yet). Last updated 2026-05-17.

This document captures the design for a `sweetpad` CLI and backing server that lets agents like Claude Code drive the
same engine the VS Code extension uses today. It's the result of a deliberate design pass through every meaningful fork;
the rationale for each call is included so the doc stays useful when implementation starts.

## 1. Motivation

The trigger scenario: a user clicks Build in VS Code, the build fails, and they ask Claude Code (in a terminal) "what's
wrong?" or "please rebuild". Claude should be able to read the structured errors and re-trigger the build without
spawning a parallel `xcodebuild` process with its own scheme/destination state.

More broadly, agents are an increasingly common consumer of dev tooling. SweetPad's current value-add over raw
`xcodebuild` — friendlier scheme discovery, structured diagnostics, xcbeautify integration, workspace auto-detection —
is exactly the value an agent needs, but it currently lives trapped inside the VS Code extension host.

## 2. Architecture

```
                ┌────────────────────────────────┐
                │       sweetpad-server          │
                │   (Node.js, vscode-free)       │
                │                                │
                │  • build orchestration         │
                │  • scheme/destination state    │
                │  • diagnostic parsing          │
                │  • workspace cache             │
                │  • writes .sweetpad/builds/*   │
                │  • pub/sub event bus           │
                └─┬─────────────────────┬────────┘
                  │                     │
       (private internal protocol — only these two implementations need to agree)
                  │                     │
        ┌─────────▼──────┐  ┌───────────▼─────────┐
        │ VS Code ext    │  │ sweetpad CLI        │
        │ (tree, status, │  │ (commands, --json,  │
        │  problems, …)  │  │  exit codes, stream)│
        └────────────────┘  └──────────▲──────────┘
                                       │
                              shell invocation
                                       │
                            ┌──────────┴───────────┐
                            │ Agent (Claude Code,  │
                            │   Cursor, scripts,   │
                            │   CI, humans)        │
                            └──────────────────────┘
```

Three tiers, two clients, one source of truth.

- **Server** is the engine of record. Holds build orchestration, persistent state (selected scheme/destination/config),
  the build registry, the diagnostic pipeline, and the event bus. Runs as a standalone Node.js process; has zero
  `vscode` imports.
- **Extension** becomes a thin client: subscribes to server events, renders state in the tree view / status bar /
  Problems panel, translates user clicks into server commands.
- **CLI** is the other client: a `sweetpad` binary that speaks the same internal protocol. Bundled with the extension
  and also distributed via Homebrew. Agents and CI invoke this CLI; they never connect to the server directly.

### Why no MCP server (and why no direct agent-server API)

Two reasons:

1. **Token economy.** 2025–2026 benchmark data shows CLI invocations beat MCP by ~10–32× on tokens and ~100% vs ~72%
   reliability in identical agent tasks. MCP tool descriptions sit permanently in the agent's context; a CLI is only
   paid for when invoked.
2. **Surface stability.** With the CLI as the only public agent contract, the CLI-server wire protocol stays private. We
   can change it freely without breaking external consumers, because the only external consumers see the CLI's commands
   and JSON output.

If MCP becomes worth adding later, a 50-line stdio adapter that shells out to the CLI gets us there.

### Why no BSP

The `sweetpad-bsp` project is a separate effort focused on replacing `xcode-build-server` for sourcekit-lsp. It is
deliberately **not merged** with this work. Two independent surfaces, two independent rollouts.

## 3. Migration plan (architecture-first)

The chosen sequencing is architecture-first: refactor first, ship the architecture together. Three phases:

### Phase 1 — extract the engine

Make `BuildManager`, `commands.ts` handlers, scheme cache, destination manager, diagnostic pipeline, and shell-env
warming **vscode-free**.

The lower layers (`src/common/cli/`, `src/common/xcode/`, `src/build/diagnostics-parser.ts`) are already clean. The work
is in the orchestration layer: replace direct `vscode.window.showQuickPick`, `vscode.tasks.executeTask`,
`vscode.workspace.getConfiguration`, etc. with injected "asker", "task runner", and "config provider" interfaces. The
extension provides VS Code-backed implementations; the future server provides network-backed ones.

No user-visible change in this phase. Validate the engine works as an in-process library inside the existing extension.

### Phase 2 — server binary + CLI

Wrap the engine in a `sweetpad-server` process. Build the `sweetpad` CLI as a client of it. Both run as new artifacts;
the extension still uses the engine in-process. Server and CLI work in standalone mode without VS Code.

This is the phase where agents get full control. Snapshot files, build IDs, event streams, all of it.

### Phase 3 — extension becomes a client

Switch the extension from in-process engine use to client-of-server. Activation spawns or connects to the server; the
extension just renders state. Behind a config flag at first (`sweetpad.experimental.serverMode = true`), then
default-on, then the in-process path is removed.

Each phase is independently shippable. Phases 1 and 2 don't disturb the existing user experience; Phase 3 is the
cutover.

## 4. Server

### Lifecycle

- **Spawn**: on-demand, by the first client that needs it. Extension activation, CLI invocation, or
  `sweetpad server start` all work the same.
- **Discovery**: workspace-keyed socket at `~/.sweetpad/run/<sha1(canonical-workspace-path)>/server.sock`, mode `0600`.
  CLI computes the hash from `process.cwd()` walking up to the workspace root. Extension uses the VS Code workspace
  folder API.
- **Idle timeout**: **5 minutes** after the last client disconnects, then graceful exit. While the extension is
  connected, the timer never starts.
- **Stale socket detection**: lockfile at `…/server.json` contains PID + socket path. On startup the new server checks:
  if the recorded PID is dead, claim the slot.
- **Multi-window**: two VS Code windows on the same workspace path both connect to the same server. State is shared;
  both tree views render the same events. Worktrees have different canonical paths → different servers, no collision.

### State

- **Persisted** (in `<workspace>/.sweetpad/state.json`): selected scheme, destination, configuration, scheme cache (with
  mtime key), recent destinations, retention config, last-built timestamp.
- **In-memory** (lost on restart, rebuilt on demand): connected clients, running build processes, live event
  subscriptions.
- **Recoverable from disk** (reloaded on server startup): the build registry. The server scans `.sweetpad/builds/<id>/`
  and re-populates the in-memory list. Build ID counter resumes from `max(id) + 1`.

### Build registry

Each finished build keeps a directory:

```
<workspace>/.sweetpad/builds/<id>/
  snapshot.json    # the full Build object + diagnostics array
  log.txt          # raw xcodebuild output
```

Plus a convenience pointer at `.sweetpad/last-build.json` for the simple "Claude, what just broke?" flow that doesn't
need IDs.

Build IDs are short, monotonically growing, workspace-scoped: `b1`, `b2`, `b3`, …. Not UUIDs — agents and humans both
type these.

**Retention**: keep last **10** finished builds; evict oldest. No byte cap in v1; add `retention.maxBytes` later if
individual logs get huge.

### `.gitignore` handling

On the first creation of `.sweetpad/`, the server appends to `.gitignore` (with a marker comment) silently. Re-runs are
idempotent. Marker is so it's clear who modified the file and what entry is owned by SweetPad.

### Workspace lock

Only one server may own a given workspace path at a time. Lock is held via the BSD-locked `server.json` lockfile.
Conflicting attempts return `WORKSPACE_LOCKED`.

## 5. CLI — agent-first, human-second

### Surface

```
# Workflow (each is one orchestrated operation, not a chain)
sweetpad build   [--scheme=X] [--destination=Y] [--config=Z]
                 [--json] [--raw] [--fields=...] [--background]
                 [--wait=DUR] [--force]
sweetpad run     [...same flags...]            # build + install + launch
sweetpad test    [...same flags...]            # build-for-testing + test
sweetpad clean
sweetpad stop    [<id-or-prefix>]

# Build management (first-class entities)
sweetpad builds                                # list running + recent (last 10)
sweetpad builds --running
sweetpad attach <id-or-prefix>                 # follow stream; replays finished
sweetpad show   <id-or-prefix>                 # full Build object; authoritative

# State (mirrors extension UI; mutations push events to all subscribers)
sweetpad scheme       [set <name> | list]
sweetpad destination  [set <id>   | list]
sweetpad config       [set <name>]
sweetpad status                                # everything at once

# Inspection (read-only)
sweetpad errors  [--fields=...]                # last build's diagnostics
sweetpad logs    [--tail=N] [--errors-only]
sweetpad events                                # tail live stream (debug)

# Discovery (agent-first)
sweetpad usage                                 # all commands + intents
sweetpad schema <cmd>                          # JSON schema for one command

# Server lifecycle
sweetpad server [start | stop | status]
sweetpad install-cli                           # symlink bundled binary
sweetpad --version
```

### Principles

- **Agent-first, human-second.** Bare invocations target agent ergonomics; humans get TTY niceties (colors, spinners,
  interactive prompts) on top.
- **Workflow-first commands**, not API mirrors. `sweetpad run` does build + install + launch as one operation; agents
  don't have to chain primitives.
- **Block by default**, `--background` and `--wait <duration>` opt-ins.
- **`--json`** strips all TTY behavior unconditionally (no colors, no spinners, no interactive prompts). Honors
  `NO_COLOR` in human mode.
- **`--raw`** minifies JSON whitespace (~40% token reduction).
- **`--fields=a,b,c`** projects to those fields only. Highest-leverage token-saver.
- **stdout = data contract, stderr = everything else** (progress, diagnostics, warnings, debug). Agents trust stdout iff
  exit code == 0.
- **Build ID prefix resolution**: `sweetpad attach b1a` resolves uniquely or errors with the candidate list.
- **DWIM on single-option scenarios**: if only one scheme/destination/config exists and none is selected, use it
  implicitly; don't mutate persisted selection. Explicit `set` is the only way to change persisted state.

### TTY behavior

If `stdin && stdout` are both TTYs and a request is ambiguous, prompt interactively. Otherwise (pipe, agent context)
return a structured error. `--json` disables prompting regardless of TTY status.

### Exit code taxonomy

| Exit | Meaning                                                 | Agent action             |
| ---- | ------------------------------------------------------- | ------------------------ |
| 0    | Success (or `--wait` timeout with `status: "running"`)  | Done or poll             |
| 1    | Build failed, server unreachable, transient             | Read errors, maybe retry |
| 2    | User error: invalid scheme, bad flag, ambiguous request | Don't retry the same way |

### Discovery

Instead of long `--help` walls in agent contexts:

- `sweetpad usage` — one entry point listing every command with a one-line intent description.
- `sweetpad schema <cmd>` — runtime JSON schema for that command's params, output shape (referenced by name), and exit
  code meanings.

Agents query at runtime instead of carrying static help in context.

## 6. JSON contract

### Envelope

Every command response wraps in:

```json
{
  "ok": true,
  "schemaVersion": "1.0",
  "data": { ...command-specific... }
}
```

or on failure:

```json
{
  "ok": false,
  "schemaVersion": "1.0",
  "error": {
    "code": "BUILD_NOT_FOUND",
    "message": "no build with id 'b9' found",
    "hint": "sweetpad builds list"
  }
}
```

The `hint` field is always a **literal next-command string**, not prose. Agents can paste it directly.

Stream items (NDJSON from `sweetpad attach`) **skip** the outer `ok` wrap. Each line is a self-describing event with
`schemaVersion`, `event`, `buildId`, `ts`, `data`.

### Versioning

`schemaVersion` is **per artifact**. The schema for a Build evolves independently from the schema for a Diagnostic.
Breaking changes bump major; additive changes don't.

### Core entities

#### `Build`

```json
{
  "schemaVersion": "1.0",
  "buildId": "b1",
  "scheme": "MyApp",
  "destination": "iPhone 15 (iOS 18.0)",
  "config": "Debug",
  "command": "build",
  "status": "failed",
  "exitCode": 65,
  "originator": "vscode",
  "startedAt": "2026-05-17T14:31:12Z",
  "finishedAt": "2026-05-17T14:31:24Z",
  "durationMs": 12340,
  "errorCount": 2,
  "warningCount": 0,
  "snapshotPath": ".sweetpad/builds/b1/snapshot.json",
  "logPath": ".sweetpad/builds/b1/log.txt"
}
```

Enums:

- `status`: `running | succeeded | failed | cancelled | interrupted` (`interrupted` = server crashed mid-build)
- `command`: `build | run | test | clean`
- `originator`: `vscode | cli`

#### `Diagnostic`

```json
{
  "file": "MyApp/View.swift",
  "line": 42,
  "column": 16,
  "endLine": 42,
  "endColumn": 19,
  "severity": "error",
  "message": "cannot find 'Foo' in scope",
  "source": "swift"
}
```

#### `Destination`

```json
{
  "id": "E2A7C5D4-4F1A-4E9F-A0B1-...",
  "name": "iPhone 15",
  "kind": "simulator",
  "platform": "iOS",
  "osVersion": "18.0",
  "arch": "arm64",
  "state": "booted",
  "available": true,
  "isSelected": false
}
```

#### `Scheme`

```json
{
  "name": "MyApp",
  "configurations": ["Debug", "Release"],
  "isSelected": true,
  "isTestable": false,
  "isRunnable": true
}
```

#### `Configuration`

```json
{ "name": "Debug", "isSelected": true }
```

### Event stream (NDJSON from `sweetpad attach`)

Typed semantic events only — raw xcodebuild lines stay in `log.txt`, not in the stream.

```jsonl
{"schemaVersion":"1.0","event":"build.started","buildId":"b1","ts":"…","data":{"scheme":"MyApp","destination":"iPhone 15","config":"Debug"}}
{"schemaVersion":"1.0","event":"build.target","buildId":"b1","ts":"…","data":{"target":"MyApp","phase":"compile","file":"View.swift"}}
{"schemaVersion":"1.0","event":"build.diagnostic","buildId":"b1","ts":"…","data":{"file":"View.swift","line":42,"column":16,"severity":"error","message":"cannot find 'Foo' in scope","source":"swift"}}
{"schemaVersion":"1.0","event":"build.finished","buildId":"b1","ts":"…","data":{"status":"failed","exitCode":65,"errorCount":1,"warningCount":0,"durationMs":12340}}
```

Polling is the source of truth: events accelerate but `sweetpad show <id>` is always authoritative. If the event stream
drops mid-attach, the CLI falls back to polling `show`.

### Error code enum (closed)

Per-resource granularity (~20 codes):

```
# Workspace / setup
WORKSPACE_NOT_DETECTED       no xcworkspace/xcodeproj/Package.swift/Project.swift in cwd
WORKSPACE_LOCKED             another sweetpad instance holds the workspace lock

# Server lifecycle
SERVER_UNREACHABLE           socket exists but no response (stale or crashed)
SERVER_VERSION_MISMATCH      CLI and server major versions differ
SERVER_START_FAILED          tried to spawn server and it crashed

# Scheme
SCHEME_NOT_FOUND             named scheme doesn't exist in workspace
SCHEME_AMBIGUOUS             prefix or partial name matched >1 scheme
NO_SCHEME_SELECTED           operation needs a scheme; none picked and none passed

# Destination
DESTINATION_NOT_FOUND        named destination doesn't exist
DESTINATION_AMBIGUOUS        prefix matched >1
DESTINATION_UNAVAILABLE      exists but is shutdown/unpaired/etc.
NO_DESTINATION_SELECTED      operation needs one; none picked and none passed

# Configuration
CONFIG_NOT_FOUND             named configuration doesn't exist for scheme

# Build lifecycle
BUILD_NOT_FOUND              referenced build id doesn't exist
BUILD_AMBIGUOUS              prefix matched >1
BUILD_IN_PROGRESS            another build for this workspace is running
BUILD_NOT_RUNNING            tried to stop a finished build (use show instead)
BUILD_FAILED                 build ran to completion but produced errors
BUILD_CANCELLED              user/agent issued stop

# Generic
INVALID_ARGUMENT             bad flag value or missing required flag
INTERNAL                     unexpected error; include in logs
```

## 7. Behavioral details

### `sweetpad build` when another build is running

Returns `BUILD_IN_PROGRESS` with the running build(s) inline so the agent has everything in one round-trip:

```json
{
  "ok": false,
  "schemaVersion": "1.0",
  "error": {
    "code": "BUILD_IN_PROGRESS",
    "message": "a build is already running in this workspace",
    "hint": "sweetpad attach b1"
  },
  "running": [
    {
      "buildId": "b1",
      "scheme": "MyApp",
      "destination": "iPhone 15 (iOS 18.0)",
      "config": "Debug",
      "startedAt": "2026-05-17T14:31:12Z",
      "originator": "vscode"
    }
  ]
}
```

`--force` cancels the running build (marked `cancelled` in registry) and starts a new one.

One running build per workspace in v1. Agents asking for a build while one runs must explicitly attach, stop, or force.

### `--wait <duration>`

Blocks up to N seconds. On finish → returns the final Build with its actual status, exit code reflects build result. On
timeout → exit 0, returns the Build with `status: "running"`. Agent reads the status field to decide whether to attach
or poll.

### `sweetpad attach <id>`

- Live build → streams events until finish, exits with the build's status.
- Finished build → full replay of the typed event stream from disk, then exits with the build's status. Same UX
  whichever side of "finished" the build is on.

### CLI auto-spawns the server

If the CLI doesn't find a server socket for the current workspace, it spawns `sweetpad-server` as a detached background
process and waits up to ~2s for readiness, then issues the command. Agents don't have to know about server lifecycle.

## 8. Distribution

- **Bundled in the VS Code extension** — primary channel. Extension ships both the server and CLI binaries (or scripts;
  Node-based). On activation, the extension knows where the bundled binary is. `sweetpad install-cli` symlinks it into
  `/usr/local/bin/sweetpad` (or `~/.local/bin`) for terminal access.
- **Homebrew formula** — `brew install sweetpad/tap/cli` for terminal-first users, CI runners, and folks without the
  extension. Lockstep versioned with the extension.

No npm publish in v1. Mac iOS devs lean Homebrew.

## 9. Deferred

- **Wire protocol details** between CLI and server (JSON-RPC? newline-JSON? custom?). It's private to those two
  implementations — pick anything that supports request/response + server-to-client notifications.
- **Authentication / handshake**: Unix socket permissions only for v1.
- **Configuration file format** (`~/.sweetpad/config.json`, per-workspace overrides): defer until the first config knob
  beyond what's covered.
- **Logging / telemetry**: existing Sentry integration moves server-side; CLI errors opt-in/opt-out structure TBD.
- **MCP adapter**: a 50-line stdio shim that wraps the CLI. Trivial when needed; not in v1.
- **Concrete code-level refactor steps** for Phase 1. Best done with the source open, file by file.

## 10. Open issues to revisit

- Whether the build registry needs richer audit fields (e.g. a `replacedBuildId` link when `--force` cancels and
  replaces) — we chose the Standard Build shape which doesn't carry that, but it may be worth adding one field.
- Retention `maxBytes` cap: deferred but likely needed once we see real disk usage patterns.
- Whether `originator` should grow beyond `vscode | cli` if multi-window or agent-attribution becomes important.
