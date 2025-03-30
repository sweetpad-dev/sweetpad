import * as vscode from "vscode";
import { assertUnreachable, checkUnreachable } from "../common/types.js";
import type {
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "../devices/types.js";
import type {
  iOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
  watchOSSimulatorDestination,
} from "../simulators/types.js";
import type { DestinationsManager } from "./manager.js";
import type { Destination, DestinationType, SelectedDestination, macOSDestination } from "./types.js";

function addSelectedMarks(options: {
  description: string;
  current: Destination;
  selectedForBuild: SelectedDestination | undefined;
  selectedForTesting: SelectedDestination | undefined;
}): string {
  const { current, selectedForBuild, selectedForTesting } = options;

  let description = options.description;
  const isSelectedForBuild = selectedForBuild?.type === current.type && selectedForBuild.id === current.id;
  const isSelectedForTesting = selectedForTesting?.type === current.type && selectedForTesting.id === current.id;

  if (isSelectedForBuild) {
    description = `${description} ✓`;
  }
  if (isSelectedForTesting) {
    description = `${description} (t)`;
  }

  return description;
}

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
    // - destination-group-watchOSSimulator
    // - destination-group-tvOSSimulator
    // - destination-group-visionOSSimulator
    // - destination-group-macOS
    // - destination-group-iOSDevice
    // - destination-group-watchOSDevice
    // - destination-group-tvOSDevice
    // - destination-group-visionOSDevice
    // this.contextValue = `destination-group-${this.type}`;
    this.contextValue = options.contextValue;
  }
}

/**
 * Common interface for destination tree item (items under tree group, second level)
 */
export interface IDestinationTreeItem extends vscode.TreeItem {
  type: DestinationType | "Recent";
}

/**
 * Base class for destination item (not group) that provides common functionality
 * like context value management in uniform way
 */
class BaseDestinationTreeItem extends vscode.TreeItem {
  contextPrefix: string;
  contextState: Record<string, string> = {};

  constructor(options: {
    label: string;
    collapsibleState: vscode.TreeItemCollapsibleState;
    contextPrefix: string;
  }) {
    super(options.label, options.collapsibleState);
    this.contextPrefix = options.contextPrefix;
    this.contextState = {};
    // No automatic updating of contextValue here
  }

  setContextState(key: string, value: string | undefined): void {
    if (value === undefined) {
      delete this.contextState[key];
    } else {
      this.contextState[key] = value;
    }
    // !Important: remember to call refreshContextValue() after setting context state
  }

  refreshContextValue(): void {
    let updated = `${this.contextPrefix}`;
    const sortedContextState = Object.entries(this.contextState).sort(([keyA], [keyB]) => keyA.localeCompare(keyB));
    for (const [key, value] of sortedContextState) {
      updated += `&${key}=${value}`;
    }
    this.contextValue = updated;
  }
}

/**
 * Tree item representing a iOSSimulator destination
 */
export class iOSSimulatorDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "iOSSimulator" as const;
  simulator: iOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { simulator: iOSSimulatorDestination; provider: DestinationsTreeProvider; isRecent?: boolean }) {
    super({
      label: options.simulator.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-simulator",
    });
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    let color: vscode.ThemeColor | undefined = undefined;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.simulator,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });
    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);

    if (this.simulator.isBooted) {
      this.setContextState("status", "booted");
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.setContextState("status", "shutdown");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): iOSSimulatorDestination {
    return this.simulator;
  }
}

/**
 * Tree item representing a watchOSSimulator destination
 */
export class watchOSSimulatorDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "watchOSSimulator" as const;
  simulator: watchOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: {
    simulator: watchOSSimulatorDestination;
    provider: DestinationsTreeProvider;
    isRecent?: boolean;
  }) {
    super({
      label: options.simulator.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-simulator",
    });
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    let color: vscode.ThemeColor | undefined = undefined;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.simulator,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });
    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);

    if (this.simulator.isBooted) {
      this.setContextState("status", "booted");
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.setContextState("status", "shutdown");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): watchOSSimulatorDestination {
    return this.simulator;
  }
}

