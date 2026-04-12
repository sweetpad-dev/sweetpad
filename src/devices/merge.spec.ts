/**
 * Unit tests for mergeDeviceSources and resolveDeviceType.
 */

import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";
import { mergeDeviceSources, resolveDeviceType } from "./merge";

function makeDevicectl(overrides: Partial<DeviceCtlDevice> = {}): DeviceCtlDevice {
  return {
    capabilities: [],
    connectionProperties: {
      tunnelState: "connected",
      pairingState: "paired",
    },
    deviceProperties: {
      name: "iPhone 15 Pro",
      osVersionNumber: "17.0",
    },
    hardwareProperties: {
      deviceType: "iPhone",
      marketingName: "iPhone 15 Pro",
      productType: "iPhone16,1",
      udid: "00008110-000DC0001",
      platform: "iOS",
    },
    identifier: "urn:x-ios-devicectl:device-DC1",
    visibilityClass: "default",
    ...overrides,
  };
}

function makeXcdevice(overrides: Partial<XcdeviceDevice> = {}): XcdeviceDevice {
  return {
    identifier: "00008110-000XC0001",
    modelCode: "iPhone14,2",
    name: "My iPhone",
    operatingSystemVersion: "16.4.1",
    platform: "com.apple.platform.iphoneos",
    simulator: false,
    available: true,
    ...overrides,
  };
}

describe("resolveDeviceType", () => {
  it("prefers devicectl deviceType when present", () => {
    expect(resolveDeviceType({ devicectl: makeDevicectl() })).toBe("iPhone");
  });

  it("infers iPhone from iphoneos platform + iPhone modelCode", () => {
    expect(resolveDeviceType({ xcdevice: makeXcdevice({ modelCode: "iPhone14,2" }) })).toBe("iPhone");
  });

  it("infers iPad from iphoneos platform + iPad modelCode", () => {
    expect(resolveDeviceType({ xcdevice: makeXcdevice({ modelCode: "iPad14,5" }) })).toBe("iPad");
  });

  it("infers appleWatch from watchos platform", () => {
    expect(
      resolveDeviceType({ xcdevice: makeXcdevice({ platform: "com.apple.platform.watchos", modelCode: "Watch6,1" }) }),
    ).toBe("appleWatch");
  });

  it("infers appleTV from appletvos platform", () => {
    expect(
      resolveDeviceType({
        xcdevice: makeXcdevice({ platform: "com.apple.platform.appletvos", modelCode: "AppleTV11,1" }),
      }),
    ).toBe("appleTV");
  });

  it("infers appleVision from xros platform", () => {
    expect(
      resolveDeviceType({
        xcdevice: makeXcdevice({ platform: "com.apple.platform.xros", modelCode: "RealityDevice14,1" }),
      }),
    ).toBe("appleVision");
  });

  it("returns null when neither source can classify the device", () => {
    const dc = makeDevicectl({
      hardwareProperties: {} as any,
    });
    expect(resolveDeviceType({ devicectl: dc })).toBeNull();
  });
});

describe("mergeDeviceSources", () => {
  it("returns empty for empty inputs", () => {
    expect(mergeDeviceSources([], [])).toEqual([]);
  });

  it("keeps a devicectl-only iOS 17 device", () => {
    const result = mergeDeviceSources([makeDevicectl()], []);
    expect(result).toHaveLength(1);
    expect(result[0].devicectl).toBeDefined();
    expect(result[0].xcdevice).toBeUndefined();
  });

  it("keeps an xcdevice-only iOS 16 device (Wi-Fi case)", () => {
    const xc = makeXcdevice({ identifier: "00008110-0000IOS16WIFI" });
    const result = mergeDeviceSources([], [xc]);
    expect(result).toHaveLength(1);
    expect(result[0].devicectl).toBeUndefined();
    expect(result[0].xcdevice).toBe(xc);
  });

  it("deduplicates when both sources report the same UDID", () => {
    const dc = makeDevicectl({
      hardwareProperties: {
        deviceType: "iPhone",
        productType: "iPhone16,1",
        udid: "00008110-MATCH",
        platform: "iOS",
      },
    });
    const xc = makeXcdevice({ identifier: "00008110-MATCH" });
    const result = mergeDeviceSources([dc], [xc]);
    expect(result).toHaveLength(1);
    expect(result[0].devicectl).toBe(dc);
    expect(result[0].xcdevice).toBe(xc);
  });

  it("matches UDID case-insensitively", () => {
    const dc = makeDevicectl({
      hardwareProperties: {
        deviceType: "iPhone",
        productType: "iPhone16,1",
        udid: "00008110-abcdef",
        platform: "iOS",
      },
    });
    const xc = makeXcdevice({ identifier: "00008110-ABCDEF" });
    const result = mergeDeviceSources([dc], [xc]);
    expect(result).toHaveLength(1);
    expect(result[0].xcdevice).toBe(xc);
  });

  it("drops devicectl entries with empty hardwareProperties and no xcdevice match", () => {
    const dc = makeDevicectl({
      deviceProperties: {},
      hardwareProperties: {} as any,
    });
    expect(mergeDeviceSources([dc], [])).toEqual([]);
  });

  it("recovers an iOS 16 USB device via xcdevice when devicectl has empty hardwareProperties", () => {
    const dc = makeDevicectl({
      deviceProperties: {},
      hardwareProperties: {} as any,
      identifier: "urn:x-ios-devicectl:device-ORPHAN",
    });
    const xc = makeXcdevice({ identifier: "00008110-IOS16USB" });
    const result = mergeDeviceSources([dc], [xc]);
    expect(result).toHaveLength(1);
    expect(result[0].devicectl).toBeUndefined();
    expect(result[0].xcdevice).toBe(xc);
  });

  it("keeps both entries when UDIDs do not overlap", () => {
    const dc = makeDevicectl();
    const xc = makeXcdevice({ identifier: "00008110-DIFFERENT" });
    const result = mergeDeviceSources([dc], [xc]);
    expect(result).toHaveLength(2);
  });
});
