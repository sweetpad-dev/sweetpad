import { XMLParser } from "fast-xml-parser";
import { askXcodeWorkspacePath, getWorkspacePath } from "../../build/utils";
import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { findFiles, findFilesRecursive, readFile } from "../files";

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

export type SimulatorsOutput = {
  devices: { [key: string]: SimulatorOutput[] };
};

interface XcodeBuildListOutput {
  project: {
    configurations: string[];
    name: string;
    schemes: string[];
    targets: string[];
  };
}

export async function getSimulators() {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });

  const simulators = JSON.parse(simulatorsRaw) as SimulatorsOutput;
  return simulators;
}

export type BuildSettingsOutput = BuildSettingOutput[];

export type BuildSettingOutput = {
  action: string;
  target: string;
  buildSettings: {
    [key: string]: string;
  };
};

export async function getBuildSettings(options: {
  scheme: string;
  configuration: string;
  sdk: string;
  xcodeWorkspacePath: string;
}) {
  const stdout = await exec({
    command: "xcodebuild",
    args: [
      "-showBuildSettings",
      "-scheme",
      options.scheme,
      "-workspace",
      options.xcodeWorkspacePath,
      "-configuration",
      options.configuration,
      "-sdk",
      options.sdk,
      "-json",
    ],
  });

  // First few lines can be invalid json, so we need to skip them, untill we find "{" or "[" at the beginning of the line
  const lines = stdout.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("{") || line.startsWith("[")) {
      const data = lines.slice(i).join("\n");
      return JSON.parse(data) as BuildSettingsOutput;
    }
  }

  throw new ExtensionError("Error parsing build settings");
}

export async function removeDirectory(directory: string) {
  return exec({
    command: "rm",
    args: ["-rf", directory],
  });
}

export async function createDirectory(directory: string) {
  return exec({
    command: "mkdir",
    args: ["-p", directory],
  });
}

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

/**
 * Find xcode project in a given directory
 */
export async function getXcodeProjectPath(options: { cwd: string }): Promise<string> {
  const projects = await findFiles(options.cwd, (file, stats) => {
    return stats.isDirectory() && file.endsWith(".xcodeproj");
  });
  if (projects.length === 0) {
    throw new ExtensionError("No xcode projects found", {
      cwd: options.cwd,
    });
  }
  return projects[0];
}

let _basicProjectInfoCache: XcodeBuildListOutput | null = null;

export async function getBasicProjectInfo(): Promise<XcodeBuildListOutput> {
  if (_basicProjectInfoCache) {
    return _basicProjectInfoCache;
  }
  const stdout = await exec({
    command: "xcodebuild",
    args: ["-list", "-json"],
  });

  const data = JSON.parse(stdout) as XcodeBuildListOutput;
  _basicProjectInfoCache = data;
  return data;
}

type ParsedScheme = {
  name: string;
  launchAction: {
    buildConfiguration: string;
  };
};

let _schemesCache: ParsedScheme[] = [];
/**
 * Get list of schemes for a given project
 */
export async function getSchemes(): Promise<ParsedScheme[]> {
  // Return from cache if available
  if (_schemesCache.length) {
    return _schemesCache;
  }

  // Find all .xcscheme files in the workspace
  const workspacePath = getWorkspacePath();
  const schemesPath = await findFilesRecursive(
    workspacePath,
    (file, stats) => {
      return stats.isFile() && file.endsWith(".xcscheme");
    },
    {
      ignore: ["Pods", "node_modules"],
      depth: 5,
    }
  );

  // Parse all XML schemes files and extract basic information, like name
  // and launch configuration
  const schemes = await Promise.all(
    schemesPath.map(async (schemePath) => {
      const content = await readFile(schemePath);
      const name = schemePath.split("/").pop()!.replace(".xcscheme", "");
      const parser = new XMLParser({
        ignoreAttributes: false,
      });
      const parsed = parser.parse(content);
      return {
        name: name,
        launchAction: {
          buildConfiguration: parsed.Scheme.LaunchAction["@_buildConfiguration"],
        },
      };
    })
  );
  _schemesCache = schemes;
  return schemes;
}

/**
 * Generate xcode-build-server config
 */
export async function generateBuildServerConfig(options: { projectPath: string; scheme: string }) {
  await exec({
    command: "xcode-build-server",
    args: ["config", "-project", options.projectPath, "-scheme", options.scheme],
  });
}
