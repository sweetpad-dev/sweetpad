import path from "node:path";

import * as vscode from "vscode";

import { getBuildServerProvider } from "../bsp/commands";
import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import {
  type XcodeScheme,
  getBuildConfigurations,
  getIsNodeInstalled,
  getIsXBSInstalled as getIsXBSInstalled,
} from "../common/cli/scripts";
import { type AppDeps, warnNodeRuntimeMissing } from "../common/commands";
import { updateWorkspaceConfig } from "../common/config";
import { ExecBaseError } from "../common/errors";
import { exec } from "../common/exec";
import { getWorkspaceRelativePath, isFileExists, removeDirectory } from "../common/files";
import { showInputBox, showQuickPick } from "../common/quick-pick";
import { runTask } from "../common/tasks/run";
import type { BuildTreeItem } from "./tree";
import {
  askSchemeForBuild,
  askXcodeWorkspacePath,
  detectGitWorktrees,
  detectXcodeWorkspacesPaths,
  findXcodeWorkspaceInDirectory,
  getCurrentXcodeWorkspacePath,
  getWorkspacePath,
  prepareStoragePath,
  refreshBuildServer,
  selectXcodeWorkspace,
  XBSMissingError,
} from "./utils";

/**
 * Build app without running
 */
export async function buildCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Starting build command");
  return deps.buildManager.buildCommand(item, { debug: false });
}

/**
 * Build app in debug mode without running
 */
export async function debuggingBuildCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Building the app (debug mode)");
  return deps.buildManager.buildCommand(item, { debug: true });
}

/**
 * Build and run application on the simulator or device
 */
export async function launchCommand(deps: AppDeps, item?: BuildTreeItem) {
  return deps.buildManager.launchCommand(item, { debug: false });
}

/**
 * Builds and launches the application in debug mode
 * This is a convenience wrapper around launchCommand that sets the debug flag
 */
export async function debuggingLaunchCommand(deps: AppDeps, item?: BuildTreeItem) {
  return deps.buildManager.launchCommand(item, { debug: true });
}

/**
 * Run application on the simulator or device without building
 */
export async function runCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Starting run command");
  return deps.buildManager.runCommand(item, { debug: false });
}

/**
 * Run application on the simulator or device without building in debug mode
 */
export async function debuggingRunCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Starting debugging command");
  return deps.buildManager.runCommand(item, { debug: true });
}

/**
 * Clean build artifacts
 */
export async function cleanCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.buildManager.cleanCommand(item);
}

export async function testCommand(deps: AppDeps, item?: BuildTreeItem) {
  return deps.buildManager.testCommand(item);
}

/**
 * Resolve dependencies for the Xcode project
 */
export async function resolveDependenciesCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);

  deps.progressStatusBar.updateText("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(deps.progressStatusBar, deps.buildManager, {
      title: "Select scheme to resolve dependencies",
      xcworkspace: xcworkspace,
    }));

  deps.buildManager.resolveDependenciesCommand({
    xcworkspace: xcworkspace,
    scheme: scheme,
  });
}

/**
 * Remove directory with build artifacts.
 *
 * Context: we are storing build artifacts in the `build` directory in the storage path for support xcode-build-server.
 */
export async function removeBundleDirCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Removing build artifacts directory");
  const storagePath = await prepareStoragePath(deps.vscodeContext);
  const bundleDir = path.join(storagePath, "build");

  await removeDirectory(bundleDir);
  vscode.window.showInformationMessage(`Bundle directory was removed: ${bundleDir}`);
}

/**
 * Generate buildServer.json in the workspace root for xcode-build-server —
 * a tool that enable LSP server to see packages from the Xcode project.
 */
