import * as vscode from "vscode";
import { ToolTreeItem } from "./tree.js";

/**
 * Comamnd to install tool from the tool tree view in the sidebar using brew
 */
export async function installToolCommand(item: ToolTreeItem) {
  const task = new vscode.Task(
    { type: "shell" },
    vscode.TaskScope.Workspace,
    "Install Tool",
    "brew",
    new vscode.ShellExecution(item.installCommand)
  );

  const execution = await vscode.tasks.executeTask(task);

  vscode.tasks.onDidEndTaskProcess((e) => {
    if (e.execution === execution) {
      item.refresh();
    }
  });
}

/**
 * Command to open documentation in the browser from the tool tree view in the sidebar
 */
export async function openDocumentationCommand(item: ToolTreeItem) {
  await vscode.env.openExternal(vscode.Uri.parse(item.documentation));
}
