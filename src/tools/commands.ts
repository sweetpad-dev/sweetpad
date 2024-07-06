import * as vscode from "vscode";
import { ToolTreeItem } from "./tree.js";
import { CommandExecution } from "../common/commands.js";
import { runTask } from "../common/tasks.js";
import { askTool } from "./utils.js";

/**
 * Comamnd to install tool from the tool tree view in the sidebar using brew
 */
export async function installToolCommand(execution: CommandExecution, item?: ToolTreeItem) {
  const tool = item?.tool ?? (await askTool({ title: "Select tool to install" }));

  await runTask(execution.context, {
    name: "Install Tool",
    error: "Error installing tool",
    callback: async (terminal) => {
      await terminal.execute({
        command: tool.install.command,
        args: tool.install.args,
        env: {
          // We don't run the command in ptty, that's why we need to tell homebrew to use color
          // output explicitly
          HOMEBREW_COLOR: "1",
        },
      });

      execution.context.refreshTools();
    },
  });
}

/**
 * Command to open documentation in the browser from the tool tree view in the sidebar
 */
export async function openDocumentationCommand(execution: CommandExecution, item?: ToolTreeItem) {
  const tool = item?.tool ?? (await askTool({ title: "Select tool to open documentation" }));
  await vscode.env.openExternal(vscode.Uri.parse(tool.documentation));
}
