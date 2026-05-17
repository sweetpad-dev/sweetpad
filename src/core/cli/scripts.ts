import { promises as fs } from "node:fs";
import path from "node:path";

import { cache } from "../cache";
import type { ConfigProvider } from "../config/types";
import type { DestinationPlatform } from "../destination/constants";
import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { readJsonFile } from "../files";
import { prepareEnvVars, uniqueFilter } from "../helpers";
import type { Logger } from "../logger/types";
import { assertUnreachable } from "../types";
import { XcodeWorkspace } from "../xcode/workspace";

export type XcodeCliDeps = {
  /** Working directory for xcodebuild / swift / xcode-build-server invocations. */
  cwd: string;
  config: ConfigProvider;
  logger: Logger;
};

export type SimulatorOutput = {
  dataPath: string;
  dataPathSize: number;
  logPath: string;
  udid: string;
  isAvailable: boolean;
  deviceTypeIdentifier: string;
  state: string;
  name: string;
};

type SimulatorsOutput = {
  devices: { [key: string]: SimulatorOutput[] };
};

interface XcodebuildListProjectOutput {
  type: "project";
  project: {
    configurations: string[];
    name: string;
    schemes: string[];
    targets: string[];
  };
}

interface XcodebuildListWorkspaceOutput {
  type: "workspace";
  workspace: {
    name: string;
    schemes: string[];
  };
}

type XcodebuildListOutput = XcodebuildListProjectOutput | XcodebuildListWorkspaceOutput;

export type XcodeScheme = {
  name: string;
};

export type XcodeConfiguration = {
  name: string;
};

export type WorkspaceType = "spm" | "xcode";

export function detectWorkspaceType(xcworkspace: string): WorkspaceType {
  return xcworkspace.endsWith("Package.swift") ? "spm" : "xcode";
}

export function getSwiftPMDirectory(xcworkspace: string): string {
  return path.dirname(xcworkspace);
}

export function parseCliJsonOutput<T>(output: string, logger: Logger): T {
  try {
    return JSON.parse(output) as T;
  } catch (error1) {
    // Parsing might fail if there are some warnings printed before or after the JSON output
    logger.debug("Output contains invalid JSON, attempting to extract JSON part", {
      output: output,
      error: error1,
    });

    try {
      const startObject = output.indexOf("{");
      const endObject = output.lastIndexOf("}");
      const startArray = output.indexOf("[");
      const endArray = output.lastIndexOf("]");
      const isObjectFound = startObject !== -1 && endObject !== -1;
      const isArrayFound = startArray !== -1 && endArray !== -1;

      if (isObjectFound && (!isArrayFound || startObject < startArray)) {
        const jsonString = output.slice(startObject, endObject + 1);
        return JSON.parse(jsonString) as T;
      }

      if (isArrayFound && (!isObjectFound || startArray < startObject)) {
        const jsonString = output.slice(startArray, endArray + 1);
        return JSON.parse(jsonString) as T;
      }
    } catch (error2) {
      logger.debug("Failed to extract JSON part from output", {
        output: output,
        error: error2,
      });
    }
    throw new ExtensionError("No valid JSON found in CLI output", {
      context: {
        output: output,
        error1: error1,
      },
    });
  }
}

export async function getSimulators(deps: XcodeCliDeps): Promise<SimulatorsOutput> {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
    cwd: deps.cwd,
    logger: deps.logger,
  });
  return parseCliJsonOutput<SimulatorsOutput>(simulatorsRaw, deps.logger);
}

export type BuildSettingsOutput = BuildSettingOutput[];

type BuildSettingOutput = {
  action: string;
  target: string;
  buildSettings: {
    [key: string]: string;
  };
};

export class XcodeBuildSettings {
  private settings: { [key: string]: string };
  public target: string;

  constructor(options: { settings: { [key: string]: string }; target: string }) {
    this.settings = options.settings;
    this.target = options.target;
  }

  private get targetBuildDir() {
    return this.settings.TARGET_BUILD_DIR;
  }

  /**
   * Path to the executable file (inside the .app bundle) to be used for running macOS apps
   */
  get executablePath() {
    return path.join(this.targetBuildDir, this.settings.EXECUTABLE_PATH);
  }

