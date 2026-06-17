import * as vscode from "vscode";

import { askSimulator } from "../build/utils.js";
import type { AppDeps } from "../common/commands.js";
import { runTask } from "../common/tasks/run.js";
import type { DestinationTreeItem, iOSSimulatorDestinationTreeItem } from "../destination/tree.js";
import type { SimulatorDestination } from "./types.js";

/**
 * Tree items that carry a simulator destination. They all share the
 * `destination-item-simulator` context prefix, so the serve-sim menu entries
 * surface for iOS/watchOS/tvOS/visionOS simulators alike.
 */
type SimulatorDestinationTreeItem = Extract<DestinationTreeItem, { simulator: SimulatorDestination }>;

/**
 * Resolve the simulator a serve-sim command should act on: either the one the
 * command was invoked on from the destinations tree, or (when run from the
 * command palette) a booted simulator picked by the user.
 */
async function resolveSimulator(deps: AppDeps, item?: SimulatorDestinationTreeItem): Promise<SimulatorDestination> {
  if (item) {
    return item.simulator;
  }
  return await askSimulator(deps.destinationsManager, {
    title: "Select simulator to stream",
    state: "Booted",
    error: "No booted simulators to stream. Start a simulator first.",
  });
}

/**
 * Command to start simulator from the simulator tree view in the sidebar
 */
export async function startSimulatorCommand(deps: AppDeps, item?: iOSSimulatorDestinationTreeItem) {
  let simulatorUdid: string;
  if (item) {
    simulatorUdid = item.simulator.udid;
  } else {
    deps.progressStatusBar.updateText("Searching for simulator to start");
    const simulator = await askSimulator(deps.destinationsManager, {
      title: "Select simulator to start",
      state: "Shutdown",
      error: "No available simulators to start",
    });
    simulatorUdid = simulator.udid;
  }

  deps.progressStatusBar.updateText("Starting simulator");
  await runTask(deps.execution, {
    name: "Start Simulator",
    lock: "sweetpad.simulators",
    terminateLocked: true,
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "boot", simulatorUdid],
      });

      await deps.destinationsManager.refreshSimulators();
    },
  });
}

/**
 * Command to stop simulator from the simulator tree view in the sidebar
 */
export async function stopSimulatorCommand(deps: AppDeps, item?: iOSSimulatorDestinationTreeItem) {
  deps.progressStatusBar.updateText("Searching for simulator to stop");
  let simulatorId: string;
  if (item) {
    simulatorId = item.simulator.udid;
  } else {
    const simulator = await askSimulator(deps.destinationsManager, {
      title: "Select simulator to stop",
      state: "Booted",
      error: "No available simulators to stop",
    });
    simulatorId = simulator.udid;
  }

  deps.progressStatusBar.updateText("Stopping simulator");
  await runTask(deps.execution, {
    name: "Stop Simulator",
    lock: "sweetpad.simulators",
    terminateLocked: true,
    callback: async (terminal) => {
      await terminal.execute({
        command: "xcrun",
        args: ["simctl", "shutdown", simulatorId],
      });

      await deps.destinationsManager.refreshSimulators();
    },
  });
}

/**
 * Command to delete simulator from top of the simulator tree view in the sidebar
 */
export async function openSimulatorCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Opening Simulator.app");
  await runTask(deps.execution, {
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
export async function removeSimulatorCacheCommand(deps: AppDeps) {
  deps.progressStatusBar.updateText("Removing Simulator cache");
  await runTask(deps.execution, {
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

/**
 * Stream the simulator into an in-editor webview using serve-sim. Works from
 * the destinations tree (acts on the clicked simulator) or the command palette
 * (prompts for a booted simulator).
 */
export async function streamSimulatorCommand(deps: AppDeps, item?: SimulatorDestinationTreeItem) {
  const simulator = await resolveSimulator(deps, item);
  deps.progressStatusBar.updateText(`Starting stream for ${simulator.name}`);
  await deps.serveSimManager.stream(simulator);
}

/**
 * Open the serve-sim live preview for the simulator in the default browser.
 */
export async function openSimulatorStreamInBrowserCommand(deps: AppDeps, item?: SimulatorDestinationTreeItem) {
  const simulator = await resolveSimulator(deps, item);
  deps.progressStatusBar.updateText(`Starting stream for ${simulator.name}`);
  await deps.serveSimManager.openInBrowser(simulator);
}

/**
 * Copy the serve-sim preview URL for the simulator to the clipboard, so it can
 * be shared or opened elsewhere (e.g. through a tunnel).
 */
export async function copySimulatorStreamUrlCommand(deps: AppDeps, item?: SimulatorDestinationTreeItem) {
  const simulator = await resolveSimulator(deps, item);
  deps.progressStatusBar.updateText(`Starting stream for ${simulator.name}`);
  await deps.serveSimManager.copyUrl(simulator);
}
