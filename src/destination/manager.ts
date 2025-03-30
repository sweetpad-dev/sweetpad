import events from "node:events";
import type { ExtensionContext } from "../common/commands";
import { checkUnreachable } from "../common/types";
import type { DevicesManager } from "../devices/manager";
import type {
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "../devices/types";
import type { SimulatorsManager } from "../simulators/manager";
import type {
  SimulatorDestination,
  iOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
  watchOSSimulatorDestination,
} from "../simulators/types";
import { DESTINATION_TYPE_PRIORITY, SIMULATOR_TYPE_PRIORITY, SUPPORTED_DESTINATION_PLATFORMS } from "./constants";
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
  private _context: ExtensionContext | undefined;

  // Event emitter to signal changes in the destinations
  private emitter = new events.EventEmitter<IEventMap>();

  constructor(options: { simulatorsManager: SimulatorsManager; devicesManager: DevicesManager }) {
    this.simulatorsManager = options.simulatorsManager;
    this.devicesManager = options.devicesManager;
    this._context = undefined;

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

  get context() {
    if (!this._context) {
      throw new Error("Context is not set");
    }
    return this._context;
  }

  set context(context: ExtensionContext) {
    this._context = context;
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
    const recent = this.context.getWorkspaceState("build.xcodeDestinationsRecent");
    return Array.isArray(recent) && recent.length > 0;
  }

  async getRecentDestinations(): Promise<Destination[]> {
    const rawDestinations = this.context.getWorkspaceState("build.xcodeDestinationsRecent") ?? [];

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

  async getiOSDevices(): Promise<iOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    return devices.filter((device) => device.type === "iOSDevice");
  }

  async getWatchOSDevices(): Promise<watchOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    return devices.filter((device) => device.type === "watchOSDevice");
  }

  async gettvOSDevices(): Promise<tvOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    return devices.filter((device) => device.type === "tvOSDevice");
  }

  async getVisionOSDevices(): Promise<visionOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    return devices.filter((device) => device.type === "visionOSDevice");
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
      const aPriority = SIMULATOR_TYPE_PRIORITY.findIndex((type) => type === a.simulatorType);
      const bPriority = SIMULATOR_TYPE_PRIORITY.findIndex((type) => type === b.simulatorType);
      if (aPriority !== bPriority) {
        return aPriority - bPriority;
      }
    }

    // if (a.type === "iOSDevice" && b.type === "iOSDevice") {
    //   const aPriority = DESTINATION_IOS_DEVICE_TYPE_PRIORITY.findIndex((type) => type === a.deviceType);
    //   const bPriority = DESTINATION_IOS_DEVICE_TYPE_PRIORITY.findIndex((type) => type === b.deviceType);
    //   if (aPriority !== bPriority) {
    //     return aPriority - bPriority;
    //   }
    // }

    // In any other cases, fallback to sorting by name
    return a.name.localeCompare(b.name);
  }

  async getDestinations(options?: {
    mostUsedSort?: boolean;
  }): Promise<Destination[]> {
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
      const usageStats = this.context.getWorkspaceState("build.xcodeDestinationsUsageStatistics") ?? {};
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
  async findDestination(options: {
    destinationId: string;
    type?: DestinationType;
  }): Promise<Destination | undefined> {
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
    const prevStat = this.context.getWorkspaceState("build.xcodeDestinationsUsageStatistics") ?? {};
    const count: number = prevStat[destination.id] ?? 0;
    this.context.updateWorkspaceState("build.xcodeDestinationsUsageStatistics", {
      ...prevStat,
      [destination.id]: count + 1,
    });
  }

  trackRecentDestination(destination: Destination) {
    // Add to recent destinations
    const recentDestinations = this.context.getWorkspaceState("build.xcodeDestinationsRecent") ?? [];
    const recentDestination = recentDestinations.find((d) => d.id === destination.id);
    if (!recentDestination) {
      const newRecentDestination: SelectedDestination = {
        id: destination.id,
        type: destination.type,
        name: destination.name,
      };
      this.context.updateWorkspaceState("build.xcodeDestinationsRecent", [...recentDestinations, newRecentDestination]);
    }
  }

  removeRecentDestination(destination: Destination) {
    const recentDestinations = this.context.getWorkspaceState("build.xcodeDestinationsRecent") ?? [];
    const newRecentDestinations = recentDestinations.filter((d) => d.id !== destination.id);
    this.context.updateWorkspaceState("build.xcodeDestinationsRecent", newRecentDestinations);
    this.emitter.emit("recentDestinationsUpdated");
  }

  setWorkspaceDestinationForBuild(destination: Destination | undefined) {
    if (!destination) {
      this.context.updateWorkspaceState("build.xcodeDestination", undefined);
      this.emitter.emit("xcodeDestinationForBuildUpdated", undefined);
      return;
    }

    const selectedDestination: SelectedDestination = {
      id: destination.id,
      type: destination.type,
      name: destination.name,
    };
    this.context.updateWorkspaceState("build.xcodeDestination", selectedDestination);
    this.trackSelectedDestination(destination);

    this.emitter.emit("xcodeDestinationForBuildUpdated", selectedDestination);
  }

  setWorkspaceDestinationForTesting(destination: Destination | undefined) {
    if (!destination) {
      this.context.updateWorkspaceState("testing.xcodeDestination", undefined);
      this.emitter.emit("xcodeDestinationForTestingUpdated", undefined);
      return;
    }

    const selectedDestination: SelectedDestination = {
      id: destination.id,
      type: destination.type,
      name: destination.name,
    };
    this.context.updateWorkspaceState("testing.xcodeDestination", selectedDestination);
    this.trackSelectedDestination(destination);
    this.emitter.emit("xcodeDestinationForTestingUpdated", selectedDestination);
  }

  /**
   * Get selected destination from the workspace state
   */
  getSelectedXcodeDestinationForBuild(): SelectedDestination | undefined {
    return this.context.getWorkspaceState("build.xcodeDestination");
  }

  getSelectedXcodeDestinationForTesting(): SelectedDestination | undefined {
    return this.context.getWorkspaceState("testing.xcodeDestination");
  }
}
