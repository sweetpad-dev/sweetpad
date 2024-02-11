import * as vscode from "vscode";
import { exec } from "../common/exec.js";
import { getSimulators } from "../common/cli/scripts.js";

export class SimulatorTreeItem extends vscode.TreeItem {
  udid: string;
  state: string;
  private provider: SimulatorsTreeProvider;

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    command?: vscode.Command;
    udid: string;
    state: string;
    provider: SimulatorsTreeProvider;
  }) {
    super(options.label, options.collapsibleState);

    this.command = options.command;
    this.udid = options.udid;
    this.state = options.state;
    this.contextValue = this.state === "Booted" ? "booted" : "shutdown";
    this.provider = options.provider;

    if (this.state === "Booted") {
      this.iconPath = new vscode.ThemeIcon("vm-running");
    }
  }

  refresh() {
    this.provider.refresh();
  }
}

export class SimulatorsTreeProvider implements vscode.TreeDataProvider<SimulatorTreeItem> {
  private _onDidChangeTreeData: vscode.EventEmitter<SimulatorTreeItem | undefined | null | void> =
    new vscode.EventEmitter<SimulatorTreeItem | undefined | null | void>();
  readonly onDidChangeTreeData: vscode.Event<SimulatorTreeItem | undefined | null | void> =
    this._onDidChangeTreeData.event;

  refresh(): void {
    this._onDidChangeTreeData.fire();
  }

  getChildren(element?: SimulatorTreeItem | undefined): vscode.ProviderResult<SimulatorTreeItem[]> {
    // get elements only for root
    if (!element) {
      return this.getSimulators();
    }

    return [];
  }

  getTreeItem(element: SimulatorTreeItem): vscode.TreeItem | Thenable<vscode.TreeItem> {
    return element;
  }

  async getSimulators(): Promise<SimulatorTreeItem[]> {
    const output = await getSimulators();
    const devices = output.devices;
    return Object.entries(devices)
      .map(([key, value]) => {
        return value
          .filter((simulator) => simulator.isAvailable)
          .map((simulator) => {
            return new SimulatorTreeItem({
              label: simulator.name,
              collapsibleState: vscode.TreeItemCollapsibleState.None,
              udid: simulator.udid,
              state: simulator.state,
              provider: this,
            });
          });
      })
      .flat();
  }
}
