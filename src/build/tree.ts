import * as vscode from "vscode";
import type { XcodeScheme } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger";
import type { BuildManager } from "./manager";

type EventData = BuildTreeItem | undefined | null | undefined;

export class BuildTreeItem extends vscode.TreeItem {
  public provider: BuildTreeProvider;
  public scheme: string;

  constructor(options: {
    scheme: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    provider: BuildTreeProvider;
  }) {
    super(options.scheme, options.collapsibleState);
    this.provider = options.provider;
    this.scheme = options.scheme;
    const color = new vscode.ThemeColor("sweetpad.scheme");
    this.iconPath = new vscode.ThemeIcon("sweetpad-package", color);

    if (this.scheme === this.provider.defaultScheme) {
      this.description = "✓";
    }
  }
}
export class BuildTreeProvider implements vscode.TreeDataProvider<BuildTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;
  public context: ExtensionContext | undefined;
  public buildManager: BuildManager;
  public defaultScheme: string | undefined;

  constructor(options: { context: ExtensionContext; buildManager: BuildManager }) {
    this.context = options.context;
    this.buildManager = options.buildManager;
    this.buildManager.on("updated", () => {
      this.refresh();
    });
    this.buildManager.on("defaultSchemeUpdated", (scheme) => {
      this.defaultScheme = scheme;
      this.refresh();
    });
    this.defaultScheme = this.buildManager.getDefaultScheme();
  }

  private refresh(): void {
    this._onDidChangeTreeData.fire(null);
  }

  async getChildren(element?: BuildTreeItem | undefined): Promise<BuildTreeItem[]> {
    // get elements only for root
    if (!element) {
      const schemes = await this.getSchemes();
      return schemes;
    }

    return [];
  }

  async getTreeItem(element: BuildTreeItem): Promise<BuildTreeItem> {
    return element;
  }

  async getSchemes(): Promise<BuildTreeItem[]> {
    let schemes: XcodeScheme[] = [];
    try {
      schemes = await this.buildManager.getSchemas();
    } catch (error) {
      commonLogger.error("Failed to get schemes", {
        error,
      });
    }

    if (schemes.length === 0) {
      // Display welcome screen with explanation what to do.
      // See "viewsWelcome": [ {"view": "sweetpad.build.view", ...} ] in package.json
      vscode.commands.executeCommand("setContext", "sweetpad.build.noSchemes", true);
    }

    // return list of schemes
    return schemes.map(
      (scheme) =>
        new BuildTreeItem({
          scheme: scheme.name,
          collapsibleState: vscode.TreeItemCollapsibleState.None,
          provider: this,
        }),
    );
  }
}
