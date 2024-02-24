import { exec, execPrepared } from "../exec";
import { Stats, promises as fs } from "fs";
import * as path from "path";
import { findFiles } from "../files";

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
  const { stdout: simulatorsRaw, error: simulatorsError } = await exec`xcrun simctl list --json devices`;
  // TODO: add error handling
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

export async function getBuildSettings(options: { scheme: string; cwd: string; configuration: string; sdk: string }) {
  const { stdout, error } = await execPrepared(
    `xcodebuild -showBuildSettings -scheme ${options.scheme} -configuration ${options.configuration} -sdk ${options.sdk} -json`,
    {
      cwd: options.cwd,
    }
  );
  if (error) {
    // proper error handling
    console.error("Error fetching build settings", error);
    throw error;
  }

  // first few lines can be invalid json, so we need to skip them, untill we find "{" or "[" at the beginning of the line
  const lines = stdout.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.startsWith("{") || line.startsWith("[")) {
      const data = lines.slice(i).join("\n");
      return JSON.parse(data) as BuildSettingsOutput;
    }
  }

  //todo: proper error handling
  throw Error("Error parsing build settings");
}

export async function removeDirectory(directory: string) {
  return await exec`rm -rf ${directory}`;
}

export async function createDirectory(directory: string) {
  return await exec`mkdir -p ${directory}`;
}

export async function getIsXcbeautifyInstalled() {
  const { error } = await exec`which xcbeautify`;
  return !error;
}

/**
 * Find if xcode-build-server is installed
 */
export async function getIsXcodeBuildServerInstalled() {
  const { error } = await exec`which xcode-build-server`;
  return !error;
}

/**
 * Find xcode project in a given directory
 */
export async function getXcodeProjectPath(options: { cwd: string }): Promise<string> {
  const xcodeProjects = await findFiles(options.cwd, (file, stats) => {
    return stats.isDirectory() && file.endsWith(".xcodeproj");
  });
  if (xcodeProjects.length === 0) {
    throw new Error("No xcode projects found");
  }
  return xcodeProjects[0];
}

/**
 * Get list of schemes for a given project
 */
export async function getSchemes(options: { cwd: string }) {
  const { stdout, error } = await execPrepared("xcodebuild -list -json", { cwd: options.cwd });

  const data = JSON.parse(stdout) as XcodeBuildListOutput;
  return data.project.schemes;
}

/**
 * Generate xcode-build-server config
 */
export async function generateBuildServerConfig(options: { projectPath: string; scheme: string; cwd: string }) {
  // xcode-build-server config -project *.xcodeproj -scheme <XXX>
  const { error, stdout } = await execPrepared(
    `xcode-build-server config -project ${options.projectPath} -scheme ${options.scheme}`,
    {
      cwd: options.cwd,
    }
  );
  console.log("generateBuildServerConfig", error);
}
