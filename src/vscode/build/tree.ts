import * as vscode from "vscode";

import type { BuildManager } from "../../core/build/manager";
import type { XcodeScheme } from "../../core/cli/scripts";
import { getWorkspaceConfig } from "../config";
import { commonLogger } from "../logger";

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
  #onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData = this.#onDidChangeTreeData.event;
  public buildManager: BuildManager;
  public defaultSchemeForBuild: string | undefined;
  public defaultSchemeForTesting: string | undefined;
  private isLoading = false;
  private schemeFilterPaused = false;
  private schemeIncludeRegexes: RegExp[] = [];
  private schemeExcludeRegexes: RegExp[] = [];

  constructor(options: { buildManager: BuildManager }) {
    this.buildManager = options.buildManager;
  }

  async start(): Promise<void> {
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

    this.updateSchemeFilterPausedContext();
    this.recomputeSchemeFilterPatterns();

    vscode.workspace.onDidChangeConfiguration((event) => {
      if (
        event.affectsConfiguration("sweetpad.build.schemes.include") ||
        event.affectsConfiguration("sweetpad.build.schemes.exclude")
      ) {
        this.recomputeSchemeFilterPatterns();
        this.updateView();
      }
    });
  }

  private updateView(): void {
    this.#onDidChangeTreeData.fire(null);
  }

  private recomputeSchemeFilterPatterns(): void {
    const include = getWorkspaceConfig("build.schemes.include") ?? [];
    const exclude = getWorkspaceConfig("build.schemes.exclude") ?? [];
    this.schemeIncludeRegexes = include.map((p) => this.patternToRegex(p));
    this.schemeExcludeRegexes = exclude.map((p) => this.patternToRegex(p));
    const hasFilter = this.schemeIncludeRegexes.length > 0 || this.schemeExcludeRegexes.length > 0;
    vscode.commands.executeCommand("setContext", "sweetpad.build.hasSchemeFilter", hasFilter);
  }

  private updateSchemeFilterPausedContext(): void {
    vscode.commands.executeCommand("setContext", "sweetpad.build.schemeFilterPaused", this.schemeFilterPaused);
  }

  public toggleSchemeFilterPaused(paused: boolean): void {
    if (this.schemeFilterPaused === paused) {
      return;
    }
    this.schemeFilterPaused = paused;
    this.updateSchemeFilterPausedContext();
    this.updateView();
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

    vscode.commands.executeCommand("setContext", "sweetpad.build.noSchemes", schemes.length === 0);

    if (schemes.length === 0) {
      // Display welcome screen with explanation what to do.
      // See "viewsWelcome": [ {"view": "sweetpad.build.view", ...} ] in package.json
      return [];
    }

    if (!this.schemeFilterPaused) {
      schemes = this.applySchemeFilter(schemes);
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

  /** Converts a `*`-wildcard glob pattern into an anchored regex (e.g. `Feature*` → `/^Feature.*$/`). */
  private patternToRegex(pattern: string): RegExp {
    const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&");
    return new RegExp(`^${escaped.replace(/\*/g, ".*")}$`);
  }

  private applySchemeFilter(schemes: XcodeScheme[]): XcodeScheme[] {
    if (this.schemeIncludeRegexes.length === 0 && this.schemeExcludeRegexes.length === 0) {
      return schemes;
    }
    return schemes.filter((scheme) => {
      if (this.schemeIncludeRegexes.length > 0 && !this.schemeIncludeRegexes.some((re) => re.test(scheme.name))) {
        return false;
      }
      if (this.schemeExcludeRegexes.some((re) => re.test(scheme.name))) {
        return false;
      }
      return true;
    });
  }
}
