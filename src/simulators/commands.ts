import * as vscode from "vscode";
import { askSimulator } from "../build/utils.js";
import type { ExtensionContext } from "../common/commands.js";
import { runTask } from "../common/tasks.js";
import type { iOSSimulatorDestinationTreeItem } from "../destination/tree.js";

/**
 * Command to start simulator from the simulator tree view in the sidebar
 */
export async function startSimulatorCommand(context: ExtensionContext, item?: iOSSimulatorDestinationTreeItem) {
  let simulatorUdid: string;
  if (item) {
    simulatorUdid = item.simulator.udid;
  } else {
    context.updateProgressStatus("Searching for simulator to start");
    const simulator = await askSimulator(context, {
      title: "Select simulator to start",
      state: "Shutdown",
      error: "No available simulators to start",
    });
    simulatorUdid = simulator.udid;
  }

  context.updateProgressStatus("Starting simulator");
  await runTask(context, {
    name: "Start Simulator",
    lock: "sweetpad.simulators",
    terminateLocked: true,
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulatorUdid],
      });

      await context.destinationsManager.refreshSimulators();
    },
  });
}

/**
 * Command to stop simulator from the simulator tree view in the sidebar
 */
export async function stopSimulatorCommand(context: ExtensionContext, item?: iOSSimulatorDestinationTreeItem) {
  context.updateProgressStatus("Searching for simulator to stop");
  let simulatorId: string;
  if (item) {
    simulatorId = item.simulator.udid;
  } else {
    const simulator = await askSimulator(context, {
      title: "Select simulator to stop",
      state: "Booted",
      error: "No available simulators to stop",
    });
    simulatorId = simulator.udid;
  }

  context.updateProgressStatus("Stopping simulator");
  await runTask(context, {
    name: "Stop Simulator",
    lock: "sweetpad.simulators",
    terminateLocked: true,
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "shutdown", simulatorId],
      });

      await context.destinationsManager.refreshSimulators();
    },
  });
}

/**
 * Command to delete simulator from top of the simulator tree view in the sidebar
 */
export async function openSimulatorCommand(context: ExtensionContext) {
  context.updateProgressStatus("Opening Simulator.app");
  await runTask(context, {
    name: "Open Simulator",
    error: "Could not open simulator app",
    lock: "sweetpad.simulators",
    terminateLocked: true,
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
export async function removeSimulatorCacheCommand(context: ExtensionContext) {
  context.updateProgressStatus("Removing Simulator cache");
  await runTask(context, {
    name: "Remove Simulator Cache",
    error: "Error removing simulator cache",
    lock: "sweetpad.build",
    terminateLocked: true,
    callback: async (terminal) => {
      await terminal.execute({
        command: "rm",
        args: ["-rf", "~/Library/Developer/CoreSimulator/Caches"],
      });
      vscode.commands.executeCommand("sweetpad.simulators.refresh");
    },
  });
}
