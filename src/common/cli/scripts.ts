import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { cache } from "../cache";
import { XcodeWorkspace } from "../xcode/workspace";
import { uniqueFilter } from "../helpers";
import { ExtensionContext } from "../commands";
import { prepareDerivedDataPath } from "../../build/utils";

type SimulatorOutput = {
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

enum SimDeviceOSType {
  iOS = "iOS",
  watchOS = "WatchOS",
  tvOS = "tvOS"
}

export class IosSimulator {
  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";
  public name: string;
  public runtime: string;
  public iosVersion: string;
  public runtimeType: SimDeviceOSType;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    runtime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.runtime = options.runtime;

    // iOS-14-5 => 14.5
    const rawiOSVersion = options.runtime.split(".").slice(-1)[0];
    this.iosVersion = rawiOSVersion.replace(/^(\w+)-(\d+)-(\d+)$/, "$2.$3");

    // "com.apple.CoreSimulator.SimRuntime.iOS-16-4"
    // "com.apple.CoreSimulator.SimRuntime.WatchOS-8-0"
    // extract iOS, tvOS, watchOS
    const regex = /com\.apple\.CoreSimulator\.SimRuntime\.(iOS|tvOS|watchOS)-\d+-\d+/;
    const match = this.runtime.match(regex);
    this.runtimeType = match ? match[1] as SimDeviceOSType : SimDeviceOSType.iOS;
  }

  get label() {
    // iPhone 12 Pro Max (14.5)
    return `${this.name} (${this.iosVersion})`;
  }

  /**
   * ID for uniquely identifying simulator saved in workspace state
   */
  get storageId() {
    return this.udid;
  }
}

export async function getSimulators(filterOSTypes: SimDeviceOSType[] = [SimDeviceOSType.iOS]): Promise<IosSimulator[]> {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });

  const output = JSON.parse(simulatorsRaw) as SimulatorsOutput;
  const simulators = Object.entries(output.devices)
    .flatMap(([key, value]) =>
      value.map((simulator) => {
        return new IosSimulator({
          udid: simulator.udid,
          isAvailable: simulator.isAvailable,
          state: simulator.state as "Booted",
          name: simulator.name,
          runtime: key,
        });
      }),
    )
    .filter((simulator) => filterOSTypes.includes(simulator.runtimeType))
    .filter((simulator) => simulator.isAvailable);
  return simulators;
}

export async function getSimulatorByUdid(
  context: ExtensionContext,
  options: {
    udid: string;
    refresh: boolean;
  },
): Promise<IosSimulator> {
  const simulators = await context.simulatorsManager.getSimulators({ refresh: options.refresh ?? false });
  for (const simulator of simulators) {
    if (simulator.udid === options.udid) {
      return simulator;
    }
  }
  throw new ExtensionError("Simulator not found", { context: { udid: options.udid } });
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
  xcworkspace: string;
}) {
  const derivedDataPath = prepareDerivedDataPath();
  const stdout = await exec({
    command: "xcodebuild",
    args: [
      "-showBuildSettings",
      "-scheme",
      options.scheme,
      "-workspace",
      options.xcworkspace,
      "-configuration",
      options.configuration,
      "-sdk",
      options.sdk,
      ...(derivedDataPath ? ["-derivedDataPath", derivedDataPath] : []),
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
    } else {
      return {
        type: "workspace",
        ...parsed,
      } as XcodebuildListWorkspaceOutput;
    }
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
  } else {
    return output.workspace.schemes.map((scheme) => {
      return {
        name: scheme,
      };
    });
  }
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
    return projects
      .flatMap((project) => {
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
