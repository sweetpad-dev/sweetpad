import { CommandExecution } from "../common/commands";
import { ExtensionError } from "../common/errors";
import * as vscode from "vscode";

const DEBUG_DOCUMENTATION_URL = "https://github.com/sweetpad-dev/sweetpad/blob/main/docs/wiki/debug.md";

export async function getAppPathCommand(execution: CommandExecution): Promise<string> {
  const sessionPath = execution.context.getWorkspaceState("build.lastLaunchedAppPath");
  if (!sessionPath) {
    throw new ExtensionError(`No last launched app path found, please launch the app first using the extension`, {
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
