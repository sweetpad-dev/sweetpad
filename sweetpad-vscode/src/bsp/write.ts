import { promises as fs } from "node:fs";
import * as path from "node:path";

import { ensureDir, getProjectStateDir } from "../cli-server/paths";
import { registerBspConfig } from "../cli-server/registry";
import { getWorkspaceConfig } from "../common/config";
import { isFileExists, readJsonFile } from "../common/files";
import { getBspConfigFile, getBspLogPath, getBspSocketPath } from "./paths";

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
  /**
   * The platform of the destination builds go to (`iphonesimulator`, `watchos`, …), or null when
   * none is selected. A target that builds for this platform is analyzed for it, and a file that
   * several targets compile is read as the target the selected scheme builds, then as the one for
   * this platform.
   */
  destinationPlatform: string | null;
  /**
   * The DerivedData the extension's builds write (`prepareDerivedDataPath`), which follows a
   * `-derivedDataPath` in `buildArgs`. The server reads it once at startup.
   */
  derivedDataPath: string | null;
  /** Debug log file. Defaults to the per-project state dir (out of the project tree); overridable via `sweetpad.buildServer.logPath`. */
  logPath: string;
  /** Unix socket the BSP server binds for telemetry; the extension connects to it for live logs/status. */
  socket: string;
  /**
   * `sweetpad.build.args`, which the extension's builds add to the xcodebuild command line. The
   * server applies its `KEY=VALUE` settings and `-xcconfig` (relative to `workspacePath`, where
   * those builds run) so the index resolves each target the way they build it.
   */
  buildArgs: string[];
};

/**
 * Assemble the config from values the caller already holds, for the
 * `buildServer.json` writer, which cannot reach the extension's services.
 */
export function assembleBspConfig(parts: {
  workspacePath: string;
  xcworkspace: string;
  developerDir: string | null;
  scheme: string | null;
  configuration: string;
  destinationPlatform: string | null;
  derivedDataPath: string | null;
  buildArgs: string[];
}): BspResolvedConfig {
  return {
    workspacePath: parts.workspacePath,
    projectPath: resolveProjectPath(parts.workspacePath, parts.xcworkspace),
    developerDir: parts.developerDir,
    scheme: parts.scheme,
    configuration: parts.configuration,
    destinationPlatform: parts.destinationPlatform,
    derivedDataPath: parts.derivedDataPath,
    logPath: resolveBspLogPath(parts.workspacePath),
    socket: getBspSocketPath(parts.workspacePath),
    buildArgs: parts.buildArgs,
  };
}

function resolveProjectPath(workspacePath: string, xcworkspace: string): string {
  let projectPath = xcworkspace;
  if (path.basename(projectPath) === "project.xcworkspace") {
    projectPath = path.dirname(projectPath);
  }
  if (!path.isAbsolute(projectPath)) {
    projectPath = path.join(workspacePath, projectPath);
  }
  return projectPath;
}

/**
 * Write `bsp.json` and advertise its path in the discovery index. One function
 * because the server takes either route in — `--config` from
 * `buildServer.json`, or the index when that carries no flag — so writing one
 * without the other leaves a route pointing at nothing.
 */
export async function writeBspConfig(config: BspResolvedConfig): Promise<string> {
  const workspacePath = config.workspacePath;
  await ensureDir(getProjectStateDir(workspacePath));
  const configFile = getBspConfigFile(workspacePath);
  await fs.writeFile(configFile, `${JSON.stringify(config, null, 2)}\n`, { mode: 0o600 });
  await registerBspConfig(workspacePath, configFile);
  return configFile;
}

export async function hasBspConfig(workspacePath: string): Promise<boolean> {
  return await isFileExists(getBspConfigFile(workspacePath));
}

/** The `bsp.json` on disk for `workspacePath`, or `undefined` when there is none to read. */
export async function readBspConfig(workspacePath: string): Promise<Partial<BspResolvedConfig> | undefined> {
  try {
    return await readJsonFile<Partial<BspResolvedConfig>>(getBspConfigFile(workspacePath));
  } catch {
    return undefined;
  }
}

/**
 * The BSP log path. Defaults to the per-project state dir (`getBspLogPath`) so
 * logs are always captured without cluttering the project tree;
 * `sweetpad.buildServer.logPath` overrides it (with `${workspaceFolder}`/relative
 * resolved absolute against the workspace folder).
 */
export function resolveBspLogPath(workspacePath: string): string {
  const raw = getWorkspaceConfig("buildServer.logPath");
  if (raw) {
    const expanded = raw.split("${workspaceFolder}").join(workspacePath);
    return path.isAbsolute(expanded) ? expanded : path.join(workspacePath, expanded);
  }
  return getBspLogPath(workspacePath);
}
