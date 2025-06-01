import path from "node:path";
import * as vscode from "vscode";
import type { BuildTreeItem } from "./tree";

import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import {
  type XcodeScheme,
  generateBuildServerConfig,
  getBuildConfigurations,
  getBuildSettingsToAskDestination,
  getBuildSettingsToLaunch,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { ExecBaseError, ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceRelativePath, isFileExists, readJsonFile, removeDirectory, tempFilePath } from "../common/files";
import { commonLogger } from "../common/logger";
import { showInputBox } from "../common/quick-pick";
import { type Command, type TaskTerminal, runTask } from "../common/tasks";
import { assertUnreachable } from "../common/types";
import type { Destination } from "../destination/types";
import type { DeviceDestination } from "../devices/types";
import type { SimulatorDestination } from "../simulators/types";
import { getSimulatorByUdid } from "../simulators/utils";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  detectXcodeWorkspacesPaths,
  getCurrentXcodeWorkspacePath,
  getWorkspacePath,
  prepareBundleDir,
  prepareDerivedDataPath,
  prepareStoragePath,
  restartSwiftLSP,
  selectXcodeWorkspace,
} from "./utils";

function writeWatchMarkers(terminal: TaskTerminal) {
  terminal.write("🍭 SweetPad: watch marker (start)\n");
  terminal.write("🍩 SweetPad: watch marker (end)\n\n");
}

async function ensureAppPathExists(appPath: string | undefined): Promise<string> {
  if (!appPath) {
    throw new ExtensionError("App path is empty. Something went wrong.");
  }

  const isExists = await isFileExists(appPath);
  if (!isExists) {
    throw new ExtensionError(`App path does not exist. Have you built the app? Path: ${appPath}`);
  }
  return appPath;
}

export async function runOnMac(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    xcworkspace: string;
    configuration: string;
    watchMarker: boolean;
    launchArgs: string[];
    launchEnv: Record<string, string>;
  },
) {
  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToLaunch({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: "macosx",
    xcworkspace: options.xcworkspace,
  });

  const executablePath = await ensureAppPathExists(buildSettings.executablePath);

  context.updateWorkspaceState("build.lastLaunchedApp", {
    type: "macos",
    appPath: executablePath,
  });
  if (options.watchMarker) {
    writeWatchMarkers(terminal);
  }

  context.updateProgressStatus(`Running "${options.scheme}" on Mac`);
  await terminal.execute({
    command: executablePath,
    env: options.launchEnv,
    args: options.launchArgs,
  });
}

export async function runOniOSSimulator(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    destination: SimulatorDestination;
    sdk: string;
    configuration: string;
    xcworkspace: string;
    watchMarker: boolean;
    launchArgs: string[];
    launchEnv: Record<string, string>;
    debug: boolean;
  },
) {
  const simulatorId = options.destination.udid;

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToLaunch({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: options.xcworkspace,
  });
  const appPath = await ensureAppPathExists(buildSettings.appPath);
  const bundlerId = buildSettings.bundleIdentifier;

  // Open simulator
  context.updateProgressStatus("Launching Simulator.app");
  await terminal.execute({
    command: "open",
    args: ["-g", "-a", "Simulator"],
  });

  // Get simulator with fresh state
  context.updateProgressStatus(`Searching for simulator "${simulatorId}"`);
  const simulator = await getSimulatorByUdid(context, {
    udid: simulatorId,
  });

  // Boot device
  if (!simulator.isBooted) {
    context.updateProgressStatus(`Booting simulator "${simulator.name}"`);
    await terminal.execute({
      command: "xcrun",
      args: ["simctl", "boot", simulator.udid],
    });

    // Refresh list of simulators after we start new simulator
    context.destinationsManager.refreshSimulators();
  }

  // Install app
  context.updateProgressStatus(`Installing "${options.scheme}" on "${simulator.name}"`);
  await terminal.execute({
    command: "xcrun",
    args: ["simctl", "install", simulator.udid, appPath],
  });

  context.updateWorkspaceState("build.lastLaunchedApp", {
    type: "simulator",
    appPath: appPath,
  });
  if (options.watchMarker) {
    writeWatchMarkers(terminal);
  }

  const launchArgs = [
    "simctl",
    "launch",
    "--console-pty",
    // This instructs app to wait for the debugger to be attached before launching,
    // ensuring you can debug issues happening early on.
    ...(options.debug ? ["--wait-for-debugger"] : []),
    "--terminate-running-process",
    simulator.udid,
    bundlerId,
    ...options.launchArgs,
  ];

  // Run app
  context.updateProgressStatus(`Running "${options.scheme}" on "${simulator.name}"`);
  await terminal.execute({
    command: "xcrun",
    args: launchArgs,
    // should be prefixed with `SIMCTL_CHILD_` to pass to the child process
    env: Object.fromEntries(Object.entries(options.launchEnv).map(([key, value]) => [`SIMCTL_CHILD_${key}`, value])),
  });
}

