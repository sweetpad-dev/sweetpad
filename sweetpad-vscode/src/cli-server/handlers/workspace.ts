import { promises as fs } from "node:fs";
import * as path from "node:path";

import { getCurrentXcodeWorkspacePath, getWorkspaceFolderPaths } from "../../build/utils";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import { requireString } from "./_common";
import type { HandlerFn } from "./context";

const RECENT_MAX = 10;
const SKIP_DIRS = new Set(["node_modules", ".build", "DerivedData", ".git"]);

type Candidate = { path: string; kind: "xcworkspace" | "xcodeproj" | "spm" };

export const workspaceDetect: HandlerFn<
  { depth?: number },
  { workspacePath: string; current: string | undefined; candidates: Candidate[] }
> = async (params, ctx) => {
  const depth = typeof params?.depth === "number" && params.depth > 0 ? Math.min(params.depth, 6) : 3;
  // The server is advertised under every folder of the window, so a detect run from any of
  // them has to list the whole window's projects, not just the active folder's. `scan`
  // reports an unreadable directory as empty, so one bad folder can't sink the others.
  const roots = getWorkspaceFolderPaths();
  const scanned = await Promise.all((roots.length > 0 ? roots : [ctx.workspacePath]).map((root) => scan(root, depth)));
  // Nested folders (both `/repo` and `/repo/ios` in the window) reach the same project twice.
  const candidates = [...new Map(scanned.flat().map((candidate) => [candidate.path, candidate])).values()];
  candidates.sort((a, b) => order(a.kind) - order(b.kind) || a.path.localeCompare(b.path));
  return {
    workspacePath: ctx.workspacePath,
    current: getCurrentXcodeWorkspacePath(ctx.workspaceState),
    candidates,
  };
};

export const workspaceUse: HandlerFn<{ path?: string }, { workspacePath: string; recent: string[] }> = async (
  params,
  ctx,
) => {
  const target = requireString(params?.path, "workspace.use", "path");
  try {
    await fs.access(target);
  } catch {
    throw new SweetpadRpcError(ERROR_CODES.WORKSPACE_NOT_FOUND, `No file or directory at ${target}`);
  }
  ctx.workspaceState.update("build.xcodeWorkspacePath", target);
  // This is one of the points a project becomes current, so the folder every "workspace root"
  // lookup resolves against moves with it.
  ctx.workspaceContext.setActiveFolder(target);

  const recent = ctx.workspaceState.get("build.xcodeWorkspacePathRecent") ?? [];
  const next = [target, ...recent.filter((p) => p !== target)].slice(0, RECENT_MAX);
  ctx.workspaceState.update("build.xcodeWorkspacePathRecent", next);

  return { workspacePath: target, recent: next };
};

export const workspaceRecent: HandlerFn<unknown, { recent: string[] }> = (_params, ctx) => {
  return { recent: ctx.workspaceState.get("build.xcodeWorkspacePathRecent") ?? [] };
};

function order(kind: Candidate["kind"]): number {
  if (kind === "xcworkspace") return 0;
  if (kind === "xcodeproj") return 1;
  return 2;
}

async function scan(root: string, depth: number): Promise<Candidate[]> {
  if (depth < 0) return [];
  let entries: { name: string; isDirectory(): boolean; isFile(): boolean }[];
  try {
    entries = await fs.readdir(root, { withFileTypes: true });
  } catch {
    return [];
  }
  // Direct hits + nested-directory descents are independent — fan them out
  // and let Promise.all flatten the result.
  const tasks: Promise<Candidate[]>[] = [];
  const direct: Candidate[] = [];
  for (const e of entries) {
    if (e.name.startsWith(".")) continue;
    if (SKIP_DIRS.has(e.name)) continue;
    const child = path.join(root, e.name);
    if (e.isFile() && e.name === "Package.swift") {
      direct.push({ path: child, kind: "spm" });
      continue;
    }
    if (!e.isDirectory()) continue;
    if (e.name.endsWith(".xcworkspace")) {
      direct.push({ path: child, kind: "xcworkspace" });
      continue;
    }
    if (e.name.endsWith(".xcodeproj")) {
      // Inner xcworkspace is structural — stop here so we don't list it twice.
      direct.push({ path: child, kind: "xcodeproj" });
      continue;
    }
    tasks.push(scan(child, depth - 1));
  }
  const nested = (await Promise.all(tasks)).flat();
  return [...direct, ...nested];
}
