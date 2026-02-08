import path from "node:path";
import { detectWorkspaceType, getSwiftPMDirectory, getWorkspacePath, prepareDerivedDataPath } from "../../build/utils";
import type { DestinationPlatform } from "../../destination/constants";
import { cache } from "../cache";
import { getWorkspaceConfig } from "../config";
import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { readJsonFile } from "../files";
import { uniqueFilter } from "../helpers";
import { commonLogger } from "../logger";
import { assertUnreachable } from "../types";
import { XcodeWorkspace } from "../xcode/workspace";

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

export function parseCliJsonOutput<T>(output: string): T {
  try {
    return JSON.parse(output) as T;
  } catch (error1) {
    // Parsing might fail if there are some warnings printed before or after the JSON output
    commonLogger.debug("Output contains invalid JSON, attempting to extract JSON part", {
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
      commonLogger.debug("Failed to extract JSON part from output", {
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

export async function getSimulators(): Promise<SimulatorsOutput> {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });
  return parseCliJsonOutput<SimulatorsOutput>(simulatorsRaw);
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

  constructor(options: {
    settings: { [key: string]: string };
    target: string;
  }) {
    this.settings = options.settings;
    this.target = options.target;
  }

  private get targetBuildDir() {
    // Example:
    // - /Users/hyzyla/Library/Developer/Xcode/DerivedData/ControlRoom-gdvrildvemgjaiameavxoegdskby/Build/Products/Debug
    return this.settings.TARGET_BUILD_DIR;
  }

  /**
   * Path to the executable file (inside the .app bundle) to be used for running macOS apps
   */
  get executablePath() {
    // Example:
    // - {targetBuildDir}/Control Room.app/Contents/MacOS/Control Room
    return path.join(this.targetBuildDir, this.settings.EXECUTABLE_PATH);
  }

  /**
   * Path to the .app bundle to be used for installation on iOS simulator or device
   */
  get appPath() {
    // Example:
    // - {targetBuildDir}/Control Room.app
    return path.join(this.targetBuildDir, this.appName);
  }

  get appName() {
    // Example:
    // - "Control Room.app"
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

  private get targetName() {
    // Example:
    // - "ControlRoom"
    return this.settings.TARGET_NAME;
  }

  get bundleIdentifier() {
    // Example:
    // - "com.hackingwithswift.ControlRoom"
    return this.settings.PRODUCT_BUNDLE_IDENTIFIER;
  }

  get supportedPlatforms(): DestinationPlatform[] | undefined {
    // ex: ["iphonesimulator", "iphoneos"]
    const platformsRaw = this.settings.SUPPORTED_PLATFORMS; // ex: "iphonesimulator iphoneos"
    if (!platformsRaw) {
      return undefined;
    }
    return platformsRaw.split(" ").map((platform) => {
      return platform as DestinationPlatform;
    });
  }
}

/**
 * Extract build settings for the given scheme and configuration
 *
 * Pay attention that this function can return an empty array, if the build settings are not available.
 * Also it can return several build settings, if there are several targets assigned to the scheme.
 */
async function getBuildSettingsList(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings[]> {
  const derivedDataPath = prepareDerivedDataPath();

  // Build common arguments
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

  // Determine workspace type and set up execution context
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  let cwd: string | undefined;

  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
  } else if (workspaceType === "xcode") {
    args.push("-workspace", options.xcworkspace);
  } else {
    assertUnreachable(workspaceType);
  }

  const stdout = await exec({
    command: getXcodeBuildCommand(),
    args: args,
    cwd: cwd,
  });

  // Parse the output - first few lines can be invalid json, so we need to skip them
  const lines = stdout.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (!line) {
      commonLogger.warn("Empty line in build settings output", {
        stdout: stdout,
        index: i,
      });
      continue;
    }

    if (line.startsWith("{") || line.startsWith("[")) {
      const data = lines.slice(i).join("\n");
      const output = parseCliJsonOutput<BuildSettingsOutput>(data);
      if (output.length === 0) {
        return [];
      }
      return output.map((output) => {
        return new XcodeBuildSettings({
          settings: output.buildSettings,
          target: output.target,
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
export async function getBuildSettingsToAskDestination(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings | null> {
  try {
    const settings = await getBuildSettingsList(options);

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
    commonLogger.error("Error getting build settings", {
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
export async function getBuildSettingsToLaunch(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings> {
  const settings = await getBuildSettingsList(options);

  // Build settings are required to run the app because we use them to locate the executable file or
  // the .app bundle. So let's just give up here if -showBuildSettings didn't return anything.
  if (settings.length === 0) {
    throw new ExtensionError("Empty build settings");
  }

  // I think this is the most common case, when there is only one target in the scheme. Higly likely that
  // this is the target to launch. Technically, scheme mightn't have any target to launch, but I believe
  // this is a rare case.
  if (settings.length === 1) {
    return settings[0];
  }

  // > 1 target in the scheme
  // Looking for such pattern in the .xcscheme XML file:
  // <Scheme>
  //    <LaunchAction>
  //      <BuildableProductRunnable>
  //        <BuildableReference BlueprintName=...>
  const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace);
  const scheme = await workspace.getScheme({ name: options.scheme });
  if (!scheme) {
    return settings[0];
  }

  const target = await scheme.getTargetToLaunch();
  const targetSettings = settings.find((settings) => settings.target === target);
  if (targetSettings) {
    return targetSettings;
  }

  // As a last resort, let's just return the first settings object (can we handle this case better?)
  return settings[0];
}

/**
 * Find if xcbeautify is installed
 */
export async function getIsXcbeautifyInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcbeautify"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * Get the xcode-build-server command path from config or default
 */
function getXcodeBuildServerCommand(): string {
  const customPath = getWorkspaceConfig("xcodebuildserver.path");
  return customPath || "xcode-build-server";
}

/**
 * Get the xcodebuild command from config or default
 */
export function getXcodeBuildCommand(): string {
  const customCommand = getWorkspaceConfig("build.xcodebuildCommand");
  return customCommand || "xcodebuild";
}

export function getSwiftCommand(): string {
  const customCommand = getWorkspaceConfig("build.swiftCommand");
  return customCommand || "swift";
}

/**
 * Find if xcode-build-server is installed
 */
export async function getIsXcodeBuildServerInstalled() {
  const command = getXcodeBuildServerCommand();

  try {
    await exec({
      command: "which",
      args: [command],
    });
    return true;
  } catch (e) {
    return false;
  }
}

export const getBasicProjectInfo = cache(
  async (options: { xcworkspace: string | undefined }): Promise<XcodebuildListOutput> => {
    const workspaceType = detectWorkspaceType(options.xcworkspace ?? "");
    if (workspaceType === "spm") {
      // For SPM projects, we create a mock workspace output since SPM doesn't have traditional schemes
      // The schemes will be handled by the getSchemes function
      return {
        type: "workspace",
        workspace: {
          name: path.basename(path.dirname(options.xcworkspace ?? "")), // todo: understand that
          schemes: [], // Will be populated by getSchemes function
        },
      } as XcodebuildListWorkspaceOutput;
    }

    if (workspaceType === "xcode") {
      const stdout = await exec({
        command: getXcodeBuildCommand(),
        args: ["-list", "-json", ...(options?.xcworkspace ? ["-workspace", options?.xcworkspace] : [])],
      });
      const parsed = parseCliJsonOutput<any>(stdout);
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
);

export async function getSchemes(options: { xcworkspace: string | undefined }): Promise<XcodeScheme[]> {
  commonLogger.log("Getting schemes", { xcworkspace: options?.xcworkspace ?? "undefined" });

  const workspaceType = detectWorkspaceType(options.xcworkspace ?? "");
  if (workspaceType === "spm") {
    try {
      const packageDir = getSwiftPMDirectory(options.xcworkspace ?? "");
      const stdout = await exec({
        command: getSwiftCommand(),
        args: ["package", "dump-package"],
        cwd: packageDir,
      });
      const packageInfo = JSON.parse(stdout);

      const schemeNames = new Set<string>();

      // Add all library/executable products
      if (packageInfo.products) {
        for (const product of packageInfo.products) {
          if (product.type?.executable || product.type?.library) {
            schemeNames.add(product.name);
          }
        }
      }

      // Add standalone executable targets not already covered
      if (packageInfo.targets) {
        for (const target of packageInfo.targets) {
          if (target.type === "executable" && !schemeNames.has(target.name)) {
            schemeNames.add(target.name);
          }
        }
      }

      // Fallback to the package name if nothing else found
      if (schemeNames.size === 0 && packageInfo.name) {
        schemeNames.add(packageInfo.name);
      }

      return Array.from(schemeNames).map((name) => ({ name }));
    } catch (error) {
      commonLogger.error("Failed to get SPM package info, falling back to xcodebuild", {
        error,
        packagePath: options.xcworkspace,
      });
      // continue on to next approach
    }
  }

  // 2. Use custom workspace parser if enabled
  const useWorkspaceParser = getWorkspaceConfig("system.customXcodeWorkspaceParser") ?? false;
  if (options.xcworkspace && useWorkspaceParser) {
    try {
      const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace);
      const projects = await workspace.getProjects();

      // Get schemes from all projects in the workspace
      const schemes = await Promise.all(projects.map((project) => project.getSchemes()));

      // Flatten and deduplicate schemes, only keep the names
      const uniqueSchemes = schemes
        .flat()
        .map((scheme) => ({ name: scheme.name }))
        .filter(uniqueFilter);

      return uniqueSchemes;
    } catch (error) {
      commonLogger.error("Error getting schemes with workspace parser, falling back to xcodebuild", {
        error,
        xcworkspace: options.xcworkspace,
      });
      // continue on to xcodebuild fallback
    }
  }

  // 3. Fallback to xcodebuild -list (via getBasicProjectInfo)
  const output = await getBasicProjectInfo({
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

export async function getTargets(options: { xcworkspace: string }): Promise<string[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    try {
      const packageDir = getSwiftPMDirectory(options.xcworkspace ?? "");
      const stdout = await exec({
        command: getSwiftCommand(),
        args: ["package", "dump-package"],
        cwd: packageDir,
      });
      const packageInfo = JSON.parse(stdout);

      const targets: string[] = [];

      // Add all targets
      if (packageInfo.targets) {
        for (const target of packageInfo.targets) {
          targets.push(target.name);
        }
      }

      return targets;
    } catch (error) {
      commonLogger.error("Failed to get SPM targets", {
        error: error,
        packagePath: options.xcworkspace,
      });
      return [];
    }
  }

  if (workspaceType === "xcode") {
    const output = await getBasicProjectInfo({
      xcworkspace: options.xcworkspace,
    });
    if (output.type === "project") {
      return output.project.targets;
    }
    if (output.type === "workspace") {
      const xcworkspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace);
      const projects = await xcworkspace.getProjects();
      return projects.flatMap((project) => project.getTargets());
    }
    assertUnreachable(output);
  }
  assertUnreachable(workspaceType);
}

export async function getBuildConfigurations(options: { xcworkspace: string }): Promise<XcodeConfiguration[]> {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  if (workspaceType === "spm") {
    // SPM projects typically use Debug and Release configurations
    // TODO: try to extract custom configurations from Package.swift if possible, but for now let's just return the defaults
    return [{ name: "Debug" }, { name: "Release" }];
  }

  if (workspaceType === "xcode") {
    commonLogger.log("Getting build configurations", { xcworkspace: options?.xcworkspace });

    const useWorkspaceParser = getWorkspaceConfig("system.customXcodeWorkspaceParser") ?? false;

    if (useWorkspaceParser) {
      try {
        const workspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace);
        const projects = await workspace.getProjects();

        commonLogger.debug("Projects", {
          paths: projects.map((project) => project.projectPath),
        });

        // Get configurations from all projects in the workspace
        const configurations = projects
          .flatMap((project) => {
            commonLogger.debug("Project configurations", {
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
        commonLogger.error("Error getting build configurations with workspace parser, falling back to xcodebuild", {
          error,
          xcworkspace: options.xcworkspace,
        });
        // Fallback to xcodebuild -list (via getBasicProjectInfo) if workspace parser fails
      }
    }

    // Original implementation using xcodebuild -list
    const output = await getBasicProjectInfo({
      xcworkspace: options.xcworkspace,
    });
    if (output.type === "project") {
      // todo: if workspace option is required, can this happen at all? 🤔
      return output.project.configurations.map((configuration) => {
        return {
          name: configuration,
        };
      });
    }
    if (output.type === "workspace") {
      const xcworkspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace);
      const projects = await xcworkspace.getProjects();

      commonLogger.debug("Projects", {
        paths: projects.map((project) => project.projectPath),
      });

      return projects
        .flatMap((project) => {
          commonLogger.debug("Project configurations", {
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
 * Generate xcode-build-server config
 */
export async function generateBuildServerConfig(options: { xcworkspace: string; scheme: string }) {
  const workspaceType = detectWorkspaceType(options.xcworkspace);
  const command = getXcodeBuildServerCommand();
  let cwd: string;
  let args: string[];

  if (workspaceType === "spm") {
    cwd = getSwiftPMDirectory(options.xcworkspace);
    args = ["config", "-scheme", options.scheme];
  } else if (workspaceType === "xcode") {
    cwd = getWorkspacePath();
    args = ["config", "-workspace", options.xcworkspace, "-scheme", options.scheme];
  } else {
    assertUnreachable(workspaceType);
  }
  await exec({
    command: command,
    args: args,
    cwd: cwd,
  });
}

export type XcodeBuildServerConfig = {
  scheme: string;
  workspace: string;
  build_root: string;
  // there might be more properties, like "kind", "args", "name", but we don't need them for now
};

/**
 * Read xcode-build-server config with proper types
 */
export async function readXcodeBuildServerConfig(): Promise<XcodeBuildServerConfig> {
  const buildServerJsonPath = path.join(getWorkspacePath(), "buildServer.json");
  return await readJsonFile<XcodeBuildServerConfig>(buildServerJsonPath);
}

/**
 * Is XcodeGen installed?
 */
export async function getIsXcodeGenInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcodegen"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

export async function generateXcodeGen() {
  await exec({
    command: "xcodegen",
    args: ["generate"],
  });
}

export async function getIsTuistInstalled() {
  try {
    await exec({
      command: "which",
      args: ["tuist"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

export async function tuistGenerate() {
  const env = getWorkspaceConfig("tuist.generate.env");
  return await exec({
    command: "tuist",
    args: ["generate", "--no-open"],
    env: env,
  });
}

export async function tuistClean() {
  await exec({
    command: "tuist",
    args: ["clean"],
  });
}

export async function tuistInstall() {
  await exec({
    command: "tuist",
    args: ["install"],
  });
}

export async function tuistEdit() {
  await exec({
    command: "tuist",
    args: ["edit"],
  });
}

export async function tuistTest() {
  await exec({
    command: "tuist",
    args: ["test"],
  });
}

/**
 * Get the Xcode version installed on the system using xcodebuild
 *
 * This version works properly with Xcodes.app, so it's the recommended one
 */
export async function getXcodeVersionInstalled(): Promise<{
  major: number;
}> {
  //~ xcodebuild -version
  // Xcode 16.0
  // Build version 16A242d
  const stdout = await exec({
    command: "xcrun",
    args: ["xcodebuild", "-version"],
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

/**
 * Get the Xcode version installed on the system using pgkutils
 *
 * This version doesn't work properly with Xcodes.app, leave it for reference
 */
export async function getXcodeVersionInstalled_pkgutils(): Promise<{
  major: number;
}> {
  const stdout = await exec({
    command: "pkgutil",
    args: ["--pkg-info=com.apple.pkg.CLTools_Executables"],
  });

  /*
  package-id: com.apple.pkg.CLTools_Executables
  version: 15.3.0.0.1.1708646388
  volume: /
  location: /
  install-time: 1718529452
  */
  const versionMatch = stdout.match(/version:\s*(\d+)\./);
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
