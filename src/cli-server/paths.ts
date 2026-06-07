import { randomBytes } from "node:crypto";
import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// SweetPad keeps per-project runtime state in `<workspace>/.sweetpad` — the CLI
// server's `cli.json`, the BSP server's `bsp.json`, and build history. It's fully
// ephemeral and meant to be gitignored.
//
// The Unix sockets themselves do NOT live here: `sun_path` caps at ~104 bytes,
// and a deep project path would blow that. Sockets live at a short tmpdir path
// (`getSocketPath`), and `cli.json` points at one.
export const SWEETPAD_DIR_NAME = ".sweetpad";

export function getStateRoot(workspacePath: string): string {
  return path.join(workspacePath, SWEETPAD_DIR_NAME);
}

// The CLI control server's connection file: a single `.sweetpad/cli.json` holding
// the running server's socket + metadata. Last-writer-wins — a second window
// overwrites it (no multi-server registry); the CLI reads it to find the socket.
export function getCliConfigFile(workspacePath: string): string {
  return path.join(getStateRoot(workspacePath), "cli.json");
}

export function getBuildsDir(workspacePath: string): string {
  return path.join(getStateRoot(workspacePath), "builds");
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