export async function runOniOSDevice(
  context: ExtensionContext,
  terminal: TaskTerminal,
  option: {
    scheme: string;
    configuration: string;
    destination: DeviceDestination;
    sdk: string;
    xcworkspace: string;
    watchMarker: boolean;
    launchArgs: string[];
    launchEnv: Record<string, string>;
  },
) {
  const { scheme, configuration, destination } = option;
  const { udid: deviceId, type: destinationType, name: destinationName } = destination;

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToLaunch({
    scheme: scheme,
    configuration: configuration,
    sdk: option.sdk,
    xcworkspace: option.xcworkspace,
  });

  const targetPath = await ensureAppPathExists(buildSettings.appPath);
  const bundlerId = buildSettings.bundleIdentifier;

  // Install app on device
  context.updateProgressStatus(`Installing "${scheme}" on "${destinationName}"`);
  await terminal.execute({
    command: "xcrun",
    args: ["devicectl", "device", "install", "app", "--device", deviceId, targetPath],
  });

  context.updateWorkspaceState("build.lastLaunchedApp", {
    type: "device",
    appPath: targetPath,
    appName: buildSettings.appName,
    destinationId: deviceId,
    destinationType: destinationType,
  });

  await using jsonOuputPath = await tempFilePath(context, {
    prefix: "json",
  });

  context.updateProgressStatus("Extracting Xcode version");
  const xcodeVersion = await getXcodeVersionInstalled();
  const isConsoleOptionSupported = xcodeVersion.major >= 16;

  if (option.watchMarker) {
    writeWatchMarkers(terminal);
  }

  // Prepare the launch arguments
  const launchArgs = [
    "devicectl",
    "device",
    "process",
    "launch",
    // Attaches the application to the console and waits for it to exit
    isConsoleOptionSupported ? "--console" : null,
    "--json-output",
    jsonOuputPath.path,
    // Terminates any already-running instances of the app prior to launch. Not supported on all platforms.
    "--terminate-existing",
    "--device",
    deviceId,
    bundlerId,
    ...option.launchArgs,
  ].filter((arg) => arg !== null); // Filter out null arguments

  // Launch app on device
  context.updateProgressStatus(`Running "${option.scheme}" on "${option.destination.name}"`);
  await terminal.execute({
    command: "xcrun",
    args: launchArgs,
    // Should be prefixed with `DEVICECTL_CHILD_` to pass to the child process
    env: Object.fromEntries(Object.entries(option.launchEnv).map(([key, value]) => [`DEVICECTL_CHILD_${key}`, value])),
  });

  let jsonOutput: any;
  try {
    jsonOutput = await readJsonFile(jsonOuputPath.path);
  } catch (e) {
    throw new ExtensionError("Error reading json output");
  }

  if (jsonOutput.info.outcome !== "success") {
    terminal.write("Error launching app on device", {
      newLine: true,
    });
    terminal.write(JSON.stringify(jsonOutput.result, null, 2), {
      newLine: true,
    });
    return;
  }
  terminal.write(`App launched on device with PID: ${jsonOutput.result.process.processIdentifier}`, {
    newLine: true,
  });
}

