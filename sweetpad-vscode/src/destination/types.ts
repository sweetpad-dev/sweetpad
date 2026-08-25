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
  | "visionOSDevice"
  // A device-less "Any <platform> Device" destination (xcodebuild's
  // `generic/platform=…`). Build-only — it can't be run or debugged.
  | "generic";

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
  "generic",
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
  generic: "generic-",
};

/**
 * Expand a "sweetpad.build.destination" id into the canonical form the destination
 * classes produce, so a hand-written setting can name just the udid.
 */
/**
 * The platform slug inside a generic id: lowercase, spaces hyphenated. "iOS Simulator"
 * becomes "ios-simulator".
 */
function genericPlatformSlug(platformArg: string): string {
  return platformArg.toLowerCase().replace(/\s+/g, "-");
}

export function normalizeDestinationId(id: string, type: DestinationType): string {
  // A hand-edited setting can name a type that doesn't exist, and there is no prefix to
  // apply for one. Leave the id alone so a fully qualified one still matches; the picker
  // rewrites the setting either way.
  const prefix = DESTINATION_ID_PREFIX[type];
  if (!prefix) {
    return id;
  }
  // A generic id names a platform rather than a udid, so a hand-written "iOS Simulator"
  // is the same destination as "ios-simulator" and folds into the canonical slug.
  const value = type === "generic" ? genericPlatformSlug(id) : id;
  return value.startsWith(prefix) ? value : `${prefix}${value}`;
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

/**
 * A device-less "Any <platform> Device" destination — xcodebuild's
 * `generic/platform=…`. It binds no specific simulator or device, so it is
 * **build-only**: building/archiving works, but running and debugging do not.
 * These are synthetic (no I/O to enumerate), mirroring `macOSDestination`.
 */
export class GenericDestination implements IDestination {
  type = "generic" as const;
  typeLabel = "Generic";
  icon = "vm";

  readonly name: string;
  readonly platform: DestinationPlatform;
  /** The label after `generic/platform=`, e.g. "iOS" or "iOS Simulator". */
  readonly platformArg: string;

  constructor(options: { name: string; platform: DestinationPlatform; platformArg: string }) {
    this.name = options.name;
    this.platform = options.platform;
    this.platformArg = options.platformArg;
  }

  get id(): string {
    return `generic-${genericPlatformSlug(this.platformArg)}`;
  }

  get label(): string {
    return this.name;
  }

  get quickPickDetails(): string {
    return `Type: ${this.typeLabel} (build-only), Destination: ${this.xcodebuildDestination}`;
  }

  /** The value passed to `xcodebuild -destination` for a device-less platform build. */
  get xcodebuildDestination(): string {
    return `generic/platform=${this.platformArg}`;
  }
}

/**
 * The generic build-only destinations Xcode exposes as "Any … Device". Static, so
 * they're always offered (a scheme's supported platforms filter them in the picker).
 */
export const GENERIC_DESTINATIONS: readonly GenericDestination[] = [
  new GenericDestination({ name: "Any iOS Device", platform: "iphoneos", platformArg: "iOS" }),
  new GenericDestination({
    name: "Any iOS Simulator Device",
    platform: "iphonesimulator",
    platformArg: "iOS Simulator",
  }),
  new GenericDestination({ name: "Any Mac", platform: "macosx", platformArg: "macOS" }),
  new GenericDestination({ name: "Any watchOS Device", platform: "watchos", platformArg: "watchOS" }),
  new GenericDestination({
    name: "Any watchOS Simulator Device",
    platform: "watchsimulator",
    platformArg: "watchOS Simulator",
  }),
  new GenericDestination({ name: "Any tvOS Device", platform: "appletvos", platformArg: "tvOS" }),
  new GenericDestination({
    name: "Any tvOS Simulator Device",
    platform: "appletvsimulator",
    platformArg: "tvOS Simulator",
  }),
  new GenericDestination({ name: "Any visionOS Device", platform: "xros", platformArg: "visionOS" }),
  new GenericDestination({
    name: "Any visionOS Simulator Device",
    platform: "xrsimulator",
    platformArg: "visionOS Simulator",
  }),
];

export type Destination =
  | iOSSimulatorDestination
  | watchOSSimulatorDestination
  | tvOSSimulatorDestination
  | visionOSSimulatorDestination
  | macOSDestination
  | iOSDeviceDestination
  | watchOSDeviceDestination
  | tvOSDeviceDestination
  | visionOSDeviceDestination
  | GenericDestination;

/**
 * Lightweight representation of a selected destination that can be stored in the workspace state (we can't
 * store the full destination object because it contains non-serializable properties)
 */
export type SelectedDestination = {
  id: string;
  type: DestinationType;
  name: string;
};
