/**
 * DESTINATION_ID_PREFIX is a hand-written table, and the ids it describes are built
 * independently by each destination class. These tests pin the two together, so
 * renaming a prefix in a class fails here instead of silently breaking a pinned
 * "sweetpad.build.destination".
 */
import { createMockDevice } from "../__mocks__/devices";
import {
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "../devices/types";
import {
  iOSSimulatorDestination,
  tvOSSimulatorDestination,
  visionOSSimulatorDestination,
  watchOSSimulatorDestination,
} from "../simulators/types";
import {
  ALL_DESTINATION_TYPES,
  DESTINATION_ID_PREFIX,
  type Destination,
  GenericDestination,
  macOSDestination,
  normalizeDestinationId,
} from "./types";

const UDID = "AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE";
const FULL_ID_FOR_TEST = `iossimulator-${UDID}`;

function simulator<T>(
  Ctor: new (options: any) => T,
  options: { simulatorType: string; os: string; runtime: string },
): T {
  return new Ctor({
    udid: UDID,
    isAvailable: true,
    state: "Shutdown",
    name: "Test Simulator",
    simulatorType: options.simulatorType,
    os: options.os,
    osVersion: "17.0",
    rawDeviceTypeIdentifier: `com.apple.CoreSimulator.SimDeviceType.${options.simulatorType}`,
    rawRuntime: options.runtime,
  });
}

function device<T>(Ctor: new (options: any) => T): T {
  return new Ctor({
    devicectl: createMockDevice({ hardwareProperties: { udid: UDID } as any }),
  });
}

const DESTINATIONS: Destination[] = [
  simulator(iOSSimulatorDestination, {
    simulatorType: "iPhone",
    os: "iOS",
    runtime: "com.apple.CoreSimulator.SimRuntime.iOS-17-0",
  }),
  simulator(watchOSSimulatorDestination, {
    simulatorType: "AppleWatch",
    os: "watchOS",
    runtime: "com.apple.CoreSimulator.SimRuntime.watchOS-10-0",
  }),
  simulator(tvOSSimulatorDestination, {
    simulatorType: "AppleTV",
    os: "tvOS",
    runtime: "com.apple.CoreSimulator.SimRuntime.tvOS-17-0",
  }),
  simulator(visionOSSimulatorDestination, {
    simulatorType: "AppleVision",
    os: "xrOS",
    runtime: "com.apple.CoreSimulator.SimRuntime.xrOS-1-0",
  }),
  new macOSDestination({ name: "My Mac", arch: "arm64" }),
  device(iOSDeviceDestination),
  device(watchOSDeviceDestination),
  device(tvOSDeviceDestination),
  device(visionOSDeviceDestination),
  new GenericDestination({ name: "Any iOS Device", platform: "iphoneos", platformArg: "iOS" }),
];

describe("DESTINATION_ID_PREFIX", () => {
  it("covers every destination type", () => {
    expect(Object.keys(DESTINATION_ID_PREFIX).toSorted()).toEqual([...ALL_DESTINATION_TYPES].toSorted());
  });

  it("has one destination under test per type", () => {
    expect(DESTINATIONS.map((d) => d.type).toSorted()).toEqual([...ALL_DESTINATION_TYPES].toSorted());
  });

  for (const destination of DESTINATIONS) {
    it(`matches the id ${destination.type} builds`, () => {
      expect(destination.id.startsWith(DESTINATION_ID_PREFIX[destination.type])).toBe(true);
    });
  }
});

describe("normalizeDestinationId", () => {
  it("expands a bare udid using the type", () => {
    expect(normalizeDestinationId(UDID, "iOSSimulator")).toBe(`iossimulator-${UDID}`);
    expect(normalizeDestinationId(UDID, "visionOSSimulator")).toBe(`visionsimulator-${UDID}`);
    expect(normalizeDestinationId("My Mac", "macOS")).toBe("macos-My Mac");
  });

  it("leaves the id alone for a type the settings file made up", () => {
    const madeUp = "iossimulator" as never;
    expect(normalizeDestinationId(FULL_ID_FOR_TEST, madeUp)).toBe(FULL_ID_FOR_TEST);
    expect(normalizeDestinationId(UDID, madeUp)).toBe(UDID);
  });

  it("leaves an already-prefixed id alone", () => {
    for (const destination of DESTINATIONS) {
      expect(normalizeDestinationId(destination.id, destination.type)).toBe(destination.id);
    }
  });

  it("round-trips every type's bare id back to what the class builds", () => {
    for (const destination of DESTINATIONS) {
      const bare = destination.id.slice(DESTINATION_ID_PREFIX[destination.type].length);
      expect(normalizeDestinationId(bare, destination.type)).toBe(destination.id);
    }
  });
});
