import { iOSSimulator } from "../common/cli/scripts";
import { DestinationPlatform } from "./constants";
import { iOSDevice } from "../common/xcode/devicectl";

/**
 * Generic interface for a destination (iOS simulator, iOS device, etc.)
 */
interface IDestination {
  type: "iOSSimulator" | "iOSDevice";
  typeLabel: string;
  label: string;
  icon: string;
  udid: string;

  platform: DestinationPlatform;
}

/**
 * Thin wrapper around an iOS simulator class to implement the Destination interface
 */
export class iOSSimulatorDestination implements IDestination {
  type = "iOSSimulator" as const;
  typeLabel = "iOS Simulator";
  platform = DestinationPlatform.iphonesimulator;

  udid: string;
  name: string;
  osVersion: string;
  state: "Booted" | "Shutdown";

  private simulator: iOSSimulator;

  constructor(options: { simulator: iOSSimulator }) {
    this.simulator = options.simulator;
    this.udid = this.simulator.udid;
    this.name = this.simulator.name;
    this.osVersion = this.simulator.osVersion;
    this.state = this.simulator.state;
  }

  get label(): string {
    // iPhone 12 Pro Max (14.5)
    return `${this.simulator.name} (${this.simulator.osVersion})`;
  }

  get isBooted(): boolean {
    return this.simulator.state === "Booted";
  }

  get icon(): string {
    if (this.isBooted) {
      return "sweetpad-device-mobile";
    } else {
      return "sweetpad-device-mobile-pause";
    }
  }
}

export class iOSDeviceDestination implements IDestination {
  type = "iOSDevice" as const;
  typeLabel = "iOS Device";
  platform = DestinationPlatform.iphoneos;

  udid: string;
  osVersion: string;
  name: string;
  deviceType: "iPhone" | "iPad";

  private device: iOSDevice;

  constructor(options: { device: iOSDevice }) {
    this.device = options.device;
    this.udid = this.device.udid;
    this.osVersion = this.device.osVersion;
    this.name = this.device.name;
    this.deviceType = this.device.deviceType;
  }

  get label(): string {
    // iPhone 12 Pro Max (14.5)
    return `${this.device.name} (${this.device.osVersion})`;
  }

  get isConnected(): boolean {
    return this.device.state === "connected";
  }

  get icon(): string {
    if (this.device.deviceType === "iPad") {
      if (this.isConnected) {
        return "sweetpad-device-ipad";
      } else {
        return "sweetpad-device-ipad-x";
      }
    } else if (this.device.deviceType === "iPhone") {
      if (this.isConnected) {
        return "sweetpad-device-mobile";
      } else {
        return "sweetpad-device-mobile-x";
      }
    }
    return "sweetpad-device-mobile";
  }
}

export type Destination = iOSSimulatorDestination | iOSDeviceDestination;

/**
 * Lightweight representation of a selected destination that can be stored in the workspace state (we can't
 * store the full destination object because it contains non-serializable properties)
 */
export type SelectedDestination = {
  type: "iOSSimulator" | "iOSDevice";
  udid: string;
  name: string;
};
