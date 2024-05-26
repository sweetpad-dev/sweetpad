import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { TOOLS } from "./constants.js";

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
      })
    );
    return results.map((item) => {
      return new ToolTreeItem({
        label: item.label,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        isInstalled: item.isInstalled,
        commandName: item.install.command,
        commandArgs: item.install.args,
        documentation: item.documentation,
        provider: this,
      });
    });
  }
}

export class ToolTreeItem extends vscode.TreeItem {
  private provider: ToolTreeProvider;
  commandName: string;
  commandArgs: string[];
  documentation: string;

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    isInstalled: boolean;
    documentation: string;
    commandName: string;
    commandArgs: string[];
    provider: ToolTreeProvider;
  }) {
    super(options.label, options.collapsibleState);

    this.provider = options.provider;
    this.contextValue = options.isInstalled ? "installed" : "notInstalled";
    this.documentation = options.documentation;
    this.commandName = options.commandName;
    this.commandArgs = options.commandArgs;

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
