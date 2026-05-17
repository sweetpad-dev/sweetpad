import type {
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "../devices/types";
import type {
  iOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
  watchOSSimulatorDestination,
} from "../simulators/types";
import type { DestinationPlatform } from "./constants";

// Sometimes it can be called as "platform" or "DestinationPlatform"
export type DestinationType =
  | "iOSSimulator"
  | "watchOSSimulator"
  | "tvOSSimulator"
  | "visionOSSimulator"
  | "macOS"
  | "iOSDevice"
  | "watchOSDevice"
  | "tvOSDevice"
  | "visionOSDevice";

export type DestinationArch = "arm64" | "x86_64";

export const ALL_DESTINATION_TYPES: DestinationType[] = [
  "iOSSimulator",
  "watchOSSimulator",
  "tvOSSimulator",
  "visionOSSimulator",
  "macOS",
  "iOSDevice",
  "watchOSDevice",
  "tvOSDevice",
  "visionOSDevice",
];

/**
 * Generic interface for a destination (iOS simulator, iOS device, etc.)
 */
export interface IDestination {
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

export class macOSDestination implements IDestination {
  type = "macOS" as const;
  typeLabel = "macOS Device";
  platform = "macosx" as const;

  name: string;
  arch: DestinationArch;

  constructor(options: { name: string; arch: DestinationArch }) {
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

export type Destination =
  | iOSSimulatorDestination
  | watchOSSimulatorDestination
  | tvOSSimulatorDestination
  | visionOSSimulatorDestination
  | macOSDestination
  | iOSDeviceDestination
  | watchOSDeviceDestination
  | tvOSDeviceDestination
  | visionOSDeviceDestination;

/**
 * Lightweight representation of a selected destination that can be stored in the workspace state (we can't
 * store the full destination object because it contains non-serializable properties)
 */
export type SelectedDestination = {
  id: string;
  type: DestinationType;
  name: string;
};