  /**
   * Path to the .app bundle to be used for installation on iOS simulator or device
   */
  get appPath() {
    return path.join(this.targetBuildDir, this.appName);
  }

  get appName() {
    if (this.settings.WRAPPER_NAME) {
      return this.settings.WRAPPER_NAME;
    }
    if (this.settings.FULL_PRODUCT_NAME) {
      return this.settings.FULL_PRODUCT_NAME;
    }
    if (this.settings.PRODUCT_NAME) {
      return `${this.settings.PRODUCT_NAME}.app`;
    }
    return `${this.targetName}.app`;
  }

  get executableName() {
    // On iOS this is CFBundleExecutable — the string that appears as the process name in
    // os_log / syslog output. Usually matches PRODUCT_NAME but can diverge (e.g. spaces stripped).
    if (this.settings.EXECUTABLE_NAME) {
      return this.settings.EXECUTABLE_NAME;
    }
    if (this.settings.PRODUCT_NAME) {
      return this.settings.PRODUCT_NAME;
    }
    return this.targetName;
  }

  private get targetName() {
    return this.settings.TARGET_NAME;
  }

  get bundleIdentifier() {
    return this.settings.PRODUCT_BUNDLE_IDENTIFIER;
  }

  get enableDebugDylib(): boolean {
    // Xcode 15+ Debug Dylib Support: when YES, app code is loaded from
    // <EXECUTABLE>.debug.dylib instead of the main binary.
    return this.settings.ENABLE_DEBUG_DYLIB === "YES";
  }

  get supportedPlatforms(): DestinationPlatform[] | undefined {
    const platformsRaw = this.settings.SUPPORTED_PLATFORMS;
    if (!platformsRaw) {
      return undefined;
    }
    return platformsRaw.split(" ").map((platform) => {
      return platform as DestinationPlatform;
    });
  }
}

function prepareDerivedDataPath(deps: XcodeCliDeps): string | null {
  const configPath = deps.config.get("build.derivedDataPath");
  if (!configPath) return null;
  if (path.isAbsolute(configPath)) return configPath;
  return path.join(deps.cwd, configPath);
}

/**
 * Extract build settings for the given scheme and configuration
 *
 * Pay attention that this function can return an empty array, if the build settings are not available.
 * Also it can return several build settings, if there are several targets assigned to the scheme.
 */
async function getBuildSettingsList(
  deps: XcodeCliDeps,
  options: {
    scheme: string;
    configuration: string;
    sdk: string | undefined;
    xcworkspace: string;
  },
): Promise<XcodeBuildSettings[]> {
  const derivedDataPath = prepareDerivedDataPath(deps);

  const args = [
    "-showBuildSettings",
    "-scheme",
    options.scheme,
    "-configuration",
    options.configuration,
    ...(derivedDataPath ? ["-derivedDataPath", derivedDataPath] : []),
    "-json",
  ];

  if (options.sdk !== undefined) {
    args.push("-sdk", options.sdk);
  }

  const workspaceType = detectWorkspaceType(options.xcworkspace);
  let cwd: string;

  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
  } else if (workspaceType === "xcode") {
    args.push("-workspace", options.xcworkspace);
    cwd = deps.cwd;
  } else {
    assertUnreachable(workspaceType);
  }

  const stdout = await exec({
    command: getXcodeBuildCommand(deps),
    args: args,
    cwd: cwd,
    logger: deps.logger,
  });

  // Parse the output - first few lines can be invalid json, so we need to skip them
  const lines = stdout.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line) {
      deps.logger.warn("Empty line in build settings output", {
        stdout: stdout,
        index: i,
      });
      continue;
    }

    if (line.startsWith("{") || line.startsWith("[")) {
      const data = lines.slice(i).join("\n");
      const output = parseCliJsonOutput<BuildSettingsOutput>(data, deps.logger);
      if (output.length === 0) {
        return [];
      }
      return output.map((entry) => {
        return new XcodeBuildSettings({
          settings: entry.buildSettings,
          target: entry.target,
        });
      });
    }
  }
  return [];
}

/**
 * Extract build settings for the given scheme and configuration to suggest the destination
 * for the user to select
 */
