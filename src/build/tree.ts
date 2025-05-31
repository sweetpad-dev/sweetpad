import * as vscode from "vscode";
import type { XcodeScheme } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger";
import type { BuildManager } from "./manager";

type EventData = BuildTreeItem | LoadingTreeItem | undefined | null | undefined;

export class LoadingTreeItem extends vscode.TreeItem {
  constructor(message = "Loading schemes...") {
    super(message, vscode.TreeItemCollapsibleState.None);
    this.iconPath = new vscode.ThemeIcon("gear~spin");
    this.description = "";
    this.contextValue = "loading";
    // Make it non-selectable
    this.command = undefined;
  }
}

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

    let description = "";
    if (this.scheme === this.provider.defaultSchemeForBuild) {
      description = `${description} ✓`;
    }
    if (this.scheme === this.provider.defaultSchemeForTesting) {
      description = `${description} (t)`;
    }
    if (description) {
      this.description = description;
    }
  }
}
export class BuildTreeProvider implements vscode.TreeDataProvider<BuildTreeItem | LoadingTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;
  public context: ExtensionContext | undefined;
  public buildManager: BuildManager;
  public defaultSchemeForBuild: string | undefined;
  public defaultSchemeForTesting: string | undefined;
  private isLoading = false;

  constructor(options: { context: ExtensionContext; buildManager: BuildManager }) {
    this.context = options.context;
    this.buildManager = options.buildManager;
    this.buildManager.on("refreshStarted", () => {
      this.setLoading(true);
    });
    this.buildManager.on("updated", () => {
      this.setLoading(false);
      this.refresh();
    });
    this.buildManager.on("refreshError", (error) => {
      this.setLoading(false);
      this.refresh();
      commonLogger.error("Failed to refresh schemes", { error });
    });
    this.buildManager.on("defaultSchemeForBuildUpdated", (scheme) => {
      this.defaultSchemeForBuild = scheme;
      this.refresh();
    });
    this.buildManager.on("defaultSchemeForTestingUpdated", (scheme) => {
      this.defaultSchemeForTesting = scheme;
      this.refresh();
    });
    this.defaultSchemeForBuild = this.buildManager.getDefaultSchemeForBuild();
    this.defaultSchemeForTesting = this.buildManager.getDefaultSchemeForTesting();
  }

  private setLoading(loading: boolean): void {
    if (this.isLoading !== loading) {
      this.isLoading = loading;
      this.refresh();
    }
  }

  private refresh(): void {
    this._onDidChangeTreeData.fire(null);
  }

  async getChildren(
    element?: BuildTreeItem | LoadingTreeItem | undefined,
  ): Promise<(BuildTreeItem | LoadingTreeItem)[]> {
    // get elements only for root
    if (!element) {
      if (this.isLoading) {
        return [new LoadingTreeItem("Updating schemes...")];
      }
      const schemes = await this.getSchemes();
      return schemes;
    }

    return [];
  }

  async getTreeItem(element: BuildTreeItem | LoadingTreeItem): Promise<BuildTreeItem | LoadingTreeItem> {
    return element;
  }

  async getSchemes(): Promise<BuildTreeItem[]> {
    let schemes: XcodeScheme[] = [];
    try {
      schemes = await this.buildManager.getSchemes();
    } catch (error) {
      commonLogger.error("Failed to get schemes", {
        error: error,
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
