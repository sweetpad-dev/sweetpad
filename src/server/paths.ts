import { createHash, randomBytes } from "node:crypto";
import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// SweetPad's runtime state is XDG-style state (sockets, active-server pointer,
// build history) — not cache (evictable) and not config (user-edited).
export function getStateRoot(): string {
  const xdg = process.env.XDG_STATE_HOME;
  if (xdg && path.isAbsolute(xdg)) {
    return path.join(xdg, "sweetpad");
  }
  return path.join(os.homedir(), ".local", "state", "sweetpad");
}

export function getSocketsDir(): string {
  return path.join(getStateRoot(), "sockets");
}

export function getWorkspacesDir(): string {
  return path.join(getStateRoot(), "workspaces");
}

export function getActiveFile(): string {
  return path.join(getStateRoot(), "active.json");
}

export function getSocketPath(name: string): string {
  return path.join(getSocketsDir(), `${name}.sock`);
}

export function getMetadataPath(name: string): string {
  return path.join(getSocketsDir(), `${name}.json`);
}

export function getWorkspaceDir(workspacePath: string): string {
  return path.join(getWorkspacesDir(), workspacePathHash(workspacePath));
}

export function getBuildsDir(workspacePath: string): string {
  return path.join(getWorkspaceDir(workspacePath), "builds");
}

export function getBuildDir(workspacePath: string, buildId: string): string {
  return path.join(getBuildsDir(workspacePath), buildId);
}

export function workspacePathHash(workspacePath: string): string {
  return createHash("sha1").update(workspacePath).digest("hex");
}

// Symlink-resolved so two paths into the same tree dedupe to one directory.
export async function canonicalizeWorkspacePath(p: string): Promise<string> {
  try {
    return await fs.realpath(p);
  } catch {
    return path.resolve(p);
  }
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
