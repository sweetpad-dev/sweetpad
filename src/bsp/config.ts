import * as path from "node:path";

import type { BuildManager } from "../build/manager";
import { getCurrentXcodeWorkspacePath, prepareDerivedDataPath } from "../build/utils";
import { getDeveloperDir } from "../common/cli/scripts";
import { getWorkspaceConfig } from "../common/config";
import type { WorkspaceStateService } from "../common/workspace-state";
import { getBspLogPath, getBspSocketPath } from "./paths";

/**
 * Everything the BSP server needs, written by the extension to the per-project
 * `bsp.json` under the XDG state home (the server reads it at startup — via the
 * `--config` path in `buildServer.json` — and watches it for changes), so
 * `buildServer.json` stays a minimal launch stub. The config lives outside the
 * project tree, so all paths are written absolute.
 */
export type BspResolvedConfig = {
  workspacePath: string;
  /** The `.xcodeproj` the server parses (Xcode addresses a plain project through its embedded `project.xcworkspace`). */
  projectPath: string;
  /** Xcode developer dir for `DEVELOPER_DIR` / toolchain lookup, or null if undetectable. */
  developerDir: string | null;
  scheme: string | null;
  configuration: string;
  derivedDataPath: string | null;
  /** Debug log file. Defaults to the per-project state dir (out of the project tree); overridable via `sweetpad.buildServer.logPath`. */
  logPath: string;
  /** Unix socket the BSP server binds for telemetry; the extension connects to it for live logs/status. */
  socket: string;
};

/**
 * Resolve the BSP config from the current selection, or `null` when no Xcode
 * workspace is detected. This is what the extension writes to the per-project
 * `bsp.json` for the BSP server to read.
 */
export async function buildBspResolvedConfig(deps: {
  workspaceState: WorkspaceStateService;
  workspacePath: string;
  buildManager: BuildManager;
}): Promise<BspResolvedConfig | null> {
  const xcworkspace = getCurrentXcodeWorkspacePath(deps.workspaceState);
  if (!xcworkspace) {
    return null;
  }
  let projectPath = xcworkspace;
  if (path.basename(projectPath) === "project.xcworkspace") {
    projectPath = path.dirname(projectPath);
  }
  if (!path.isAbsolute(projectPath)) {
    projectPath = path.join(deps.workspacePath, projectPath);
  }
  const derivedDataPath = prepareDerivedDataPath();
  return {
    workspacePath: deps.workspacePath,
    projectPath: projectPath,
    developerDir: (await getDeveloperDir()) ?? null,
    scheme: deps.buildManager.getDefaultSchemeForBuild() ?? null,
    configuration: deps.buildManager.getDefaultConfigurationForBuild() ?? "Debug",
    derivedDataPath: derivedDataPath ?? null,
    logPath: resolveLogPath(deps.workspacePath),
    socket: getBspSocketPath(deps.workspacePath),
  };
}

/**
 * The BSP log path. Defaults to the per-project state dir (`getBspLogPath`) so
 * logs are always captured without cluttering the project tree;
 * `sweetpad.buildServer.logPath` overrides it (with `${workspaceFolder}`/relative
 * resolved absolute against the workspace folder).
 */
function resolveLogPath(workspacePath: string): string {
  const raw = getWorkspaceConfig("buildServer.logPath");
  if (raw) {
    const expanded = raw.split("${workspaceFolder}").join(workspacePath);
    return path.isAbsolute(expanded) ? expanded : path.join(workspacePath, expanded);
  }
  return getBspLogPath(workspacePath);
}
