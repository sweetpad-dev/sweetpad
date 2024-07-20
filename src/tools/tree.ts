import * as vscode from "vscode";
import { Tool } from "./constants.js";
import { ToolsManager } from "./manager.js";

type EventData = ToolTreeItem | undefined | null | void;

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
    this.manager.on("refresh", () => {
      this.refresh();
    });
  }

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