export function isXcbeautifyEnabled() {
  return getWorkspaceConfig("build.xcbeautifyEnabled") ?? true;
}

/**
 * Build destination string for xcodebuild command.
 *
 * Examples:
 * - `platform=iOS Simulator,id=12345678-1234-1234-1234-123456789012,arch=x86_64`
 * - `platform=macOS,arch=arm64`
 * - `platform=iOS,arch=arm64`
 */
function buildDestinationString(options: {
  platform: string;
  id?: string;
  arch?: string;
}): string {
  const { platform, id, arch } = options;
  if (id && arch) {
    return `platform=${platform},id=${id},arch=${arch}`;
  }
  if (id && !arch) {
    return `platform=${platform},id=${id}`;
  }
  if (!id && arch) {
    return `platform=${platform},arch=${arch}`;
  }
  return `platform=${platform}`; // no id and no arch
}

function getSimulatorArch(): string | undefined {
  // Rosetta is technology that allows running x86_64 code on Apple Silicon Macs.
  // This function instructs xcodebuild to build for x86_64 architecture when Rosetta destinations
  // enabled in Xcode
  const useRosetta = getWorkspaceConfig("build.rosettaDestination") ?? false;
  if (useRosetta) {
    return "x86_64";
  }
  return undefined; // let xcodebuild decide the architecture
}

/**
 * Prepare and return destination string for xcodebuild command.
 *
 * WARN: Do not use result of this function to anything else than xcodebuild command.
 */
export function getXcodeBuildDestinationString(options: { destination: Destination }): string {
  const destination = options.destination;

  if (destination.type === "iOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "iOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "watchOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "watchOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "tvOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "tvOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "visionOSSimulator") {
    const arch = getSimulatorArch();
    return buildDestinationString({ platform: "visionOS Simulator", id: destination.udid, arch: arch });
  }
  if (destination.type === "macOS") {
    // note: without arch, xcodebuild will show warning like this:
    // --- xcodebuild: WARNING: Using the first of multiple matching destinations:
    // { platform:macOS, arch:arm64, id:00008103-000109910EC3001E, name:My Mac }
    // { platform:macOS, arch:x86_64, id:00008103-000109910EC3001E, name:My Mac }
    // return `platform=macOS,arch=${destination.arch}`;
    return buildDestinationString({ platform: "macOS", arch: destination.arch });
  }
  if (destination.type === "iOSDevice") {
    return buildDestinationString({ platform: "iOS", id: destination.udid });
  }
  if (destination.type === "watchOSDevice") {
    return buildDestinationString({ platform: "watchOS", id: destination.udid });
  }
  if (destination.type === "tvOSDevice") {
    return buildDestinationString({ platform: "tvOS", id: destination.udid });
  }
  if (destination.type === "visionOSDevice") {
    return buildDestinationString({ platform: "visionOS", id: destination.udid });
  }
  return assertUnreachable(destination);
}

class XcodeCommandBuilder {
  NO_VALUE = "__NO_VALUE__";

  private xcodebuild = "xcodebuild";
  private parameters: {
    arg: string;
    value: string | "__NO_VALUE__";
  }[] = [];
  private buildSettings: { key: string; value: string }[] = [];
  private actions: string[] = [];

  addBuildSettings(key: string, value: string) {
    this.buildSettings.push({
      key: key,
      value: value,
    });
  }

  addOption(flag: string) {
    this.parameters.push({
      arg: flag,
      value: this.NO_VALUE,
    });
  }

  addParameters(arg: string, value: string) {
    this.parameters.push({
      arg: arg,
      value: value,
    });
  }

  addAction(action: string) {
    this.actions.push(action);
  }

