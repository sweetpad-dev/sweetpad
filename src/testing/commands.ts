import * as vscode from "vscode";
import type { BuildTreeItem } from "../build/tree";
import { askXcodeWorkspacePath } from "../build/utils";
import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import { getBuildConfigurations } from "../common/cli/scripts";
import type { CommandExecution } from "../common/commands";
import { updateWorkspaceConfig } from "../common/config";
import { showInputBox } from "../common/quick-pick";
import { askSchemeForTesting, askTestingTarget } from "./utils";

export async function selectTestingTargetCommand(execution: CommandExecution): Promise<void> {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  await askTestingTarget(execution.context, {
    title: "Select default testing target",
    xcworkspace: xcworkspace,
    force: true,
  });
}

export async function buildForTestingCommand(execution: CommandExecution): Promise<void> {
  return await execution.context.testingManager.buildForTestingCommand(execution);
}

export async function testWithoutBuildingCommand(
  execution: CommandExecution,
  ...items: vscode.TestItem[]
): Promise<void> {
  const request = new vscode.TestRunRequest(items, [], undefined, undefined);
  const tokenSource = new vscode.CancellationTokenSource();
  execution.context.testingManager.runTestsWithoutBuilding(request, tokenSource.token);
}

export async function selectXcodeSchemeForTestingCommand(execution: CommandExecution, item?: BuildTreeItem) {
  if (item) {
    item.provider.buildManager.setDefaultSchemeForTesting(item.scheme);
    return;
  }

  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  await askSchemeForTesting(execution, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

/**
 * Ask user to select configuration for testing
 */
export async function selectConfigurationForTestingCommand(execution: CommandExecution): Promise<void> {
  const xcworkspace = await askXcodeWorkspacePath(execution.context);
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
    execution.context.buildManager.setDefaultConfigurationForTesting(undefined);
  } else {
    execution.context.buildManager.setDefaultConfigurationForTesting(selected);
  }
}