export async function getBuildSettingsToAskDestination(
  deps: XcodeCliDeps,
  options: {
    scheme: string;
    configuration: string;
    sdk: string | undefined;
    xcworkspace: string;
  },
): Promise<XcodeBuildSettings | null> {
  try {
    const settings = await getBuildSettingsList(deps, options);

    if (settings.length === 0) {
      return null;
    }
    if (settings.length === 1) {
      return settings[0];
    }
    // To ask destination, we might omit the build settings, since they are needed only to
    // to suggest the destination and nothing bad will happen if we don't have them here
    return null;
  } catch (e) {
    deps.logger.error("Error getting build settings", {
      error: e,
    });
    return null;
  }
}

/**
 * Get build settings to launch the app
 *
 * Each scheme might have several targets. That's why -showBuildSettings might return different
 * build settings for each target. In the ideal scenario where there is a single target and settings
 * and I just can use first settings object. But there is a cases when there are several targets
 * for the scheme and I need to find which target is set to launch in .xcscheme XML file.
 */
export async function getBuildSettingsToLaunch(
  deps: XcodeCliDeps,
  options: {
    scheme: string;
    configuration: string;
    sdk: string | undefined;
    xcworkspace: string;
  },
): Promise<XcodeBuildSettings> {
  const settings = await getBuildSettingsList(deps, options);

  if (settings.length === 0) {
    throw new ExtensionError("Empty build settings");
  }

  if (settings.length === 1) {
    return settings[0];
  }

  // > 1 target in the scheme — look up which one LaunchAction points at.
  const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace, { logger: deps.logger });
  const scheme = await workspace.getScheme({ name: options.scheme });
  if (!scheme) {
    return settings[0];
  }

  const target = await scheme.getTargetToLaunch();
  const targetSettings = settings.find((s) => s.target === target);
  if (targetSettings) {
    return targetSettings;
  }

  return settings[0];
}

/**
 * Find if xcbeautify is installed
 */
