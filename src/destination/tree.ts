import * as vscode from "vscode";
import { assertUnreachable, checkUnreachable } from "../common/types.js";
import type { iOSDeviceDestination } from "../devices/types.js";
import type { iOSSimulatorDestination, watchOSSimulatorDestination } from "../simulators/types.js";
import type { DestinationsManager } from "./manager.js";
import type { DestinationType, SelectedDestination, macOSDestination } from "./types.js";

/**
 * Tree item representing a group of destinations (iOSSimulator, iOSDevice, etc.) at the root level
 */
class DestinationGroupTreeItem extends vscode.TreeItem {
  type: DestinationType | "Recent";

  constructor(options: {
    label: string;
    type: DestinationType | "Recent";
    contextValue: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    icon: string;
  }) {
    super(options.label, options.collapsibleState);
    this.type = options.type;
    this.iconPath = new vscode.ThemeIcon(options.icon);

    // - destination-group-iOSSimulator
    // - destination-group-iOSDevice
    // - destination-group-macOS
    // - destination-group-watchOSSimulator
    // this.contextValue = `destination-group-${this.type}`;
    this.contextValue = options.contextValue;
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

    const contextPrefix = "destination-item-simulator";

    const isSelected =
      this.provider.selectedDestination?.type === "iOSSimulator" &&
      this.provider.selectedDestination.id === this.simulator.id;

    let color: vscode.ThemeColor | undefined = undefined;
    if (isSelected) {
      this.description = `${this.description} ✓`;
    }

    if (this.simulator.isBooted) {
      this.contextValue = `${contextPrefix}-booted`; // "destination-item-simulator-booted"
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.contextValue = `${contextPrefix}-shutdown`; // "destination-item-simulator-shutdown"
    }

    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);
  }

  get destination(): iOSSimulatorDestination {
    return this.simulator;
  }
}

/**
 * Tree item representing a watchOSSimulator destination
 */
export class watchOSSimulatorDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "watchOSSimulator" as const;
  simulator: watchOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { simulator: watchOSSimulatorDestination; provider: DestinationsTreeProvider }) {
    super(options.simulator.name, vscode.TreeItemCollapsibleState.None);
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    const contextPrefix = "destination-item-simulator";

    const isSelected =
      this.provider.selectedDestination?.type === "watchOSSimulator" &&
      this.provider.selectedDestination.id === this.simulator.id;

    let color: vscode.ThemeColor | undefined = undefined;
    if (isSelected) {
      this.description = `${this.description} ✓`;
    }

    if (this.simulator.isBooted) {
      this.contextValue = `${contextPrefix}-booted`; // "destination-item-simulator-booted"
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.contextValue = `${contextPrefix}-shutdown`; // "destination-item-simulator-shutdown"
    }

    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);
  }

  get destination(): watchOSSimulatorDestination {
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

export class macOSDestinationTreeItem extends vscode.TreeItem implements IDestinationTreeItem {
  type = "macOS" as const;
  device: macOSDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { device: macOSDestination; provider: DestinationsTreeProvider }) {
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

  get destination(): macOSDestination {
    return this.device;
  }
}

// Tagged union type for destination tree item (second level)
export type DestinationTreeItem =
  | iOSSimulatorDestinationTreeItem
  | iOSDeviceDestinationTreeItem
  | watchOSSimulatorDestinationTreeItem
  | macOSDestinationTreeItem;

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

  async getChildren(element?: DestinationGroupTreeItem | DestinationTreeItem): Promise<vscode.TreeItem[]> {
    if (!element) {
      return await this.getRootElements();
    }

    if (element instanceof DestinationGroupTreeItem) {
      if (element.type === "iOSSimulator") {
        return await this.getiOSSimulators();
      }
      if (element.type === "watchOSSimulator") {
        return await this.getwatchOSSimulators();
      }
      if (element.type === "iOSDevice") {
        return await this.getiOSDevices();
      }
      if (element.type === "macOS") {
        return await this.getmacOSDevices();
      }
      if (element.type === "Recent") {
        return await this.getRecentDestinations();
      }
      assertUnreachable(element.type);
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
      if (destination.type === "watchOSSimulator") {
        return new watchOSSimulatorDestinationTreeItem({
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
        return new macOSDestinationTreeItem({
          device: destination,
          provider: this,
        });
      }
      checkUnreachable(destination);
      return destination;
    });
  }

  getRootElements(): vscode.TreeItem[] {
    const isMacosEnabled = this.manager.isMacOSDestinationEnabled();
    const groups = [];

    // Special group that shows destinations of all types that were used recently
    const isUsageStat = this.manager.isUsageStatsExist();
    if (isUsageStat) {
      groups.push(
        new DestinationGroupTreeItem({
          label: "Recent",
          type: "Recent",
          contextValue: "destination-group-recent",
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
          contextValue: "destination-group-simulator-ios",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-s",
        }),
        new DestinationGroupTreeItem({
          label: "watchOS Simulators",
          type: "watchOSSimulator",
          contextValue: "destination-group-simulator-watchos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-s",
        }),
        new DestinationGroupTreeItem({
          label: "iOS Devices",
          type: "iOSDevice",
          contextValue: "destination-group-device-ios",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-d",
        }),
        ...(isMacosEnabled
          ? [
              new DestinationGroupTreeItem({
                label: "macOS",
                type: "macOS",
                contextValue: "destination-group-macos",
                collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
                icon: "sweetpad-square-letter-m",
              }),
            ]
          : []),
        // todo: add watchOS simulator
        // todo: add tvOS device
        // todo: add tvOS simulator
      ],
    );

    // Make first item Expanded by default
    groups[0].collapsibleState = vscode.TreeItemCollapsibleState.Expanded;

    return groups;
  }

  async getTreeItem(element: DestinationTreeItem): Promise<vscode.TreeItem> {
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

  async getwatchOSSimulators(): Promise<DestinationTreeItem[]> {
    const simulators = await this.manager.getwatchOSSimulators();

    return simulators.map((simulator) => {
      return new watchOSSimulatorDestinationTreeItem({
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
      return new macOSDestinationTreeItem({
        device: device,
        provider: this,
      });
    });
  }
}
