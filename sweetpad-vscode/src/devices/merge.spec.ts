/**
 * Unit tests for mergeDeviceSources and resolveDeviceType.
 */

import * as fs from "node:fs";
import * as path from "node:path";

import * as sweetpadLib from "@sweetpad/native";

import type { DevicectlDevice } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";
import { mergeDeviceSources, resolveDeviceType } from "./merge";

function makeDevicectl(overrides: Partial<DevicectlDevice> = {}): DevicectlDevice {
  return {
    identifier: "urn:x-ios-devicectl:device-DC1",
    udid: "00008110-000DC0001",
    name: "iPhone 15 Pro",
    marketingName: "iPhone 15 Pro",
    productType: "iPhone16,1",
    deviceType: "iPhone",
    platform: "iOS",
    osVersion: "17.0",
    connection: "connected",
    pairing: "paired",
    ...overrides,
  };
}

/** A devicectl entry with an empty hardware section, as some USB iOS 16 devices get. */
function makeEmptyHardwareDevicectl(identifier = "urn:x-ios-devicectl:device-DC1"): DevicectlDevice {
  return { identifier: identifier, platform: "iOS", connection: "unavailable", pairing: "unsupported" };
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
    expect(resolveDeviceType({ devicectl: makeEmptyHardwareDevicectl() })).toBeNull();
    // A type devicectl names that no destination class covers is no type at all.
    expect(resolveDeviceType({ devicectl: makeDevicectl({ deviceType: "toaster" }) })).toBeNull();
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
    const dc = makeDevicectl({ udid: "00008110-MATCH" });
    const xc = makeXcdevice({ identifier: "00008110-MATCH" });
    const result = mergeDeviceSources([dc], [xc]);
    expect(result).toHaveLength(1);
    expect(result[0].devicectl).toBe(dc);
    expect(result[0].xcdevice).toBe(xc);
  });

  it("matches UDID case-insensitively", () => {
    const dc = makeDevicectl({ udid: "00008110-abcdef" });
    const xc = makeXcdevice({ identifier: "00008110-ABCDEF" });
    const result = mergeDeviceSources([dc], [xc]);
    expect(result).toHaveLength(1);
    expect(result[0].xcdevice).toBe(xc);
  });

  it("drops devicectl entries with empty hardwareProperties and no xcdevice match", () => {
    const dc = makeEmptyHardwareDevicectl();
    expect(mergeDeviceSources([dc], [])).toEqual([]);
  });

  it("recovers an iOS 16 USB device via xcdevice when devicectl has empty hardwareProperties", () => {
    const dc = makeEmptyHardwareDevicectl("urn:x-ios-devicectl:device-ORPHAN");
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

/** `devicectl list devices --json-output` from a test fixture, as the addon reads it. */
function parseFixture(name: string): DevicectlDevice[] {
  const json = fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data", name), "utf8");
  return sweetpadLib.parseDevicectlDevices(json);
}

describe("devicectl listings read by the addon", () => {
  it("classifies and pairs a jsonVersion 5 record on its properties dictionary", () => {
    const devicectl = sweetpadLib.parseDevicectlDevices(
      JSON.stringify({
        result: {
          devices: [
            {
              identifier: "urn:x-ios-devicectl:device-DC1",
              properties: {
                connection: { state: "connected" },
                hardware: { deviceType: "iPhone", marketingName: "iPhone 18 Pro", udid: "00008110-000DC0001" },
              },
            },
          ],
        },
      }),
    );
    expect(resolveDeviceType({ devicectl: devicectl[0] })).toBe("iPhone");

    const merged = mergeDeviceSources(devicectl, [makeXcdevice({ identifier: "00008110-000DC0001" })]);
    expect(merged).toHaveLength(1);
    expect(merged[0].devicectl).toBeDefined();
    expect(merged[0].xcdevice).toBeDefined();
  });

  /**
   * Xcode 27's devicectl lists the simulators beside the paired iPhone, each with a
   * deviceType, so they used to become iOS, watchOS, tvOS and visionOS devices with a
   * "platform=iOS,id=<simulator>" destination.
   */
  it("keeps the simulators Xcode 27's devicectl lists out of the devices", () => {
    const devicectl = parseFixture("devicectl-xcode-27-with-simulators.json");
    expect(devicectl.map((d) => d.name)).toEqual(["Iphone 13"]);

    const merged = mergeDeviceSources(devicectl, []);
    expect(merged).toHaveLength(1);
    expect(merged[0].devicectl?.udid).toBe("00008110-000559182E90401E");
  });
});
