import { DevicesManager } from "../devices/manager";
import { SimulatorsManager } from "../simulators/manager";
import { Destination, SelectedDestination, iOSDeviceDestination, iOSSimulatorDestination } from "./types";
import { SUPPORTED_DESTINATION_PLATFORMS } from "./constants";
import { DestinationPlatform } from "./constants";
import { DestinationOS } from "./constants";
import { ExtensionContext } from "../common/commands";
import events from "events";

type IEventMap = {
  refreshSimulators: [];
  refreshDevices: [];
  refreshWorkspaceDestination: [destination: SelectedDestination | undefined];
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
    this.simulatorsManager.on("refresh", () => {
      this.emitter.emit("refreshSimulators");
    });
    this.devicesManager.on("refresh", () => {
      this.emitter.emit("refreshDevices");
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

  refreshiOSSimulators() {
    this.simulatorsManager.refresh();
  }

  refreshiOSDevices() {
    this.devicesManager.refresh();
  }

  async getiOSSimulators(options?: { refresh?: boolean }): Promise<iOSSimulatorDestination[]> {
    const simulators = await this.simulatorsManager.getSimulators({
      refresh: options?.refresh,
    });
    return simulators
      .filter((simulator) => simulator.runtimeType === DestinationOS.iOS)
      .map((simulator) => new iOSSimulatorDestination({ simulator: simulator }));
  }

  async getiOSDevices(): Promise<iOSDeviceDestination[]> {
    const devices = await this.devicesManager.getDevices();
    return devices.map((device) => new iOSDeviceDestination({ device: device }));
  }

  async getDestinations(options?: { platformFilter?: DestinationPlatform[] }): Promise<Destination[]> {
    const destinations: Destination[] = [];

    const platforms = options?.platformFilter ?? SUPPORTED_DESTINATION_PLATFORMS;

    if (platforms.includes(DestinationPlatform.iphonesimulator)) {
      const simulators = await this.simulatorsManager.getSimulators();
      destinations.push(...simulators.map((simulator) => new iOSSimulatorDestination({ simulator: simulator })));
    }

    if (platforms.includes(DestinationPlatform.iphoneos)) {
      const devices = await this.devicesManager.getDevices();
      destinations.push(...devices.map((device) => new iOSDeviceDestination({ device: device })));
    }

    return destinations;
  }

  /**
   * Find a destination by its udid and type
   */
  async findDestination(options: {
    udid: string;
    type: "iOSSimulator" | "iOSDevice";
  }): Promise<Destination | undefined> {
    if (options.type === "iOSSimulator") {
      const simulators = await this.getiOSSimulators();
      return simulators.find((simulator) => simulator.udid === options.udid);
    } else if (options.type === "iOSDevice") {
      const devices = await this.getiOSDevices();
      return devices.find((device) => device.udid === options.udid);
    }
    return undefined;
  }

  fireSelectedDestinationRemoved() {
    this.emitter.emit("refreshWorkspaceDestination", undefined);
  }

  setWorkspaceDestination(destination: Destination) {
    const selectedDestination: SelectedDestination = {
      udid: destination.udid,
      type: destination.type,
      name: destination.name,
    };
    this.context.updateWorkspaceState("build.xcodeDestination", selectedDestination);

    this.emitter.emit("refreshWorkspaceDestination", selectedDestination);
  }

  /**
   * Get selected destination from the workspace state
   */
  getWorkspaceSelectedDestination(): SelectedDestination | undefined {
    return this.context.getWorkspaceState("build.xcodeDestination");
  }

  /*
   * Get selected destination from the workspace state. This function is async because it may need to
   * fetch the destination from the simulators and devices managers.
   */
  async findWorkspaceSelectedDestination(): Promise<Destination | undefined> {
    const destination = this.getWorkspaceSelectedDestination();
    if (!destination) {
      return undefined;
    }

    return await this.findDestination({
      udid: destination.udid,
      type: destination.type,
    });
  }
}
