import * as vscode from "vscode";
import { SimulatorTreeItem } from "./tree.js";

/**
 * Command to start simulator from the simulator tree view in the sidebar
 */
export async function startSimulatorCommand(context: vscode.ExtensionContext, item: SimulatorTreeItem) {
  const simulatorName = item.udid;

  const task = new vscode.Task(
    { type: "shell" },
    vscode.TaskScope.Workspace,
    "Start Simulator",
    "xcrun",
    new vscode.ShellExecution(`xcrun simctl boot "${simulatorName}"`)
  );

  const execution = await vscode.tasks.executeTask(task);

  vscode.tasks.onDidEndTaskProcess((e) => {
    if (e.execution === execution) {
      item.refresh();
    }
  });
}

/**
 * Command to stop simulator from the simulator tree view in the sidebar
 */
export async function stopSimulatorCommand(context: vscode.ExtensionContext, item: SimulatorTreeItem) {
  const simulatorName = item.udid;

  const task = new vscode.Task(
    { type: "shell" },
    vscode.TaskScope.Workspace,
    "Stop Simulator",
    "xcrun",
    new vscode.ShellExecution("xcrun", ["simctl", "shutdown", simulatorName])
  );

  const execution = await vscode.tasks.executeTask(task);

  vscode.tasks.onDidEndTaskProcess((e) => {
    if (e.execution === execution) {
      item.refresh();
    }
  });
}

/**
 * Command to delete simulator from top of the simulator tree view in the sidebar
 */
export async function openSimulatorCommand() {
  // open simulator using open -a Simulator
  const task = new vscode.Task(
    { type: "shell" },
    vscode.TaskScope.Workspace,
    "Open Simulator",
    "open",
    new vscode.ShellExecution("open -a Simulator")
  );

  const execution = await vscode.tasks.executeTask(task);

  vscode.tasks.onDidEndTaskProcess((e) => {
    if (e.execution === execution) {
      vscode.commands.executeCommand("sweetpad.simulators.refresh");
    }
  });
}

/**
 * Command to delete simulators cache from top of the simulator tree view in the sidebar.
 * This is useful when you have a lot of simulators and you want to free up some space.
 * Also in some cases it can help to issues with starting simulators.
 */

export async function removeSimulatorCacheCommand() {
  // remove simulator cache using rm -rf ~/Library/Developer/CoreSimulator/Devices
  const task = new vscode.Task(
    { type: "shell" },
    vscode.TaskScope.Workspace,
    "Remove Simulator Cache",
    "rm",
    new vscode.ShellExecution("rm -rf ~/Library/Developer/CoreSimulator/Caches")
  );

  const execution = await vscode.tasks.executeTask(task);

  vscode.tasks.onDidEndTaskProcess((e) => {
    if (e.execution === execution) {
      vscode.commands.executeCommand("sweetpad.simulators.refresh");
    }
  });
}
