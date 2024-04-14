import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { findFiles } from "../files";
import { cache } from "../cache";
import { getWorkspacePath } from "../../build/utils";

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

export type XcodeScheme = {
  name: string;
};

type XcodeConfiguration = {
  name: string;
};

export async function getSimulators() {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });

  const simulators = JSON.parse(simulatorsRaw) as SimulatorsOutput;
  return simulators;
}

export async function getSimulatorByUdid(udid: string) {
  const simulators = await getSimulators();
  for (const key in simulators.devices) {
    const devices = simulators.devices[key];
    for (const device of devices) {
      if (device.udid === udid) {
        return device;
      }
    }
  }
  throw new ExtensionError("Simulator not found", { udid });
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
export async function getXcodeProjectPath(): Promise<string> {
  const workspaceFolder = getWorkspacePath();
  const projects = await findFiles(workspaceFolder, (file, stats) => {
    return stats.isDirectory() && file.endsWith(".xcodeproj");
  });
  if (projects.length === 0) {
    throw new ExtensionError("No xcode projects found", {
      cwd: workspaceFolder,
    });
  }
  return projects[0];
}

export const getBasicProjectInfo = cache(async () => {
  const stdout = await exec({
    command: "xcodebuild",
    args: ["-list", "-json"],
  });

  return JSON.parse(stdout) as XcodeBuildListOutput;
});

export async function getSchemes(): Promise<XcodeScheme[]> {
  const output = await getBasicProjectInfo();
  const schemes = output.project.schemes.map((scheme) => {
    return {
      name: scheme,
    };
  });
  return schemes;
}

export async function getBuildConfigurations(): Promise<XcodeConfiguration[]> {
  const output = await getBasicProjectInfo();
  return output.project.configurations.map((configuration) => {
    return {
      name: configuration,
    };
  });
}

/**
 * Generate xcode-build-server config
 */
export async function generateBuildServerConfig(options: { xcodeWorkspacePath: string; scheme: string }) {
  await exec({
    command: "xcode-build-server",
    args: ["config", "-workspace", options.xcodeWorkspacePath, "-scheme", options.scheme],
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
