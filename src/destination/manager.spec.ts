/**
 * Unit tests for DestinationsManager.sortCompareFn — the device-tier ordering
 * (status priority + lastConnectionDate) added for sweetpad-dev/sweetpad#234.
 */

import { createMockDevice } from "../__mocks__/devices";
import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import { DevicesManager } from "../devices/manager";
import { iOSDeviceDestination } from "../devices/types";
import type { SimulatorsManager } from "../simulators/manager";
import { DestinationsManager } from "./manager";

function buildManager(): DestinationsManager {
  const simulatorsManager = { on: jest.fn() } as unknown as SimulatorsManager;
  const devicesManager = new DevicesManager();
  return new DestinationsManager({ simulatorsManager, devicesManager });
}

function makeDevice(overrides: {
  name: string;
  udid: string;
  state?: "connected" | "disconnected" | "unavailable";
  lastConnectionDate?: string | null;
}): iOSDeviceDestination {
  const dc: DeviceCtlDevice = createMockDevice({
    deviceProperties: { name: overrides.name, osVersionNumber: "17.0" },
    hardwareProperties: {
      deviceType: "iPhone",
      marketingName: "iPhone 15 Pro",
      productType: "iPhone16,1",
      udid: overrides.udid,
      platform: "iOS",
    },
    connectionProperties: {
      tunnelState: overrides.state ?? "connected",
      pairingState: "paired",
      ...(overrides.lastConnectionDate === null
        ? {}
        : overrides.lastConnectionDate !== undefined
          ? { lastConnectionDate: overrides.lastConnectionDate }
          : {}),
    },
  });
  return new iOSDeviceDestination({ devicectl: dc });
}

describe("DestinationsManager.sortCompareFn", () => {
  let manager: DestinationsManager;

  beforeEach(() => {
    manager = buildManager();
  });

  describe("device status priority", () => {
    it("orders connected before disconnected before unavailable", () => {
      const connected = makeDevice({ name: "Z phone", udid: "udid-c", state: "connected" });
      const disconnected = makeDevice({ name: "A phone", udid: "udid-d", state: "disconnected" });
      const unavailable = makeDevice({ name: "M phone", udid: "udid-u", state: "unavailable" });

      const sorted = [unavailable, connected, disconnected].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted.map((d) => d.state)).toEqual(["connected", "disconnected", "unavailable"]);
    });

    it("keeps name from leaking across status buckets (connected wins over earlier-named unavailable)", () => {
      const connected = makeDevice({ name: "Z phone", udid: "udid-c", state: "connected" });
      const unavailable = makeDevice({ name: "A phone", udid: "udid-u", state: "unavailable" });

      const sorted = [unavailable, connected].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted[0].name).toBe("Z phone");
    });
  });

  describe("lastConnectionDate ordering within a status bucket", () => {
    it("places more-recently-connected device first", () => {
      const newer = makeDevice({
        name: "B phone",
        udid: "udid-new",
        state: "unavailable",
        lastConnectionDate: "2026-04-25T10:00:00Z",
      });
      const older = makeDevice({
        name: "A phone",
        udid: "udid-old",
        state: "unavailable",
        lastConnectionDate: "2024-01-01T10:00:00Z",
      });

      const sorted = [older, newer].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted.map((d) => d.udid)).toEqual(["udid-new", "udid-old"]);
    });

    it("treats missing lastConnectionDate as oldest within the same status bucket", () => {
      const dated = makeDevice({
        name: "Z phone",
        udid: "udid-dated",
        state: "unavailable",
        lastConnectionDate: "2024-01-01T10:00:00Z",
      });
      const undated = makeDevice({
        name: "A phone",
        udid: "udid-undated",
        state: "unavailable",
        lastConnectionDate: null,
      });

      const sorted = [undated, dated].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted.map((d) => d.udid)).toEqual(["udid-dated", "udid-undated"]);
    });
  });

  describe("name fallback", () => {
    it("falls back to alphabetical name when status and date are tied", () => {
      const sameDate = "2026-04-25T10:00:00Z";
      const z = makeDevice({ name: "Z phone", udid: "udid-z", state: "connected", lastConnectionDate: sameDate });
      const a = makeDevice({ name: "A phone", udid: "udid-a", state: "connected", lastConnectionDate: sameDate });

      const sorted = [z, a].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted.map((d) => d.name)).toEqual(["A phone", "Z phone"]);
    });

    it("falls back to alphabetical name when both devices have no lastConnectionDate", () => {
      const z = makeDevice({ name: "Z phone", udid: "udid-z", state: "connected", lastConnectionDate: null });
      const a = makeDevice({ name: "A phone", udid: "udid-a", state: "connected", lastConnectionDate: null });

      const sorted = [z, a].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted.map((d) => d.name)).toEqual(["A phone", "Z phone"]);
    });
  });

  describe("doronz88 scenario", () => {
    it("surfaces the one connected iPhone above a wall of unavailable paired entries", () => {
      const stale = Array.from({ length: 10 }, (_, i) =>
        makeDevice({
          name: `Stale iPhone ${String.fromCharCode(65 + i)}`,
          udid: `udid-stale-${i}`,
          state: "unavailable",
          lastConnectionDate: `2023-0${(i % 9) + 1}-01T10:00:00Z`,
        }),
      );
      const connected = makeDevice({
        name: "iPhone 11", // alphabetically-late name to prove status wins, not name
        udid: "udid-connected",
        state: "connected",
      });

      const sorted = [...stale, connected].sort((a, b) => manager.sortCompareFn(a, b));

      expect(sorted[0].udid).toBe("udid-connected");
    });
  });
});

describe("DeviceDestinationBase.lastConnectionDate", () => {
  it("parses ISO 8601 lastConnectionDate from devicectl", () => {
    const dc = createMockDevice({
      connectionProperties: {
        tunnelState: "connected",
        pairingState: "paired",
        lastConnectionDate: "2026-04-25T10:00:00Z",
      },
    });
    const dest = new iOSDeviceDestination({ devicectl: dc });

    expect(dest.lastConnectionDate?.toISOString()).toBe("2026-04-25T10:00:00.000Z");
  });

  it("returns null when devicectl omits lastConnectionDate", () => {
    const dc = createMockDevice();
    const dest = new iOSDeviceDestination({ devicectl: dc });

    expect(dest.lastConnectionDate).toBeNull();
  });

  it("returns null for xcdevice-only devices (no devicectl source)", () => {
    const dest = new iOSDeviceDestination({
      xcdevice: {
        identifier: "00008110-001234567890001E",
        modelCode: "iPhone16,1",
        name: "Old iPhone",
        operatingSystemVersion: "16.4",
        platform: "com.apple.platform.iphoneos",
      } as any,
    });

    expect(dest.lastConnectionDate).toBeNull();
  });

  it("returns null when devicectl lastConnectionDate is unparseable", () => {
    const dc = createMockDevice({
      connectionProperties: {
        tunnelState: "connected",
        pairingState: "paired",
        lastConnectionDate: "not-a-date",
      },
    });
    const dest = new iOSDeviceDestination({ devicectl: dc });

    expect(dest.lastConnectionDate).toBeNull();
  });
});
