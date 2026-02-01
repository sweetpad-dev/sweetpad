import * as vscode from "vscode";
import type { XcodeScheme } from "../common/cli/scripts";
import type { ExtensionContext } from "../common/commands";
import { commonLogger } from "../common/logger";
import type { BuildManager } from "./manager";

type EventData = BuildTreeItem | undefined | null | undefined;

export class BuildTreeItem extends vscode.TreeItem {
  public scheme: string;

  constructor(options: {
    scheme: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    isRunning: boolean;
    isDefaultForBuild: boolean;
    isDefaultForTesting: boolean;
  }) {
    super(options.scheme, options.collapsibleState);
    this.scheme = options.scheme;
    const color = new vscode.ThemeColor("sweetpad.scheme");
    this.iconPath = new vscode.ThemeIcon("sweetpad-package", color);

    let description = "";
    if (options.isDefaultForBuild) {
      description = `${description} ✓`;
    }
    if (options.isDefaultForTesting) {
      description = `${description} (t)`;
    }
    if (description) {
      this.description = description;
    }

    const status = options.isRunning ? "running" : "idle";
    this.contextValue = `build-item&status=${status}`;
  }
}

export class BuildTreeProvider implements vscode.TreeDataProvider<BuildTreeItem> {
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

    this.buildManager.on("refreshSchemesStarted", () => {
      this.isLoading = true;
      this.updateView();
    });
    this.buildManager.on("refreshSchemesCompleted", () => {
      this.isLoading = false;
      this.updateView();
    });
    this.buildManager.on("refreshSchemesFailed", () => {
      this.isLoading = false;
      this.updateView();
    });
    this.buildManager.on("schemeBuildStarted", () => {
      this.updateView();
    });
    this.buildManager.on("schemeBuildStopped", () => {
      this.updateView();
    });

    this.buildManager.on("defaultSchemeForBuildUpdated", (scheme) => {
      this.defaultSchemeForBuild = scheme;
      this.updateView();
    });
    this.buildManager.on("defaultSchemeForTestingUpdated", (scheme) => {
      this.defaultSchemeForTesting = scheme;
      this.updateView();
    });
    this.defaultSchemeForBuild = this.buildManager.getDefaultSchemeForBuild();
    this.defaultSchemeForTesting = this.buildManager.getDefaultSchemeForTesting();
  }

  private updateView(): void {
    this._onDidChangeTreeData.fire(null);
  }

  async getChildren(element?: BuildTreeItem | undefined): Promise<BuildTreeItem[]> {
    // we only have one level of children, so if element is defined, we return empty array
    // to prevent vscode from expanding the item further
    if (element !== undefined) {
      return [];
    }

    // If we have a refresh event in progress, we wait for it to finish.
    // NOTE: it's prone to race conditions, but let's keep it simple for now and fix it later if needed.
    if (this.isLoading) {
      const deadline = Date.now() + 10 * 1000; // 10 seconds timeout
      await new Promise<void>((resolve) => {
        const interval = setInterval(() => {
          if (!this.isLoading || Date.now() > deadline) {
            clearInterval(interval);
            resolve();
          }
        }, 100); // check every 100ms
      });
    }

    // After loading is done, we already have the schemes in the build manager, so
    // this operation should be fast and not require any additional processing.
    return await this.getSchemes();
  }

  async getTreeItem(element: BuildTreeItem): Promise<BuildTreeItem> {
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
    return schemes.map((scheme) => {
      const isRunning = this.buildManager.isSchemeRunning(scheme.name);
      const isDefaultForBuild = scheme.name === this.defaultSchemeForBuild;
      const isDefaultForTesting = scheme.name === this.defaultSchemeForTesting;
      return new BuildTreeItem({
        scheme: scheme.name,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        isRunning: isRunning,
        isDefaultForBuild: isDefaultForBuild,
        isDefaultForTesting: isDefaultForTesting,
      });
    });
  }
}
