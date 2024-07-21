import * as vscode from "vscode";
import { DestinationsManager } from "./manager.js";
import { SelectedDestination, iOSDeviceDestination, iOSSimulatorDestination } from "./types.js";

/**
 * Tree item representing a group of destinations (iOSSimulator, iOSDevice, etc.) at the root level
 */
class DestinationGroupTreeItem extends vscode.TreeItem {
  type: "iOSSimulator" | "iOSDevice";

  constructor(options: {
    label: string;
    type: "iOSSimulator" | "iOSDevice";
    collapsibleState: vscode.TreeItemCollapsibleState;
    icon: string;
  }) {
    super(options.label, options.collapsibleState);
    this.type = options.type;
    this.iconPath = new vscode.ThemeIcon(options.icon);
    this.contextValue = `destination-group-${this.type}`;
  }
}

/**
 * Common interface for destination tree item (items under tree group, second level)
 */
export interface IDestinationTreeItem {
  type: "iOSSimulator" | "iOSDevice";
}

/**
 * Tree item representing a iOSSimulator destination
 */
export class iOSSimulatorDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "iOSSimulator" as const;
  simulator: iOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: {
    collapsibleState: vscode.TreeItemCollapsibleState;
    simulator: iOSSimulatorDestination;
    provider: DestinationsTreeProvider;
  }) {
    super(options.simulator.name, options.collapsibleState);
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    const contextPrefix = "destination-item-iOSSimulator";

    const isSelected =
      this.provider.selectedDestination?.type === "iOSSimulator" &&
      this.provider.selectedDestination.udid === this.simulator.udid;

    if (isSelected) {
      this.description = `${this.description} •`;
    }

    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, undefined);

    if (this.simulator.isBooted) {
      this.contextValue = `${contextPrefix}-booted`; // "destination-item-iOSSimulator-booted"
    } else {
      this.contextValue = `${contextPrefix}-shutdown`; // "destination-item-iOSSimulator-shutdown"
    }
  }
}

/**
 * Tree item representing a iOS device destination
 */
export class iOSDeviceDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "iOSDevice" as const;
  device: iOSDeviceDestination;
  provider: DestinationsTreeProvider;

  constructor(options: {
    collapsibleState: vscode.TreeItemCollapsibleState;
    device: iOSDeviceDestination;
    provider: DestinationsTreeProvider;
  }) {
    super(options.device.name, options.collapsibleState);
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.osVersion;
    const contextPrefix = "destination-item-iOSDevice";

    const isSelected =
      this.provider.selectedDestination?.type === "iOSDevice" &&
      this.provider.selectedDestination.udid === this.device.udid;
    if (isSelected) {
      this.description = `${this.description} •`;
    }

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    if (this.device.isConnected) {
      this.contextValue = `${contextPrefix}-connected`; // "destination-item-iOSDevice-connected"
    } else {
      this.contextValue = `${contextPrefix}-disconnected`; // "destination-item-iOSDevice-disconnected
    }
  }
}

// Tagged union type for destination tree item (second level)
export type DestinationTreeItem = iOSSimulatorDestinationTreeItem | iOSDeviceDestinationTreeItem;

export class DestinationsTreeProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  public manager: DestinationsManager;
  public selectedDestination: SelectedDestination | undefined;

  private _onDidChangeTreeData = new vscode.EventEmitter<DestinationTreeItem | undefined | null | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(options: { manager: DestinationsManager }) {
    this.manager = options.manager;
    this.manager.on("refreshSimulators", () => {
      this._onDidChangeTreeData.fire();
    });
    this.manager.on("refreshDevices", () => {
      this._onDidChangeTreeData.fire();
    });
    this.manager.on("refreshWorkspaceDestination", (destination) => {
      this.selectedDestination = destination;
      this._onDidChangeTreeData.fire(); // todo: update only the selected destination
    });
    this.selectedDestination = this.manager.getWorkspaceSelectedDestination();
  }

  getChildren(element?: DestinationGroupTreeItem | DestinationTreeItem): vscode.ProviderResult<vscode.TreeItem[]> {
    if (!element) {
      return this.getRootElements();
    }

    if (element instanceof DestinationGroupTreeItem) {
      if (element.type === "iOSSimulator") {
        return this.getiOSSimulators();
      } else if (element.type === "iOSDevice") {
        return this.getiOSDevices();
      }
      return [];
    }

    return [];
  }

  getRootElements(): vscode.TreeItem[] {
    return [
      new DestinationGroupTreeItem({
        label: "iOS Simulators",
        type: "iOSSimulator",
        collapsibleState: vscode.TreeItemCollapsibleState.Expanded,
        icon: "sweetpad-square-letter-s",
      }),
      new DestinationGroupTreeItem({
        label: "iOS Devices",
        type: "iOSDevice",
        collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
        icon: "sweetpad-square-letter-d",
      }),
      // todo: add macOS device
      // todo: add watchOS device
      // todo: add watchOS simulator
      // todo: add tvOS device
      // todo: add tvOS simulator
    ];
  }

  getTreeItem(element: DestinationTreeItem): vscode.TreeItem {
    return element;
  }

  async getiOSSimulators(): Promise<DestinationTreeItem[]> {
    const simulators = await this.manager.getiOSSimulators();

    return simulators.map((simulator) => {
      return new iOSSimulatorDestinationTreeItem({
        simulator: simulator,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        provider: this,
      });
    });
  }

  async getiOSDevices(): Promise<DestinationTreeItem[]> {
    const device = await this.manager.getiOSDevices();

    return device.map((device) => {
      return new iOSDeviceDestinationTreeItem({
        device: device,
        collapsibleState: vscode.TreeItemCollapsibleState.None,
        provider: this,
      });
    });
  }
}
