import * as vscode from "vscode";
import { SimulatorsManager } from "./manager.js";
import { OS } from "../common/destinationTypes.js";

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
    this.provider.manager.refresh([OS.iOS, OS.watchOS, OS.macOS]);
  }
}

export class SimulatorsTreeProvider implements vscode.TreeDataProvider<SimulatorTreeItem> {
  public manager: SimulatorsManager;

  private _onDidChangeTreeData = new vscode.EventEmitter<SimulatorTreeItem | undefined | null | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(options: { manager: SimulatorsManager }) {
    this.manager = options.manager;
    this.manager.on("refresh", () => {
      this._onDidChangeTreeData.fire();
    });
  }

  refresh() {
    void this.manager.refresh([OS.iOS, OS.watchOS, OS.macOS]);
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
    const simulatorsRaw = await this.manager.getSimulators({
      refresh: false,
      filterOSTypes: [OS.iOS, OS.watchOS, OS.macOS],
    });
    const simulators = simulatorsRaw.map((simulator) => {
      return new SimulatorTreeItem({
        label: simulator.label,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        udid: simulator.udid,
        state: simulator.state,
        provider: this,
      });
    });
    return simulators;
  }
}
