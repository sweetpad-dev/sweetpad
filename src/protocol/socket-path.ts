import * as crypto from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

/**
 * Workspace-keyed socket path: `~/.sweetpad/run/<sha1(canonical-path)>/server.sock`.
 * Canonicalises the workspace path via `realpathSync` when the path exists, so
 * `/private/var/...` symlinks resolve to a single key. Falls back to
 * `path.resolve` when the path doesn't exist (e.g. tests).
 */
export function getServerSocketPath(workspacePath: string): string {
  const canonical = canonicalisePath(workspacePath);
  const hash = crypto.createHash("sha1").update(canonical).digest("hex");
  return path.join(getRunDir(), hash, "server.sock");
}

export function getServerLockfilePath(workspacePath: string): string {
  return path.join(path.dirname(getServerSocketPath(workspacePath)), "server.json");
}

export function getRunDir(): string {
  return path.join(os.homedir(), ".sweetpad", "run");
}

function canonicalisePath(p: string): string {
  const resolved = path.resolve(p);
  try {
    return fs.realpathSync.native(resolved);
  } catch {
    return resolved;
  }
}
