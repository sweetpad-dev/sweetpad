import * as vscode from "vscode";

import { getPickedConfiguration } from "../build/configuration";
import type { BuildTreeItem } from "../build/tree";
import { askXcodeWorkspacePath } from "../build/utils";
import { askConfigurationForScheme, showYesNoQuestion } from "../common/askers";
import { getSchemeConfiguration } from "../common/cli/scripts";
import type { AppDeps } from "../common/commands";
import { getWorkspaceConfig, updateWorkspaceConfig } from "../common/config";
import { askSchemeForTesting, askTestingTarget } from "./utils";

export async function selectTestingTargetCommand(deps: AppDeps): Promise<void> {
  deps.progressStatusBar.updateText("Searching for workspace");
  const xcworkspace = await askXcodeWorkspacePath({
    workspaceState: deps.workspaceState,
    workspaceContext: deps.workspaceContext,
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
    workspaceContext: deps.workspaceContext,
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
    workspaceContext: deps.workspaceContext,
    buildManager: deps.buildManager,
  });
  const scheme = await askSchemeForTesting(deps.progressStatusBar, deps.buildManager, {
    title: "Select scheme to pick a configuration for",
    xcworkspace: xcworkspace,
  });

  deps.progressStatusBar.updateText("Searching for configurations");
  const answer = await askConfigurationForScheme({
    xcworkspace: xcworkspace,
    scheme: scheme,
    schemeDefault: getSchemeConfiguration({ xcworkspace: xcworkspace, scheme: scheme, action: "test" }),
    current:
      getWorkspaceConfig("testing.configuration") ??
      getPickedConfiguration({ workspaceState: deps.workspaceState, scope: "testing", xcworkspace, scheme }),
  });
  if (!answer) {
    vscode.window.showErrorMessage("Configuration was not selected");
    return;
  }

  // The scheme's own configuration is what applies without a pick, so going back to it drops the pick.
  if (answer.configuration === undefined) {
    deps.buildManager.setConfigurationForTesting({ xcworkspace, scheme, configuration: undefined });
    return;
  }

  const saveAnswer = await showYesNoQuestion({
    title: "Do you want to update configuration in the workspace settings (.vscode/settings.json)?",
  });
  if (saveAnswer) {
    await updateWorkspaceConfig("testing.configuration", answer.configuration);
    deps.buildManager.setConfigurationForTesting({ xcworkspace, scheme, configuration: undefined });
  } else {
    deps.buildManager.setConfigurationForTesting({ xcworkspace, scheme, configuration: answer.configuration });
  }
}
