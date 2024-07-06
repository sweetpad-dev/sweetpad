import * as vscode from "vscode";
import { SimulatorTreeItem } from "./tree.js";
import { CommandExecution } from "../common/commands.js";
import { runTask } from "../common/tasks.js";
import { askBootedSimulator, askDestinationToRunOn } from "../build/utils.js";

/**
 * Command to start simulator from the simulator tree view in the sidebar
 */
export async function startSimulatorCommand(execution: CommandExecution, item: SimulatorTreeItem) {
  const simulatorName = item.udid;

  await runTask(execution.context, {
    name: "Start Simulator",
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulatorName],
      });

      item.refresh();
    },
  });
}

/**
 * Command to stop simulator from the simulator tree view in the sidebar
 */
export async function stopSimulatorCommand(execution: CommandExecution, item?: SimulatorTreeItem) {
  let simulatorId: string;
  if (item) {
    simulatorId = item.udid;
  } else {
    const simulator = await askBootedSimulator(execution.context, {
      title: "Select simulator to stop",
    });
    simulatorId = simulator.udid;
  }

  await runTask(execution.context, {
    name: "Stop Simulator",
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "shutdown", simulatorId],
      });

      execution.context.refreshSimulators();
    },
  });
}

/**
 * Command to delete simulator from top of the simulator tree view in the sidebar
 */
export async function openSimulatorCommand(execution: CommandExecution) {
  await runTask(execution.context, {
    name: "Open Simulator",
    error: "Could not open simulator app",
    callback: async (terminal) => {
      await terminal.execute({
        command: "open",
        args: ["-a", "Simulator"],
      });

      vscode.commands.executeCommand("sweetpad.simulators.refresh");
    },
  });
}

/**
 * Command to delete simulators cache from top of the simulator tree view in the sidebar.
 * This is useful when you have a lot of simulators and you want to free up some space.
 * Also in some cases it can help to issues with starting simulators.
 */
export async function removeSimulatorCacheCommand(execution: CommandExecution) {
  await runTask(execution.context, {
    name: "Remove Simulator Cache",
    error: "Error removing simulator cache",
    callback: async (terminal) => {
      await terminal.execute({
        command: "rm",
        args: ["-rf", "~/Library/Developer/CoreSimulator/Caches"],
      });
      vscode.commands.executeCommand("sweetpad.simulators.refresh");
    },
  });
}
