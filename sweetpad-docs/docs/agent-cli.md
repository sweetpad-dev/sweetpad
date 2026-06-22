---
sidebar_position: 15
---

# Agent CLI & RPC Server

SweetPad ships a standalone `sweetpad` command-line tool plus an opt-in JSON-RPC server, so other
processes — scripts, CI jobs, AI coding agents — can drive your Xcode project and your live VS Code
session without screen-scraping the UI.

The CLI has two halves:

- **`sweetpad <command>`** — a standalone, headless CLI ("xcodebuild for humans"): inspect schemes
  and destinations, build, run, manage simulators, resolve Swift Package dependencies, and more. It
  needs nothing running — run `sweetpad --help` to explore.
- **`sweetpad vscode <method>`** — a JSON-RPC client that talks to a running VS Code window's SweetPad
  server (read state, trigger builds, drive simulators _inside_ your editor session). This page is
  mostly about this half.

:::tip

This page is for power users who want to script SweetPad or wire it into an AI agent. If you just want
to build and run your app, use the SweetPad sidebar and skip this page.

:::

## Install

The `sweetpad` CLI is distributed via Homebrew — a signed, notarized universal macOS binary,
independent of the VS Code extension:

```bash
brew install sweetpad-dev/tap/sweetpad
```

Verify it, and upgrade later with `brew upgrade sweetpad`:

```bash
sweetpad --version
```

## When you'd use the RPC server

- **AI coding agents.** Let a CLI-driven agent build the project, read diagnostics, and start the app
  on a Simulator without clicking around the VS Code window.
- **Local scripts.** Trigger a build from a `git` hook, a Makefile, or a custom watcher; poll for the
  result.
- **Multi-window workflows.** Drive several VS Code windows — each owning its own SweetPad server —
  from a single shell.

If you don't need scripted access into a live VS Code session, leave the server off and use the
standalone `sweetpad <command>` CLI or the sidebar.

## Enable the server

The server is **off by default**. Turn it on:

```json title=".vscode/settings.json"
{
  "sweetpad.cliServer.enabled": true
}
```

When enabled, SweetPad creates a per-window Unix socket and registers it for the workspace. Verify and
manage it from the Command Palette:

- **SweetPad: Show RPC server status** — prints the server name, socket path, and process info.
- **SweetPad: Copy RPC server name** — copies the name to the clipboard.
- **SweetPad: Restart RPC server** — restarts it (useful after changing settings or hitting a stuck state).

## Which window the CLI talks to

`sweetpad vscode` resolves the target window from your **current directory**: it finds the SweetPad
window whose open workspace is the nearest ancestor of where you run the command and connects to that
window's socket. So just run it from inside your project:

```bash
cd ~/Developer/MyApp
sweetpad vscode state.get
```

If more than one window has the same folder open, the most recently registered one wins — no manual
server switching needed.

## Common workflows

Each method prints JSON; pipe through `jq` to pretty-print.

```bash
# Snapshot: scheme + destination + configuration + current/latest build
sweetpad vscode state.get

# List schemes detected in the workspace
sweetpad vscode scheme.list

# Pick a scheme and a destination, then build & run
sweetpad vscode scheme.set MyApp
sweetpad vscode destination.set <udid>
sweetpad vscode build.start launch

# Wait for the build (--timeout accepts "30s" / "5m" / "1h" or bare seconds; the
# server caps each call ~30s, so poll in a loop for longer waits)
sweetpad vscode build.wait --timeout 30s

# Diagnostics from the last build
sweetpad vscode build.diagnostics

# Stop a running build / app
sweetpad vscode build.stop
```

## The method catalog

Every RPC the server exposes is discoverable at runtime:

```bash
sweetpad vscode meta.usage              # one-line summary per method
sweetpad vscode meta.schema             # JSON schema for every method
sweetpad vscode meta.schema build.start
```

Broad strokes of what's available:

- `meta.*` — server / extension version, workspace path, method catalog.
- `scheme.*`, `destination.*`, `buildConfig.*` — read and set the active selection.
- `state.get` — one-shot snapshot of the above plus the latest/active build.
- `build.start / .stop / .wait / .status / .logs / .diagnostics / .list` — drive builds and inspect output.
- `simulator.*` — list, boot, install, launch, screenshot Simulators.
- `device.install / .launch / .terminate` — the same on physical devices.
- `buildSettings.get`, `appPath.find`, `bundleId.get`, `xcodebuild.list` — resolved build info.
- `workspace.*` and `workspaceState.*` — workspace detection and persistent per-workspace KV storage.
- `vscode.executeCommand`, `vscodeSettings.*` — fall through to the VS Code command / settings API.
- `logs.tail` — stream the extension's logs.

## Security model

The socket lives under `$XDG_STATE_HOME/sweetpad/sockets/` (defaulting to
`~/.local/state/sweetpad/sockets/` on macOS) and is `chmod 0600`, so only your user can connect to it.
The server is never exposed over the network.

That said, anything with read access to your user account can drive the server while it's enabled — so
leave `sweetpad.cliServer.enabled` off unless you actually need scripted access.