  addAdditionalArgs(args: string[]) {
    // Cases:
    // ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
    // ["xcodebuild", "-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
    // ["ARG1=value1", "ARG2=value2", "ARG3", "ARG4=value4"]
    // ["xcodebuild", "ARG1=value1", "ARG2=value2", "ARG3", "ARG4=value4"]
    if (args.length === 0) {
      return;
    }

    for (let i = 0; i < args.length; i++) {
      const current = args[i];
      const next = args[i + 1];
      if (current && next && current.startsWith("-") && !next.startsWith("-")) {
        this.parameters.push({
          arg: current,
          value: next,
        });
        i++;
      } else if (current?.startsWith("-")) {
        this.parameters.push({
          arg: current,
          value: this.NO_VALUE,
        });
      } else if (current?.includes("=")) {
        const [arg, value] = current.split("=");
        this.buildSettings.push({
          key: arg,
          value: value,
        });
      } else if (["clean", "build", "test"].includes(current)) {
        this.actions.push(current);
      } else {
        commonLogger.warn("Unknown argument", {
          argument: current,
          args: args,
        });
      }
    }

    // Remove duplicates, with higher priority for the last occurrence
    const seenParameters = new Set<string>();
    this.parameters = this.parameters
      .slice()
      .reverse()
      .filter((param) => {
        if (seenParameters.has(param.arg)) {
          return false;
        }
        seenParameters.add(param.arg);
        return true;
      })
      .reverse();

    // Remove duplicates, with higher priority for the last occurrence
    const seenActions = new Set<string>();
    this.actions = this.actions.filter((action) => {
      if (seenActions.has(action)) {
        return false;
      }
      seenActions.add(action);
      return true;
    });

    // Remove duplicates, with higher priority for the last occurrence
    const seenSettings = new Set<string>();
    this.buildSettings = this.buildSettings
      .slice()
      .reverse()
      .filter((setting) => {
        if (seenSettings.has(setting.key)) {
          return false;
        }
        seenSettings.add(setting.key);
        return true;
      })
      .reverse();
  }

  build(): string[] {
    const commandParts = [this.xcodebuild];

    for (const { key, value } of this.buildSettings) {
      commandParts.push(`${key}=${value}`);
    }

    for (const { arg, value } of this.parameters) {
      commandParts.push(arg);
      if (value !== this.NO_VALUE) {
        commandParts.push(value);
      }
    }

    for (const action of this.actions) {
      commandParts.push(action);
    }
    return commandParts;
  }
}

export async function buildApp(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    sdk: string;
    configuration: string;
    shouldBuild: boolean;
    shouldClean: boolean;
    shouldTest: boolean;
    xcworkspace: string;
    destinationRaw: string;
    debug: boolean;
  },
) {
  const useXcbeatify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
  const bundlePath = await prepareBundleDir(context, options.scheme);
  const derivedDataPath = prepareDerivedDataPath();

  const arch = getWorkspaceConfig("build.arch") || undefined;
  const allowProvisioningUpdates = getWorkspaceConfig("build.allowProvisioningUpdates") ?? true;

  // ex: ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
  const additionalArgs: string[] = getWorkspaceConfig("build.args") || [];

  // ex: { "ARG1": "value1", "ARG2": null, "ARG3": "value3" }
  const env = getWorkspaceConfig("build.env") || {};

  const command = new XcodeCommandBuilder();
  if (arch) {
    command.addBuildSettings("ARCHS", arch);
    command.addBuildSettings("VALID_ARCHS", arch);
    command.addBuildSettings("ONLY_ACTIVE_ARCH", "NO");
  }

  // Add debug-specific build settings if in debug mode
  if (options.debug) {
    // This tells the compiler to generate debugging symbols and include them in the compiled binary.
    // Without this, LLDB wont know how to match lines of code to machine instructions. This is normally
    // set to YES on XCode debug builds, but forcing it here, ensures you'll always get them in
    // sweetpad: debugging-launch
    command.addBuildSettings("GCC_GENERATE_DEBUGGING_SYMBOLS", "YES");
    // In Xcode, ONLY_ACTIVE_ARCH is a build setting that controls whether you compile for only the architecture
    // of the machine (or simulator/device) you're currently targeting, or for all architectures listed in your
    // project's ARCHS setting.
    // It speeds up compile times, especially in Debug, because Xcode skips generating unused slices.
    command.addBuildSettings("ONLY_ACTIVE_ARCH", "YES");
  }

  command.addParameters("-scheme", options.scheme);
  command.addParameters("-configuration", options.configuration);
  command.addParameters("-workspace", options.xcworkspace);
  command.addParameters("-destination", options.destinationRaw);
  command.addParameters("-resultBundlePath", bundlePath);
  if (derivedDataPath) {
    command.addParameters("-derivedDataPath", derivedDataPath);
  }
  if (allowProvisioningUpdates) {
    command.addOption("-allowProvisioningUpdates");
  }

  if (options.shouldClean) {
    command.addAction("clean");
  }
  if (options.shouldBuild) {
    command.addAction("build");
  }
  if (options.shouldTest) {
    command.addAction("test");
  }
  command.addAdditionalArgs(additionalArgs);

  const commandParts = command.build();
  let pipes: Command[] | undefined = undefined;
  if (useXcbeatify) {
    pipes = [{ command: "xcbeautify", args: [] }];
  }

  if (options.shouldClean) {
    context.updateProgressStatus(`Cleaning "${options.scheme}"`);
  } else if (options.shouldBuild) {
    context.updateProgressStatus(`Building "${options.scheme}"`);
  } else if (options.shouldTest) {
    context.updateProgressStatus(`Building "${options.scheme}"`);
  }
  await terminal.execute({
    command: commandParts[0],
    args: commandParts.slice(1),
    pipes: pipes,
    env: env,
  });

  await restartSwiftLSP();
}

