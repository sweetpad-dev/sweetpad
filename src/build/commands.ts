import path from "path";
import { BuildTreeItem } from "./tree";
import * as vscode from "vscode";

import {
  generateBuildServerConfig,
  getBuildSettings,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getProductOutputInfoFromBuildSettings,
  getSimulatorByUdid,
  getSupportedPlatforms,
} from "../common/cli/scripts";
import {
  askScheme,
  askXcodeWorkspacePath,
  prepareBundleDir,
  prepareStoragePath,
  askConfiguration,
  selectXcodeWorkspace,
  restartSwiftLSP,
  askDestinationToRunOn,
  prepareDerivedDataPath,
} from "./utils";
import { CommandExecution, ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { TaskTerminal, runTask } from "../common/tasks";
import { getWorkspaceRelativePath, readJsonFile, removeDirectory, tempFilePath } from "../common/files";
import { showQuickPick } from "../common/quick-pick";
import { Platform, getDestinationName, isSimulator } from "../common/destinationTypes";

export async function runOnMac(
  context: ExtensionContext, 
  terminal: TaskTerminal,
  options: { 
    scheme: string; 
    xcworkspace: string;
    configuration: string;
  }) {
  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: Platform.macosx,
    xcworkspace: options.xcworkspace,
  });

  const productOutputInfo = getProductOutputInfoFromBuildSettings(buildSettings);
  await startDebuggingMacApp(productOutputInfo.productName, productOutputInfo.productPath);
}

async function startDebuggingMacApp(appName: string, binaryPath: string) {
  const debugConfig: vscode.DebugConfiguration = {
    type: 'lldb',
    request: 'launch',
    name: `Debug with LLDB (appName)`,
    program: binaryPath,
    args: [],
    cwd: '${workspaceFolder}',
  };

  await vscode.debug.startDebugging(undefined, debugConfig);
}

export async function runOnSimulator(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    simulatorId: string;
    sdk: string;
    configuration: string;
    xcworkspace: string;
  },
) {
  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: options.xcworkspace,
  });
  const settings = buildSettings[0]?.buildSettings;
  if (!settings) {
    throw new ExtensionError("Error fetching build settings");
  }

  const productOutputInfo = getProductOutputInfoFromBuildSettings(buildSettings);
  const targetPath = productOutputInfo.productPath;

  // Get simulator with fresh state
  const simulator = await getSimulatorByUdid(context, {
    udid: options.simulatorId,
    refresh: true,
  });

  // Boot device
  if (simulator.state !== "Booted") {
    await terminal.execute({
      command: "xcrun",
      args: ["simctl", "boot", simulator.udid],
    });

    // Refresh list of simulators after we start new simulator
    context.refreshSimulators();
  }

  // Install app
  await terminal.execute({
    command: "xcrun",
    args: ["simctl", "install", simulator.udid, targetPath],
  });

  // Open simulatorcte
  await terminal.execute({
    command: "open",
    args: ["-a", "Simulator"],
  });

  // Run app
  context.updateSessionState("build.lastLaunchedAppPath", targetPath);

  await terminal.execute({
    command: "xcrun",
    args: ["simctl", "launch", "--console-pty", "--terminate-running-process", simulator.udid, productOutputInfo.bundleIdentifier],
  });
}

export async function runOnDevice(
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

  const productOuputInfo = getProductOutputInfoFromBuildSettings(buildSettings);
  const targetPath = productOuputInfo.productPath;

  // Install app on device
  await terminal.execute({
    command: "xcrun",
    args: ["devicectl", "device", "install", "app", "--device", device, targetPath],
  });

  await using jsonOuputPath = await tempFilePath(context, {
    prefix: "json",
  });

  // Launch app on device
  await terminal.execute({
    command: "xcrun",
    args: [
      "devicectl",
      "device",
      "process",
      "launch",
      "--json-output",
      jsonOuputPath.path,
      "--terminate-existing",
      "--device",
      device,
      productOuputInfo.bundleIdentifier,
    ],
  });

  let jsonOutput;
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
  } else {
    terminal.write("App launched on device with PID: " + jsonOutput.result.process.processIdentifier, {
      newLine: true,
    });
  }
}

