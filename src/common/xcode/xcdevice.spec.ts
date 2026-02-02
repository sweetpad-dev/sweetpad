/**
 * Unit tests for xcdevice fallback device listing
 */

import { exec } from "../exec";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  createNameLookup,
  createOsVersionLookup,
  createUdidLookup,
  getNameForDevice,
  getOsVersionForDevice,
  getUdidForDevice,
  listDevicesWithXcdevice,
} from "./xcdevice";
import { createMockContext } from "../../../tests/__mocks__/devices";

// Mock the exec function
jest.mock("../exec", () => ({
  exec: jest.fn(),
}));

// Mock logger to avoid output in tests
jest.mock("../logger", () => ({
  commonLogger: {
    debug: jest.fn(),
    error: jest.fn(),
  },
}));

describe("xcdevice", () => {
  describe("listDevicesWithXcdevice", () => {
    const mockContext = createMockContext();

    beforeEach(() => {
      jest.clearAllMocks();
    });

    it("returns iOS devices from xcdevice list output", async () => {
      const mockData = fs.readFileSync(
        path.join(__dirname, "../../../tests/xcdevice-data/xcdevice-ios-devices.json"),
        "utf8",
      );
      (exec as jest.Mock).mockResolvedValue(mockData);

      const devices = await listDevicesWithXcdevice(mockContext);

      expect(devices).toHaveLength(3);
      expect(devices[0].name).toBe("iPhone 14 Pro");
      expect(devices[0].modelCode).toBe("iPhone15,2");
      expect(devices[0].operatingSystemVersion).toBe("16.7.12");
      expect(devices[0].platform).toBe("com.apple.platform.iphoneos");
    });

    it("filters to only iOS devices from mixed platforms", async () => {
      const mockData = fs.readFileSync(
        path.join(__dirname, "../../../tests/xcdevice-data/xcdevice-mixed-devices.json"),
        "utf8",
      );
      (exec as jest.Mock).mockResolvedValue(mockData);

      const devices = await listDevicesWithXcdevice(mockContext);

      expect(devices).toHaveLength(2);
      expect(devices.every((d) => d.platform === "com.apple.platform.iphoneos")).toBe(true);
      expect(devices[0].name).toBe("iPhone 14 Pro");
      expect(devices[1].name).toBe("iPhone 14 Plus");
    });

    it("returns empty array when no devices found", async () => {
      const mockData = fs.readFileSync(
        path.join(__dirname, "../../../tests/xcdevice-data/xcdevice-empty.json"),
        "utf8",
      );
      (exec as jest.Mock).mockResolvedValue(mockData);

      const devices = await listDevicesWithXcdevice(mockContext);

      expect(devices).toEqual([]);
    });

    it("returns empty array and logs error for malformed JSON", async () => {
      const mockData = fs.readFileSync(
        path.join(__dirname, "../../../tests/xcdevice-data/xcdevice-malformed.json"),
        "utf8",
      );
      (exec as jest.Mock).mockResolvedValue(mockData);

      const devices = await listDevicesWithXcdevice(mockContext);

      expect(devices).toEqual([]);
    });

    it("returns empty array when exec throws an error", async () => {
      (exec as jest.Mock).mockRejectedValue(new Error("Command failed"));

      const devices = await listDevicesWithXcdevice(mockContext);

      expect(devices).toEqual([]);
    });

    it("returns empty array when xcdevice command not found", async () => {
      const error: any = new Error("Command not found");
      error.code = "ENOENT";
      (exec as jest.Mock).mockRejectedValue(error);

      const devices = await listDevicesWithXcdevice(mockContext);

      expect(devices).toEqual([]);
    });
  });

  describe("createOsVersionLookup", () => {
    it("creates a map from modelCode to OS version", () => {
      const devices = [
        {
          identifier: "udid1",
          modelCode: "iPhone15,2",
          name: "iPhone 14 Pro",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "udid2",
          modelCode: "iPhone14,8",
          name: "iPhone 14 Plus",
          operatingSystemVersion: "16.6.1",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createOsVersionLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("16.7.12");
      expect(lookup.get("iPhone14,8")).toBe("16.6.1");
      expect(lookup.size).toBe(2);
    });

    it("handles devices with missing modelCode or OS version", () => {
      const devices = [
        {
          identifier: "udid1",
          modelCode: "iPhone15,2",
          name: "iPhone 14 Pro",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "udid2",
          modelCode: "",
          name: "Unknown",
          operatingSystemVersion: "16.6.1",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "udid3",
          modelCode: "iPhone14,8",
          name: "iPhone 14 Plus",
          operatingSystemVersion: "",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createOsVersionLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("16.7.12");
      expect(lookup.get("iPhone14,8")).toBeUndefined();
      expect(lookup.get("")).toBeUndefined();
      expect(lookup.size).toBe(1);
    });

    it("keeps first entry when multiple devices have same modelCode", () => {
      const devices = [
        {
          identifier: "udid1",
          modelCode: "iPhone15,2",
          name: "iPhone 14 Pro 1",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "udid2",
          modelCode: "iPhone15,2",
          name: "iPhone 14 Pro 2",
          operatingSystemVersion: "16.7.13",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createOsVersionLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("16.7.12");
      expect(lookup.size).toBe(1);
    });

    it("returns empty map for empty device list", () => {
      const lookup = createOsVersionLookup([]);

      expect(lookup.size).toBe(0);
    });
  });

  describe("createUdidLookup", () => {
    it("creates a map from modelCode to UDID", () => {
      const devices = [
        {
          identifier: "00008110-001234567890001E",
          modelCode: "iPhone15,2",
          name: "iPhone 14 Pro",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "00008120-001234567890002E",
          modelCode: "iPhone14,8",
          name: "iPhone 14 Plus",
          operatingSystemVersion: "16.6.1",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createUdidLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("00008110-001234567890001E");
      expect(lookup.get("iPhone14,8")).toBe("00008120-001234567890002E");
      expect(lookup.size).toBe(2);
    });

    it("handles devices with missing modelCode or identifier", () => {
      const devices = [
        {
          identifier: "00008110-001234567890001E",
          modelCode: "iPhone15,2",
          name: "iPhone 14 Pro",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "",
          modelCode: "iPhone14,8",
          name: "iPhone 14 Plus",
          operatingSystemVersion: "16.6.1",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "00008130-001234567890003E",
          modelCode: "",
          name: "Unknown",
          operatingSystemVersion: "16.5",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createUdidLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("00008110-001234567890001E");
      expect(lookup.get("iPhone14,8")).toBeUndefined();
      expect(lookup.get("")).toBeUndefined();
      expect(lookup.size).toBe(1);
    });

    it("returns empty map for empty device list", () => {
      const lookup = createUdidLookup([]);

      expect(lookup.size).toBe(0);
    });
  });

  describe("getOsVersionForDevice", () => {
    it("returns OS version for known productType", () => {
      const lookup = new Map([
        ["iPhone15,2", "16.7.12"],
        ["iPhone14,8", "16.6.1"],
      ]);

      expect(getOsVersionForDevice(lookup, "iPhone15,2")).toBe("16.7.12");
      expect(getOsVersionForDevice(lookup, "iPhone14,8")).toBe("16.6.1");
    });

    it("returns undefined for unknown productType", () => {
      const lookup = new Map([["iPhone15,2", "16.7.12"]]);

      expect(getOsVersionForDevice(lookup, "iPhone99,9")).toBeUndefined();
    });
  });

  describe("getUdidForDevice", () => {
    it("returns UDID for known productType", () => {
      const lookup = new Map([
        ["iPhone15,2", "00008110-001234567890001E"],
        ["iPhone14,8", "00008120-001234567890002E"],
      ]);

      expect(getUdidForDevice(lookup, "iPhone15,2")).toBe("00008110-001234567890001E");
      expect(getUdidForDevice(lookup, "iPhone14,8")).toBe("00008120-001234567890002E");
    });

    it("returns undefined for unknown productType", () => {
      const lookup = new Map([["iPhone15,2", "00008110-001234567890001E"]]);

      expect(getUdidForDevice(lookup, "iPhone99,9")).toBeUndefined();
    });
  });

  describe("createNameLookup", () => {
    it("creates a map from modelCode to device name", () => {
      const devices = [
        {
          identifier: "00008110-001234567890001E",
          modelCode: "iPhone15,2",
          name: "John's iPhone",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "00008120-001234567890002E",
          modelCode: "iPhone14,8",
          name: "Sarah's iPhone",
          operatingSystemVersion: "16.6.1",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createNameLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("John's iPhone");
      expect(lookup.get("iPhone14,8")).toBe("Sarah's iPhone");
      expect(lookup.size).toBe(2);
    });

    it("handles devices with missing modelCode or name", () => {
      const devices = [
        {
          identifier: "00008110-001234567890001E",
          modelCode: "iPhone15,2",
          name: "John's iPhone",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "00008120-001234567890002E",
          modelCode: "",
          name: "Invalid Device",
          operatingSystemVersion: "16.6.1",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "00008130-001234567890003E",
          modelCode: "iPhone14,8",
          name: "",
          operatingSystemVersion: "16.5",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createNameLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("John's iPhone");
      expect(lookup.get("iPhone14,8")).toBeUndefined();
      expect(lookup.get("")).toBeUndefined();
      expect(lookup.size).toBe(1);
    });

    it("keeps first entry when multiple devices have same modelCode", () => {
      const devices = [
        {
          identifier: "00008110-001234567890001E",
          modelCode: "iPhone15,2",
          name: "John's iPhone",
          operatingSystemVersion: "16.7.12",
          platform: "com.apple.platform.iphoneos",
        },
        {
          identifier: "00008120-001234567890002E",
          modelCode: "iPhone15,2",
          name: "Jane's iPhone",
          operatingSystemVersion: "16.7.13",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      const lookup = createNameLookup(devices);

      expect(lookup.get("iPhone15,2")).toBe("John's iPhone");
      expect(lookup.size).toBe(1);
    });

    it("returns empty map for empty device list", () => {
      const lookup = createNameLookup([]);

      expect(lookup.size).toBe(0);
    });
  });

  describe("getNameForDevice", () => {
    it("returns device name for known productType", () => {
      const lookup = new Map([
        ["iPhone15,2", "John's iPhone"],
        ["iPhone14,8", "Sarah's iPhone"],
      ]);

      expect(getNameForDevice(lookup, "iPhone15,2")).toBe("John's iPhone");
      expect(getNameForDevice(lookup, "iPhone14,8")).toBe("Sarah's iPhone");
    });

    it("returns undefined for unknown productType", () => {
      const lookup = new Map([["iPhone15,2", "John's iPhone"]]);

      expect(getNameForDevice(lookup, "iPhone99,9")).toBeUndefined();
    });
  });
});
