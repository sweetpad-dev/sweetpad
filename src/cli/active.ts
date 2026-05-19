import { promises as fs } from "node:fs";

import { ensureDir, getActiveFile, getStateRoot } from "../server/paths";
import type { ActiveServer } from "../server/types";

export async function readActive(): Promise<ActiveServer | undefined> {
  try {
    const raw = await fs.readFile(getActiveFile(), "utf8");
    const parsed = JSON.parse(raw) as ActiveServer;
    if (typeof parsed?.server !== "string") return undefined;
    return parsed;
  } catch {
    return undefined;
  }
}

// tmp + rename keeps the read side from ever seeing a half-written file.
export async function writeActive(name: string): Promise<void> {
  const payload: ActiveServer = { server: name, setAt: new Date().toISOString() };
  await ensureDir(getStateRoot());
  const tmp = `${getActiveFile()}.${process.pid}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(payload, null, 2));
  await fs.rename(tmp, getActiveFile());
}

export async function clearActive(): Promise<void> {
  try {
    await fs.unlink(getActiveFile());
  } catch {
    // no-op
  }
}
