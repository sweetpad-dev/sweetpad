import { promises as fs } from "node:fs";
import * as net from "node:net";
import * as path from "node:path";

import { findProjectRoot, getConnectionFile, getRunDir, safeUnlink } from "../server/paths";
import type { ServerListEntry, ServerMetadata } from "../server/types";
import { readActive } from "./active";

const PROBE_TIMEOUT_MS = 250;

/**
 * Enumerate running extension servers in this project's `.sweetpad/run` by
 * reading each connection file and probing its socket. Dead entries are cleaned
 * up lazily — the connection file and its socket get unlinked — so the next
 * `servers list` returns a clean set. BSP entries are ignored (not CLI targets).
 */
export async function listServers(): Promise<ServerListEntry[]> {
  const root = await findProjectRoot(process.cwd());
  if (!root) return [];
  const dir = getRunDir(root);
  let entries: string[];
  try {
    entries = await fs.readdir(dir);
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code === "ENOENT") return [];
    throw err;
  }

  const active = await readActive();
  const activeName = active?.server;
  const results: ServerListEntry[] = [];

  for (const file of entries.filter((e) => e.endsWith(".json"))) {
    const connPath = path.join(dir, file);
    let meta: ServerMetadata;
    try {
      meta = JSON.parse(await fs.readFile(connPath, "utf8")) as ServerMetadata;
    } catch {
      await safeUnlink(connPath);
      continue;
    }
    if (meta.kind !== "extension") continue;

    if (!(await pingSocket(meta.socket))) {
      await safeUnlink(meta.socket);
      await safeUnlink(connPath);
      continue;
    }

    results.push({
      name: meta.name,
      workspacePath: meta.workspacePath,
      isActive: meta.name === activeName,
    });
  }

  results.sort((a, b) => a.name.localeCompare(b.name));
  return results;
}

/** Full connection metadata for one server in the current project, or undefined. */
export async function readMetadata(name: string): Promise<ServerMetadata | undefined> {
  const root = await findProjectRoot(process.cwd());
  if (!root) return undefined;
  try {
    return JSON.parse(await fs.readFile(getConnectionFile(root, name), "utf8")) as ServerMetadata;
  } catch {
    return undefined;
  }
}

async function pingSocket(socketPath: string): Promise<boolean> {
  return await new Promise<boolean>((resolve) => {
    const socket = net.createConnection(socketPath);
    const cleanup = (ok: boolean) => {
      socket.removeAllListeners();
      socket.destroy();
      resolve(ok);
    };
    const timer = setTimeout(() => cleanup(false), PROBE_TIMEOUT_MS);
    timer.unref?.();
    socket.once("connect", () => {
      clearTimeout(timer);
      cleanup(true);
    });
    socket.once("error", () => {
      clearTimeout(timer);
      cleanup(false);
    });
  });
}
