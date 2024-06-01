import path from "path";
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
  restartSwiftLSP,
} from "./utils";
import { CommandExecution, ExtensionContext } from "../common/commands";
import { ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { TaskTerminal, runTask } from "../common/tasks";
import { getWorkspaceRelativePath } from "../common/files";
import { showQuickPick } from "../common/quick-pick";

export const DEFAULT_SDK = "iphonesimulator";

export async function runOnDevice(
  context: ExtensionContext,
  terminal: TaskTerminal,
  options: {
    scheme: string;
    simulator: SimulatorOutput;
    sdk: string;
    configuration: string;
  }
) {
  const xcworkspace = await askXcodeWorkspacePath(context);

  const buildSettings = await getBuildSettings({
    scheme: options.scheme,
    configuration: options.configuration,
    sdk: options.sdk,
    xcworkspace: xcworkspace,
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
    args: ["simctl", "launch", "--console-pty", "--terminate-running-process", simulator.udid, bundleIdentifier],
  });
}

function isXcbeautifyEnabled() {
  return getWorkspaceConfig("build.xcbeautifyEnabled") ?? true;
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
    xcworkspace: string;
  }
) {
  const useXcbeatify = isXcbeautifyEnabled() && (await getIsXcbeautifyInstalled());
  const bundleDir = await prepareBundleDir(context, options.scheme);

  const commandParts: string[] = [
    "xcodebuild",
    "-scheme",
    options.scheme,
    "-sdk",
    options.sdk,
    "-configuration",
    options.configuration,
    "-workspace",
    options.xcworkspace,
    "-destination",
    "generic/platform=iOS Simulator",
    "-resultBundlePath",
    bundleDir,
    "-allowProvisioningUpdates",
    ...(options.shouldClean ? ["clean"] : []),
    ...(options.shouldBuild ? ["build"] : []),
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

  await runTask(execution.context, {
    name: "Build",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: DEFAULT_SDK,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        xcworkspace: xcworkspace,
      });
    },
  });
}

/**
 * Build and run application on the simulator
 */
export async function launchCommand(execution: CommandExecution, item?: BuildTreeItem) {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);

  const scheme =
    item?.scheme ?? (await askScheme({ title: "Select scheme to build and run", xcworkspace: xcworkspace }));

  const configuration = await askConfiguration(execution.context, { xcworkspace: xcworkspace });

  // Ask simulator to run on before we start building to not distract user
  // during build command execution
  const simulator = await askSimulatorToRunOn(execution.context);

  await runTask(execution.context, {
    name: "Launch",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: DEFAULT_SDK,
        configuration: configuration,
        shouldBuild: true,
        shouldClean: false,
        xcworkspace: xcworkspace,
      });

      await runOnDevice(execution.context, terminal, {
        scheme: scheme,
        simulator: simulator,
        sdk: DEFAULT_SDK,
        configuration: configuration,
      });
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

  await runTask(execution.context, {
    name: "Clean",
    callback: async (terminal) => {
      await buildApp(execution.context, terminal, {
        scheme: scheme,
        sdk: DEFAULT_SDK,
        configuration: configuration,
        shouldBuild: false,
        shouldClean: true,
        xcworkspace: xcworkspace,
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
  } else {
    execution.context.updateWorkspaceState("build.xcodeWorkspacePath", workspace);
  }
  execution.context.refreshBuildView();
}
