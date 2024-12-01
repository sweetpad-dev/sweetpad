import path from "node:path";
import * as vscode from "vscode";
import type { BuildTreeItem } from "./tree";

import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import {
  generateBuildServerConfig,
  getBuildConfigurations,
  getBuildSettings,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import type { CommandExecution, ExtensionContext } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceRelativePath, isFileExists, readJsonFile, removeDirectory, tempFilePath } from "../common/files";
import { commonLogger } from "../common/logger";
import { showInputBox } from "../common/quick-pick";
import { type Command, type TaskTerminal, runTask } from "../common/tasks";
import { assertUnreachable } from "../common/types";
import type { Destination } from "../destination/types";
import { getSimulatorByUdid } from "../simulators/utils";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  prepareBundleDir,
  prepareDerivedDataPath,
  prepareStoragePath,
  restartSwiftLSP,
  selectXcodeWorkspace,
} from "./utils";

function writeWatchMarkers(terminal: TaskTerminal) {
  terminal.write("🍭 Sweetpad: watch marker (start)\n");
  terminal.write("🍩 Sweetpad: watch marker (end)\n\n");
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
  },
) {
  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: "macosx",
    xcworkspace: options.xcworkspace,
  });

  const executablePath = await ensureAppPathExists(buildSettings.executablePath);

  context.updateWorkspaceState("build.lastLaunchedAppPath", executablePath);
  if (options.watchMarker) {
    writeWatchMarkers(terminal);
  }

  await terminal.execute({
    command: executablePath,
  });
}

export async function runOniOSSimulator(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    simulatorId: string;
    sdk: string;
    configuration: string;
    xcworkspace: string;
    watchMarker: boolean;
  },
) {
  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: options.xcworkspace,
  });
  const appPath = await ensureAppPathExists(buildSettings.appPath);
  const bundlerId = buildSettings.bundleIdentifier;

  // Open simulator
  await terminal.execute({
    command: "open",
    args: ["-a", "Simulator"],
  });

  // Get simulator with fresh state
  const simulator = await getSimulatorByUdid(context, {
    udid: options.simulatorId,
  });

  // Boot device
  if (!simulator.isBooted) {
    await terminal.execute({
      command: "xcrun",
      args: ["simctl", "boot", simulator.udid],
    });

    // Refresh list of simulators after we start new simulator
    context.destinationsManager.refreshSimulators();
  }

  // Install app
  await terminal.execute({
    command: "xcrun",
    args: ["simctl", "install", simulator.udid, appPath],
  });

  context.updateWorkspaceState("build.lastLaunchedAppPath", appPath);

  if (options.watchMarker) {
    writeWatchMarkers(terminal);
  }

  // Run app
  await terminal.execute({
    command: "xcrun",
    args: ["simctl", "launch", "--console-pty", "--terminate-running-process", simulator.udid, bundlerId],
  });
}

