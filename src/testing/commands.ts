import * as vscode from "vscode";
import { askXcodeWorkspacePath } from "../build/utils";
import type { CommandExecution } from "../common/commands";
import { askTestingTarget } from "./utils";

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

export async function testWithoutBuildingCommand(
  execution: CommandExecution,
  ...items: vscode.TestItem[]
): Promise<void> {
  const request = new vscode.TestRunRequest(items, [], undefined, undefined, undefined);
  const tokenSource = new vscode.CancellationTokenSource();
  execution.context.testingManager.runTestsWithoutBuilding(request, tokenSource.token);
}