/**
 * Build app without running
 */
export async function buildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting build command");
  return commonBuildCommand(context, item, { debug: false });
}

/**
 * Build app in debug mode without running
 */
export async function debuggingBuildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Building the app (debug mode)");
  return commonBuildCommand(context, item, { debug: true });
}

/**
 * Build app without running
 */
async function commonBuildCommand(
  context: ExtensionContext,
  item: BuildTreeItem | undefined,
  options: { debug: boolean },
) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to build", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(context, {
    name: "Build",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
        debug: options.debug,
      });
    },
  });
}

/**
 * Build and run application on the simulator or device
 */
export async function launchCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return commonLaunchCommand(context, item, { debug: false });
}

/**
 * Builds and launches the application in debug mode
 * This is a convenience wrapper around launchCommand that sets the debug flag
 */
export async function debuggingLaunchCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return commonLaunchCommand(context, item, { debug: true });
}

/**
 * Build and run application on the simulator or device
 */
async function commonLaunchCommand(
  context: ExtensionContext,
  item: BuildTreeItem | undefined,
  options: { debug: boolean },
) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);

  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  const launchArgs = getWorkspaceConfig("build.launchArgs") ?? [];
  const launchEnv = getWorkspaceConfig("build.launchEnv") ?? {};

  await runTask(context, {
    name: options.debug ? "Debug" : "Launch",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
        debug: options.debug,
      });

      if (destination.type === "macOS") {
        await runOnMac(context, terminal, {
          scheme: scheme,
          xcworkspace: xcworkspace,
          configuration: configuration,
          watchMarker: false,
          launchArgs: launchArgs,
          launchEnv: launchEnv,
        });
      } else if (
        destination.type === "iOSSimulator" ||
        destination.type === "watchOSSimulator" ||
        destination.type === "tvOSSimulator" ||
        destination.type === "visionOSSimulator"
      ) {
        await runOniOSSimulator(context, terminal, {
          scheme: scheme,
          destination: destination,
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
          launchArgs: launchArgs,
          launchEnv: launchEnv,
          debug: options.debug,
        });
      } else if (
        destination.type === "iOSDevice" ||
        destination.type === "watchOSDevice" ||
        destination.type === "tvOSDevice" ||
        destination.type === "visionOSDevice"
      ) {
        await runOniOSDevice(context, terminal, {
          scheme: scheme,
          destination: destination,
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
          launchArgs: launchArgs,
          launchEnv: launchEnv,
        });
      } else {
        assertUnreachable(destination);
      }
    },
  });
}

