import * as vscode from "vscode";
import { CommandExecution } from "../common/commands.js";
import { runTask } from "../common/tasks.js";
import { askSimulator } from "../build/utils.js";
import { iOSSimulatorDestinationTreeItem } from "../destination/tree.js";

/**
 * Command to start simulator from the simulator tree view in the sidebar
 */
export async function startSimulatorCommand(execution: CommandExecution, item?: iOSSimulatorDestinationTreeItem) {
  let simulatorUdid: string;
  if (item) {
    simulatorUdid = item.simulator.udid;
  } else {
    const simulator = await askSimulator(execution.context, {
      title: "Select simulator to start",
      state: "Shutdown",
      error: "No available simulators to start",
    });
    simulatorUdid = simulator.udid;
  }

  await runTask(execution.context, {
    name: "Start Simulator",
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulatorUdid],
      });

      execution.context.destinationsManager.refreshiOSSimulators();
    },
  });
}

/**
 * Command to stop simulator from the simulator tree view in the sidebar
 */
export async function stopSimulatorCommand(execution: CommandExecution, item?: iOSSimulatorDestinationTreeItem) {
  let simulatorId: string;
  if (item) {
    simulatorId = item.simulator.udid;
  } else {
    const simulator = await askSimulator(execution.context, {
      title: "Select simulator to stop",
      state: "Booted",
      error: "No available simulators to stop",
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

      execution.context.destinationsManager.refreshiOSSimulators();
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
