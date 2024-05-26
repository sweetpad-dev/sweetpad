import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { findFiles, readFile } from "../files";
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
  throw new ExtensionError("Simulator not found", { context: { udid } });
}

type BuildSettingsOutput = BuildSettingOutput[];

type BuildSettingOutput = {
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
 * Find xcode workspace in a given directory
 */
async function getXcodeWorkspacePath(): Promise<string> {
  const workspaceFolder = getWorkspacePath();
  const workspaces = await findFiles({
    directory: workspaceFolder,
    matcher: (file) => {
      return file.isDirectory() && file.name.endsWith(".xcworkspace");
    },
  });
  if (workspaces.length === 0) {
    throw new ExtensionError("No xcode workspaces found", {
      context: {
        cwd: workspaceFolder,
      },
    });
  }
  return workspaces[0];
}

export const getBasicProjectInfo = cache(async (): Promise<XcodebuildListOutput> => {
  const stdout = await exec({
    command: "xcodebuild",
    args: ["-list", "-json"],
  });
  const parsed = JSON.parse(stdout);
  if (parsed.project) {
    return {
      type: "project",
      ...parsed,
    } as XcodebuildListProjectOutput;
  } else {
    return {
      type: "workspace",
      ...parsed,
    } as XcodebuildListWorkspaceOutput;
  }
});

export async function getSchemes(): Promise<XcodeScheme[]> {
  const output = await getBasicProjectInfo();
  if (output.type === "project") {
    return output.project.schemes.map((scheme) => {
      return {
        name: scheme,
      };
    });
  } else {
    return output.workspace.schemes.map((scheme) => {
      return {
        name: scheme,
      };
    });
  }
}

export async function getBuildConfigurations(): Promise<XcodeConfiguration[]> {
  const output = await getBasicProjectInfo();
  if (output.type === "project") {
    return output.project.configurations.map((configuration) => {
      return {
        name: configuration,
      };
    });
  }
  if (output.type === "workspace") {
    // how to get configurations for workspace?
    // todo: pass scheme here and try to get configurations from it
  }
  return [];
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
