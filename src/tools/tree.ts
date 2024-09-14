import * as vscode from "vscode";
import type { Tool } from "./constants.js";
import type { ToolsManager } from "./manager.js";

type EventData = ToolTreeItem | undefined | null | undefined;

/**
 * Tree view that helps to install basic ios development tools. It should have inline button to install and check if
 * tools are installed or empty state when it's not installed.
 */
export class ToolTreeProvider implements vscode.TreeDataProvider<ToolTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData: vscode.Event<EventData> = this._onDidChangeTreeData.event;

  manager: ToolsManager;

  constructor(options: { manager: ToolsManager }) {
    this.manager = options.manager;
    this.manager.on("updated", () => {
      this.refresh();
    });
  }

  private refresh(): void {
    this._onDidChangeTreeData.fire(null);
  }

  async getChildren(element?: ToolTreeItem | undefined): Promise<ToolTreeItem[]> {
    // get elements only for root
    if (!element) {
      return await this.getTools();
    }

    return [];
  }

  async getTreeItem(element: ToolTreeItem): Promise<vscode.TreeItem> {
    return element;
  }

  async getTools(): Promise<ToolTreeItem[]> {
    const results = await this.manager.getTools();
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

    this.contextValue = options.isInstalled ? "installed" : "notInstalled";
    this.documentation = options.tool.documentation;
    this.commandName = options.tool.install.command;
    this.commandArgs = options.tool.install.args;
    this.tool = options.tool;

    if (options.isInstalled) {
      this.iconPath = new vscode.ThemeIcon("sweetpad-check");
    } else {
      this.iconPath = new vscode.ThemeIcon("sweetpad-x");
    }
  }
}
