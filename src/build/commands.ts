import path from "node:path";
import * as vscode from "vscode";
import type { BuildTreeItem } from "./tree";

import {
  generateBuildServerConfig,
  getBuildSettings,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getXcodeVersionInstalled,
} from "../common/cli/scripts";
import type { CommandExecution, ExtensionContext } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceRelativePath, readJsonFile, removeDirectory, tempFilePath } from "../common/files";
import { showQuickPick } from "../common/quick-pick";
import { type TaskTerminal, runTask } from "../common/tasks";
import { assertUnreachable } from "../common/types";
import type { Destination } from "../destination/types";
import { getSimulatorByUdid } from "../simulators/utils";
import { DEFAULT_BUILD_PROBLEM_MATCHERS } from "./constants";
import {
  askConfiguration,
  askDestinationToRunOn,
  askScheme,
  askXcodeWorkspacePath,
  prepareBundleDir,
  prepareDerivedDataPath,
  prepareStoragePath,
  restartSwiftLSP,
  selectXcodeWorkspace,
  findEntitlementsFile,
} from "./utils";

function writeWatchMarkers(terminal: TaskTerminal) {
  terminal.write("🍭 Sweetpad: watch marker (start)\n");
  terminal.write("🍩 Sweetpad: watch marker (end)\n\n");
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

  context.updateWorkspaceState("build.lastLaunchedAppPath", buildSettings.executablePath);
  if (options.watchMarker) {
    writeWatchMarkers(terminal);
  }

  await terminal.execute({
    command: buildSettings.executablePath,
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
  const appPath = buildSettings.appPath;
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

  const targetPath = buildSettings.appPath;
  const bundlerId = buildSettings.bundleIdentifier;

  // Install app on device
  await terminal.execute({
    command: "xcrun",
    args: ["devicectl", "device", "install", "app", "--device", device, targetPath],
  });

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

function isXcbeautifyEnabled() {
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
  assertUnreachable(destination);
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
  // ex: ["-arg1", "value1", "-arg2", "value2", "-arg3", "-arg4", "value4"]
  const additionalArgs: string[] = getWorkspaceConfig("build.args") || [];

  const commandParts: string[] = [
    "xcodebuild",
    ...(arch ? [`ARCHS=${arch}`, `VALID_ARCHS=${arch}`, "ONLY_ACTIVE_ARCH=NO"] : []),
    "-scheme",
    options.scheme,
    "-configuration",
    options.configuration,
    "-workspace",
    options.xcworkspace,
    "-destination",
    options.destinationRaw,
    "-resultBundlePath",
    bundlePath,
    "-allowProvisioningUpdates",
    ...additionalArgs,
    ...(derivedDataPath ? ["-derivedDataPath", derivedDataPath] : []),
    ...(options.shouldClean ? ["clean"] : []),
    ...(options.shouldBuild ? ["build"] : []),
    ...(options.shouldTest ? ["test"] : []),
  ];

  const pipes = useXcbeatify ? [{ command: "xcbeautify", args: [], setvbuf: true }] : undefined;

  await terminal.execute({
    command: commandParts[0],
    args: commandParts.slice(1),
    pipes: pipes,
  });

  // Add code signing step
  const enableCodeSigning = getWorkspaceConfig("build.codesign.enabled") ?? false;
  if (enableCodeSigning && options.shouldBuild) {
    await codeSignApp(context, terminal, options);
  }

  await restartSwiftLSP();
}

async function codeSignApp(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    configuration: string;
    xcworkspace: string;
  },
) {
  const enableCodeSigning = getWorkspaceConfig("build.codesign.enabled") ?? false;
  const useHardenedRuntime = getWorkspaceConfig("build.codesign.useHardenedRuntime") ?? false;
  const signingIdentity = getWorkspaceConfig("build.codesign.signingIdentity") ?? "-";
  const useEntitlements = getWorkspaceConfig("build.codesign.useEntitlements") ?? false;

  if (!enableCodeSigning) {
    terminal.write("Code signing is disabled", { newLine: true });
    return;
  }

  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: undefined,
    xcworkspace: options.xcworkspace,
  });

  const appPath = buildSettings.appPath;

  const codesignArgs = [
    "--force",
    "--sign",
    signingIdentity,
  ];

  if (useHardenedRuntime) {
    codesignArgs.push("--options=runtime");
  }

  if (useEntitlements) {
    const entitlementsPath = await findEntitlementsFile(context, buildSettings);
    if (entitlementsPath) {
      codesignArgs.push("--entitlements", entitlementsPath);
    } else {
      terminal.write("Warning: Entitlements file not found", { newLine: true });
    }
  }

  codesignArgs.push(appPath);

  await terminal.execute({
    command: "codesign",
    args: codesignArgs,
  });

  terminal.write(`App successfully code signed${useHardenedRuntime ? " with hardened runtime" : ""}${useEntitlements ? " and entitlements" : ""}`, { newLine: true });
}

/**
 * Build app without running
 */
export async function buildCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme =
    item?.scheme ?? (await askScheme(execution.context, { title: "Select scheme to build", xcworkspace: xcworkspace }));
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
    (await askScheme(execution.context, { title: "Select scheme to build and run", xcworkspace: xcworkspace }));
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
      } else if (destination.type === "iOSSimulator" || destination.type === "watchOSSimulator") {
        await runOniOSSimulator(execution.context, terminal, {
          scheme: scheme,
          simulatorId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
          watchMarker: false,
        });
      } else if (destination.type === "iOSDevice") {
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
    item?.scheme ?? (await askScheme(execution.context, { title: "Select scheme to clean", xcworkspace: xcworkspace }));
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
    item?.scheme ?? (await askScheme(execution.context, { title: "Select scheme to test", xcworkspace: xcworkspace }));
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
    (await askScheme(execution.context, { title: "Select scheme to resolve dependencies", xcworkspace: xcworkspace }));

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
  vscode.window.showInformationMessage("Bundle directory was removed");
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

  const scheme = await askScheme(execution.context, {
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
  const selected = await showQuickPick({
    title: "Do you want to update path to xcode workspace in the workspace settings (.vscode/settings.json)?",
    items: [
      {
        label: "Yes",
        context: {
          answer: true,
        },
      },
      {
        label: "No",
        context: {
          answer: false,
        },
      },
    ],
  });
  if (selected.context.answer) {
    const relative = getWorkspaceRelativePath(workspace);
    await updateWorkspaceConfig("build.xcodeWorkspacePath", relative);
    execution.context.updateWorkspaceState("build.xcodeWorkspacePath", undefined);
  } else {
    execution.context.updateWorkspaceState("build.xcodeWorkspacePath", workspace);
  }

  execution.context.buildManager.refresh();
}

export async function selectXcodeSchemeCommand(execution: CommandExecution, item?: BuildTreeItem) {
  if (item) {
    item.provider.buildManager.setDefaultScheme(item.scheme);
    return;
  }

  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  await askScheme(execution.context, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

export async function codeSignCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme =
    item?.scheme ?? (await askScheme(execution.context, { title: "Select scheme to code sign", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  await runTask(execution.context, {
    name: "Code Sign",
    callback: async (terminal) => {
      await codeSignApp(execution.context, terminal, {
        scheme: scheme,
        configuration: configuration,
        xcworkspace: xcworkspace,
      });
    },
  });
}