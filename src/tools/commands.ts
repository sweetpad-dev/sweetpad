import * as vscode from "vscode";
import { ToolTreeItem } from "./tree.js";
import { CommandExecution } from "../common/commands.js";
import { runShellTask } from "../common/tasks.js";

/**
 * Comamnd to install tool from the tool tree view in the sidebar using brew
 */
export async function installToolCommand(execution: CommandExecution, item: ToolTreeItem) {
  await runShellTask({
    name: "Install Tool",
    command: item.commandName,
    args: item.commandArgs,
    error: "Error installing tool",
  });

  item.refresh();
}

/**
 * Command to open documentation in the browser from the tool tree view in the sidebar
 */
export async function openDocumentationCommand(execution: CommandExecution, item: ToolTreeItem) {
  await vscode.env.openExternal(vscode.Uri.parse(item.documentation));
}
