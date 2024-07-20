import * as vscode from "vscode";
import { XcodeScheme, getBasicProjectInfo, getSchemes } from "../common/cli/scripts";
import { commonLogger } from "../common/logger";
import { getCurrentXcodeWorkspacePath } from "./utils";
import { ExtensionContext } from "../common/commands";

type EventData = BuildTreeItem | undefined | null | void;

export class BuildTreeItem extends vscode.TreeItem {
  private provider: BuildTreeProvider;
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
  }

  refresh() {
    this.provider.refresh();
  }
}
export class BuildTreeProvider implements vscode.TreeDataProvider<BuildTreeItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;
  public context: ExtensionContext | undefined;

  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
  }

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getChildren(element?: BuildTreeItem | undefined): vscode.ProviderResult<BuildTreeItem[]> {
    // get elements only for root
    if (!element) {
      const schemes = this.getSchemes();
      return schemes;
    }

    return [];
  }

  getTreeItem(element: BuildTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getSchemes(): Promise<BuildTreeItem[]> {
    const xcworkspace = this.context ? getCurrentXcodeWorkspacePath(this.context) : undefined;

    let schemes: XcodeScheme[] = [];
    try {
      schemes = await getSchemes({
        xcworkspace: xcworkspace,
      });
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
