import * as vscode from "vscode";
import type { CommandExecution } from "../common/commands";
import { ExtensionError } from "../common/errors";

const DEBUG_DOCUMENTATION_URL = "https://github.com/sweetpad-dev/sweetpad/blob/main/docs/wiki/debug.md";

/**
 * This function is deprecated. Previously we use "lldb" as the debugger type and
 * "{command:sweetpad.debugger.getAppPath}" as a way to provide the app path to the debugger.
 * Now we use "sweetpad-lldb" as the debugger type, which wraps around "lldb" and provides the app path
 * directly to the debugger during resolving the debug configuration.
 */
export async function getAppPathCommand(execution: CommandExecution): Promise<string> {
  const sessionPath = execution.context.getWorkspaceState("build.lastLaunchedAppPath");
  if (!sessionPath) {
    throw new ExtensionError("No last launched app path found, please launch the app first using the extension", {
      actions: [
        {
          label: "Open documentation",
          callback: () => vscode.env.openExternal(vscode.Uri.parse(DEBUG_DOCUMENTATION_URL)),
        },
      ],
    });
  }
  return sessionPath;
}
