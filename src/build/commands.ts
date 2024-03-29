import path from "path";
import { runShellTask } from "../common/tasks";
import { BuildTreeItem } from "./tree";
import * as vscode from "vscode";

import {
  SimulatorOutput,
  generateBuildServerConfig,
  getBuildSettings,
  getIsXcbeautifyInstalled,
  getIsXcodeBuildServerInstalled,
  getSimulatorByUdid,
  removeDirectory,
} from "../common/cli/scripts";
import {
  askScheme,
  askSimulatorToRunOn,
  askXcodeWorkspacePath,
  prepareBundleDir,
  prepareStoragePath,
  askConfiguration,
  selectXcodeWorkspace,
} from "./utils";
import { CommandExecution, ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { commonLogger } from "../common/logger";
import { exec } from "../common/exec";
import { getWorkspaceConfig } from "../common/config";

export const DEFAULT_SDK = "iphonesimulator";

export async function runOnDevice(
  context: ExtensionContext,
  options: {
    scheme: string;
    simulator: SimulatorOutput;
    sdk: string;
    configuration: string;
  }
) {
  const xcodeWorkspacePath = await askXcodeWorkspacePath(context);

  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcodeWorkspacePath: xcodeWorkspacePath,
  });
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

  const targetPath = path.join(targetBuildDir, appName);

  // Get simulator with fresh state
  const simulator = await getSimulatorByUdid(options.simulator.udid);

  // Boot device
  if (simulator.state !== "Booted") {
    await runShellTask({
      name: "Run",
      command: "xcrun",
      args: ["simctl", "boot", simulator.udid],
      error: "Error booting simulator",
    });

    // Refresh list of simulators after we start new simulator
    // TODO: make it less hacky, but let's keep it for now
    context.refreshSimulators();
  }

  // Install app
  await runShellTask({
    name: "Install",
    command: "xcrun",
    args: ["simctl", "install", simulator.udid, targetPath],
    error: "Error installing app",
  });

  // Open simulatorcte
  await runShellTask({
    name: "Open Simulator",
    command: "open",
    args: ["-a", "Simulator"],
    error: "Could not open simulator app",
  });

  // Run app
  await runShellTask({
    name: "Run",
    command: "xcrun",
    args: ["simctl", "launch", "--console-pty", "--terminate-running-process", simulator.udid, bundleIdentifier],
    error: "Error running app",
  });
}

export function isXcbeautifyEnabled() {
  return getWorkspaceConfig<boolean>("build.xcbeautifyEnabled") ?? true;
}

export async function buildApp(
  context: ExtensionContext,
  options: {
    scheme: string;
    sdk: string;
    configuration: string;
    shouldBuild: boolean;
    shouldClean: boolean;
  }
) {
  const useXcbeatify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
  const bundleDir = await prepareBundleDir(context, options.scheme);

  const xcodeWorkspacePath = await askXcodeWorkspacePath(context);

  const commandParts: string[] = [
    "xcodebuild",
    "-scheme",
    options.scheme,
    "-sdk",
    options.sdk,
    "-configuration",
    options.configuration,
    "-workspace",
    xcodeWorkspacePath,
    "-destination",
    "generic/platform=iOS Simulator",
    "-resultBundlePath",
    bundleDir,
    "-allowProvisioningUpdates",
    ...(options.shouldClean ? ["clean"] : []),
    ...(options.shouldBuild ? ["build"] : []),
  ];

  if (useXcbeatify) {
    commandParts.unshift("set", "-o", "pipefail", "&&");
    commandParts.push("|", "xcbeautify");
  }

  await runShellTask({
    name: "Build",
    command: commandParts[0],
    args: commandParts.slice(1),
    error: "Error building project",
  });

  // Restart SourceKit Language Server
  try {
    await vscode.commands.executeCommand("swift.restartLSPServer");
  } catch (error) {
    commonLogger.warn("Error restarting SourceKit Language Server", {
      error: error,
    });
  }
}

/**
 * Build without running
 */
export async function buildCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to build" }));
  const configuration = await askConfiguration(execution.context);

  await buildApp(execution.context, {
    scheme: scheme,
    sdk: DEFAULT_SDK,
    configuration: configuration,
    shouldBuild: true,
    shouldClean: false,
  });
}

/**
 * Build and run application on the simulator
 */
export async function launchCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to build and run" }));

  const configuration = await askConfiguration(execution.context);

  // Ask simulator to run on before we start building to not distract user
  // during build command execution
  const simulator = await askSimulatorToRunOn(execution.context);

  await buildApp(execution.context, {
    scheme: scheme,
    sdk: DEFAULT_SDK,
    configuration: configuration,
    shouldBuild: true,
    shouldClean: false,
  });

  await runOnDevice(execution.context, {
    scheme: scheme,
    simulator: simulator,

    sdk: DEFAULT_SDK,
    configuration: configuration,
  });
}

/**
 * Clean build artifacts
 */
export async function cleanCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to clean" }));
  const configuration = await askConfiguration(execution.context);
  await buildApp(execution.context, {
    scheme: scheme,
    sdk: DEFAULT_SDK,
    configuration: configuration,
    shouldBuild: false,
    shouldClean: true,
  });
}

export async function resolveDependencies(options: { scheme: string; xcodeWorkspacePath: string }) {
  await runShellTask({
    name: "Resolve Dependencies",
    command: "xcodebuild",
    args: ["-resolvePackageDependencies", "-scheme", options.scheme, "-workspace", options.xcodeWorkspacePath],
    error: "Error resolving dependencies",
  });
}

/**
 * Resolve dependencies for the Xcode project
 */
export async function resolveDependenciesCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const scheme = item?.scheme ?? (await askScheme({ title: "Select scheme to resolve dependencies" }));
  const xcworkspacePath = await askXcodeWorkspacePath(execution.context);

  await resolveDependencies({
    scheme: scheme,
    xcodeWorkspacePath: xcworkspacePath,
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
  vscode.window.showInformationMessage("Bundle directory removed");
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

  const xcodeWorkspacePath = await askXcodeWorkspacePath(execution.context);

  const scheme = await askScheme({
    title: "Select scheme for build server",
  });
  await generateBuildServerConfig({
    xcodeWorkspacePath: xcodeWorkspacePath,
    scheme: scheme,
  });

  vscode.window.showInformationMessage(`buildServer.json generated in workspace root`);
}

/**
 *
 * Open current project in Xcode
 */
export async function openXcodeCommand(execution: CommandExecution) {
  const xcodeWorkspacePath = await askXcodeWorkspacePath(execution.context);

  await exec({
    command: "open",
    args: [xcodeWorkspacePath],
  });
}

/**
 * Select Xcode workspace and save it to the workspace state
 */
export async function selectXcodeWorkspaceCommand(execution: CommandExecution) {
  const workspace = await selectXcodeWorkspace();
  execution.context.updateWorkspaceState("build.xcodeWorkspacePath", workspace);
}
