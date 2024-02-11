import { exec, execPrepared } from "../exec";

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
