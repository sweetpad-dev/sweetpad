import path from "node:path";
import * as vscode from "vscode";
import type { BuildTreeItem } from "./tree";

import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import {
  type XcodeScheme,
  generateBuildServerConfig,
  getBuildConfigurations,
  getIsXcodeBuildServerInstalled,
} from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { updateWorkspaceConfig } from "../common/config";
import { ExecBaseError, ExtensionError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceRelativePath, isFileExists, removeDirectory } from "../common/files";
import { showInputBox, showQuickPick } from "../common/quick-pick";
import { runTask } from "../common/tasks";
import {
  askSchemeForBuild,
  askXcodeWorkspacePath,
  detectGitWorktrees,
  detectXcodeWorkspacesPaths,
  findXcodeWorkspaceInDirectory,
  getCurrentXcodeWorkspacePath,
  getWorkspacePath,
  prepareStoragePath,
  restartSwiftLSP,
  selectXcodeWorkspace,
} from "./utils";

/**
 * Build app without running
 */
export async function buildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting build command");
  return context.buildManager.buildCommand(item, { debug: false });
}

/**
 * Build app in debug mode without running
 */
export async function debuggingBuildCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Building the app (debug mode)");
  return context.buildManager.buildCommand(item, { debug: true });
}

/**
 * Build and run application on the simulator or device
 */
export async function launchCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return context.buildManager.launchCommand(item, { debug: false });
}

/**
 * Builds and launches the application in debug mode
 * This is a convenience wrapper around launchCommand that sets the debug flag
 */
export async function debuggingLaunchCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return context.buildManager.launchCommand(item, { debug: true });
}

/**
 * Run application on the simulator or device without building
 */
export async function runCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting run command");
  return context.buildManager.runCommand(item, { debug: false });
}

/**
 * Run application on the simulator or device without building in debug mode
 */
export async function debuggingRunCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Starting debugging command");
  return context.buildManager.runCommand(item, { debug: true });
}

/**
 * Clean build artifacts
 */
export async function cleanCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.buildManager.cleanCommand(item);
}

export async function testCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return context.buildManager.testCommand(item);
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

  context.buildManager.resolveDependenciesCommand({
    xcworkspace: xcworkspace,
    scheme: scheme,
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
    context.buildManager.setDefaultSchemeForBuild(item.scheme);
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

export async function stopSchemeCommand(context: ExtensionContext, item?: BuildTreeItem) {
  return context.buildManager.stopSchemeCommand(item);
}

/**
 * Switch the Xcode workspace to a different git worktree.
 * Detects worktrees via `git worktree list`, finds Xcode projects in each,
 * and lets the user pick which one to build from.
 */
export async function switchWorktreeCommand(context: ExtensionContext) {
  context.updateProgressStatus("Detecting git worktrees");

  const worktrees = await detectGitWorktrees();
  if (worktrees.length <= 1) {
    vscode.window.showInformationMessage("No additional git worktrees found. Create one with `git worktree add`.");
    return;
  }

  const currentWorkspace = getCurrentXcodeWorkspacePath(context);

  type WorktreePickContext = { worktreePath: string; xcworkspace: string };
  const items: { label: string; description: string; detail?: string; context: WorktreePickContext }[] = [];

  for (const wt of worktrees) {
    const xcworkspace = await findXcodeWorkspaceInDirectory(wt.path);
    if (!xcworkspace) {
      continue;
    }

    const isCurrent = currentWorkspace === xcworkspace;
    const dirName = path.basename(wt.path);

    items.push({
      label: `${isCurrent ? "$(check) " : ""}${dirName}`,
      description: wt.branch,
      detail: wt.path,
      context: { worktreePath: wt.path, xcworkspace },
    });
  }

  if (items.length === 0) {
    vscode.window.showWarningMessage("No Xcode projects found in any git worktree.");
    return;
  }

  const selected = await showQuickPick<WorktreePickContext>({
    title: "Select git worktree to build from",
    items,
  });

  const workspacePath = getWorkspacePath();
  const isMainWorkspace = selected.context.worktreePath === workspacePath;

  if (isMainWorkspace) {
    const relative = getWorkspaceRelativePath(selected.context.xcworkspace);
    await updateWorkspaceConfig("build.xcodeWorkspacePath", relative);
  } else {
    await updateWorkspaceConfig("build.xcodeWorkspacePath", selected.context.xcworkspace);
  }

  context.updateWorkspaceState("build.xcodeWorkspacePath", undefined);
  context.buildManager.refreshSchemes();

  const dirName = path.basename(selected.context.worktreePath);
  vscode.window.showInformationMessage(`SweetPad now builds from: ${dirName} (${selected.description ?? ""})`);
}
