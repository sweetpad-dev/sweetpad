---
sidebar_position: 15
---

# Agent CLI & RPC Server

SweetPad ships an opt-in JSON-RPC server and a bundled `sweetpad` command-line tool so other processes — scripts, CI
jobs, AI coding agents — can read state and trigger builds in your live VSCode session without screen-scraping the UI.

:::tip

This page is for power users who want to script SweetPad or wire it into an AI agent. If you just want to build and
run your app from the SweetPad sidebar, skip this page.

:::

## When you'd use it

- **AI coding agents.** Let a CLI-driven agent build the project, read diagnostics, and start the app on a Simulator
  without you having to click around the VSCode window.
- **Local scripts.** Trigger a build from a `git` hook, a Makefile, or a custom watcher; poll for the result.
- **Multi-window workflows.** Drive multiple VSCode windows (each owning its own SweetPad server) from a single
  shell.

If you don't need scripted access from outside VSCode, leave the server off and use the sidebar as normal.

## Enable the server

The server is **off by default**. Turn it on:

```json title=".vscode/settings.json"
{
  "sweetpad.server.enabled": true
}
```

When enabled, SweetPad creates a per-window Unix socket and registers it under a unique server name. You can verify
it's running with:

- `> SweetPad: Show RPC server status` — prints the server name, socket path, and process info.
- `> SweetPad: Copy RPC server name` — copies the name to the clipboard so you can pass it to a CLI invocation.
- `> SweetPad: Restart RPC server` — restarts the server (useful if you change settings or hit a stuck state).

## Install the CLI on `PATH`

The `sweetpad` CLI is bundled with the extension. Symlink it somewhere on `PATH` with:

- `> SweetPad: Install CLI on PATH`

You'll be offered two defaults — `/usr/local/bin/sweetpad` (system-wide, may need `sudo` rights) and
`~/.local/bin/sweetpad` (user-only, no privileges needed) — or you can type a custom path. The CLI is a symlink to
the bundled file, so when the extension updates, the CLI updates with it.

Verify the install:

```bash
sweetpad meta.version
```

## Multiple VSCode windows

Each VSCode window with `sweetpad.server.enabled` runs its own server. The CLI picks which one to talk to using this
precedence:

1. `--server <name>` on the command line.
2. `SWEETPAD_SERVER` environment variable.
3. The "active" server, set by `sweetpad servers switch <name>`.

List what's running and switch:

```bash
sweetpad servers list
sweetpad servers switch myapp
```

Names accept unique prefixes — `sweetpad servers switch my` works as long as only one server starts with `my`.

## Common workflows

Each command prints JSON; pipe through `jq` if you want pretty printing.

```bash
# Snapshot: scheme + destination + configuration + current/latest build
sweetpad state.get

# List schemes detected in the workspace
sweetpad scheme.list

# Pick a scheme and a destination, then build & run
sweetpad scheme.set MyApp
sweetpad destination.set <udid>
sweetpad build.start launch

# Wait for the build to finish (--timeout accepts "30s" / "5m" / "1h" or
# bare seconds; the server caps each call at 30s, so poll in a loop for
# longer waits).
sweetpad build.wait --timeout 30s

# Fetch the diagnostics from the last build
sweetpad build.diagnostics

# Stop a running build / app
sweetpad build.stop
```

The full method catalog (every RPC the server exposes) is available at runtime:

```bash
sweetpad meta.usage      # one-line summary per method
sweetpad meta.schema     # JSON schema for every method
sweetpad meta.schema build.start
```

## What you can do with it

- `meta.*` — server / extension version, workspace path, method catalog.
- `scheme.*`, `destination.*`, `buildConfig.*` — read and set the active selection.
- `state.get` — one-shot snapshot of everything above plus the latest/active build.
- `build.start / .stop / .wait / .status / .logs / .diagnostics / .list` — drive builds and inspect their output.
- `simulator.*` — list, boot, install, launch, screenshot Simulators.
- `device.install / .launch / .terminate` — same on physical devices.
- `scheme.reveal / scheme.write` — read or rewrite `.xcscheme` XML.
- `workspace.*` and `workspaceState.*` — workspace detection and persistent per-workspace KV storage.
- `vscode.executeCommand`, `vscodeSettings.*` — fall through to the VSCode command palette / settings API.

## Security model

The socket lives under `$XDG_STATE_HOME/sweetpad/sockets/` (defaulting to `~/.local/state/sweetpad/sockets/` on
macOS) and is `chmod 0600`, so only your user can connect to it. The server isn't exposed over the network.

That said, anything with read access to your user account can drive the server while it's enabled — so leave
`sweetpad.server.enabled` off unless you actually need scripted access.
