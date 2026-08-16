import { promises as fs } from "node:fs";
import * as path from "node:path";

import { ensureDir, getProjectsIndexFile, getSweetpadStateHome } from "./paths";
import type { CliServerMetadata } from "./types";

export const PROJECTS_INDEX_VERSION = 1;

/**
 * One project's entry in the discovery index. Each subsystem contributes its own
 * pointer, keyed by canonical workspace path: the control server its socket
 * metadata, the BSP service the absolute path to its `bsp.json`. They're
 * independent — BSP works with the control server disabled and vice versa.
 */
export type ProjectEntry = {
  /** The running CLI control server (when `sweetpad.cliServer.enabled`). */
  control?: CliServerMetadata;
  /** Absolute path to the project's BSP `bsp.json` (when SweetPad is the build-server provider). */
  bspConfig?: string;
};

/**
 * The host-wide discovery index the extension maintains at
 * `<stateHome>/sweetpad/projects.json`. Keyed by canonical workspace path (the
 * `fs.realpath` of the VS Code workspace folder). The `sweetpad vscode` CLI and
 * the BSP server both walk up from their cwd and look up the nearest registered
 * ancestor — the index-backed replacement for the old walk-up to an in-project
 * `.sweetpad/` directory.
 */
type ProjectsIndex = {
  version: number;
  projects: Record<string, ProjectEntry>;
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

// tmp+rename keeps the readers from ever seeing a half-written file. The
// read-modify-write is not locked across processes: concurrent windows race, but
// each subsystem owns its own field and the loser re-registers on its next write
// — the same last-writer-wins contract the old per-project cli.json had.
async function writeIndex(index: ProjectsIndex): Promise<void> {
  await ensureDir(getSweetpadStateHome());
  const file = getProjectsIndexFile();
  const tmp = `${file}.${process.pid}.tmp`;
  await fs.writeFile(tmp, JSON.stringify(index, null, 2), { mode: 0o600 });
  await fs.rename(tmp, file);
}

/**
 * The canonical index key for a workspace folder: its `realpath`, so the spelling
 * matches what the CLI / BSP server compute when they canonicalize their cwd's
 * ancestors. Falls back to the resolved path if the folder can't be realpath'd.
 */
export async function projectKey(workspacePath: string): Promise<string> {
  try {
    return await fs.realpath(workspacePath);
  } catch {
    return path.resolve(workspacePath);
  }
}

/**
 * Read-modify-write the entry for `workspacePath`. `mutate` receives the current
 * entry (empty if none) and returns the next one; returning an entry with no
 * fields (or `undefined`) drops the key entirely.
 */
async function mutateEntry(
  workspacePath: string,
  mutate: (entry: ProjectEntry) => ProjectEntry | undefined,
): Promise<void> {
  const key = await projectKey(workspacePath);
  const index = await readIndex();
  const next = mutate(index.projects[key] ?? {});
  if (!next || (next.control === undefined && next.bspConfig === undefined)) {
    delete index.projects[key];
  } else {
    index.projects[key] = next;
  }
  await writeIndex(index);
}

/**
 * Whether `pid` is still running. Signal 0 runs the existence and permission checks without
 * delivering anything; `EPERM` means the process is there but owned by another user.
 */
function isProcessAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === "EPERM";
  }
}

/**
 * Advertise (or refresh) the control server for `workspacePath`. Returns whether the entry
 * now points at us. By default it claims the key outright, the index's last-writer-wins
 * contract.
 *
 * `secondary` marks a folder that is merely open in the window rather than the one this
 * window's project lives in. Such a folder is only somewhere the CLI might be run from, so
 * it steps aside for a live window that holds the folder as its own project: otherwise
 * having a folder open here would redirect that folder's `sweetpad` invocations to this
 * window, and closing this window would take the other one's pointer down with it.
 */
export async function registerControlServer(
  workspacePath: string,
  meta: CliServerMetadata,
  options?: { secondary?: boolean },
): Promise<boolean> {
  let claimed = false;
  await mutateEntry(workspacePath, (entry) => {
    const owner = entry.control;
    if (options?.secondary && owner && owner.pid !== meta.pid && isProcessAlive(owner.pid)) {
      return entry;
    }
    claimed = true;
    return { ...entry, control: meta };
  });
  return claimed;
}

/**
 * Drop our control-server pointer, but only if it still points at us — a newer
 * window may have replaced it (last-writer-wins). The BSP pointer is left intact.
 */
export async function unregisterControlServer(workspacePath: string, pid: number): Promise<void> {
  await mutateEntry(workspacePath, (entry) => {
    if (entry.control?.pid !== pid) return entry;
    const rest = { ...entry };
    delete rest.control;
    return rest;
  });
}

/** Advertise the BSP `bsp.json` path for `workspacePath` (absolute). */
export async function registerBspConfig(workspacePath: string, bspConfig: string): Promise<void> {
  await mutateEntry(workspacePath, (entry) => ({ ...entry, bspConfig }));
}

/** Drop our BSP pointer (best-effort, on shutdown). */
export async function unregisterBspConfig(workspacePath: string): Promise<void> {
  await mutateEntry(workspacePath, (entry) => {
    const rest = { ...entry };
    delete rest.bspConfig;
    return rest;
  });
}
