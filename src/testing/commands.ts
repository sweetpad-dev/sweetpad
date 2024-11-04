import * as vscode from "vscode";
import type { BuildTreeItem } from "../build/tree";
import { askXcodeWorkspacePath } from "../build/utils";
import type { CommandExecution } from "../common/commands";
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
  return await execution.context.testingManager.buildForTestingCommand();
}

export async function testBuildingCommand(
  execution: CommandExecution,
  ...items: vscode.TestItem[]
): Promise<void> {
  const actualItems = items.length ? items : [...execution.context.testingManager.controller.items].map(([, item]) => item);
  const request = new vscode.TestRunRequest(actualItems, [], undefined, undefined);
  const tokenSource = new vscode.CancellationTokenSource();

  execution.context.testingManager.buildAndRunTests(request, tokenSource.token);
}

export async function testWithoutBuildingCommand(
  execution: CommandExecution,
  ...items: vscode.TestItem[]
): Promise<void> {
  const actualItems = items.length ? items : [...execution.context.testingManager.controller.items].map(([, item]) => item);
  const request = new vscode.TestRunRequest(actualItems, [], undefined, undefined);
  const tokenSource = new vscode.CancellationTokenSource();

  execution.context.testingManager.runTestsWithoutBuilding(request, tokenSource.token);
}

export async function selectXcodeSchemeForTestingCommand(execution: CommandExecution, item?: BuildTreeItem) {
  if (item) {
    item.provider.buildManager.setDefaultSchemeForTesting(item.scheme);
    return;
  }

  const xcworkspace = await askXcodeWorkspacePath(execution.context);
  await askSchemeForTesting(execution.context, {
    title: "Select scheme to set as default",
    xcworkspace: xcworkspace,
    ignoreCache: true,
  });
}