/**
 * Run application on the simulator or device without building
 */
export async function runCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting run command");
  return commonRunCommand(context, item, { debug: false });
}

/**
 * Run application on the simulator or device without building in debug mode
 */
export async function debuggingRunCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting debugging command");
  return commonRunCommand(context, item, { debug: true });
}

/**
 * Run application on the simulator or device without building
 */
async function commonRunCommand(
  context: ExtensionContext,
  item: BuildTreeItem | undefined,
  options: { debug: boolean },
) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);

  const sdk = destination.platform;

  const launchArgs = getWorkspaceConfig("build.launchArgs") ?? [];
  const launchEnv = getWorkspaceConfig("build.launchEnv") ?? {};

  await runTask(context, {
    name: "Run",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      if (destination.type === "macOS") {
        await runOnMac(context, terminal, {
          scheme: scheme,
          xcworkspace: xcworkspace,
          configuration: configuration,
          watchMarker: false,
          launchArgs: launchArgs,
          launchEnv: launchEnv,
        });
      } else if (
        destination.type === "iOSSimulator" ||
        destination.type === "watchOSSimulator" ||
        destination.type === "visionOSSimulator" ||
        destination.type === "tvOSSimulator"
      ) {
        await runOniOSSimulator(context, terminal, {
          scheme: scheme,
          destination: destination,
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
          launchArgs: launchArgs,
          launchEnv: launchEnv,
          debug: options.debug,
        });
      } else if (
        destination.type === "iOSDevice" ||
        destination.type === "watchOSDevice" ||
        destination.type === "tvOSDevice" ||
        destination.type === "visionOSDevice"
      ) {
        await runOniOSDevice(context, terminal, {
          scheme: scheme,
          destination: destination,
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
          launchArgs: launchArgs,
          launchEnv: launchEnv,
        });
      } else {
        assertUnreachable(destination);
      }
    },
  });
}

/**
 * Clean build artifacts
 */
export async function cleanCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to clean", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(context, {
    name: "Clean",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: true,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
        debug: false,
      });
    },
  });
}

export async function testCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ?? (await askSchemeForBuild(context, { title: "Select scheme to test", xcworkspace: xcworkspace }));

  context.updateProgressStatus("Searching for configuration");
  const configuration = await askConfiguration(context, { xcworkspace: xcworkspace });

  context.updateProgressStatus("Extracting build settings");
  const buildSettings = await getBuildSettingsToAskDestination({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  context.updateProgressStatus("Searching for destination");
  const destination = await askDestinationToRunOn(context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(context, {
    name: "Test",
    lock: "sweetpad.build",
    terminateLocked: true,
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: false,
        shouldTest: true,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
        debug: false,
      });
    },
  });
}

export async function resolveDependencies(
  context: ExtensionContext,
  options: {
    scheme: string;
    xcworkspace: string;
  },
): Promise<void> {
  context.updateProgressStatus("Resolving dependencies");

  await runTask(context, {
    name: "Resolve Dependencies",
    lock: "sweetpad.build",
    terminateLocked: true,
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcodebuild",
        args: ["-resolvePackageDependencies", "-scheme", options.scheme, "-workspace", options.xcworkspace],
      });
    },
  });
}

/**
 * Resolve dependencies for the Xcode project
 */
export async function resolveDependenciesCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, {
      title: "Select scheme to resolve dependencies",
      xcworkspace: xcworkspace,
    }));

  await resolveDependencies(context, {
    scheme: scheme,
    xcworkspace: xcworkspace,
  });
}

/**
 * Remove directory with build artifacts.
 *
 * Context: we are storing build artifacts in the `build` directory in the storage path for support xcode-build-server.
 */
