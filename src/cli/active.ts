import { promises as fs } from "node:fs";

import { ensureDir, findProjectRoot, getActiveFile, getStateRoot } from "../server/paths";
import type { ActiveServer } from "../server/types";

export async function readActive(): Promise<ActiveServer | undefined> {
  const root = await findProjectRoot(process.cwd());
  if (!root) return undefined;
  try {
    const raw = await fs.readFile(getActiveFile(root), "utf8");
    const parsed = JSON.parse(raw) as ActiveServer;
    if (typeof parsed?.server !== "string") return undefined;
    return parsed;
  } catch {
    return undefined;
  }
}

// tmp + rename keeps the read side from ever seeing a half-written file.
export async function writeActive(name: string): Promise<void> {
  const root = await findProjectRoot(process.cwd());
  if (!root) throw new Error("No .sweetpad project found from the current directory");
  const payload: ActiveServer = { server: name, setAt: new Date().toISOString() };
  await ensureDir(getStateRoot(root));
  const target = getActiveFile(root);
  const tmp = `${target}.${process.pid}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(payload, null, 2));
  await fs.rename(tmp, target);
}

export async function clearActive(): Promise<void> {
  const root = await findProjectRoot(process.cwd());
  if (!root) return;
  try {
    await fs.unlink(getActiveFile(root));
  } catch {
    // no-op
  }
}
