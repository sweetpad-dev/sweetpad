import { ExtensionError } from "../errors";
import { exec } from "../exec";
import { cache } from "../cache";
import { XcodeWorkspace } from "../xcode/workspace";
import { uniqueFilter } from "../helpers";
import { ExtensionContext } from "../commands";
import { prepareDerivedDataPath } from "../../build/utils";
import { DestinationPlatform } from "../../destination/constants";
import { DestinationOS } from "../../destination/constants";
import path from "path";
import { iOSSimulatorDestination } from "../../destination/types";

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

export type iOSSimulatorDeviceType = "iPhone" | "iPad" | "iPod" | "AppleTV" | "AppleWatch" | "AppleVision";

export class iOSSimulator {
  public udid: string;
  public isAvailable: boolean;
  public state: "Booted" | "Shutdown";

  public deviceType: iOSSimulatorDeviceType | null;
  public name: string;
  public runtime: string;
  public osVersion: string;
  public runtimeType: DestinationOS;

  constructor(options: {
    udid: string;
    isAvailable: boolean;
    state: "Booted" | "Shutdown";
    name: string;
    rawDeviceType: string;
    runtime: string;
  }) {
    this.udid = options.udid;
    this.isAvailable = options.isAvailable;
    this.state = options.state;
    this.name = options.name;
    this.deviceType = iOSSimulator.parseDeviceType(options.rawDeviceType);
    this.runtime = options.runtime;

    // iOS-14-5 => 14.5
    const rawiOSVersion = options.runtime.split(".").slice(-1)[0];
    this.osVersion = rawiOSVersion.replace(/^(\w+)-(\d+)-(\d+)$/, "$2.$3");

    // "com.apple.CoreSimulator.SimRuntime.iOS-16-4"
    // "com.apple.CoreSimulator.SimRuntime.WatchOS-8-0"
    // extract iOS, tvOS, watchOS
    const regex = /com\.apple\.CoreSimulator\.SimRuntime\.(iOS|tvOS|watchOS)-\d+-\d+/;
    const match = this.runtime.match(regex);
    this.runtimeType = match ? (match[1] as DestinationOS) : DestinationOS.iOS;
  }

  static parseDeviceType(rawDeviceType: string): iOSSimulatorDeviceType | null {
    // examples:
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-Vision-Pro"
    // - "com.apple.CoreSimulator.SimDeviceType.iPhone-8"
    // - "com.apple.CoreSimulator.SimDeviceType.iPhone-11-Pro"
    // - "com.apple.CoreSimulator.SimDeviceType.iPod-touch--7th-generation-"
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-TV-1080p"
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-TV-4K-3rd-generation-4K"
    // - "com.apple.CoreSimulator.SimDeviceType.Apple-Watch-Series-5-40mm"

    // common prefix amoung all device types (hope so)
    const prefix = "com.apple.CoreSimulator.SimDeviceType.";
    if (!rawDeviceType.startsWith(prefix)) {
      return null;
    }

    const deviceType = rawDeviceType.slice(prefix.length);
    if (deviceType.startsWith("iPhone")) {
      return "iPhone";
    } else if (deviceType.startsWith("iPad")) {
      return "iPad";
    } else if (deviceType.startsWith("iPod")) {
      return "iPod";
    } else if (deviceType.startsWith("Apple-TV")) {
      return "AppleTV";
    } else if (deviceType.startsWith("Apple-Watch")) {
      return "AppleWatch";
    } else if (deviceType.startsWith("Apple-Vision")) {
      return "AppleVision";
    }
    return null;
  }

  /**
   * ID for uniquely identifying simulator saved in workspace state
   */
  get storageId() {
    return this.udid;
  }
}

export async function getSimulators(): Promise<iOSSimulator[]> {
  const simulatorsRaw = await exec({
    command: "xcrun",
    args: ["simctl", "list", "--json", "devices"],
  });

  const output = JSON.parse(simulatorsRaw) as SimulatorsOutput;
  const simulators = Object.entries(output.devices)
    .flatMap(([key, value]) =>
      value.map((simulator) => {
        return new iOSSimulator({
          udid: simulator.udid,
          isAvailable: simulator.isAvailable,
          state: simulator.state as "Booted",
          name: simulator.name,
          rawDeviceType: simulator.deviceTypeIdentifier,
          runtime: key,
        });
      }),
    )
    .filter((simulator) => simulator.isAvailable);
  return simulators;
}

export async function getSimulatorByUdid(
  context: ExtensionContext,
  options: {
    udid: string;
    refresh: boolean;
  },
): Promise<iOSSimulatorDestination> {
  const simulators = await context.destinationsManager.getiOSSimulators({
    refresh: options.refresh ?? false,
  });
  for (const simulator of simulators) {
    if (simulator.udid === options.udid) {
      return simulator;
    }
  }
  throw new ExtensionError("Simulator not found", { context: { udid: options.udid } });
}

export type BuildSettingsOutput = BuildSettingOutput[];

type BuildSettingOutput = {
  action: string;
  target: string;
  buildSettings: {
    [key: string]: string;
  };
};

type ProductOutputInfo = {
  productPath: string;
  productName: string;
  binaryPath: string;
  bundleIdentifier: string;
};

export async function getBuildSettings(options: {
  scheme: string;
  configuration: string;
  sdk: string | undefined;
  xcworkspace: string;
}) {
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
    if (line.startsWith("{") || line.startsWith("[")) {
      const data = lines.slice(i).join("\n");
      return JSON.parse(data) as BuildSettingsOutput;
    }
  }

  throw new ExtensionError("Error parsing build settings");
}

export function getProductOutputInfoFromBuildSettings(buildSettings: BuildSettingsOutput) {
  const settings = buildSettings[0]?.buildSettings;
  if (!settings) {
    throw new ExtensionError("Error fetching build settings");
  }

  const bundleIdentifier = settings.PRODUCT_BUNDLE_IDENTIFIER;
  const targetBuildDir = settings.TARGET_BUILD_DIR;
  const targetName = settings.TARGET_NAME;
  let appName;
  if (settings.WRAPPER_NAME) {
    appName = settings.WRAPPER_NAME;
  } else if (settings.FULL_PRODUCT_NAME) {
    appName = settings.FULL_PRODUCT_NAME;
  } else if (settings.PRODUCT_NAME) {
    appName = `${settings.PRODUCT_NAME}.app`;
  } else {
    appName = `${targetName}.app`;
  }

  const executablePath = settings.EXECUTABLE_PATH;
  const productPath = path.join(targetBuildDir, appName);
  const binaryPath = path.join(targetBuildDir, executablePath);

  return {
    productPath: productPath,
    productName: appName,
    binaryPath: binaryPath,
    bundleIdentifier: bundleIdentifier,
  } as ProductOutputInfo;
}

/**
 * Check which platforms current project supports by looking at build settings
 */
export function getSupportedPlatforms(buildSettings: BuildSettingsOutput): DestinationPlatform[] {
  const settings = buildSettings[0]?.buildSettings;
  if (!settings) {
    throw new ExtensionError("Error fetching build settings");
  }

  // ex: "iphonesimulator iphoneos"
  const platformsRaw = settings.SUPPORTED_PLATFORMS;
  return platformsRaw.split(" ").map((platform) => {
    return platform as DestinationPlatform;
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