export async function removeBundleDirCommand(context: ExtensionContext) {
  context.updateProgressStatus("Removing build artifacts directory");
  const storagePath = await prepareStoragePath(context);
  const bundleDir = path.join(storagePath, "build");

  await removeDirectory(bundleDir);
  vscode.window.showInformationMessage(`Bundle directory was removed: ${bundleDir}`);
}

/**
 * Generate buildServer.json in the workspace root for xcode-build-server —
 * a tool that enable LSP server to see packages from the Xcode project.
 */
export async function generateBuildServerConfigCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting buildServer.json generation");

  const isServerInstalled = await getIsXcodeBuildServerInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("xcode-build-server is not installed");
  }

  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(context, {
      title: "Select scheme for build server",
      xcworkspace: xcworkspace,
    }));

  context.updateProgressStatus("Generating buildServer.json");
  await generateBuildServerConfig({
    xcworkspace: xcworkspace,
    scheme: scheme,
  });
  await restartSwiftLSP();

  vscode.window.showInformationMessage("buildServer.json generated in workspace root", "Open").then((selected) => {
    if (selected === "Open") {
      const workspacePath = getWorkspacePath();
      const buildServerPath = vscode.Uri.file(path.join(workspacePath, "buildServer.json"));
      vscode.commands.executeCommand("vscode.open", buildServerPath);
    }
  });
}

/**
 *
 * Open current project in Xcode
 */
export async function openXcodeCommand(context: ExtensionContext) {
  context.updateProgressStatus("Opening project in Xcode");
  const xcworkspace = await askXcodeWorkspacePath(context);

  await exec({
    command: "open",
    args: [xcworkspace],
  });
}

/**
 * Select Xcode workspace and save it to the workspace state
 */
export async function selectXcodeWorkspaceCommand(context: ExtensionContext) {
  context.updateProgressStatus("Searching for workspace");
  const workspace = await selectXcodeWorkspace({
    autoselect: false,
  });
  const updateAnswer = await showYesNoQuestion({
    title: "Do you want to update path to xcode workspace in the workspace settings (.vscode/settings.json)?",
  });
  if (updateAnswer) {
    const relative = getWorkspaceRelativePath(workspace);
    await updateWorkspaceConfig("build.xcodeWorkspacePath", relative);
    context.updateWorkspaceState("build.xcodeWorkspacePath", undefined);
  } else {
    context.updateWorkspaceState("build.xcodeWorkspacePath", workspace);
  }

  context.buildManager.refreshSchemes();
}

export async function selectXcodeSchemeForBuildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  if (item) {
    item.provider.buildManager.setDefaultSchemeForBuild(item.scheme);
    return;
  }

  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for scheme");
  await askSchemeForBuild(context, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

/**
 * Ask user to select configuration for build and save it to the build manager cache
 */
export async function selectConfigurationForBuildCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(context);

  context.updateProgressStatus("Searching for configurations");
  const configurations = await getBuildConfigurations({
    xcworkspace: xcworkspace,
  });

  let selected: string | undefined;
  if (configurations.length === 0) {
    selected = await showInputBox({
      title: "No configurations found. Please enter configuration name manually",
    });
  } else {
    selected = await showConfigurationPicker(configurations);
  }

  if (!selected) {
    vscode.window.showErrorMessage("Configuration was not selected");
    return;
  }

  const saveAnswer = await showYesNoQuestion({
    title: "Do you want to update configuration in the workspace settings (.vscode/settings.json)?",
  });
  if (saveAnswer) {
    await updateWorkspaceConfig("build.configuration", selected);
    context.buildManager.setDefaultConfigurationForBuild(undefined);
  } else {
    context.buildManager.setDefaultConfigurationForBuild(selected);
  }
}