export async function generateBuildServerConfigCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Starting buildServer.json generation");

  // SweetPad's own provider ships with the extension; only xcode-build-server
  // needs the external tool installed.
  const usingXBS = getBuildServerProvider() === "xcode-build-server";
  if (usingXBS && !(await getIsXBSInstalled())) {
    throw XBSMissingError();
  }

  // SweetPad's own BSP server launches via `#!/usr/bin/env node`. Warn (without
  // blocking) when Node is missing: the config still writes, so it's ready once
  // Node is on PATH, but the server can't start until then.
  if (!usingXBS && !(await getIsNodeInstalled())) {
    void warnNodeRuntimeMissing("The SweetPad BSP server");
  }

  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);

  deps.progressStatusBar.updateText("Searching for scheme");
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(deps.progressStatusBar, deps.buildManager, {
      title: "Select scheme for build server",
      xcworkspace: xcworkspace,
    }));

  deps.progressStatusBar.updateText("Generating buildServer.json");
  // User explicitly invoked this command — always restart, regardless of build.autoRestartSwiftLSP.
  await refreshBuildServer({
    xcworkspace: xcworkspace,
    scheme: scheme,
    forceRestartLSP: true,
  });

  vscode.window.showInformationMessage("buildServer.json generated in workspace root", "Open").then((selected) => {
    if (selected === "Open") {
      const workspacePath = getWorkspacePath();
      const buildServerPath = vscode.Uri.file(path.join(workspacePath, "buildServer.json"));
      vscode.commands.executeCommand("vscode.open", buildServerPath);
    }
  });
}

/**
 * Enable verbose LSP / build-server logging: set env vars on xcode-build-server
 * (XBS_LOGPATH) and sourcekit-lsp (SOURCEKIT_LOGGING=3), regenerate
 * buildServer.json with the env injection so the long-running build server
 * picks it up, restart the Swift LSP, and stream the XBS log file into the
 * "SweetPad: xcode-build-server logs" output channel.
 */
export async function enableLspDiagnosticsCommand(deps: AppDeps, item?: BuildTreeItem) {
  const isServerInstalled = await getIsXBSInstalled();
  if (!isServerInstalled) {
    throw XBSMissingError();
  }

  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(deps.progressStatusBar, deps.buildManager, {
      title: "Select scheme for build server",
      xcworkspace: xcworkspace,
    }));

  await deps.lspDiagnostics.enable();
  await refreshBuildServer({
    xcworkspace: xcworkspace,
    scheme: scheme,
    forceRestartLSP: true,
  });
  // Env-var changes only take effect after a window reload — beat VS Code's
  // own "Changing environment variables requires reload" prompt to it. The
  // success notification is deferred to the next activation; see
  // `LspDiagnosticsService.showPostReloadNotificationIfPending`.
  await vscode.commands.executeCommand("workbench.action.reloadWindow");
}

/**
 * Disable LSP / build-server logging: clear the env-var entries, regenerate
 * buildServer.json so sourcekit-lsp re-spawns xcode-build-server without
 * XBS_LOGPATH, restart the Swift LSP, and stop the log stream.
 */
export async function disableLspDiagnosticsCommand(deps: AppDeps, item?: BuildTreeItem) {
  const isServerInstalled = await getIsXBSInstalled();
  if (!isServerInstalled) {
    throw XBSMissingError();
  }

  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);
  const scheme =
    item?.scheme ??
    (await askSchemeForBuild(deps.progressStatusBar, deps.buildManager, {
      title: "Select scheme for build server",
      xcworkspace: xcworkspace,
    }));

  await deps.lspDiagnostics.disable();
  await refreshBuildServer({
    xcworkspace: xcworkspace,
    scheme: scheme,
    forceRestartLSP: true,
  });
  await vscode.commands.executeCommand("workbench.action.reloadWindow");
}

/**
 * Trigger VS Code's built-in tree find on the Build view. Workaround until
 * `showFindControl` lands in TreeViewOptions (microsoft/vscode#173742).
 */
export async function searchBuildViewCommand(_deps: AppDeps) {
  await vscode.commands.executeCommand("sweetpad.build.view.focus");
  await vscode.commands.executeCommand("list.find");
}

/**
 *
 * Open current project in Xcode
 */
export async function openXcodeCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Opening project in Xcode");
  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);

  await exec({
    command: "open",
    args: [xcworkspace],
  });
}

/**
 * Select Xcode workspace and save it to the workspace state
 */
export async function selectXcodeWorkspaceCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Searching for workspace");
  const workspace = await selectXcodeWorkspace({
    autoselect: false,
  });
  const updateAnswer = await showYesNoQuestion({
    title: "Do you want to update path to xcode workspace in the workspace settings (.vscode/settings.json)?",
  });
  if (updateAnswer) {
    const relative = getWorkspaceRelativePath(workspace);
    await updateWorkspaceConfig("build.xcodeWorkspacePath", relative);
    deps.workspace.update("build.xcodeWorkspacePath", undefined);
  } else {
    deps.workspace.update("build.xcodeWorkspacePath", workspace);
  }

  deps.buildManager.refreshSchemes();
}

export async function selectXcodeSchemeForBuildCommand(deps: AppDeps, item?: BuildTreeItem) {
  if (item) {
    deps.buildManager.setDefaultSchemeForBuild(item.scheme);
    return;
  }

  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);

  deps.progressStatusBar.updateText("Searching for scheme");
  await askSchemeForBuild(deps.progressStatusBar, deps.buildManager, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

/**
 * Ask user to select configuration for build and save it to the build manager cache
 */
export async function selectConfigurationForBuildCommand(deps: AppDeps): Promise<void> {
  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath(deps.workspace, deps.buildManager);

  deps.progressStatusBar.updateText("Searching for configurations");
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
    deps.buildManager.setDefaultConfigurationForBuild(undefined);
  } else {
    deps.buildManager.setDefaultConfigurationForBuild(selected);
  }
}

export async function diagnoseBuildSetupCommand(deps: AppDeps): Promise<void> {
  deps.progressStatusBar.updateText("Diagnosing build setup");

  await runTask(deps.execution, {
    name: "Diagnose Build Setup",
    lock: "sweetpad.build",
    terminateLocked: true,
    callback: async (terminal) => {
      const diagWrite = (message: string) =>
        terminal.write(message, {
          newLine: true,
        });

      const diagWriteQuote = (message: string) => {
        const splited = message.split("\n");
        for (const line of splited) {
          diagWrite(`   ${line}`);
        }
      };

      diagWrite("SweetPad: Diagnose Build Setup");
      diagWrite("================================");

      const hostPlatform = process.platform;
      diagWrite("🔎 Checking OS");
      if (hostPlatform !== "darwin") {
        diagWrite(
          `❌ Host platform ${hostPlatform} is not supported. This extension depends on Xcode which is available only on macOS`,
        );
        return;
      }
      diagWrite(`✅ Host platform: ${hostPlatform}\n`);
      diagWrite("================================");

      const workspacePath = getWorkspacePath();
      diagWrite("🔎 Checking VS Code workspace path");
      diagWrite(`✅ VSCode workspace path: ${workspacePath}\n`);
      diagWrite("================================");

      const xcWorkspacePath = getCurrentXcodeWorkspacePath(deps.workspace);
      diagWrite("🔎 Checking current xcode worskpace path");
      diagWrite(`✅ Xcode workspace path: ${xcWorkspacePath ?? "<project-root>"}\n`);
      diagWrite("================================");

      diagWrite("🔎 Getting schemes");
      let schemes: XcodeScheme[] = [];
      try {
        schemes = await deps.buildManager.getSchemes({ refresh: true });
      } catch (e) {
        diagWrite("❌ Getting schemes failed");
        if (e instanceof ExecBaseError) {
          const strerr = e.options?.context?.stderr as string | undefined;
          if (
            strerr?.startsWith("xcode-select: error: tool 'xcodebuild' requires Xcode, but active developer directory")
          ) {
            diagWrite("❌ Xcode build tools are not activated");
            const isXcodeExists = await isFileExists("/Applications/Xcode.app");
            if (!isXcodeExists) {
              diagWrite("❌ Xcode is not installed");
              diagWrite("🌼 Try this:");
              diagWrite("   1. Download Xcode from App Store https://appstore.com/mac/apple/xcode");
              diagWrite("   2. Accept the Terms and Conditions");
              diagWrite("   3. Ensure Xcode app is in the /Applications directory (NOT /Users/{user}/Applications)");
              diagWrite("   4. Run command `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`");
              diagWrite("   5. Restart VS Code");
              diagWrite("🌼 See more: https://stackoverflow.com/a/17980786/7133756");
              return;
            }
            diagWrite("✅ Xcode is installed and located in /Applications/Xcode.app");
            diagWrite("🌼 Try to activate Xcode:");
            diagWrite("   1. Execute this command `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`");
            diagWrite("   2. Restart VS Code");
            diagWrite("🌼 See more: https://stackoverflow.com/a/17980786/7133756\n");
            return;
          }
          if (strerr?.includes("does not contain an Xcode project, workspace or package")) {
            diagWrite("❌ Xcode workspace not found");
            diagWrite("❌ Error message from xcodebuild:");
            diagWriteQuote(strerr);
            diagWrite(
              "🌼 Check whether your project folder contains folders with the extensions .xcodeproj or .xcworkspace",
            );
            const xcodepaths = await detectXcodeWorkspacesPaths();
            if (xcodepaths.length > 0) {
              diagWrite("✅ Found Xcode and Xcode workspace paths:");
              for (const xcodePath of xcodepaths) {
                diagWrite(`   - ${xcodePath}`);
              }
            }
            return;
          }
          diagWrite("❌ Error message from xcodebuild:");
          diagWriteQuote(strerr ?? "Unknown error");
          return;
        }
        diagWrite("❌ Error message from xcodebuild:");
        diagWriteQuote(e instanceof Error ? e.message : String(e));
        return;
      }
      if (schemes.length === 0) {
        diagWrite("❌ No schemes found");
        return;
      }

      diagWrite(`✅ Found ${schemes.length} schemes\n`);
      diagWrite("   Schemes:");
      for (const scheme of schemes) {
        diagWrite(`   - ${scheme.name}`);
      }
      diagWrite("================================");

      diagWrite("✅ Everything looks good!");
    },
  });
}