export async function getIsXcbeautifyInstalled(deps: XcodeCliDeps): Promise<boolean> {
  try {
    await exec({
      command: "which",
      args: ["xcbeautify"],
      cwd: deps.cwd,
      logger: deps.logger,
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * Get the xcode-build-server command path from config or default
 */
function getXcodeBuildServerCommand(deps: XcodeCliDeps): string {
  const customPath = deps.config.get("xcodebuildserver.path");
  return customPath || "xcode-build-server";
}

/**
 * Get the xcodebuild command from config or default
 */
export function getXcodeBuildCommand(deps: XcodeCliDeps): string {
  const customCommand = deps.config.get("build.xcodebuildCommand");
  return customCommand || "xcodebuild";
}

export function getSwiftCommand(deps: XcodeCliDeps): string {
  const customCommand = deps.config.get("build.swiftCommand");
  return customCommand || "swift";
}

/**
 * Find if xcode-build-server is installed
 */
export async function getIsXcodeBuildServerInstalled(deps: XcodeCliDeps): Promise<boolean> {
  const command = getXcodeBuildServerCommand(deps);

  try {
    await exec({
      command: "which",
      args: [command],
      cwd: deps.cwd,
      logger: deps.logger,
    });
    return true;
  } catch (e) {
    return false;
  }
}

export const getBasicProjectInfo = cache(
  async (deps: XcodeCliDeps, options: { xcworkspace: string | undefined }): Promise<XcodebuildListOutput> => {
    const workspaceType = detectWorkspaceType(options.xcworkspace ?? "");
    if (workspaceType === "spm") {
      // For SPM projects, schemes are discovered via `swift package dump-package`.
      // Return a stub here; getSchemes() handles SPM separately.
      return {
        type: "workspace",
        workspace: {
          name: path.basename(path.dirname(options.xcworkspace ?? "")),
          schemes: [],
        },
      } as XcodebuildListWorkspaceOutput;
    }

    if (workspaceType === "xcode") {
      const stdout = await exec({
        command: getXcodeBuildCommand(deps),
        args: ["-list", "-json", ...(options?.xcworkspace ? ["-workspace", options?.xcworkspace] : [])],
        cwd: deps.cwd,
        logger: deps.logger,
      });
      const parsed = parseCliJsonOutput<any>(stdout, deps.logger);
      if (parsed.project) {
        return {
          type: "project",
          ...parsed,
        } as XcodebuildListProjectOutput;
      }
      return {
        type: "workspace",
        ...parsed,
      } as XcodebuildListWorkspaceOutput;
    }
    assertUnreachable(workspaceType);
  },
  // Key only on the workspace path — `deps` holds a Logger/ConfigProvider whose
  // adapters may carry non-serializable state (timer refs, event emitters).
  (_deps, options) => `xcworkspace:${options.xcworkspace ?? ""}`,
);

export async function getSchemes(
  deps: XcodeCliDeps,
  options: { xcworkspace: string | undefined },
): Promise<XcodeScheme[]> {
  deps.logger.log("Getting schemes", { xcworkspace: options?.xcworkspace ?? "undefined" });

  const workspaceType = detectWorkspaceType(options.xcworkspace ?? "");
  if (workspaceType === "spm") {
    try {
      const packageDir = getSwiftPMDirectory(options.xcworkspace ?? "");
      const stdout = await exec({
        command: getSwiftCommand(deps),
        args: ["package", "dump-package"],
        cwd: packageDir,
        logger: deps.logger,
      });
      const packageInfo = JSON.parse(stdout);

      const schemeNames = new Set<string>();

      if (packageInfo.products) {
        for (const product of packageInfo.products) {
          if (product.type?.executable || product.type?.library) {
            schemeNames.add(product.name);
          }
        }
      }

      if (packageInfo.targets) {
        for (const target of packageInfo.targets) {
          if (target.type === "executable" && !schemeNames.has(target.name)) {
            schemeNames.add(target.name);
          }
        }
      }

      if (schemeNames.size === 0 && packageInfo.name) {
        schemeNames.add(packageInfo.name);
      }

      return Array.from(schemeNames).map((name) => ({ name }));
    } catch (error) {
      deps.logger.error("Failed to get SPM package info, falling back to xcodebuild", {
        error,
        packagePath: options.xcworkspace,
      });
      // continue on to next approach
    }
  }

  // 2. Use custom workspace parser if enabled
  const useWorkspaceParser = deps.config.get("system.customXcodeWorkspaceParser") ?? false;
  if (options.xcworkspace && useWorkspaceParser) {
    try {
      const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace, { logger: deps.logger });
      const projects = await workspace.getProjects();

      const schemes = await Promise.all(projects.map((project) => project.getSchemes()));

      const uniqueSchemes = schemes
        .flat()
        .map((scheme) => ({ name: scheme.name }))
        .filter(uniqueFilter);

      return uniqueSchemes;
    } catch (error) {
      deps.logger.error("Error getting schemes with workspace parser, falling back to xcodebuild", {
        error,
        xcworkspace: options.xcworkspace,
      });
    }
  }

  // 3. Fallback to xcodebuild -list (via getBasicProjectInfo)
  const output = await getBasicProjectInfo(deps, {
    xcworkspace: options?.xcworkspace,
  });
  if (output.type === "project") {
    return output.project.schemes.map((scheme) => ({ name: scheme }));
  }
  if (output.type === "workspace") {
    return output.workspace.schemes.map((scheme) => ({ name: scheme }));
  }
  assertUnreachable(output);
}

export async function getTargets(deps: XcodeCliDeps, options: { xcworkspace: string }): Promise<string[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    try {
      const packageDir = getSwiftPMDirectory(options.xcworkspace ?? "");
      const stdout = await exec({
        command: getSwiftCommand(deps),
        args: ["package", "dump-package"],
        cwd: packageDir,
        logger: deps.logger,
      });
      const packageInfo = JSON.parse(stdout);

      const targets: string[] = [];

      if (packageInfo.targets) {
        for (const target of packageInfo.targets) {
          targets.push(target.name);
        }
      }

      return targets;
    } catch (error) {
      deps.logger.error("Failed to get SPM targets", {
        error: error,
        packagePath: options.xcworkspace,
      });
      return [];
    }
  }

  if (workspaceType === "xcode") {
    const output = await getBasicProjectInfo(deps, {
      xcworkspace: options.xcworkspace,
    });
    if (output.type === "project") {
      return output.project.targets;
    }
    if (output.type === "workspace") {
      const xcworkspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace, { logger: deps.logger });
      const projects = await xcworkspace.getProjects();
      return projects.flatMap((project) => project.getTargets());
    }
    assertUnreachable(output);
  }
  assertUnreachable(workspaceType);
}

export async function getBuildConfigurations(
  deps: XcodeCliDeps,
  options: { xcworkspace: string },
): Promise<XcodeConfiguration[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    return [{ name: "Debug" }, { name: "Release" }];
  }

  if (workspaceType === "xcode") {
    deps.logger.log("Getting build configurations", { xcworkspace: options?.xcworkspace });

    const useWorkspaceParser = deps.config.get("system.customXcodeWorkspaceParser") ?? false;

    if (useWorkspaceParser) {
      try {
        const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace, { logger: deps.logger });
        const projects = await workspace.getProjects();

        deps.logger.debug("Projects", {
          paths: projects.map((project) => project.projectPath),
        });

        const configurations = projects
          .flatMap((project) => {
            deps.logger.debug("Project configurations", {
              configurations: project.getConfigurations(),
            });
            return project.getConfigurations();
          })
          .filter(uniqueFilter)
          .map((configuration) => {
            return {
              name: configuration,
            };
          });

        return configurations;
      } catch (error) {
        deps.logger.error("Error getting build configurations with workspace parser, falling back to xcodebuild", {
          error,
          xcworkspace: options.xcworkspace,
        });
      }
    }

    // Original implementation using xcodebuild -list
    const output = await getBasicProjectInfo(deps, {
      xcworkspace: options.xcworkspace,
    });
    if (output.type === "project") {
      return output.project.configurations.map((configuration) => {
        return {
          name: configuration,
        };
      });
    }
    if (output.type === "workspace") {
      const xcworkspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace, { logger: deps.logger });
      const projects = await xcworkspace.getProjects();

      deps.logger.debug("Projects", {
        paths: projects.map((project) => project.projectPath),
      });

      return projects
        .flatMap((project) => {
          deps.logger.debug("Project configurations", {
            configurations: project.getConfigurations(),
          });
          return project.getConfigurations();
        })
        .filter(uniqueFilter)
        .map((configuration) => {
          return {
            name: configuration,
          };
        });
    }
    return [];
  }
  assertUnreachable(workspaceType);
}