export class visionOSSimulatorDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "visionOSSimulator" as const;
  simulator: visionOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: {
    simulator: visionOSSimulatorDestination;
    provider: DestinationsTreeProvider;
    isRecent?: boolean;
  }) {
    super({
      label: options.simulator.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-simulator",
    });
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    let color: vscode.ThemeColor | undefined = undefined;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.simulator,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });
    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);

    if (this.simulator.isBooted) {
      this.setContextState("status", "booted");
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.setContextState("status", "shutdown");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): visionOSSimulatorDestination {
    return this.simulator;
  }
}

class tvOSSimulatorDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "tvOSSimulator" as const;
  simulator: tvOSSimulatorDestination;
  provider: DestinationsTreeProvider;

  constructor(options: {
    simulator: tvOSSimulatorDestination;
    provider: DestinationsTreeProvider;
    isRecent?: boolean;
  }) {
    super({
      label: options.simulator.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-simulator",
    });
    this.description = options.simulator.osVersion;
    this.simulator = options.simulator;
    this.provider = options.provider;

    let color: vscode.ThemeColor | undefined = undefined;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.simulator,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });
    this.iconPath = new vscode.ThemeIcon(this.simulator.icon, color);

    if (this.simulator.isBooted) {
      this.setContextState("status", "booted");
      color = new vscode.ThemeColor("sweetpad.simulator.booted");
    } else {
      this.setContextState("status", "shutdown");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): tvOSSimulatorDestination {
    return this.simulator;
  }
}

/**
 * Tree item representing a iOS device destination
 */
export class iOSDeviceDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "iOSDevice" as const;
  device: iOSDeviceDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { device: iOSDeviceDestination; provider: DestinationsTreeProvider; isRecent?: boolean }) {
    super({
      label: options.device.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-ios",
    });
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.osVersion;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.device,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    if (this.device.isConnected) {
      this.setContextState("status", "connected");
    } else {
      this.setContextState("status", "disconnected");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): iOSDeviceDestination {
    return this.device;
  }
}

export class macOSDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "macOS" as const;
  device: macOSDestination;
  provider: DestinationsTreeProvider;
  constructor(options: { device: macOSDestination; provider: DestinationsTreeProvider; isRecent?: boolean }) {
    super({
      label: options.device.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-macos",
    });
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.arch;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.device,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    this.setContextState("status", "connected");
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): macOSDestination {
    return this.device;
  }
}

/**
 * Tree item representing a watchOS device destination
 */
export class watchOSDeviceDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "watchOSDevice" as const;
  device: watchOSDeviceDestination;
  provider: DestinationsTreeProvider;
  constructor(options: { device: watchOSDeviceDestination; provider: DestinationsTreeProvider; isRecent?: boolean }) {
    super({
      label: options.device.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-watchos",
    });
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.osVersion;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.device,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    if (this.device.isConnected) {
      this.setContextState("status", "connected");
    } else {
      this.setContextState("status", "disconnected");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): watchOSDeviceDestination {
    return this.device;
  }
}

export class tvOSDeviceDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "tvOSDevice" as const;
  device: tvOSDeviceDestination;

  provider: DestinationsTreeProvider;

  constructor(options: { device: tvOSDeviceDestination; provider: DestinationsTreeProvider; isRecent?: boolean }) {
    super({
      label: options.device.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-tvos",
    });
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.osVersion;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.device,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    if (this.device.isConnected) {
      this.setContextState("status", "connected");
    } else {
      this.setContextState("status", "disconnected");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): tvOSDeviceDestination {
    return this.device;
  }
}

export class visionOSDeviceDestinationTreeItem extends BaseDestinationTreeItem implements IDestinationTreeItem {
  type = "visionOSDevice" as const;
  device: visionOSDeviceDestination;
  provider: DestinationsTreeProvider;

