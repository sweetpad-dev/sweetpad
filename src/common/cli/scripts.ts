import path from "node:path";
import { prepareDerivedDataPath } from "../../build/utils";
import type { DestinationPlatform } from "../../destination/constants";
import { cache } from "../cache";
import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { uniqueFilter } from "../helpers";
import { commonLogger } from "../logger";
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

export async function getSimulators(): Promise<SimulatorsOutput> {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });
  return JSON.parse(simulatorsRaw) as SimulatorsOutput;
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

  constructor(output: BuildSettingsOutput) {
    if (output.length === 0) {
      // todo: improve logging
      throw new ExtensionError("Error fetching build settings");
    }
    this.settings = output[0]?.buildSettings;
  }

  get targetBuildDir() {
    // Example:
    // - /Users/hyzyla/Library/Developer/Xcode/DerivedData/ControlRoom-gdvrildvemgjaiameavxoegdskby/Build/Products/Debug
    return this.settings.TARGET_BUILD_DIR;
  }

  get executablePath() {
    // Example:
    // - {targetBuildDir}/Control Room.app/Contents/MacOS/Control Room
    return path.join(this.targetBuildDir, this.settings.EXECUTABLE_PATH);
  }

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

  get targetName() {
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
 */
async function getBuildSettingsBase(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings | null> {
  const derivedDataPath = prepareDerivedDataPath();

  const args = [
    "-showBuildSettings",
    "-scheme",
    options.scheme,
    "-workspace",
    options.xcworkspace,
    "-configuration",
    options.configuration,
    ...(derivedDataPath ? ["-derivedDataPath", derivedDataPath] : []),
    "-json",
  ];

  if (options.sdk !== undefined) {
    args.push("-sdk", options.sdk);
  }

  const stdout = await exec({
    command: "xcodebuild",
    args: args,
  });

  // First few lines can be invalid json, so we need to skip them, untill we find "{" or "[" at the beginning of the line
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
      const output = JSON.parse(data) as BuildSettingsOutput;
      if (output.length === 0) {
        return null;
      }
      return new XcodeBuildSettings(output);
    }
  }
  return null;
}

export async function getOptionalBuildSettings(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings | null> {
  try {
    return await getBuildSettingsBase(options);
  } catch (e) {
    commonLogger.error("Error getting build settings", {
      error: e,
    });
    return null;
  }
}

/**
 * Extract build settings for the given scheme and configuration
 */
export async function getBuildSettings(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}): Promise<XcodeBuildSettings> {
  const settings = await getOptionalBuildSettings(options);
  if (!settings) {
    throw new ExtensionError("Empty build settings");
  }
  return settings;
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
 * Find if xcode-build-server is installed
 */
export async function getIsXcodeBuildServerInstalled() {
  try {
    await exec({
      command: "which",
      args: ["xcode-build-server"],
    });
    return true;
  } catch (e) {
    return false;
  }
}

export const getBasicProjectInfo = cache(
  async (options: { xcworkspace: string | undefined }): Promise<XcodebuildListOutput> => {
    const stdout = await exec({
      command: "xcodebuild",
      args: ["-list", "-json", ...(options?.xcworkspace ? ["-workspace", options?.xcworkspace] : [])],
    });
    const parsed = JSON.parse(stdout);
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
  },
);

export async function getSchemes(options: { xcworkspace: string | undefined }): Promise<XcodeScheme[]> {
  const output = await getBasicProjectInfo({
    xcworkspace: options?.xcworkspace,
  });
  if (output.type === "project") {
    return output.project.schemes.map((scheme) => {
      return {
        name: scheme,
      };
    });
  }
  return output.workspace.schemes.map((scheme) => {
    return {
      name: scheme,
    };
  });
}

export async function getTargets(options: { xcworkspace: string }): Promise<string[]> {
  const output = await getBasicProjectInfo({
    xcworkspace: options.xcworkspace,
  });
  if (output.type === "project") {
    return output.project.targets;
  }
  const xcworkspace = await XcodeWorkspace.parseWorkspace(options.xcworkspace);
  const projects = await xcworkspace.getProjects();
  return projects.flatMap((project) => project.getTargets());
}

export async function getBuildConfigurations(options: { xcworkspace: string }): Promise<XcodeConfiguration[]> {
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

/**
 * Generate xcode-build-server config
 */
export async function generateBuildServerConfig(options: { xcworkspace: string; scheme: string }) {
  await exec({
    command: "xcode-build-server",
    args: ["config", "-workspace", options.xcworkspace, "-scheme", options.scheme],
  });
}

/**
 * Is XcodeGen installed?s
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
  return await exec({
    command: "tuist",
    args: ["generate", "--no-open"],
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
    command: "xcodebuild",
    args: ["-version"],
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
