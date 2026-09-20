/**
 * Unit tests for the accessors that read devicectl's two device shapes.
 */

import { createMockDevice, createMockDeviceV5 } from "../../__mocks__/devices";
import {
  type DeviceCtlDevice,
  deviceLastConnectionDate,
  deviceMarketingName,
  deviceName,
  deviceOsVersion,
  deviceProductType,
  deviceTunnelState,
  deviceType,
  deviceUdid,
} from "./devicectl";

describe("devicectl device accessors", () => {
  it("reads a jsonVersion 4 device from the deprecated property bags", () => {
    const device = createMockDevice();

    expect(deviceUdid(device)).toBe("00008110-001234567890001E");
    expect(deviceName(device)).toBe("iPhone 14 Pro");
    expect(deviceMarketingName(device)).toBe("iPhone 14 Pro");
    expect(deviceProductType(device)).toBe("iPhone15,2");
    expect(deviceType(device)).toBe("iPhone");
    expect(deviceOsVersion(device)).toBe("17.0");
    expect(deviceTunnelState(device)).toBe("connected");
  });

  it("reads a jsonVersion 5 device from the properties dictionary", () => {
    const device = createMockDeviceV5();

    expect(deviceUdid(device)).toBe("00008110-001234567890001E");
    expect(deviceName(device)).toBe("iPhone 14 Pro");
    expect(deviceMarketingName(device)).toBe("iPhone 14 Pro");
    expect(deviceProductType(device)).toBe("iPhone15,2");
    expect(deviceType(device)).toBe("iPhone");
    // The version-5 osVersionNumber is an object, not the string itself.
    expect(deviceOsVersion(device)).toBe("17.0");
    // …and tunnelState is spelled "state" under "connection".
    expect(deviceTunnelState(device)).toBe("connected");
  });

  it("returns undefined for a device with neither shape", () => {
    const device: DeviceCtlDevice = { capabilities: [], identifier: "ID-1", visibilityClass: "default" };

    expect(deviceUdid(device)).toBeUndefined();
    expect(deviceName(device)).toBeUndefined();
    expect(deviceOsVersion(device)).toBeUndefined();
    expect(deviceTunnelState(device)).toBeUndefined();
  });

  /**
   * Xcode 27 fills both shapes at once and they agree; this pins which one is
   * believed if they ever don't.
   */
  it("prefers the properties dictionary over the deprecated bags", () => {
    const device = createMockDevice({
      properties: {
        connection: { state: "disconnected" },
        hardware: { marketingName: "iPhone 18 Pro", udid: "UDID-NEW" },
        software: { osVersionNumber: { stringValue: "27.0" } },
        state: { name: "My iPhone" },
      },
    });

    expect(deviceUdid(device)).toBe("UDID-NEW");
    expect(deviceName(device)).toBe("My iPhone");
    expect(deviceMarketingName(device)).toBe("iPhone 18 Pro");
    expect(deviceOsVersion(device)).toBe("27.0");
    expect(deviceTunnelState(device)).toBe("disconnected");
    // Not carried in the override, so the deprecated bag still answers.
    expect(deviceProductType(device)).toBe("iPhone15,2");
  });
});

describe("deviceLastConnectionDate", () => {
  it("parses the deprecated ISO-8601 string", () => {
    const device = createMockDevice({
      connectionProperties: { pairingState: "paired", lastConnectionDate: "2026-09-19T22:21:56.000Z" },
    });

    expect(deviceLastConnectionDate(device)?.toISOString()).toBe("2026-09-19T22:21:56.000Z");
  });

  it("converts the version-5 value from Core Foundation absolute time", () => {
    const device = createMockDeviceV5({
      properties: { connection: { lastConnectionDate: 811_549_316 } },
    });

    expect(deviceLastConnectionDate(device)?.toISOString()).toBe("2026-09-19T22:21:56.000Z");
  });

  it("is null when the field is absent or unparseable", () => {
    expect(deviceLastConnectionDate(createMockDevice())).toBeNull();
    expect(
      deviceLastConnectionDate(
        createMockDevice({ connectionProperties: { pairingState: "paired", lastConnectionDate: "not a date" } }),
      ),
    ).toBeNull();
  });
});
