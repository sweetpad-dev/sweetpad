import * as vscode from "vscode";
import type { BuildTreeItem } from "../build/tree";
import { askXcodeWorkspace } from "../build/utils";
import { showConfigurationPicker, showYesNoQuestion } from "../common/askers";
import { getBuildConfigurations } from "../common/cli/scripts";
import { updateWorkspaceConfig } from "../common/config";
import type { ExtensionContext } from "../common/context";
import { showInputBox } from "../common/quick-pick";
import { askSchemeForTesting, askTestingTarget } from "./utils";

export async function selectTestingTargetCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspace(context);

  context.updateProgressStatus("Selecting testing target");
  await askTestingTarget(context, {
    title: "Select default testing target",
    xcworkspace: xcworkspace,
    force: true,
  });
}

export async function buildForTestingCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Building for testing");
  return await context.testingManager.buildForTestingCommand(context);
}

export async function testWithoutBuildingCommand(
  context: ExtensionContext,
  ...items: vscode.TestItem[]
): Promise<void> {
  context.updateProgressStatus("Running tests without building");
  const request = new vscode.TestRunRequest(items, [], undefined, undefined);
  const tokenSource = new vscode.CancellationTokenSource();
  await context.testingManager.runTestsWithoutBuilding(request, tokenSource.token);
}

export async function selectXcodeSchemeForTestingCommand(context: ExtensionContext, item?: BuildTreeItem) {
  context.updateProgressStatus("Selecting scheme for testing");

  if (item) {
    item.provider.buildManager.setDefaultSchemeForTesting(item.scheme);
    return;
  }

  const xcworkspace = await askXcodeWorkspace(context);
  await askSchemeForTesting(context, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}

/**
 * Ask user to select configuration for testing
 */
export async function selectConfigurationForTestingCommand(context: ExtensionContext): Promise<void> {
  context.updateProgressStatus("Searching for workspace");
  const xcworkspace = await askXcodeWorkspace(context);

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
    await updateWorkspaceConfig("testing.configuration", selected);
    context.buildManager.setDefaultConfigurationForTesting(undefined);
  } else {
    context.buildManager.setDefaultConfigurationForTesting(selected);
  }
}
