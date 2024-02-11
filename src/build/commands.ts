import path from "path";
import { runShellTask } from "../common/tasks";
import { BuildTreeItem } from "./tree";
import * as vscode from "vscode";
import { showQuickPick } from "../common/quick-pick";

import {
  SimulatorOutput,
  createDirectory,
  getBuildSettings,
  getIsXcbeautifyInstalled,
  getSimulators,
  removeDirectory,
} from "../common/cli/scripts";
import { askSimulatorToRunOn, getWorkspacePath, prepareBundleDir } from "./utils";

async function runOnDevice(options: { scheme: string; simulator: SimulatorOutput; item: BuildTreeItem }) {
  const workspaceFolder = await getWorkspacePath();

  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    cwd: workspaceFolder,
    configuration: "Debug", // todo: make it configurable
    sdk: "iphonesimulator", // todo: make it configurable
  });
  const settings = buildSettings[0]?.buildSettings;
  if (!settings) {
    vscode.window.showErrorMessage("Error fetching build settings");
    return;
  }

  const bundleIdentifier = settings.PRODUCT_BUNDLE_IDENTIFIER;
  const targetBuildDir = settings.TARGET_BUILD_DIR;
  const targetName = settings.TARGET_NAME;
  const targetPath = path.join(targetBuildDir, `${targetName}.app`);

  let response;
  const simulator = options.simulator;

  // Boot device
  if (simulator.state !== "Booted") {
    response = await runShellTask({
      name: "Run",
      command: "xcrun",
      args: ["simctl", "boot", simulator.udid],
    });
    if (response.type === "error") {
      vscode.window.showErrorMessage("Error running simulator");
    }

    // Refresh list of simulators after we start new simulator
    // TODO: make it less hacky, but let's keep it for now
    options.item.refreshSimulators();
  }

  // Install app
  response = await runShellTask({
    name: "Install",
    command: "xcrun",
    args: ["simctl", "install", simulator.udid, targetPath],
  });

  // Open simulatorcte
  response = await runShellTask({
    name: "Open Simulator",
    command: "open",
    args: ["-a", "Simulator"],
  });
  if (response.type === "error") {
    vscode.window.showErrorMessage("Error opening simulator");
  }

  // Run app
  response = await runShellTask({
    name: "Run",
    command: "xcrun",
    args: ["simctl", "launch", simulator.udid, bundleIdentifier, "--console-pty"],
  });
}

async function buildApp(options: { scheme: string; context: vscode.ExtensionContext }) {
  const isXcbeautifyInstalled = await getIsXcbeautifyInstalled();
  const bundleDir = await prepareBundleDir(options.context, options.scheme);

  let response = await runShellTask({
    name: "Build",
    command: "xcodebuild",
    args: [
      "-scheme",
      options.scheme,
      "-destination",
      "generic/platform=iOS Simulator",
      "-resultBundlePath",
      bundleDir,
      "clean",
      "build",
      ...(isXcbeautifyInstalled ? ["|", "xcbeautify"] : []),
    ],
  });
  if (response.type === "error") {
    vscode.window.showErrorMessage("Error building project");
  }

  // Restart SourceKit Language Server
  await vscode.commands.executeCommand("swift.restartLSPServer");
}

export async function buildCommand(context: vscode.ExtensionContext, item: BuildTreeItem) {
  await buildApp({
    scheme: item.scheme,
    context: context,
  });
}

export async function buildAndRunCommand(context: vscode.ExtensionContext, item: BuildTreeItem) {
  // Ask simulator to run on before we start building to not distract user
  // during build command execution
  const simulator = await askSimulatorToRunOn();

  await buildApp({
    scheme: item.scheme,
    context: context,
  });

  await runOnDevice({
    scheme: item.scheme,
    simulator: simulator,
    item: item,
  });
}

export async function removeBundleDirCommand(context: vscode.ExtensionContext) {}