export async function runOniOSDevice(
  context: ExtensionContext,
  terminal: TaskTerminal,
  option: {
    scheme: string;
    configuration: string;
    deviceId: string;
    sdk: string;
    xcworkspace: string;
  },
) {
  const { scheme, configuration, deviceId: device } = option;

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: option.sdk,
    xcworkspace: option.xcworkspace,
  });

  const targetPath = await ensureAppPathExists(buildSettings.appPath);
  const bundlerId = buildSettings.bundleIdentifier;

  // Install app on device
  await terminal.execute({
    command: "xcrun",
    args: ["devicectl", "device", "install", "app", "--device", device, targetPath],
  });

  context.updateWorkspaceState("build.lastLaunchedAppPath", targetPath);

  await using jsonOuputPath = await tempFilePath(context, {
    prefix: "json",
  });

  const xcodeVersion = await getXcodeVersionInstalled();
  const isConsoleOptionSupported = xcodeVersion.major >= 16;

  // Launch app on device
  await terminal.execute({
    command: "xcrun",
    args: [
      "devicectl",
      "device",
      "process",
      "launch",
      isConsoleOptionSupported ? "--console" : null,
      "--json-output",
      jsonOuputPath.path,
      "--terminate-existing",
      "--device",
      device,
      bundlerId,
    ],
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
 * Prepare and return destination string for xcodebuild command.
 *
 * WARN: Do not use result of this function to anything else than xcodebuild command.
 */
export function getXcodeBuildDestinationString(options: { destination: Destination }): string {
  const destination = options.destination;

  if (destination.type === "macOS") {
    // note: without arch, xcodebuild will show warning like this:
    // --- xcodebuild: WARNING: Using the first of multiple matching destinations:
    // { platform:macOS, arch:arm64, id:00008103-000109910EC3001E, name:My Mac }
    // { platform:macOS, arch:x86_64, id:00008103-000109910EC3001E, name:My Mac }
    return `platform=macOS,arch=${destination.arch}`;
  }
  if (destination.type === "iOSSimulator") {
    return `platform=iOS Simulator,id=${destination.udid}`;
  }
  if (destination.type === "iOSDevice") {
    return `platform=iOS,id=${destination.udid}`;
  }
  if (destination.type === "watchOSSimulator") {
    return `platform=watchOS Simulator,id=${destination.udid}`;
  }
  if (destination.type === "tvOSSimulator") {
    return `platform=tvOS Simulator,id=${destination.udid}`;
  }
  if (destination.type === "visionOSSimulator") {
    return `platform=visionOS Simulator,id=${destination.udid}`;
  }
  if (destination.type === "watchOSDevice") {
    return `platform=watchOS,id=${destination.udid}`;
  }
  if (destination.type === "visionOSDevice") {
    return `platform=visionOS,id=${destination.udid}`;
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
  },
) {
  const useXcbeatify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
  const bundlePath = await prepareBundleDir(context, options.scheme);
  const derivedDataPath = prepareDerivedDataPath();

  const arch = getWorkspaceConfig("build.arch") || undefined;
  const allowProvisioningUpdates = getWorkspaceConfig("build.allowProvisioningUpdates") ?? true;

  // ex: ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
  const additionalArgs: string[] = getWorkspaceConfig("build.args") || [];

  const command = new XcodeCommandBuilder();
  if (arch) {
    command.addBuildSettings("ARCHS", arch);
    command.addBuildSettings("VALID_ARCHS", arch);
    command.addBuildSettings("ONLY_ACTIVE_ARCH", "NO");
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
    const isSetvbufEnabled = getWorkspaceConfig("system.enableSetvbuf") ?? false;
    pipes = [{ command: "xcbeautify", args: [], setvbuf: isSetvbufEnabled }];
  }

  await terminal.execute({
    command: commandParts[0],
    args: commandParts.slice(1),
    pipes: pipes,
  });

  await restartSwiftLSP();
}

/**
 * Build app without running
 */
export async function buildCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(execution.context, { title: "Select scheme to build", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(execution.context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(execution.context, {
    name: "Build",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
      });
    },
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
  });
}

/**
 * Build and run application on the simulator or device
 */
export async function launchCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(execution.context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(execution.context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(execution.context, {
    name: "Launch",
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
      });

      if (destination.type === "macOS") {
        await runOnMac(execution.context, terminal, {
          scheme: scheme,
          xcworkspace: xcworkspace,
          configuration: configuration,
          watchMarker: false,
        });
      } else if (
        destination.type === "iOSSimulator" ||
        destination.type === "watchOSSimulator" ||
        destination.type === "visionOSSimulator" ||
        destination.type === "tvOSSimulator"
      ) {
        await runOniOSSimulator(execution.context, terminal, {
          scheme: scheme,
          simulatorId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
        });
      } else if (
        destination.type === "iOSDevice" ||
        destination.type === "watchOSDevice" ||
        destination.type === "visionOSDevice"
      ) {
        await runOniOSDevice(execution.context, terminal, {
          scheme: scheme,
          deviceId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
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
export async function runCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(execution.context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(execution.context, buildSettings);

  const sdk = destination.platform;

  await runTask(execution.context, {
    name: "Run",
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      if (destination.type === "macOS") {
        await runOnMac(execution.context, terminal, {
          scheme: scheme,
          xcworkspace: xcworkspace,
          configuration: configuration,
          watchMarker: false,
        });
      } else if (
        destination.type === "iOSSimulator" ||
        destination.type === "watchOSSimulator" ||
        destination.type === "visionOSSimulator" ||
        destination.type === "tvOSSimulator"
      ) {
        await runOniOSSimulator(execution.context, terminal, {
          scheme: scheme,
          simulatorId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
        });
      } else if (
        destination.type === "iOSDevice" ||
        destination.type === "watchOSDevice" ||
        destination.type === "visionOSDevice"
      ) {
        await runOniOSDevice(execution.context, terminal, {
          scheme: scheme,
          deviceId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
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
export async function cleanCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(execution.context, { title: "Select scheme to clean", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(execution.context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(execution.context, {
    name: "Clean",
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: true,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
      });
    },
  });
}

export async function testCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(execution.context, { title: "Select scheme to test", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const destination = await askDestinationToRunOn(execution.context, buildSettings);
  const destinationRaw = getXcodeBuildDestinationString({ destination: destination });

  const sdk = destination.platform;

  await runTask(execution.context, {
    name: "Test",
    problemMatchers: DEFAULT_BUILD_PROBLEM_MATCHERS,
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: false,
        shouldTest: true,
        xcworkspace: xcworkspace,
        destinationRaw: destinationRaw,
      });
    },
  });
}

export async function resolveDependencies(context: ExtensionContext, options: { scheme: string; xcworkspace: string }) {
  await runTask(context, {
    name: "Resolve Dependencies",
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
export async function resolveDependenciesCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(execution.context, {
      title: "Select scheme to resolve dependencies",
      xcworkspace: xcworkspace,
    }));

  await resolveDependencies(execution.context, {
    scheme: scheme,
    xcworkspace: xcworkspace,
  });
}

/**
 * Remove directory with build artifacts.
 *
 * Context: we are storing build artifacts in the `build` directory in the storage path for support xcode-build-server.
 */
export async function removeBundleDirCommand(execution: CommandExecution) {
  const storagePath = await prepareStoragePath(execution.context);
  const bundleDir = path.join(storagePath, "build");

  await removeDirectory(bundleDir);
  vscode.window.showInformationMessage(`Bundle directory was removed: ${bundleDir}`);
}

/**
 * Generate buildServer.json in the workspace root for xcode-build-server —
 * a tool that enable LSP server to see packages from the Xcode project.
 */
export async function generateBuildServerConfigCommand(execution: CommandExecution) {
  const isServerInstalled = await getIsXcodeBuildServerInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("xcode-build-server is not installed");
  }

  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  const scheme = await askSchemeForBuild(execution.context, {
    title: "Select scheme for build server",
    xcworkspace: xcworkspace,
  });
  await generateBuildServerConfig({
    xcworkspace: xcworkspace,
    scheme: scheme,
  });
  await restartSwiftLSP();

  vscode.window.showInformationMessage("buildServer.json generated in workspace root");
}

/**
 *
 * Open current project in Xcode
 */
export async function openXcodeCommand(execution: CommandExecution) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  await exec({
    command: "open",
    args: [xcworkspace],
  });
}

/**
 * Select Xcode workspace and save it to the workspace state
 */
export async function selectXcodeWorkspaceCommand(execution: CommandExecution) {
  const workspace = await selectXcodeWorkspace({
    autoselect: false,
  });
  const updateAnswer = await showYesNoQuestion({
    title: "Do you want to update path to xcode workspace in the workspace settings (.vscode/settings.json)?",
  });
  if (updateAnswer) {
    const relative = getWorkspaceRelativePath(workspace);
    await updateWorkspaceConfig("build.xcodeWorkspacePath", relative);
    execution.context.updateWorkspaceState("build.xcodeWorkspacePath", undefined);
  } else {
    execution.context.updateWorkspaceState("build.xcodeWorkspacePath", workspace);
  }

  execution.context.buildManager.refresh();
}

export async function selectXcodeSchemeForBuildCommand(execution: CommandExecution, item?: BuildTreeItem) {
  if (item) {
    item.provider.buildManager.setDefaultSchemeForBuild(item.scheme);
    return;
  }

  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  await askSchemeForBuild(execution.context, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

/**
 * Ask user to select configuration for build and save it to the build manager cache
 */
export async function selectConfigurationForBuildCommand(execution: CommandExecution): Promise<void> {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
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
    execution.context.buildManager.setDefaultConfigurationForBuild(undefined);
  } else {
    execution.context.buildManager.setDefaultConfigurationForBuild(selected);
  }
}
