import * as path from "node:path";

import { getCurrentXcodeWorkspacePath, prepareDerivedDataPath } from "../../build/utils";
import { getDeveloperDir } from "../../common/cli/scripts";
import { getWorkspaceConfig } from "../../common/config";
import { BSP_LOG_LEVELS, type BspLogLevel } from "../bsp-bridge";
import { SweetpadRpcError } from "../rpc";
import { ERROR_CODES } from "../types";
import type { HandlerFn } from "./context";

/**
 * Everything the BSP server needs to resolve compiler arguments, discovered at
 * runtime instead of baked into `buildServer.json`. The server pulls this over
 * the control socket on connect, so the on-disk config stays minimal.
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
  /** Debug log file (`sweetpad.buildServer.logPath`), resolved absolute, or null. */
  logPath: string | null;
};

export const bspResolveConfig: HandlerFn<unknown, BspResolvedConfig> = async (_params, ctx) => {
  const xcworkspace = getCurrentXcodeWorkspacePath(ctx.workspace);
  if (!xcworkspace) {
    throw new SweetpadRpcError(ERROR_CODES.NO_WORKSPACE, "No Xcode workspace detected for this folder.");
  }
  let projectPath = xcworkspace;
  if (path.basename(projectPath) === "project.xcworkspace") {
    projectPath = path.dirname(projectPath);
  }
  if (!path.isAbsolute(projectPath)) {
    projectPath = path.join(ctx.workspacePath, projectPath);
  }
  return {
    workspacePath: ctx.workspacePath,
    projectPath,
    developerDir: (await getDeveloperDir()) ?? null,
    scheme: ctx.buildManager.getDefaultSchemeForBuild() ?? null,
    configuration: ctx.buildManager.getDefaultConfigurationForBuild() ?? "Debug",
    derivedDataPath: prepareDerivedDataPath(),
    logPath: resolveLogPath(ctx.workspacePath),
  };
};

/** The configured BSP log path, with `${workspaceFolder}`/relative resolved absolute. */
function resolveLogPath(workspacePath: string): string | null {
  const raw = getWorkspaceConfig("buildServer.logPath");
  if (!raw) return null;
  const expanded = raw.split("${workspaceFolder}").join(workspacePath);
  return path.isAbsolute(expanded) ? expanded : path.join(workspacePath, expanded);
}

/**
 * Set the verbosity of the BSP server's `bsp/log` stream (off | error | info |
 * debug). Pushed live to every connected BSP server via the control channel;
 * the `SWEETPAD_BSP_LOG` file is unaffected.
 */
export const bspSetLogLevel: HandlerFn<{ level?: string }, { level: BspLogLevel }> = (params, ctx) => {
  const level = params?.level;
  if (!level || !BSP_LOG_LEVELS.includes(level as BspLogLevel)) {
    throw new SweetpadRpcError(ERROR_CODES.INVALID_PARAMS, `level must be one of: ${BSP_LOG_LEVELS.join(", ")}`);
  }
  ctx.bspBridge.setLogLevel(level as BspLogLevel);
  return { level: level as BspLogLevel };
};
