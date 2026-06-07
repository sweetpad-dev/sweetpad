import { createHash, randomBytes } from "node:crypto";
import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// SweetPad keeps per-project runtime state in two places. The `<workspace>/.sweetpad`
// dir holds only the small discovery pointers — the CLI server's `cli.json` and the
// BSP server's `bsp.json` — and is meant to be gitignored. Bulky, noisy, or truly
// ephemeral state (logs, build history) lives outside the project in a per-workspace
// tmpdir (`getTmpStateRoot`) so it never clutters the tree or gets committed.
//
// The Unix sockets also live in tmpdir: `sun_path` caps at ~104 bytes, and a deep
// project path would blow that. Sockets use a short tmpdir path (`getSocketPath`),
// and `cli.json` points at one.
export const SWEETPAD_DIR_NAME = ".sweetpad";

export function getStateRoot(workspacePath: string): string {
  return path.join(workspacePath, SWEETPAD_DIR_NAME);
}

// A short, stable token derived from the workspace path, for naming per-workspace
// tmpdir entries (the BSP socket, the tmp state root) — keeps them short (well under
// `sun_path`) and collision-free across projects.
export function workspaceHash(workspacePath: string): string {
  return createHash("sha1").update(workspacePath).digest("hex").slice(0, 12);
}

// Per-workspace runtime dir under the OS temp dir, holding logs and build history
// (`bsp.log`, `builds/<id>/build.log`). Kept out of the project tree so logs never
// clutter or get committed; the OS reclaims it, which is fine for ephemeral state.
export function getTmpStateRoot(workspacePath: string): string {
  return path.join(os.tmpdir(), `sweetpad-${workspaceHash(workspacePath)}`);
}

// The CLI control server's connection file: a single `.sweetpad/cli.json` holding
// the running server's socket + metadata. Last-writer-wins — a second window
// overwrites it (no multi-server registry); the CLI reads it to find the socket.
export function getCliConfigFile(workspacePath: string): string {
  return path.join(getStateRoot(workspacePath), "cli.json");
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
// reading the connection file.
export function getSocketPath(name: string): string {
  return path.join(os.tmpdir(), `sweetpad-${name}.sock`);
}

export function generateServerName(): string {
  return randomBytes(3).toString("hex");
}

// Walk up from `startDir` to the nearest ancestor that contains a `.sweetpad`
// directory, returning that ancestor (the workspace root) — how the CLI finds
// the project it's run inside, like `git` finds `.git`. `undefined` if none.
export async function findProjectRoot(startDir: string): Promise<string | undefined> {
  let dir = path.resolve(startDir);
  for (;;) {
    try {
      const stat = await fs.stat(path.join(dir, SWEETPAD_DIR_NAME));
      if (stat.isDirectory()) return dir;
    } catch {
      // not here; keep walking up
    }
    const parent = path.dirname(dir);
    if (parent === dir) return undefined;
    dir = parent;
  }
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
