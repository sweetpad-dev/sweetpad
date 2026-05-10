import * as vscode from "vscode";
import type { ExtensionContext } from "../common/commands.js";
import { runTask } from "../common/tasks/run.js";
import type { ToolTreeItem } from "./tree.js";
import { askTool } from "./utils.js";

/**
 * Command to install tool from the tool tree view in the sidebar using brew
 */
export async function installToolCommand(context: ExtensionContext, item?: ToolTreeItem) {
  const tool = item?.tool ?? (await askTool({ title: "Select tool to install" }));

  context.updateProgressStatus("Installing tool");
  await runTask(context, {
    name: "Install Tool",
    error: "Error installing tool",
    terminateLocked: false,
    lock: "sweetpad.tools.install",
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

      context.toolsManager.refresh();
    },
  });
}

/**
 * Command to open documentation in the browser from the tool tree view in the sidebar
 */
export async function openDocumentationCommand(context: ExtensionContext, item?: ToolTreeItem) {
  const tool = item?.tool ?? (await askTool({ title: "Select tool to open documentation" }));
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

export async function installPymobiledevice3Command(_context: ExtensionContext): Promise<void> {
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
