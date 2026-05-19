import { promises as fs } from "node:fs";
import * as net from "node:net";
import * as path from "node:path";

import { getMetadataPath, getSocketPath, getSocketsDir, safeUnlink } from "../server/paths";
import type { ServerListEntry, ServerMetadata } from "../server/types";
import { readActive } from "./active";

const PROBE_TIMEOUT_MS = 250;

/**
 * Enumerate running servers by reading the sidecar metadata files and probing
 * the socket of each. Dead entries are cleaned up lazily — their .sock and
 * .json get unlinked here so the next `servers list` returns a clean set.
 */
export async function listServers(): Promise<ServerListEntry[]> {
  const dir = getSocketsDir();
  let entries: string[];
  try {
    entries = await fs.readdir(dir);
  } catch (err) {
    const code = (err as NodeJS.ErrnoException)?.code;
    if (code === "ENOENT") return [];
    throw err;
  }

  const metaFiles = entries.filter((e) => e.endsWith(".json"));
  const active = await readActive();
  const activeName = active?.server;
  const results: ServerListEntry[] = [];

  for (const file of metaFiles) {
    const name = file.slice(0, -".json".length);
    const metaPath = path.join(dir, file);
    let meta: ServerMetadata | undefined;
    try {
      const raw = await fs.readFile(metaPath, "utf8");
      meta = JSON.parse(raw) as ServerMetadata;
    } catch {
      await unlinkPair(name);
      continue;
    }

    const alive = await pingSocket(getSocketPath(name));
    if (!alive) {
      await unlinkPair(name);
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

/**
 * Read full metadata for a single server. Returns undefined if missing.
 */
export async function readMetadata(name: string): Promise<ServerMetadata | undefined> {
  try {
    const raw = await fs.readFile(getMetadataPath(name), "utf8");
    return JSON.parse(raw) as ServerMetadata;
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

async function unlinkPair(name: string): Promise<void> {
  await Promise.all([safeUnlink(getSocketPath(name)), safeUnlink(getMetadataPath(name))]);
}
