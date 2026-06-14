import { promises as fs } from "node:fs";
import * as path from "node:path";

import { ensureDir, getProjectsIndexFile, getSweetpadStateHome } from "./paths";
import type { CliServerMetadata } from "./types";

export const PROJECTS_INDEX_VERSION = 1;

/**
 * The host-wide discovery index the extension maintains at
 * `<stateHome>/sweetpad/projects.json`. Keyed by canonical workspace path (the
 * `fs.realpath` of the VS Code workspace folder), each value is the running
 * control server's metadata. The `sweetpad vscode` CLI walks up from its cwd and
 * looks up the nearest registered ancestor — the index-backed replacement for the
 * old walk-up to an in-project `.sweetpad/cli.json`.
 */
type ProjectsIndex = {
  version: number;
  projects: Record<string, CliServerMetadata>;
};

async function readIndex(): Promise<ProjectsIndex> {
  try {
    const raw = await fs.readFile(getProjectsIndexFile(), "utf8");
    const parsed = JSON.parse(raw) as ProjectsIndex;
    if (parsed && typeof parsed === "object" && parsed.projects) return parsed;
  } catch {
    // missing or corrupt — start fresh
  }
  return { version: PROJECTS_INDEX_VERSION, projects: {} };
}

// tmp+rename keeps the CLI's read side from ever seeing a half-written file. The
// read-modify-write is not locked across processes: concurrent windows race, but
// each key is owned by one window and the loser simply re-registers on its next
// write — the same last-writer-wins contract the old per-project cli.json had.
async function writeIndex(index: ProjectsIndex): Promise<void> {
  await ensureDir(getSweetpadStateHome());
  const file = getProjectsIndexFile();
  const tmp = `${file}.${process.pid}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(index, null, 2), { mode: 0o600 });
  await fs.rename(tmp, file);
}

/**
 * The canonical index key for a workspace folder: its `realpath`, so the spelling
 * matches what the CLI computes when it canonicalizes its cwd's ancestors. Falls
 * back to the resolved path if the folder can't be realpath'd.
 */
export async function projectKey(workspacePath: string): Promise<string> {
  try {
    return await fs.realpath(workspacePath);
  } catch {
    return path.resolve(workspacePath);
  }
}

/** Advertise (or refresh) the control server for `workspacePath`. */
export async function registerProject(workspacePath: string, meta: CliServerMetadata): Promise<void> {
  const key = await projectKey(workspacePath);
  const index = await readIndex();
  index.projects[key] = meta;
  await writeIndex(index);
}

/**
 * Remove our entry for `workspacePath`, but only if it still points at us — a
 * newer window may have replaced it (last-writer-wins), and we must not delete
 * its pointer.
 */
export async function unregisterProject(workspacePath: string, pid: number): Promise<void> {
  const key = await projectKey(workspacePath);
  const index = await readIndex();
  if (index.projects[key]?.pid === pid) {
    delete index.projects[key];
    await writeIndex(index);
  }
}
