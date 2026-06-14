import { createHash, randomBytes } from "node:crypto";
import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// SweetPad keeps all per-project runtime state OUT of the project tree. The
// machine-managed state lives under the XDG state home (`~/.local/state/sweetpad`
// by default) — the same place the CLI keeps its `state.toml`:
//
//   <stateHome>/projects.json        the discovery index (path -> control server)
//   <stateHome>/projects/<hash>/     per-project config (the BSP server's bsp.json)
//
// Bulky, noisy, or truly ephemeral state (logs, build history) and the Unix
// sockets stay in a per-workspace tmpdir: `sun_path` caps at ~104 bytes, and a
// state-home path can be long, so sockets use a short tmpdir path. Nothing is
// ever written into the project root — there is no `.sweetpad/` directory.

/** `$XDG_STATE_HOME` or `~/.local/state`. */
function stateHome(): string {
  const xdg = process.env.XDG_STATE_HOME;
  if (xdg && xdg.length > 0) return xdg;
  return path.join(os.homedir(), ".local", "state");
}

/** `<stateHome>/sweetpad` — the root of SweetPad's machine-managed state. */
export function getSweetpadStateHome(): string {
  return path.join(stateHome(), "sweetpad");
}

/**
 * The project-discovery index: a map of canonical workspace path → running
 * control server. The extension maintains it; the `sweetpad vscode` CLI reads it
 * to find the control socket for the project it's run inside (replacing the old
 * `.sweetpad/cli.json`).
 */
export function getProjectsIndexFile(): string {
  return path.join(getSweetpadStateHome(), "projects.json");
}

// A short, stable token derived from the workspace path, for naming per-workspace
// entries (the sockets, the tmpdir state root, the per-project state dir) — keeps
// the socket path short (well under `sun_path`) and collision-free across projects.
export function workspaceHash(workspacePath: string): string {
  return createHash("sha1").update(workspacePath).digest("hex").slice(0, 12);
}

/**
 * The per-project state directory under the state home, holding config the
 * extension writes for out-of-process consumers (the BSP server's `bsp.json`).
 * Named by `workspaceHash`, so it's stable and out of the project tree.
 */
export function getProjectStateDir(workspacePath: string): string {
  return path.join(getSweetpadStateHome(), "projects", workspaceHash(workspacePath));
}

// Per-workspace runtime dir under the OS temp dir, holding logs and build history
// (`bsp.log`, `builds/<id>/build.log`). Kept out of the project tree so logs never
// clutter or get committed; the OS reclaims it, which is fine for ephemeral state.
export function getTmpStateRoot(workspacePath: string): string {
  return path.join(os.tmpdir(), `sweetpad-${workspaceHash(workspacePath)}`);
}

export function getBuildsDir(workspacePath: string): string {
  return path.join(getTmpStateRoot(workspacePath), "builds");
}

export function getBuildDir(workspacePath: string, buildId: string): string {
  return path.join(getBuildsDir(workspacePath), buildId);
}

// The socket path for a server name: a short tmpdir path, independent of how
// deeply the project is nested, so it always fits within `sun_path`. Derivable
// from the name alone, so a client that knows the name can connect without
// reading the index.
export function getSocketPath(name: string): string {
  return path.join(os.tmpdir(), `sweetpad-${name}.sock`);
}

export function generateServerName(): string {
  return randomBytes(3).toString("hex");
}

export async function ensureDir(dir: string): Promise<void> {
  await fs.mkdir(dir, { recursive: true });
}

export async function safeUnlink(target: string): Promise<void> {
  try {
    await fs.unlink(target);
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code && code !== "ENOENT") throw err;
  }
}
