import * as vscode from "vscode";

import type { BuildTreeItem } from "../build/tree";
import { askXcodeWorkspacePath } from "../build/utils";
import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import { getBuildConfigurations } from "../common/cli/scripts";
import type { AppDeps } from "../common/commands";
import { updateWorkspaceConfig } from "../common/config";
import { showInputBox } from "../common/quick-pick";
import { askSchemeForTesting, askTestingTarget } from "./utils";

export async function selectTestingTargetCommand(deps: AppDeps): Promise<void> {
  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath({
    workspaceState: deps.workspaceState,
    buildManager: deps.buildManager,
  });

  deps.progressStatusBar.updateText("Selecting testing target");
  await askTestingTarget(deps.testingManager, {
    title: "Select default testing target",
    xcworkspace: xcworkspace,
    force: true,
  });
}

export async function buildForTestingCommand(deps: AppDeps): Promise<void> {
  deps.progressStatusBar.updateText("Building for testing");
  return await deps.testingManager.buildForTestingCommand();
}

export async function testWithoutBuildingCommand(deps: AppDeps, ...items: vscode.TestItem[]): Promise<void> {
  deps.progressStatusBar.updateText("Running tests without building");
  const request = new vscode.TestRunRequest(items, [], undefined, undefined);
  const tokenSource = new vscode.CancellationTokenSource();
  await deps.testingManager.runTestsWithoutBuilding(request, tokenSource.token);
}

export async function selectXcodeSchemeForTestingCommand(deps: AppDeps, item?: BuildTreeItem) {
  deps.progressStatusBar.updateText("Selecting scheme for testing");

  if (item) {
    deps.buildManager.setDefaultSchemeForTesting(item.scheme);
    return;
  }

  const xcworkspace = await askXcodeWorkspacePath({
    workspaceState: deps.workspaceState,
    buildManager: deps.buildManager,
  });
  await askSchemeForTesting(deps.progressStatusBar, deps.buildManager, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

/**
 * Ask user to select configuration for testing
 */
export async function selectConfigurationForTestingCommand(deps: AppDeps): Promise<void> {
  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath({
    workspaceState: deps.workspaceState,
    buildManager: deps.buildManager,
  });

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
    await updateWorkspaceConfig("testing.configuration", selected);
    deps.buildManager.setDefaultConfigurationForTesting(undefined);
  } else {
    deps.buildManager.setDefaultConfigurationForTesting(selected);
  }
}
