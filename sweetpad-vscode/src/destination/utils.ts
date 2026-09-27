import os from "node:os";

import * as sweetpadLib from "@sweetpad/native";

import { ExtensionError } from "../common/errors";
import { assertUnreachable } from "../common/types";
import type { DestinationPlatform } from "./constants";
import type { Destination, GenericDestination } from "./types";

export function getMacOSArchitecture(): "arm64" | "x86_64" | null {
  const architecture = os.arch();

  switch (architecture) {
    case "arm64":
      return "arm64"; // Apple Silicon (M1, M2, etc.)
    case "x64":
      return "x86_64"; // Intel-based Mac
    default:
      return null;
  }
}

export function splitSupportedDestinatinos(options: {
  destinations: Destination[];
  supportedPlatforms: DestinationPlatform[] | undefined;
}): {
  supported: Destination[];
  unsupported: Destination[];
} {
  const { destinations, supportedPlatforms } = options;

  const supportedDestinations: Destination[] = [];
  const unsupportedDestinations: Destination[] = [];

  // If supportedPlatforms is undefined, we support all platforms
  if (supportedPlatforms === undefined) {
    return {
      supported: destinations,
      unsupported: [],
    };
  }
  for (const destination of destinations) {
    if (supportedPlatforms.includes(destination.platform)) {
      supportedDestinations.push(destination);
    } else {
      unsupportedDestinations.push(destination);
    }
  }

  return {
    supported: supportedDestinations,
    unsupported: unsupportedDestinations,
  };
}

/** Actions that install and launch something, so they need a real simulator or device. */
export type DeviceAction = "run" | "launch" | "test";

/** What a destination was picked for. */
export type DestinationAction = "build" | "clean" | DeviceAction;

/**
 * Every destination that binds a simulator or device. A device-less "Any … Device" is
 * the one thing a `DeviceAction` cannot be handed.
 */
export type RunnableDestination = Exclude<Destination, GenericDestination>;

const DEVICE_ACTIONS: ReadonlySet<DestinationAction> = new Set(["run", "launch", "test"]);

function needsDevice(action: DestinationAction): action is DeviceAction {
  return DEVICE_ACTIONS.has(action);
}

/**
 * Drop the destinations an action can't use, so the picker never offers a dead end.
 * Resolution is left alone: a pinned or recent "Any … Device" still resolves, and the
 * asserts below are what turn it into a readable error.
 */
export function filterDestinationsForAction(destinations: Destination[], action: DestinationAction): Destination[] {
  if (!needsDevice(action)) {
    return destinations;
  }
  return destinations.filter((destination) => destination.type !== "generic");
}

/**
 * Refuse a device-less destination before it reaches xcodebuild, which would otherwise
 * fail deep inside the task with a raw "unable to find a destination matching" line.
 */
export function assertRunnableDestination(
  destination: Destination,
  action: DeviceAction,
): asserts destination is RunnableDestination {
  if (destination.type !== "generic") {
    return;
  }
  throw new ExtensionError(
    `"${destination.name}" is a build-only destination. Pick a simulator or device to ${action}.`,
  );
}

/** The same check for callers holding an action they were handed rather than a literal. */
export function assertDestinationSupportsAction(destination: Destination, action: DestinationAction): void {
  if (needsDevice(action)) {
    assertRunnableDestination(destination, action);
  }
}

/**
 * The destination a task named, out of the ones currently known.
 *
 * A task can say `destinationId`, the deprecated `simulator`, or a raw `-destination`
 * string. The string is read field by field, the way the CLI reads it: `generic/platform=`
 * names a device-less build, `platform=macOS` this Mac, and anything else a simulator or
 * device by its `id=`, else by its `name=` (and `OS=`, when given). Other fields, such as
 * the `arch=x86_64` this extension writes when Rosetta destinations are on, don't get in
 * the way.
 */
export function findDestinationForTaskInput(
  destinations: Destination[],
  input: { destinationId?: string; simulator?: string; destination?: string },
): Destination | undefined {
  const spec = input.destination ? sweetpadLib.parseDestination(input.destination) : undefined;
  const udidRaw = input.destinationId ?? input.simulator ?? spec?.id;
  const udidLower = udidRaw?.trim()?.toLowerCase();

  // A device-less destination names a platform and nothing else, so "generic/platform=macOS"
  // is Any Mac rather than this Mac.
  const genericSdk = spec?.generic ? spec.sdk : undefined;
  const isMacOS = !spec?.generic && spec?.sdk === "macosx";

  const found = destinations.find((d) => {
    switch (d.type) {
      case "iOSSimulator":
      case "watchOSSimulator":
      case "visionOSSimulator":
      case "tvOSSimulator":
      case "iOSDevice":
      case "watchOSDevice":
      case "visionOSDevice":
      case "tvOSDevice":
        return d.udid.toLowerCase() === udidLower;
      case "macOS":
        return isMacOS;
      case "generic":
        // Build-only, so there is no udid: a task names one by its `-destination` string
        // or by its id.
        return spec?.generic ? d.platform === genericSdk : d.id === udidLower;
      default:
        assertUnreachable(d);
    }
  });
  if (found || udidLower || !spec?.name || spec.generic || !spec.sdk) {
    return found;
  }

  // `platform=iOS Simulator,name=iPhone 17,OS=26.5` names one by name, on the platform the
  // string gives. A booted simulator wins a tie, as it does for xcodebuild.
  const osVersion = spec.os && spec.os.toLowerCase() !== "latest" ? spec.os : undefined;
  const named = destinations.filter(
    (d): d is RunnableDestination =>
      d.type !== "generic" &&
      d.type !== "macOS" &&
      d.platform === spec.sdk &&
      d.name === spec.name &&
      (osVersion === undefined || d.osVersion === osVersion),
  );
  return named.find((d) => "isBooted" in d && d.isBooted) ?? named[0];
}