function isXcbeautifyEnabled() {
  return getWorkspaceConfig("build.xcbeautifyEnabled") ?? true;
}

function getDestination(options: { platform: Platform; id: string | null }): string {
  const platformName = getDestinationName(options.platform);
  // ?? iPhone device can't be built with id
  if (options.id != null && isSimulator(options.platform)) {
    return `platform=${platformName},id=${options.id}`;
  }

  return `generic/platform=${platformName}`;
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
    destinationType: Platform;
    destinationId: string | null;
  },
) {
  const useXcbeatify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
  const bundlePath = await prepareBundleDir(context, options.scheme);
  const derivedDataPath = prepareDerivedDataPath();
  
  const destination = getDestination({ platform: options.destinationType, id: options.destinationId });

  const commandParts: string[] = [
    "xcodebuild",
    "-scheme",
    options.scheme,
    "-configuration",
    options.configuration,
    "-workspace",
    options.xcworkspace,
    "-destination",
    destination,
    "-resultBundlePath",
    bundlePath,
    "-allowProvisioningUpdates",
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

  await restartSwiftLSP();
}

/**
 * Build app without running
 */
export async function buildCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to build", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const supportedPlatforms = getSupportedPlatforms(buildSettings);
  const destination = await askDestinationToRunOn(execution.context, supportedPlatforms);
  const sdk = destination.getPlatform();

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
        destinationType: sdk,
        destinationId: destination.udid ?? null,
      });
    },
  });
}

/**
 * Build and run application on the simulator or device
 */
export async function launchCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  const scheme =
    item?.scheme ?? (await askScheme({ title: "Select scheme to build and run", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const supportedPlatforms = getSupportedPlatforms(buildSettings);
  const destination = await askDestinationToRunOn(execution.context, supportedPlatforms);
  const sdk = destination.getPlatform();

  await runTask(execution.context, {
    name: "Launch",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationType: sdk,
        destinationId: destination.udid ?? null,
      });

      if (sdk === Platform.macosx) {
        await runOnMac(execution.context, terminal, {
          scheme: scheme,
          xcworkspace: xcworkspace,
          configuration: configuration,
        });
        return;
      }

      if (destination.isSimulator) {
        await runOnSimulator(execution.context, terminal, {
          scheme: scheme,
          simulatorId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
        });
      } else {
        await runOnDevice(execution.context, terminal, {
          scheme: scheme,
          deviceId: destination.udid ?? "",
          sdk: sdk,
          configuration: configuration,
          xcworkspace: xcworkspace,
        });
      }
    },
  });
}

/**
 * Clean build artifacts
 */
export async function cleanCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to clean", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const supportedPlatforms = getSupportedPlatforms(buildSettings);
  const destination = await askDestinationToRunOn(execution.context, supportedPlatforms);
  const sdk = destination.getPlatform();

  await runTask(execution.context, {
    name: "Clean",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: true,
        shouldTest: false,
        xcworkspace: xcworkspace,
        destinationType: sdk,
        destinationId: null,
      });
    },
  });
}

export async function testCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to test", xcworkspace: xcworkspace }));
  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  const buildSettings = await getBuildSettings({
    scheme: scheme,
    configuration: configuration,
    sdk: undefined,
    xcworkspace: xcworkspace,
  });

  const supportedPlatforms = getSupportedPlatforms(buildSettings);
  const destination = await askDestinationToRunOn(execution.context, supportedPlatforms);
  const sdk = destination.getPlatform();

  await runTask(execution.context, {
    name: "Test",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: sdk,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: false,
        shouldTest: true,
        xcworkspace: xcworkspace,
        destinationType: sdk,
        destinationId: destination.udid ?? null,
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
    item?.scheme ?? (await askScheme({ title: "Select scheme to resolve dependencies", xcworkspace: xcworkspace }));

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

  const scheme = await askScheme({
    title: "Select scheme for build server",
    xcworkspace: xcworkspace,
  });
  await generateBuildServerConfig({
    xcworkspace: xcworkspace,
    scheme: scheme,
  });

  await restartSwiftLSP();

  vscode.window.showInformationMessage(`buildServer.json generated in workspace root`);
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
  execution.context.refreshBuildView();
}
