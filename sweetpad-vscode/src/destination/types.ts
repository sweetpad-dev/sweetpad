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
 * The prefix each destination class puts in front of its udid (or, for macOS, the
 * computer name) to form an id. Note "visionOSSimulator" drops the "os" and "macOS"
 * carries no udid, so the prefix cannot be derived from the type name — id-prefixes.spec.ts
 * pins this table to what the classes actually produce.
 *
 * Used to accept a bare udid in "sweetpad.build.destination", where the type is already
 * given and repeating it in the id is noise.
 */
export const DESTINATION_ID_PREFIX: Record<DestinationType, string> = {
  iOSSimulator: "iossimulator-",
  watchOSSimulator: "watchossimulator-",
  tvOSSimulator: "tvossimulator-",
  visionOSSimulator: "visionsimulator-",
  macOS: "macos-",
  iOSDevice: "iosdevice-",
  watchOSDevice: "watchosdevice-",
  tvOSDevice: "tvosdevice-",
  visionOSDevice: "visionosdevice-",
};

/**
 * Expand a "sweetpad.build.destination" id into the canonical form the destination
 * classes produce, so a hand-written setting can name just the udid.
 */
export function normalizeDestinationId(id: string, type: DestinationType): string {
  // A hand-edited setting can name a type that doesn't exist, and there is no prefix to
  // apply for one. Leave the id alone so a fully qualified one still matches; the picker
  // rewrites the setting either way.
  const prefix = DESTINATION_ID_PREFIX[type];
  if (!prefix) {
    return id;
  }
  return id.startsWith(prefix) ? id : `${prefix}${id}`;
}

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
