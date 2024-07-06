import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { Tool, TOOLS } from "./constants.js";

type EventData = ToolTreeItem | undefined | null | void;

/**
 * Tree view that helps to install basic ios development tools. It should have inline button to install and check if
 * tools are installed or empty state when it's not installed.
 */
export class ToolTreeProvider implements vscode.TreeDataProvider<ToolTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData: vscode.Event<EventData> = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getChildren(element?: ToolTreeItem | undefined): vscode.ProviderResult<ToolTreeItem[]> {
    // get elements only for root
    if (!element) {
      return this.getTools();
    }

    return [];
  }

  getTreeItem(element: ToolTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getTools(): Promise<ToolTreeItem[]> {
    const results = await Promise.all(
      TOOLS.map(async (item) => {
        try {
          await exec({
            command: item.check.command,
            args: item.check.args,
          });
          return {
            ...item,
            isInstalled: true,
          };
        } catch (error) {
          return {
            ...item,
            isInstalled: false,
          };
        }
      }),
    );
    return results.map((item) => {
      return new ToolTreeItem({
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        provider: this,
        isInstalled: item.isInstalled,
        tool: item,
      });
    });
  }
}

export class ToolTreeItem extends vscode.TreeItem {
  private provider: ToolTreeProvider;
  commandName: string;
  commandArgs: string[];
  documentation: string;
  tool: Tool;

  constructor(options: {
    collapsibleState: vscode.TreeItemCollapsibleState;
    isInstalled: boolean;
    provider: ToolTreeProvider;
    tool: Tool;
  }) {
    super(options.tool.label, options.collapsibleState);

    this.provider = options.provider;
    this.contextValue = options.isInstalled ? "installed" : "notInstalled";
    this.documentation = options.tool.documentation;
    this.commandName = options.tool.install.command;
    this.commandArgs = options.tool.install.args;
    this.tool = options.tool;

    if (options.isInstalled) {
      this.iconPath = new vscode.ThemeIcon("check");
    } else {
      this.iconPath = new vscode.ThemeIcon("x");
    }
  }

  refresh() {
    this.provider.refresh();
  }
}