  constructor(options: { device: visionOSDeviceDestination; provider: DestinationsTreeProvider; isRecent?: boolean }) {
    super({
      label: options.device.name,
      collapsibleState: vscode.TreeItemCollapsibleState.None,
      contextPrefix: "destination-item-visionos",
    });
    this.device = options.device;
    this.provider = options.provider;

    this.description = options.device.osVersion;
    this.description = addSelectedMarks({
      description: this.description,
      current: this.device,
      selectedForBuild: this.provider.selectedDestinationForBuild,
      selectedForTesting: this.provider.selectedDestinationForTesting,
    });

    this.iconPath = new vscode.ThemeIcon(this.device.icon, undefined);
    if (this.device.isConnected) {
      this.setContextState("status", "connected");
    } else {
      this.setContextState("status", "disconnected");
    }
    if (options.isRecent) {
      this.setContextState("recent", "true");
    }
    this.refreshContextValue();
  }

  get destination(): visionOSDeviceDestination {
    return this.device;
  }
}

// Tagged union type for destination tree item (second level)
export type DestinationTreeItem =
  | iOSSimulatorDestinationTreeItem
  | watchOSSimulatorDestinationTreeItem
  | tvOSSimulatorDestinationTreeItem
  | visionOSSimulatorDestinationTreeItem
  | macOSDestinationTreeItem
  | iOSDeviceDestinationTreeItem
  | watchOSDeviceDestinationTreeItem
  | tvOSDeviceDestinationTreeItem
  | visionOSDeviceDestinationTreeItem;

