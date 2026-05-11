import events from "node:events";

import { checkUnreachable } from "../common/types";
import type { WorkspaceStateService } from "../common/workspace-state";
import type { DevicesManager } from "../devices/manager";
import {
  DeviceDestinationBase,
  type iOSDeviceDestination,
  type tvOSDeviceDestination,
  type visionOSDeviceDestination,
  type watchOSDeviceDestination,
} from "../devices/types";
import type { SimulatorsManager } from "../simulators/manager";
import type {
  SimulatorDestination,
  iOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
  watchOSSimulatorDestination,
} from "../simulators/types";
import {
  DESTINATION_TYPE_PRIORITY,
  DEVICE_STATE_PRIORITY,
  SIMULATOR_TYPE_PRIORITY,
  SUPPORTED_DESTINATION_PLATFORMS,
} from "./constants";
import {
  ALL_DESTINATION_TYPES,
  type Destination,
  type DestinationType,
  type SelectedDestination,
  macOSDestination,
} from "./types";
import { getMacOSArchitecture } from "./utils";

type IEventMap = {
  simulatorsUpdated: [];
  devicesUpdated: [];
  xcodeDestinationForBuildUpdated: [destination: SelectedDestination | undefined];
  xcodeDestinationForTestingUpdated: [destination: SelectedDestination | undefined];
  recentDestinationsUpdated: [];
};

type IEventKey = keyof IEventMap;

export class DestinationsManager {
  private simulatorsManager: SimulatorsManager;
  private devicesManager: DevicesManager;
  private workspace: WorkspaceStateService;

  // Event emitter to signal changes in the destinations
  private emitter = new events.EventEmitter<IEventMap>();

  constructor(options: {
    simulatorsManager: SimulatorsManager;
    devicesManager: DevicesManager;
    workspace: WorkspaceStateService;
  }) {
    this.simulatorsManager = options.simulatorsManager;
    this.devicesManager = options.devicesManager;
    this.workspace = options.workspace;
  }

  async start(): Promise<void> {
    // Forward events from simulators and devices managers
    this.simulatorsManager.on("updated", () => {
      this.emitter.emit("simulatorsUpdated");
    });
    this.devicesManager.on("updated", () => {
      this.emitter.emit("devicesUpdated");
    });
  }

  on<K extends IEventKey>(event: K, listener: (...args: IEventMap[K]) => void): void {
    this.emitter.on(event, listener as any); // todo: fix this any
  }

  async refreshSimulators(): Promise<SimulatorDestination[]> {
    return await this.simulatorsManager.refresh();
  }

  async refreshDevices() {
    await this.devicesManager.refresh();
  }

  async refresh() {
    await this.refreshSimulators();
    await this.refreshDevices();
  }

  isRecentExists(): boolean {
    const recent = this.workspace.get("build.xcodeDestinationsRecent");
    return Array.isArray(recent) && recent.length > 0;
  }

  async getRecentDestinations(): Promise<Destination[]> {
    const rawDestinations = this.workspace.get("build.xcodeDestinationsRecent") ?? [];

    const destinations: Destination[] = [];
    for (const rawDestination of rawDestinations) {
      const destination = await this.findDestination({
        destinationId: rawDestination.id,
        type: rawDestination.type,
      });
      if (destination) {
        destinations.push(destination);
      }
    }

    return destinations;
  }

  async getSimulators(options?: { sort?: boolean }): Promise<SimulatorDestination[]> {
    const simulators = await this.simulatorsManager.getSimulators();

    const items = [...simulators];
    if (options?.sort) {
      items.sort((a, b) => this.sortCompareFn(a, b));
    }
    return items;
  }

  async getiOSSimulators(options?: { sort?: boolean }): Promise<iOSSimulatorDestination[]> {
    const simulators = await this.simulatorsManager.getSimulators();
    const items = [...simulators];

    if (options?.sort) {
      items.sort((a, b) => this.sortCompareFn(a, b));
    }

    return items.filter((simulator) => simulator.type === "iOSSimulator");
  }

  async getwatchOSSimulators(): Promise<watchOSSimulatorDestination[]> {
    const simulators = await this.simulatorsManager.getSimulators();
    return simulators.filter((simulator) => simulator.type === "watchOSSimulator");
  }

  async gettvOSSimulators(): Promise<tvOSSimulatorDestination[]> {
    const simulators = await this.simulatorsManager.getSimulators();
    return simulators.filter((simulator) => simulator.type === "tvOSSimulator");
  }

  async getvisionOSSimulators(): Promise<visionOSSimulatorDestination[]> {
    const simulators = await this.simulatorsManager.getSimulators();
    return simulators.filter((simulator) => simulator.type === "visionOSSimulator");
  }