/**
 * Generate xcode-build-server config.
 *
 * `sweetpad.xcodebuildserver.serverEnv` is injected into the generated
 * `buildServer.json` by prefixing `argv` with `/usr/bin/env KEY=VAL ...` so the
 * long-running build server (later spawned by sourcekit-lsp) inherits them.
 * The vars aren't passed to this short-lived `config` call itself — XBS's
 * config phase only honors them in trivial ways, and the docs flag `argv` as
 * the only stable way to pass env via BSP.
 */
export async function generateBuildServerConfig(
  deps: XcodeCliDeps,
  options: { xcworkspace: string; scheme: string },
): Promise<void> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  const command = getXcodeBuildServerCommand(deps);
  let cwd: string;
  let args: string[];

  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
    args = ["config", "-scheme", options.scheme];
  } else if (workspaceType === "xcode") {
    cwd = deps.cwd;
    args = ["config", "-workspace", options.xcworkspace, "-scheme", options.scheme];
  } else {
    assertUnreachable(workspaceType);
  }
  await exec({
    command: command,
    args: args,
    cwd: cwd,
    logger: deps.logger,
  });

  const env = deps.config.get("xcodebuildserver.serverEnv") ?? {};
  await injectEnvIntoBuildServerConfig(path.join(cwd, "buildServer.json"), env, deps.logger);
}

/**
 * Bridge `sweetpad.xcodebuildserver.serverEnv` → the long-running XBS process.
 *
 * sourcekit-lsp reads buildServer.json on project open and execs whatever's in
 * `argv` to be its build server. BSP defines no `env` field — `argv` is the
 * only knob. We use the standard `/usr/bin/env` trick to set vars at exec
 * time:
 *
 *   before:  "argv": ["/opt/homebrew/bin/xcode-build-server"]
 *   after:   "argv": ["/usr/bin/env",
 *                     "XBS_LOGPATH=/tmp/sweetpad-xbs.log",
 *                     "/opt/homebrew/bin/xcode-build-server"]
 *
 * Bails out (no-op) when there's nothing to do: empty env, missing file, or
 * already-wrapped argv.
 */