export class DestinationsTreeProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  public manager: DestinationsManager;
  public selectedDestinationForBuild: SelectedDestination | undefined;
  public selectedDestinationForTesting: SelectedDestination | undefined;

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
    this.manager.on("xcodeDestinationForBuildUpdated", (destination) => {
      this.selectedDestinationForBuild = destination;
      this._onDidChangeTreeData.fire(null); // todo: update only the selected destination
    });
    this.manager.on("xcodeDestinationForTestingUpdated", (destination) => {
      this.selectedDestinationForTesting = destination;
      this._onDidChangeTreeData.fire(null); // todo: update only the selected destination
    });
    this.manager.on("recentDestinationsUpdated", () => {
      this._onDidChangeTreeData.fire(null); // todo: update only the recent destinations
    });
    this.selectedDestinationForBuild = this.manager.getSelectedXcodeDestinationForBuild();
    this.selectedDestinationForTesting = this.manager.getSelectedXcodeDestinationForTesting();
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
      if (element.type === "tvOSSimulator") {
        return await this.gettvOSSimulators();
      }
      if (element.type === "visionOSSimulator") {
        return await this.getvisionOSSimulators();
      }
      if (element.type === "macOS") {
        return await this.getmacOSDevices();
      }
      if (element.type === "iOSDevice") {
        return await this.getiOSDevices();
      }
      if (element.type === "watchOSDevice") {
        return await this.getWatchOSDevices();
      }
      if (element.type === "tvOSDevice") {
        return this.gettvOSDevices();
      }
      if (element.type === "visionOSDevice") {
        return this.getVisionOSDevices();
      }
      if (element.type === "Recent") {
        return await this.getRecentDestinations();
      }
      assertUnreachable(element.type);
    }

    return [];
  }

  async getRecentDestinations(): Promise<vscode.TreeItem[]> {
    const mostUsed = await this.manager.getRecentDestinations();

    return mostUsed.map((destination) => {
      if (destination.type === "iOSSimulator") {
        return new iOSSimulatorDestinationTreeItem({
          simulator: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "watchOSSimulator") {
        return new watchOSSimulatorDestinationTreeItem({
          simulator: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "tvOSSimulator") {
        return new tvOSSimulatorDestinationTreeItem({
          simulator: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "visionOSSimulator") {
        return new visionOSSimulatorDestinationTreeItem({
          simulator: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "macOS") {
        return new macOSDestinationTreeItem({
          device: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "iOSDevice") {
        return new iOSDeviceDestinationTreeItem({
          device: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "watchOSDevice") {
        return new watchOSDeviceDestinationTreeItem({
          device: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "tvOSDevice") {
        return new tvOSDeviceDestinationTreeItem({
          device: destination,
          provider: this,
          isRecent: true,
        });
      }
      if (destination.type === "visionOSDevice") {
        return new visionOSDeviceDestinationTreeItem({
          device: destination,
          provider: this,
          isRecent: true,
        });
      }
      checkUnreachable(destination);
      return destination;
    });
  }

  getRootElements(): vscode.TreeItem[] {
    const groups = [];

    // Special group that shows destinations of all types that were used recently
    const isUsageStat = this.manager.isRecentExists();
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
          icon: "sweetpad-square-letter-i",
        }),
        new DestinationGroupTreeItem({
          label: "watchOS Simulators",
          type: "watchOSSimulator",
          contextValue: "destination-group-simulator-watchos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-w",
        }),
        new DestinationGroupTreeItem({
          label: "tvOS Simulators",
          type: "tvOSSimulator",
          contextValue: "destination-group-simulator-tvos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-t",
        }),
        new DestinationGroupTreeItem({
          label: "visionOS Simulators",
          type: "visionOSSimulator",
          contextValue: "destination-group-simulator-visionos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-square-letter-v",
        }),
        new DestinationGroupTreeItem({
          label: "macOS",
          type: "macOS",
          contextValue: "destination-group-macos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-circle-letter-m",
        }),
        new DestinationGroupTreeItem({
          label: "iOS Devices",
          type: "iOSDevice",
          contextValue: "destination-group-device-ios",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-circle-letter-i",
        }),
        new DestinationGroupTreeItem({
          label: "watchOS Devices",
          type: "watchOSDevice",
          contextValue: "destination-group-device-watchos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-circle-letter-w",
        }),
        new DestinationGroupTreeItem({
          label: "tvOS Devices",
          type: "tvOSDevice",
          contextValue: "destination-group-device-tvos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-circle-letter-t",
        }),
        new DestinationGroupTreeItem({
          label: "visionOS Devices",
          type: "visionOSDevice",
          contextValue: "destination-group-device-visionos",
          collapsibleState: vscode.TreeItemCollapsibleState.Collapsed,
          icon: "sweetpad-circle-letter-v",
        }),
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

  async gettvOSSimulators(): Promise<DestinationTreeItem[]> {
    const simulators = await this.manager.gettvOSSimulators();

    return simulators.map((simulator) => {
      return new tvOSSimulatorDestinationTreeItem({
        simulator: simulator,
        provider: this,
      });
    });
  }

  async getvisionOSSimulators(): Promise<DestinationTreeItem[]> {
    const simulators = await this.manager.getvisionOSSimulators();

    return simulators.map((simulator) => {
      return new visionOSSimulatorDestinationTreeItem({
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

  async getWatchOSDevices(): Promise<DestinationTreeItem[]> {
    const devices = await this.manager.getWatchOSDevices();

    return devices.map((device) => {
      return new watchOSDeviceDestinationTreeItem({
        device: device,
        provider: this,
      });
    });
  }

  async getVisionOSDevices(): Promise<DestinationTreeItem[]> {
    const devices = await this.manager.getVisionOSDevices();

    return devices.map((device) => {
      return new visionOSDeviceDestinationTreeItem({
        device: device,
        provider: this,
      });
    });
  }

  async gettvOSDevices(): Promise<DestinationTreeItem[]> {
    const devices = await this.manager.gettvOSDevices();

    return devices.map((device) => {
      return new tvOSDeviceDestinationTreeItem({
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