export async function pauseSchemeFilterCommand(deps: AppDeps): Promise<void> {
  deps.buildTreeProvider?.toggleSchemeFilterPaused(true);
}

export async function applySchemeFilterCommand(deps: AppDeps): Promise<void> {
  deps.buildTreeProvider?.toggleSchemeFilterPaused(false);
}

export async function refreshSchemesCommand(deps: AppDeps): Promise<void> {
  const xcworkspace = getCurrentXcodeWorkspacePath(deps.workspace);

  if (!xcworkspace) {
    // If there is no workspace, we should ask user to select it first.
    // This function automatically refreshes schemes, so we can just call it and move on
    // without calling to refresh schemes manually.
    await askXcodeWorkspacePath(deps.workspace, deps.buildManager);
    return;
  }

  await deps.buildManager.refreshSchemes();
}

export async function stopSchemeCommand(deps: AppDeps, item?: BuildTreeItem) {
  return deps.buildManager.stopSchemeCommand(item);
}

/**
 * Switch the Xcode workspace to a different git worktree.
 * Detects worktrees via `git worktree list`, finds Xcode projects in each,
 * and lets the user pick which one to build from.
 */
export async function switchWorktreeCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Detecting git worktrees");

  const worktrees = await detectGitWorktrees();
  if (worktrees.length <= 1) {
    vscode.window.showInformationMessage("No additional git worktrees found. Create one with `git worktree add`.");
    return;
  }

  const currentWorkspace = getCurrentXcodeWorkspacePath(deps.workspace);

  type WorktreePickContext = { worktreePath: string; xcworkspace: string };
  const items: { label: string; description: string; detail?: string; context: WorktreePickContext }[] = [];

  for (const wt of worktrees) {
    const xcworkspace = await findXcodeWorkspaceInDirectory(wt.path);
    if (!xcworkspace) {
      continue;
    }

    const isCurrent = currentWorkspace !== undefined && path.resolve(currentWorkspace) === path.resolve(xcworkspace);
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

  const relative = getWorkspaceRelativePath(selected.context.xcworkspace);
  await updateWorkspaceConfig("build.xcodeWorkspacePath", relative);

  deps.workspace.update("build.xcodeWorkspacePath", undefined);
  deps.buildManager.refreshSchemes();

  const dirName = path.basename(selected.context.worktreePath);
  vscode.window.showInformationMessage(`SweetPad now builds from: ${dirName} (${selected.description ?? ""})`);
}
