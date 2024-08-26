import * as vscode from "vscode";
import { checkUnreachable } from "../common/types.js";
import type { DestinationsManager } from "./manager.js";
import type {
  DestinationType,
  MacOSDestination,
  SelectedDestination,
  iOSDeviceDestination,
  iOSSimulatorDestination,
} from "./types.js";

/**
 * Tree item representing a group of destinations (iOSSimulator, iOSDevice, etc.) at the root level
 */
class DestinationGroupTreeItem extends vscode.TreeItem {
  type: DestinationType | "Recent";

  constructor(options: {
    label: string;
    type: DestinationType | "Recent";
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
  type: DestinationType | "Recent";
}

/**
 * Tree item representing a iOSSimulator destination
 */
export class iOSSimulatorDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "iOSSimulator" as const;
  simulator: iOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { simulator: iOSSimulatorDestination; provider: DestinationsTreeProvider }) {
    super(options.simulator.name, vscode.TreeItemCollapsibleState.None);
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    const contextPrefix = "destination-item-iOSSimulator";

    const isSelected =
      this.provider.selectedDestination?.type === "iOSSimulator" &&
      this.provider.selectedDestination.id === this.simulator.id;

    let color: vscode.ThemeColor | undefined = undefined;
    if (isSelected) {
      this.description = `${this.description} ✓`;
    }

    if (this.simulator.isBooted) {
      this.contextValue = `${contextPrefix}-booted`; // "destination-item-iOSSimulator-booted"
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.contextValue = `${contextPrefix}-shutdown`; // "destination-item-iOSSimulator-shutdown"
    }

    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);
  }

  get destination(): iOSSimulatorDestination {
    return this.simulator;
  }
}

/**
 * Tree item representing a iOS device destination
 */
export class iOSDeviceDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "iOSDevice" as const;
  device: iOSDeviceDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { device: iOSDeviceDestination; provider: DestinationsTreeProvider }) {
    super(options.device.name, vscode.TreeItemCollapsibleState.None);
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.osVersion;
    const contextPrefix = "destination-item-iOSDevice";

    const isSelected =
      this.provider.selectedDestination?.type === "iOSDevice" &&
      this.provider.selectedDestination.id === this.device.id;
    if (isSelected) {
      this.description = `${this.description} ✓`;
    }

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    if (this.device.isConnected) {
      this.contextValue = `${contextPrefix}-connected`; // "destination-item-iOSDevice-connected"
    } else {
      this.contextValue = `${contextPrefix}-disconnected`; // "destination-item-iOSDevice-disconnected
    }
  }

  get destination(): iOSDeviceDestination {
    return this.device;
  }
}

export class MacOSDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "macOS" as const;
  device: MacOSDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { device: MacOSDestination; provider: DestinationsTreeProvider }) {
    super(options.device.name, vscode.TreeItemCollapsibleState.None);
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.arch;
    const isSelected =
      this.provider.selectedDestination?.type === "macOS" && this.provider.selectedDestination.id === this.device.id;
    if (isSelected) {
      this.description = `${this.description} ✓`;
    }

    const contextPrefix = "destination-item-macos";

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    this.contextValue = `${contextPrefix}-connected`; // "destination-item-macOS-connected"
  }

  get destination(): MacOSDestination {
    return this.device;
  }
}

// Tagged union type for destination tree item (second level)
export type DestinationTreeItem =
  | iOSSimulatorDestinationTreeItem
  | iOSDeviceDestinationTreeItem
  | MacOSDestinationTreeItem;

export class DestinationsTreeProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  public manager: DestinationsManager;
  public selectedDestination: SelectedDestination | undefined;

  private _onDidChangeTreeData = new vscode.EventEmitter<DestinationTreeItem | undefined | null | undefined>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  constructor(options: { manager: DestinationsManager }) {
    this.manager = options.manager;
    this.manager.on("simulatorsUpdated", () => {
      this._onDidChangeTreeData.fire(null);
    });
    this.manager.on("devicesUpdated", () => {
      this._onDidChangeTreeData.fire(null);
    });
    this.manager.on("xcodeDestinationUpdated", (destination) => {
      this.selectedDestination = destination;
      this._onDidChangeTreeData.fire(null); // todo: update only the selected destination
    });
    this.selectedDestination = this.manager.getSelectedXcodeDestination();
  }

  getChildren(element?: DestinationGroupTreeItem | DestinationTreeItem): vscode.ProviderResult<vscode.TreeItem[]> {
    if (!element) {
      return this.getRootElements();
    }

    if (element instanceof DestinationGroupTreeItem) {
      if (element.type === "iOSSimulator") {
        return this.getiOSSimulators();
      }
      if (element.type === "iOSDevice") {
        return this.getiOSDevices();
      }
      if (element.type === "macOS") {
        return this.getmacOSDevices();
      }
      if (element.type === "Recent") {
        return this.getRecentDestinations();
      }
      return [];
    }

    return [];
  }

  async getRecentDestinations(): Promise<vscode.TreeItem[]> {
    const mostUsed = await this.manager.getMostUsedDestinations();

    return mostUsed.map((destination) => {
      if (destination.type === "iOSSimulator") {
        return new iOSSimulatorDestinationTreeItem({
          simulator: destination,
          provider: this,
        });
      }
      if (destination.type === "iOSDevice") {
        return new iOSDeviceDestinationTreeItem({
          device: destination,
          provider: this,
        });
      }
      if (destination.type === "macOS") {
        return new MacOSDestinationTreeItem({
          device: destination,
          provider: this,
        });
      }
      checkUnreachable(destination);
      return destination;
    });
  }

  getRootElements(): vscode.TreeItem[] {
    const groups = [];

    // Special group that shows destinations of all types that were used recently
    const isUsageStat = this.manager.isUsageStatsExist();
    if (isUsageStat) {
      groups.push(
        new DestinationGroupTreeItem({
          label: "Recent",
          type: "Recent",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-asterisk",
        }),
      );
    }

    groups.push(
      ...[
        new DestinationGroupTreeItem({
          label: "iOS Simulators",
          type: "iOSSimulator",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-s",
        }),
        new DestinationGroupTreeItem({
          label: "iOS Devices",
          type: "iOSDevice",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-d",
        }),
        new DestinationGroupTreeItem({
          label: "macOS",
          type: "macOS",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-m",
        }),
        // todo: add macOS device
        // todo: add watchOS device
        // todo: add watchOS simulator
        // todo: add tvOS device
        // todo: add tvOS simulator
      ],
    );

    // Make first item Expanded by default
    groups[0].collapsibleState = vscode.TreeItemCollapsibleState.Expanded;

    return groups;
  }

  getTreeItem(element: DestinationTreeItem): vscode.TreeItem {
    return element;
  }

  async getiOSSimulators(): Promise<DestinationTreeItem[]> {
    const simulators = await this.manager.getiOSSimulators({
      sort: true,
    });

    return simulators.map((simulator) => {
      return new iOSSimulatorDestinationTreeItem({
        simulator: simulator,
        provider: this,
      });
    });
  }

  async getiOSDevices(): Promise<DestinationTreeItem[]> {
    const device = await this.manager.getiOSDevices();

    return device.map((device) => {
      return new iOSDeviceDestinationTreeItem({
        device: device,
        provider: this,
      });
    });
  }

  async getmacOSDevices(): Promise<DestinationTreeItem[]> {
    const devices = await this.manager.getmacOSDevices();

    return devices.map((device) => {
      return new MacOSDestinationTreeItem({
        device: device,
        provider: this,
      });
    });
  }
}
