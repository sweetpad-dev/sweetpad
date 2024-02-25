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
  getXcodeProjectPath,
  removeDirectory,
} from "../common/cli/scripts";
import { askScheme, askSimulatorToRunOn, getWorkspacePath, prepareBundleDir, prepareStoragePath } from "./utils";
import { CommandExecution } from "../common/commands";
import { ExtensionError } from "../common/errors";

async function runOnDevice(options: { scheme: string; simulator: SimulatorOutput; item: BuildTreeItem }) {
  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: "Debug", // todo: make it configurable
    sdk: "iphonesimulator", // todo: make it configurable
  });
  const settings = buildSettings[0]?.buildSettings;
  if (!settings) {
    throw new ExtensionError("Error fetching build settings");
  }

  const bundleIdentifier = settings.PRODUCT_BUNDLE_IDENTIFIER;
  const targetBuildDir = settings.TARGET_BUILD_DIR;
  const targetName = settings.TARGET_NAME;
  const targetPath = path.join(targetBuildDir, `${targetName}.app`);

  const simulator = options.simulator;

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
    options.item.refreshSimulators();
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
    args: ["simctl", "launch", simulator.udid, bundleIdentifier, "--console-pty"],
    error: "Error running app",
  });
}

async function buildApp(options: {
  scheme: string;
  execution: CommandExecution;
  shouldBuild: boolean;
  shouldClean: boolean;
}) {
  const isXcbeautifyInstalled = await getIsXcbeautifyInstalled();
  const bundleDir = await prepareBundleDir(options.execution, options.scheme);

  await runShellTask({
    name: "Build",
    command: "xcodebuild",
    args: [
      "-scheme",
      options.scheme,
      "-destination",
      "generic/platform=iOS Simulator",
      "-resultBundlePath",
      bundleDir,
      "-allowProvisioningUpdates",
      ...(options.shouldClean ? ["clean"] : []),
      ...(options.shouldBuild ? ["build"] : []),
      ...(isXcbeautifyInstalled ? ["|", "xcbeautify"] : []),
    ],
    error: "Error building project",
  });

  // Restart SourceKit Language Server
  await vscode.commands.executeCommand("swift.restartLSPServer");
}

export async function buildCommand(execution: CommandExecution, item: BuildTreeItem) {
  await buildApp({
    scheme: item.scheme,
    execution: execution,
    shouldBuild: true,
    shouldClean: false,
  });
}

export async function buildAndRunCommand(execution: CommandExecution, item: BuildTreeItem) {
  // Ask simulator to run on before we start building to not distract user
  // during build command execution
  const simulator = await askSimulatorToRunOn();

  await buildApp({
    scheme: item.scheme,
    execution: execution,
    shouldBuild: true,
    shouldClean: false,
  });

  await runOnDevice({
    scheme: item.scheme,
    simulator: simulator,
    item: item,
  });
}

export async function cleanCommand(execution: CommandExecution, item: BuildTreeItem) {
  await buildApp({
    scheme: item.scheme,
    execution: execution,
    shouldBuild: false,
    shouldClean: true,
  });
}

export async function resolveDependenciesCommand(execution: CommandExecution, item: BuildTreeItem) {
  await runShellTask({
    name: "Resolve Dependencies",
    command: "xcodebuild",
    args: ["-resolvePackageDependencies", "-scheme", item.scheme],
    error: "Error resolving dependencies",
  });
}

export async function removeBundleDirCommand(execution: CommandExecution) {
  const storagePath = await prepareStoragePath(execution);
  const bundleDir = path.join(storagePath, "build");

  await removeDirectory(bundleDir);
  vscode.window.showInformationMessage("Bundle directory removed");
}

export async function generateBuildServerConfigCommand(execution: CommandExecution) {
  const workspacePath = getWorkspacePath();
  const isServerInstalled = await getIsXcodeBuildServerInstalled();
  if (!isServerInstalled) {
    throw new ExtensionError("xcode-build-server is not installed");
  }

  const projPath = await getXcodeProjectPath({
    cwd: workspacePath,
  });

  const scheme = await askScheme({
    title: "Select scheme for build server",
  });
  await generateBuildServerConfig({
    projectPath: projPath,
    scheme: scheme,
  });

  vscode.window.showInformationMessage(`buildServer.json generated in workspace root`);
}
