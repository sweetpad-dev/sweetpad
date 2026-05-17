import * as fs from "node:fs";
import * as path from "node:path";

/**
 * On-disk record of the currently-running server. Used to fail fast with a
 * clear WORKSPACE_LOCKED message when two servers race for the same workspace,
 * and to recover gracefully when a previous server crashed without cleanup.
 */
export type LockfileContents = {
  pid: number;
  socketPath: string;
  startedAt: string;
};

export type LockOutcome =
  | { status: "acquired" }
  | { status: "locked"; holder: LockfileContents };

/**
 * Try to claim the server slot for a workspace. Reads any existing lockfile;
 * if the recorded PID is alive, returns `locked`. If the PID is dead, removes
 * the stale lockfile (and the dangling socket next to it) and claims the slot.
 *
 * Caller is expected to listen on `socketPath` and then `commit()` to write
 * the lockfile — listening first ensures EADDRINUSE surfaces before we
 * claim anything on disk.
 */
export function tryAcquireLock(lockfilePath: string): LockOutcome {
  const existing = readLockfile(lockfilePath);
  if (existing && isProcessAlive(existing.pid)) {
    return { status: "locked", holder: existing };
  }
  if (existing) {
    // Stale — clear the lockfile and the matching socket so listen() succeeds.
    safeUnlink(lockfilePath);
    safeUnlink(existing.socketPath);
  }
  return { status: "acquired" };
}

export function writeLockfile(lockfilePath: string, contents: LockfileContents): void {
  fs.mkdirSync(path.dirname(lockfilePath), { recursive: true });
  fs.writeFileSync(lockfilePath, `${JSON.stringify(contents, null, 2)}\n`, { mode: 0o600 });
}

export function removeLockfile(lockfilePath: string): void {
  safeUnlink(lockfilePath);
}

function readLockfile(lockfilePath: string): LockfileContents | undefined {
  try {
    const raw = fs.readFileSync(lockfilePath, "utf8");
    const parsed = JSON.parse(raw) as LockfileContents;
    if (typeof parsed.pid !== "number" || typeof parsed.socketPath !== "string") {
      return undefined;
    }
    return parsed;
  } catch {
    return undefined;
  }
}

function isProcessAlive(pid: number): boolean {
  try {
    // signal 0 is "test only" — no signal sent, just probes existence.
    // ESRCH = process doesn't exist. EPERM = exists but we can't signal it
    // (different uid). Either non-error or EPERM means alive.
    process.kill(pid, 0);
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    return code === "EPERM";
  }
}

function safeUnlink(p: string): void {
  try {
    fs.unlinkSync(p);
  } catch {
    // ENOENT or similar — already gone.
  }
}
