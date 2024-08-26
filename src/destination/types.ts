import { iOSSimulator, iOSSimulatorDeviceType } from "../common/cli/scripts";
import { DestinationPlatform } from "./constants";
import { DeviceCtlDeviceType, iOSDevice } from "../common/xcode/devicectl";

export type DestinationType = "iOSSimulator" | "iOSDevice" | "macOS";

export type DestinationArch = "arm64" | "x86_64";

export const ALL_DESTINATION_TYPES: DestinationType[] = ["iOSSimulator", "iOSDevice", "macOS"];

/**
 * Generic interface for a destination (iOS simulator, iOS device, etc.)
 */
interface IDestination {

  // Unique identifier for the destination for internal use.
  // This should be unique and never null or undefined.
  id: string;
  type: DestinationType;
  typeLabel: string;
  label: string;
  icon: string;
  platform: DestinationPlatform;
  quickPickDetails: string;
}

/**
 * Thin wrapper around an iOS simulator class to implement the Destination interface
 */
export class iOSSimulatorDestination implements IDestination {
  type = "iOSSimulator" as const;
  typeLabel = "iOS Simulator";
  platform = "iphonesimulator" as const;

  udid: string;
  name: string;
  osVersion: string;
  state: "Booted" | "Shutdown";
  deviceType: iOSSimulatorDeviceType | null;

  private simulator: iOSSimulator;

  constructor(options: { simulator: iOSSimulator }) {
    this.simulator = options.simulator;
    this.udid = this.simulator.udid;
    this.name = this.simulator.name;
    this.osVersion = this.simulator.osVersion;
    this.state = this.simulator.state;
    this.deviceType = this.simulator.deviceType;
  }

  get id(): string {
    return `iossimulator-${this.udid}`;
  }

  get label(): string {
    // iPhone 12 Pro Max (14.5)
    return `${this.simulator.name} (${this.simulator.osVersion})`;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
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
  platform = "iphoneos" as const;

  udid: string;
  osVersion: string;
  name: string;
  deviceType: DeviceCtlDeviceType;

  private device: iOSDevice;

  constructor(options: { device: iOSDevice }) {
    this.device = options.device;
    this.udid = this.device.udid;
    this.osVersion = this.device.osVersion;
    this.name = this.device.name;
    this.deviceType = this.device.deviceType;
  }

  get id(): string {
    return `iosdevice-${this.udid}`;
  }

  get label(): string {
    // iPhone 12 Pro Max (14.5)
    return `${this.device.name} (${this.device.osVersion})`;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Version: ${this.osVersion}, ID: ${this.udid.toLocaleLowerCase()}`;
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

export class MacOSDestination implements IDestination {
  type = "macOS" as const;
  typeLabel = "macOS Device";
  platform = "macosx" as const;

  name: string;
  arch: DestinationArch;

  constructor(options: { name: string, arch: DestinationArch }) {
    this.name = options.name;
    this.arch = options.arch;
  }

  get id(): string {
    return `macos-${this.name}`;
  }

  get label(): string {
    return `${this.name}`;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel}, Arch: ${this.arch}`;
  }

  get icon(): string {
    return "sweetpad-device-laptop";
  }
}

export type Destination = iOSSimulatorDestination | iOSDeviceDestination | MacOSDestination;

/**
 * Lightweight representation of a selected destination that can be stored in the workspace state (we can't
 * store the full destination object because it contains non-serializable properties)
 */
export type SelectedDestination = {
  id: string;
  type: DestinationType;
  name: string;
};
