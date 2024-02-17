import * as vscode from "vscode";
import { execPrepared } from "../common/exec";
import { SimulatorsTreeProvider } from "../simulators/tree";

type EventData = BuildTreeItem | undefined | null | void;

interface XcodeBuildListOutput {
  project: {
    configurations: string[];
    name: string;
    schemes: string[];
    targets: string[];
  };
}

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
      return this.getSimulators();
    }

    return [];
  }

  getTreeItem(element: BuildTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getSimulators(): Promise<BuildTreeItem[]> {
    const cwd = vscode.workspace.workspaceFolders?.[0].uri.fsPath;
    const { stdout, error } = await execPrepared("xcodebuild -list -json", { cwd: cwd });
    if (error) {
      // proper error handling
      console.error("Error fetching simulators", error);
      return [];
    }

    const data = JSON.parse(stdout) as XcodeBuildListOutput;

    // return list of schemes
    return data.project.schemes.map(
      (scheme) =>
        new BuildTreeItem({
          scheme: scheme,
          collapsibleState: vscode.TreeItemCollapsibleState.None,
          provider: this,
        })
    );
  }
}
