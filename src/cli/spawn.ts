import * as child_process from "node:child_process";
import * as fs from "node:fs";
import * as net from "node:net";
import * as path from "node:path";

const READINESS_TIMEOUT_MS = 2000;
const READINESS_POLL_MS = 50;

/**
 * If `socketPath` isn't accepting connections, spawn the server detached and
 * wait up to ~2s for the socket to become reachable. The server's
 * `out/server.js` lives next to the CLI's bundled entry — we find it via
 * `__dirname` (`out/cli.js` → `out/server.js`).
 */
export async function ensureServerRunning(options: {
  socketPath: string;
  serverEntryPath: string;
  workspacePath: string;
}): Promise<void> {
  if (await isSocketReachable(options.socketPath)) return;

  if (!fs.existsSync(options.serverEntryPath)) {
    throw new Error(
      `Server entry not found at ${options.serverEntryPath} — expected the bundle next to the CLI binary`,
    );
  }

  const child = child_process.spawn(process.execPath, [options.serverEntryPath, "start", `--workspace=${options.workspacePath}`], {
    cwd: options.workspacePath,
    detached: true,
    stdio: "ignore",
    env: { ...process.env },
  });
  child.unref();

  const deadline = Date.now() + READINESS_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (await isSocketReachable(options.socketPath)) return;
    await sleep(READINESS_POLL_MS);
  }
  throw new Error(`sweetpad-server did not become reachable at ${options.socketPath} within ${READINESS_TIMEOUT_MS}ms`);
}

/** Returns true iff a TCP-style `connect()` to the Unix socket succeeds immediately. */
function isSocketReachable(socketPath: string): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const socket = net.createConnection({ path: socketPath });
    const finish = (ok: boolean) => {
      socket.destroy();
      resolve(ok);
    };
    socket.once("connect", () => finish(true));
    socket.once("error", () => finish(false));
  });
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** Resolves the server entry bundled next to the CLI bundle (`out/cli.js` → `out/server.js`). */
export function defaultServerEntryPath(cliEntryDir: string): string {
  return path.join(cliEntryDir, "server.js");
}
