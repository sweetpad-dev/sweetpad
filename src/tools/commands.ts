import * as vscode from "vscode";
import { ToolTreeItem } from "./tree.js";
import { CommandExecution } from "../common/commands.js";
import { runTask } from "../common/tasks.js";

/**
 * Comamnd to install tool from the tool tree view in the sidebar using brew
 */
export async function installToolCommand(execution: CommandExecution, item: ToolTreeItem) {
  await runTask({
    name: "Install Tool",
    error: "Error installing tool",
    callback: async (terminal) => {
      await terminal.execute({
        command: item.commandName,
        args: item.commandArgs,
      });

      item.refresh();
    },
  });

  item.refresh();
}

/**
 * Command to open documentation in the browser from the tool tree view in the sidebar
 */
export async function openDocumentationCommand(execution: CommandExecution, item: ToolTreeItem) {
  await vscode.env.openExternal(vscode.Uri.parse(item.documentation));
}