  async getiOSDevices(options?: { sort?: boolean }): Promise<iOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    const items = [...devices];

    if (options?.sort) {
      items.sort((a, b) => this.sortCompareFn(a, b));
    }

    return items.filter((device) => device.type === "iOSDevice");
  }

  async getWatchOSDevices(options?: { sort?: boolean }): Promise<watchOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    const items = [...devices];

    if (options?.sort) {
      items.sort((a, b) => this.sortCompareFn(a, b));
    }

    return items.filter((device) => device.type === "watchOSDevice");
  }

  async gettvOSDevices(options?: { sort?: boolean }): Promise<tvOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    const items = [...devices];

    if (options?.sort) {
      items.sort((a, b) => this.sortCompareFn(a, b));
    }

    return items.filter((device) => device.type === "tvOSDevice");
  }

  async getVisionOSDevices(options?: { sort?: boolean }): Promise<visionOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    const items = [...devices];

    if (options?.sort) {
      items.sort((a, b) => this.sortCompareFn(a, b));
    }

    return items.filter((device) => device.type === "visionOSDevice");
  }

  async getmacOSDevices(): Promise<macOSDestination[]> {
    const currentArch = getMacOSArchitecture() ?? "arm64";
    return [
      new macOSDestination({
        name: "My Mac",
        arch: currentArch,
      }),
    ];
  }

  /**
   * Function for sorting destinations. This function is not include sorting by usage statistics
   * bacause it's not needed in a lot of cases and should be done separately
   */
  sortCompareFn(a: Destination, b: Destination): number {
    const aPriority = DESTINATION_TYPE_PRIORITY.findIndex((type) => type === a.type);
    const bPriority = DESTINATION_TYPE_PRIORITY.findIndex((type) => type === b.type);
    if (aPriority !== bPriority) {
      return aPriority - bPriority;
    }

    // todo: sort ipad/iPhone/xorOS
    if (a.type === "iOSSimulator" && b.type === "iOSSimulator") {
      const aSimPriority = SIMULATOR_TYPE_PRIORITY.findIndex((type) => type === a.simulatorType);
      const bSimPriority = SIMULATOR_TYPE_PRIORITY.findIndex((type) => type === b.simulatorType);
      if (aSimPriority !== bSimPriority) {
        return aSimPriority - bSimPriority;
      }
    }

    // Show connected devices first so the one plugged in right now isn't buried under
    // every iPhone this Mac has ever been paired with.
    if (a instanceof DeviceDestinationBase && b instanceof DeviceDestinationBase) {
      const stateDelta = DEVICE_STATE_PRIORITY.indexOf(a.state) - DEVICE_STATE_PRIORITY.indexOf(b.state);
      if (stateDelta !== 0) {
        return stateDelta;
      }
      const aDate = a.lastConnectionDate?.getTime() ?? 0;
      const bDate = b.lastConnectionDate?.getTime() ?? 0;
      if (aDate !== bDate) {
        return bDate - aDate;
      }
    }

    // In any other cases, fallback to sorting by name
    return a.name.localeCompare(b.name);
  }

  async getDestinations(options?: { mostUsedSort?: boolean }): Promise<Destination[]> {
    const destinations: Destination[] = [];

    const platforms = SUPPORTED_DESTINATION_PLATFORMS;

    for (const platform of platforms) {
      if (platform === "macosx") {
        const devices = await this.getmacOSDevices();
        destinations.push(...devices);
      } else if (platform === "iphoneos") {
        const devices = await this.getiOSDevices();
        destinations.push(...devices);
      } else if (platform === "iphonesimulator") {
        const simulators = await this.getiOSSimulators();
        destinations.push(...simulators);
      } else if (platform === "appletvos") {
        const devices = await this.gettvOSDevices();
        destinations.push(...devices);
      } else if (platform === "appletvsimulator") {
        const simulators = await this.gettvOSSimulators();
        destinations.push(...simulators);
      } else if (platform === "watchos") {
        const devices = await this.getWatchOSDevices();
        destinations.push(...devices);
      } else if (platform === "watchsimulator") {
        const simulators = await this.getwatchOSSimulators();
        destinations.push(...simulators);
      } else if (platform === "xros") {
        const devices = await this.getVisionOSDevices();
        destinations.push(...devices);
      } else if (platform === "xrsimulator") {
        const simulators = await this.getvisionOSSimulators();
        destinations.push(...simulators);
      } else {
        checkUnreachable(platform);
      }
    }

    // Most used destinations should be on top of the list
    if (options?.mostUsedSort) {
      const usageStats = this.workspace.get("build.xcodeDestinationsUsageStatistics") ?? {};
      destinations.sort((a, b) => {
        const aCount = usageStats[a.id] ?? 0;
        const bCount = usageStats[b.id] ?? 0;
        if (aCount !== bCount) {
          return bCount - aCount;
        }
        return this.sortCompareFn(a, b);
      });
    }

    return destinations;
  }

  /**
   * Find a destination by its udid and type
   */
  async findDestination(options: { destinationId: string; type?: DestinationType }): Promise<Destination | undefined> {
    const types: DestinationType[] = options.type ? [options.type] : ALL_DESTINATION_TYPES;

    let destination: Destination | undefined = undefined;
    if (!destination && types.includes("iOSSimulator")) {
      const simulators = await this.getiOSSimulators();
      destination = simulators.find((simulator) => simulator.id === options.destinationId);
    }
    if (!destination && types.includes("watchOSSimulator")) {
      const simulators = await this.getwatchOSSimulators();
      destination = simulators.find((simulator) => simulator.id === options.destinationId);
    }
    if (!destination && types.includes("tvOSSimulator")) {
      const simulators = await this.gettvOSSimulators();
      destination = simulators.find((simulator) => simulator.id === options.destinationId);
    }
    if (!destination && types.includes("iOSDevice")) {
      const devices = await this.getiOSDevices();
      destination = devices.find((device) => device.id === options.destinationId);
    }
    if (!destination && types.includes("watchOSDevice")) {
      const devices = await this.getWatchOSDevices();
      destination = devices.find((device) => device.id === options.destinationId);
    }
    if (!destination && types.includes("macOS")) {
      const devices = await this.getmacOSDevices();
      destination = devices.find((device) => device.id === options.destinationId);
    }
    if (!destination && types.includes("visionOSSimulator")) {
      const simulators = await this.getvisionOSSimulators();
      destination = simulators.find((simulator) => simulator.id === options.destinationId);
    }
    return destination;
  }

  trackSelectedDestination(destination: Destination) {
    this.trackDestinationUsage(destination);
    this.trackRecentDestination(destination);
  }

  /**
   * Collect statistics about the usage of the destinations
   */
  trackDestinationUsage(destination: Destination) {
    // Incrmement usage statistics
    const prevStat = this.workspace.get("build.xcodeDestinationsUsageStatistics") ?? {};
    const count: number = prevStat[destination.id] ?? 0;
    this.workspace.update("build.xcodeDestinationsUsageStatistics", {
      ...prevStat,
      [destination.id]: count + 1,
    });
  }

  trackRecentDestination(destination: Destination) {
    // Add to recent destinations
    const recentDestinations = this.workspace.get("build.xcodeDestinationsRecent") ?? [];
    const recentDestination = recentDestinations.find((d) => d.id === destination.id);
    if (!recentDestination) {
      const newRecentDestination: SelectedDestination = {
        id: destination.id,
        type: destination.type,
        name: destination.name,
      };
      this.workspace.update("build.xcodeDestinationsRecent", [...recentDestinations, newRecentDestination]);
    }
  }

  removeRecentDestination(destination: Destination) {
    const recentDestinations = this.workspace.get("build.xcodeDestinationsRecent") ?? [];
    const newRecentDestinations = recentDestinations.filter((d) => d.id !== destination.id);
    this.workspace.update("build.xcodeDestinationsRecent", newRecentDestinations);
    this.emitter.emit("recentDestinationsUpdated");
  }

  setWorkspaceDestinationForBuild(destination: Destination | undefined) {
    if (!destination) {
      this.workspace.update("build.xcodeDestination", undefined);
      this.emitter.emit("xcodeDestinationForBuildUpdated", undefined);
      return;
    }

    const selectedDestination: SelectedDestination = {
      id: destination.id,
      type: destination.type,
      name: destination.name,
    };
    this.workspace.update("build.xcodeDestination", selectedDestination);
    this.trackSelectedDestination(destination);

    this.emitter.emit("xcodeDestinationForBuildUpdated", selectedDestination);
  }

  setWorkspaceDestinationForTesting(destination: Destination | undefined) {
    if (!destination) {
      this.workspace.update("testing.xcodeDestination", undefined);
      this.emitter.emit("xcodeDestinationForTestingUpdated", undefined);
      return;
    }

    const selectedDestination: SelectedDestination = {
      id: destination.id,
      type: destination.type,
      name: destination.name,
    };
    this.workspace.update("testing.xcodeDestination", selectedDestination);
    this.trackSelectedDestination(destination);
    this.emitter.emit("xcodeDestinationForTestingUpdated", selectedDestination);
  }

  /**
   * Get selected destination from the workspace state
   */
  getSelectedXcodeDestinationForBuild(): SelectedDestination | undefined {
    return this.workspace.get("build.xcodeDestination");
  }

  getSelectedXcodeDestinationForTesting(): SelectedDestination | undefined {
    return this.workspace.get("testing.xcodeDestination");
  }
}
