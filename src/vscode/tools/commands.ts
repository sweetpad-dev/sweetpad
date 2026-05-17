import * as vscode from "vscode";

import type { TaskTerminal } from "../../core/tasks/types.js";
import { askTool } from "../../core/tools/utils.js";
import type { AppDeps } from "../commands.js";
import type { ToolTreeItem } from "./tree.js";

/**
 * Command to install tool from the tool tree view in the sidebar using brew
 */
export async function installToolCommand(deps: AppDeps, item?: ToolTreeItem) {
  const tool = item?.tool ?? (await askTool(deps.asker, { title: "Select tool to install" }));

  deps.progressStatusBar.updateText("Installing tool");
  await deps.taskRunner.run({
    name: "Install Tool",
    error: "Error installing tool",
    terminateLocked: false,
    lock: "sweetpad.tools.install",
    callback: async (terminal: TaskTerminal) => {
      await terminal.execute({
        command: tool.install.command,
        args: tool.install.args,
        env: {
          // We don't run the command in ptty, that's why we need to tell homebrew to use color
          // output explicitly
          HOMEBREW_COLOR: "1",
        },
      });

      deps.toolsManager.refresh();
    },
  });
}

/**
 * Command to open documentation in the browser from the tool tree view in the sidebar
 */
export async function openDocumentationCommand(deps: AppDeps, item?: ToolTreeItem) {
  const tool = item?.tool ?? (await askTool(deps.asker, { title: "Select tool to open documentation" }));
  await vscode.env.openExternal(vscode.Uri.parse(tool.documentation));
}

type Pymobiledevice3InstallChoice = {
  label: string;
  description: string;
  command: string;
};

const PYMOBILEDEVICE3_INSTALL_CHOICES: Pymobiledevice3InstallChoice[] = [
  {
    label: "uv (recommended)",
    description: "uv tool install pymobiledevice3",
    command: "uv tool install pymobiledevice3",
  },
  {
    label: "Install uv",
    description: "brew install uv",
    command: "brew install uv",
  },
  {
    label: "pipx",
    description: "pipx install pymobiledevice3",
    command: "pipx install pymobiledevice3",
  },
  {
    label: "pip (user)",
    description: "pip install --user pymobiledevice3",
    command: "pip install --user pymobiledevice3",
  },
];

export async function installPymobiledevice3Command(_deps: AppDeps): Promise<void> {
  const picked = await vscode.window.showQuickPick(PYMOBILEDEVICE3_INSTALL_CHOICES, {
    title: "Install pymobiledevice3",
    placeHolder: "Choose how to install pymobiledevice3",
  });
  if (!picked) return;

  const terminal = vscode.window.createTerminal({
    name: "Install pymobiledevice3",
    iconPath: new vscode.ThemeIcon("cloud-download"),
  });
  terminal.show(true);
  terminal.sendText(picked.command);
}