export async function diagnoseBuildSetupCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Diagnosing build setup");

  await runTask(context, {
    name: "Diagnose Build Setup",
    lock: "sweetpad.build",
    terminateLocked: true,
    callback: async (terminal) => {
      const _write = (message: string) =>
        terminal.write(message, {
          newLine: true,
        });

      const _writeQuote = (message: string) => {
        const splited = message.split("\n");
        for (const line of splited) {
          _write(`   ${line}`);
        }
      };

      _write("SweetPad: Diagnose Build Setup");
      _write("================================");

      const hostPlatform = process.platform;
      _write("🔎 Checking OS");
      if (hostPlatform !== "darwin") {
        _write(
          `❌ Host platform ${hostPlatform} is not supported. This extension depends on Xcode which is available only on macOS`,
        );
        return;
      }
      _write(`✅ Host platform: ${hostPlatform}\n`);
      _write("================================");

      const workspacePath = getWorkspacePath();
      _write("🔎 Checking VS Code workspace path");
      _write(`✅ VSCode workspace path: ${workspacePath}\n`);
      _write("================================");

      const xcWorkspacePath = getCurrentXcodeWorkspacePath(context);
      _write("🔎 Checking current xcode worskpace path");
      _write(`✅ Xcode workspace path: ${xcWorkspacePath ?? "<project-root>"}\n`);
      _write("================================");

      _write("🔎 Getting schemes");
      let schemes: XcodeScheme[] = [];
      try {
        schemes = await context.buildManager.getSchemes({ refresh: true });
      } catch (e) {
        _write("❌ Getting schemes failed");
        if (e instanceof ExecBaseError) {
          const strerr = e.options?.context?.stderr as string | undefined;
          if (
            strerr?.startsWith("xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory")
          ) {
            _write("❌ Xcode build tools are not activated");
            const isXcodeExists = await isFileExists("/Applications/Xcode.app");
            if (!isXcodeExists) {
              _write("❌ Xcode is not installed");
              _write("🌼 Try this:");
              _write("   1. Download Xcode from App Store https://appstore.com/mac/apple/xcode");
              _write("   2. Accept the Terms and Conditions");
              _write("   3. Ensure Xcode app is in the /Applications directory (NOT /Users/{user}/Applications)");
              _write("   4. Run command `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`");
              _write("   5. Restart VS Code");
              _write("🌼 See more: https://stackoverflow.com/a/17980786/7133756");
              return;
            }
            _write("✅ Xcode is installed and located in /Applications/Xcode.app");
            _write("🌼 Try to activate Xcode:");
            _write("   1. Execute this command `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`");
            _write("   2. Restart VS Code");
            _write("🌼 See more: https://stackoverflow.com/a/17980786/7133756\n");
            return;
          }
          if (strerr?.includes("does not contain an Xcode project, workspace or package")) {
            _write("❌ Xcode workspace not found");
            _write("❌ Error message from xcodebuild:");
            _writeQuote(strerr);
            _write(
              "🌼 Check whether your project folder contains folders with the extensions .xcodeproj or .xcworkspace",
            );
            const xcodepaths = await detectXcodeWorkspacesPaths();
            if (xcodepaths.length > 0) {
              _write("✅ Found Xcode and Xcode workspace paths:");
              for (const path of xcodepaths) {
                _write(`   - ${path}`);
              }
            }
            return;
          }
          _write("❌ Error message from xcodebuild:");
          _writeQuote(strerr ?? "Unknown error");
          return;
        }
        _write("❌ Error message from xcodebuild:");
        _writeQuote(e instanceof Error ? e.message : String(e));
        return;
      }
      if (schemes.length === 0) {
        _write("❌ No schemes found");
        return;
      }

      _write(`✅ Found ${schemes.length} schemes\n`);
      _write("   Schemes:");
      for (const scheme of schemes) {
        _write(`   - ${scheme.name}`);
      }
      _write("================================");

      _write("✅ Everything looks good!");
    },
  });
}

export async function refreshSchemesCommand(context: ExtensionContext): Promise<void> {
  const xcworkspace = getCurrentXcodeWorkspacePath(context);

  if (!xcworkspace) {
    // If there is no workspace, we should ask user to select it first.
    // This function automatically refreshes schemes, so we can just call it and move on
    // without calling to refresh schemes manually.
    await askXcodeWorkspacePath(context);
    return;
  }

  await context.buildManager.refreshSchemes();
}
