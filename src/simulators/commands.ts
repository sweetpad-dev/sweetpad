import * as vscode from "vscode";
import * as path from "path";
import * as fs from "fs/promises";
import { askSimulator, prepareStoragePath } from "../build/utils.js";
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

/**
 * Command to take a screenshot of a running simulator and pass it to Cursor/AI as context
 */
export async function takeSimulatorScreenshotCommand(
  context: ExtensionContext,
  item?: iOSSimulatorDestinationTreeItem,
) {
  let simulatorUdid: string;
  let simulatorName: string;

  if (item) {
    simulatorUdid = item.simulator.udid;
    simulatorName = item.simulator.name;
  } else {
    context.updateProgressStatus("Searching for running simulator");
    const simulator = await askSimulator(context, {
      title: "Select simulator to screenshot",
      state: "Booted",
      error: "No running simulators found for screenshot",
    });
    simulatorUdid = simulator.udid;
    simulatorName = simulator.name;
  }

  // Generate temporary filename
  const timestamp = new Date().toISOString().replace(/[:.]/g, "-");
  const filename = `simulator-screenshot-${timestamp}.png`;

  // Get proper storage path using the extension's storage system
  const tempDir = await prepareStoragePath(context);
  const screenshotPath = path.resolve(tempDir, filename);

  context.updateProgressStatus(`Taking screenshot of ${simulatorName}`);

  try {
    await runTask(context, {
      name: "Take Simulator Screenshot",
      lock: "sweetpad.simulators",
      terminateLocked: true,
      callback: async (terminal) => {
        // Log paths for debugging
        terminal.write(`Taking screenshot to: ${screenshotPath}\n`);

        // Ensure directory exists
        await fs.mkdir(tempDir, { recursive: true });

        // Take screenshot using simctl with absolute path
        await terminal.execute({
          command: "xcrun",
          args: ["simctl", "io", simulatorUdid, "screenshot", screenshotPath],
        });

        // Verify file was created and has content
        const stats = await fs.stat(screenshotPath);
        if (stats.size === 0) {
          throw new Error("Screenshot file was created but is empty");
        }

        // Read the screenshot file as base64
        const imageBuffer = await fs.readFile(screenshotPath);
        const base64Image = imageBuffer.toString("base64");

        terminal.write(`Screenshot saved successfully (${stats.size} bytes)\n`);

        // Notify user that screenshot is available for AI analysis via MCP
        terminal.write(`Screenshot saved! Use MCP tool 'take_simulator_screenshot' to access via AI\n`);

        vscode.window
          .showInformationMessage(
            `✅ Screenshot taken of ${simulatorName} (${Math.round(stats.size / 1024)}KB) and added to AI context`,
            "Open Screenshot",
          )
          .then((selection) => {
            if (selection === "Open Screenshot") {
              vscode.commands.executeCommand("vscode.open", vscode.Uri.file(screenshotPath));
            }
          });
      },
    });
  } catch (error) {
    vscode.window.showErrorMessage(`Failed to take screenshot: ${error}`);
    // Clean up file if it exists
    try {
      await fs.unlink(screenshotPath);
    } catch (cleanupError) {
      // Ignore cleanup errors
    }
  }
}
