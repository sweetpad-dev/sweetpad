import * as vscode from "vscode";
import { SimulatorsTreeProvider } from "../simulators/tree";
import { getSchemes } from "../common/cli/scripts";

type EventData = BuildTreeItem | undefined | null | void;

export class BuildTreeItem extends vscode.TreeItem {
  private provider: BuildTreeProvider;
  public scheme: string;
  public launchConfiguration: string;

  constructor(options: {
    scheme: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    provider: BuildTreeProvider;
    launchConfiguration: string;
  }) {
    super(options.scheme, options.collapsibleState);
    this.provider = options.provider;
    this.scheme = options.scheme;
    this.launchConfiguration = options.launchConfiguration ?? "Debug";
    this.iconPath = new vscode.ThemeIcon("symbol-method");
  }

  refresh() {
    this.provider.refresh();
  }

  refreshSimulators() {
    this.provider.refreshSimulators();
  }
}
export class BuildTreeProvider implements vscode.TreeDataProvider<BuildTreeItem> {
  private simulatorsTree: SimulatorsTreeProvider;

  constructor(options: { simulatorsTree: SimulatorsTreeProvider }) {
    this.simulatorsTree = options.simulatorsTree;
  }

  private _onDidChangeTreeData = new vscode.EventEmitter<EventData>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  refreshSimulators() {
    this.simulatorsTree.refresh();
  }

  getChildren(element?: BuildTreeItem | undefined): vscode.ProviderResult<BuildTreeItem[]> {
    // get elements only for root
    if (!element) {
      return this.getSchemes();
    }

    return [];
  }

  getTreeItem(element: BuildTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getSchemes(): Promise<BuildTreeItem[]> {
    const schemes = await getSchemes();

    // return list of schemes
    return schemes.map(
      (scheme) =>
        new BuildTreeItem({
          scheme: scheme.name,
          launchConfiguration: scheme.launchAction.buildConfiguration,
          collapsibleState: vscode.TreeItemCollapsibleState.None,
          provider: this,
        })
    );
  }
}