async function injectEnvIntoBuildServerConfig(
  buildServerJsonPath: string,
  env: { [key: string]: string | null },
  logger: Logger,
): Promise<void> {
  const prepared = prepareEnvVars(env);
  const entries = Object.entries(prepared).filter(([, v]) => v !== undefined) as [string, string][];
  if (entries.length === 0) return;

  let config: { argv?: string[]; [key: string]: unknown };
  try {
    config = await readJsonFile<{ argv?: string[]; [key: string]: unknown }>(buildServerJsonPath);
  } catch (e) {
    logger.debug("buildServer.json not found after generation, skipping env injection", {
      path: buildServerJsonPath,
    });
    return;
  }

  if (!Array.isArray(config.argv) || config.argv.length === 0) return;
  if (config.argv[0] === "/usr/bin/env") return;

  const envArgs = entries.map(([k, v]) => `${k}=${v}`);
  config.argv = ["/usr/bin/env", ...envArgs, ...config.argv];

  await fs.writeFile(buildServerJsonPath, `${JSON.stringify(config, null, 2)}\n`, "utf8");
}

export type XcodeBuildServerConfig = {
  scheme: string;
  workspace: string;
  build_root: string;
};

/**
 * Read xcode-build-server config from <cwd>/buildServer.json.
 */
export async function readXcodeBuildServerConfig(deps: XcodeCliDeps): Promise<XcodeBuildServerConfig> {
  const buildServerJsonPath = path.join(deps.cwd, "buildServer.json");
  return await readJsonFile<XcodeBuildServerConfig>(buildServerJsonPath);
}

export async function getIsXcodeGenInstalled(deps: XcodeCliDeps): Promise<boolean> {
  try {
    await exec({
      command: "which",
      args: ["xcodegen"],
      cwd: deps.cwd,
      logger: deps.logger,
    });
    return true;
  } catch (e) {
    return false;
  }
}

export async function generateXcodeGen(deps: XcodeCliDeps): Promise<void> {
  await exec({
    command: "xcodegen",
    args: ["generate"],
    cwd: deps.cwd,
    logger: deps.logger,
  });
}

export async function getIsTuistInstalled(deps: XcodeCliDeps): Promise<boolean> {
  try {
    await exec({
      command: "which",
      args: ["tuist"],
      cwd: deps.cwd,
      logger: deps.logger,
    });
    return true;
  } catch (e) {
    return false;
  }
}

export async function tuistGenerate(deps: XcodeCliDeps): Promise<string> {
  const env = deps.config.get("tuist.generate.env");
  return await exec({
    command: "tuist",
    args: ["generate", "--no-open"],
    env: env,
    cwd: deps.cwd,
    logger: deps.logger,
  });
}

export async function tuistClean(deps: XcodeCliDeps): Promise<void> {
  await exec({
    command: "tuist",
    args: ["clean"],
    cwd: deps.cwd,
    logger: deps.logger,
  });
}

export async function tuistInstall(deps: XcodeCliDeps): Promise<void> {
  await exec({
    command: "tuist",
    args: ["install"],
    cwd: deps.cwd,
    logger: deps.logger,
  });
}

export async function tuistEdit(deps: XcodeCliDeps): Promise<void> {
  await exec({
    command: "tuist",
    args: ["edit"],
    cwd: deps.cwd,
    logger: deps.logger,
  });
}

export async function tuistTest(deps: XcodeCliDeps): Promise<void> {
  await exec({
    command: "tuist",
    args: ["test"],
    cwd: deps.cwd,
    logger: deps.logger,
  });
}

/**
 * Get the Xcode version installed on the system using xcodebuild
 *
 * This version works properly with Xcodes.app, so it's the recommended one
 */
export async function getXcodeVersionInstalled(deps: XcodeCliDeps): Promise<{ major: number }> {
  //~ xcodebuild -version
  // Xcode 16.0
  // Build version 16A242d
  const stdout = await exec({
    command: "xcrun",
    args: ["xcodebuild", "-version"],
    cwd: deps.cwd,
    logger: deps.logger,
  });

  const versionMatch = stdout.match(/Xcode (\d+)\./);
  if (!versionMatch) {
    throw new ExtensionError("Error parsing xcode version", {
      context: {
        stdout: stdout,
      },
    });
  }
  const major = Number.parseInt(versionMatch[1]);
  return {
    major,
  };
}
