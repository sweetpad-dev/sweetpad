import * as vscode from "vscode";
import { SimulatorTreeItem } from "./tree.js";
import { CommandExecution } from "../common/commands.js";
import { runShellTask } from "../common/tasks.js";

/**
 * Command to start simulator from the simulator tree view in the sidebar
 */
export async function startSimulatorCommand(execution: CommandExecution, item: SimulatorTreeItem) {
  const simulatorName = item.udid;

  await runShellTask({
    name: "Start Simulator",
    command: "xcrun",
    args: ["simctl", "boot", simulatorName],
    error: "Error starting simulator",
  });

  item.refresh();
}

/**
 * Command to stop simulator from the simulator tree view in the sidebar
 */
export async function stopSimulatorCommand(execution: CommandExecution, item: SimulatorTreeItem) {
  const simulatorName = item.udid;

  await runShellTask({
    name: "Stop Simulator",
    command: "xcrun",
    args: ["simctl", "shutdown", simulatorName],
    error: "Error stopping simulator",
  });

  item.refresh();
}

/**
 * Command to delete simulator from top of the simulator tree view in the sidebar
 */
export async function openSimulatorCommand() {
  await runShellTask({
    name: "Open Simulator",
    command: "open",
    args: ["-a", "Simulator"],
    error: "Could not open simulator app",
  });

  vscode.commands.executeCommand("sweetpad.simulators.refresh");
}

/**
 * Command to delete simulators cache from top of the simulator tree view in the sidebar.
 * This is useful when you have a lot of simulators and you want to free up some space.
 * Also in some cases it can help to issues with starting simulators.
 */

export async function removeSimulatorCacheCommand() {
  // remove simulator cache using rm -rf ~/Library/Developer/CoreSimulator/Devices

  await runShellTask({
    name: "Remove Simulator Cache",
    command: "rm",
    args: ["-rf", "~/Library/Developer/CoreSimulator/Caches"],
    error: "Error removing simulator cache",
  });

  vscode.commands.executeCommand("sweetpad.simulators.refresh");
}
