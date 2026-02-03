/**
 * Unit tests for device manager with dual-source fetching
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { createMockContext } from "../__mocks__/devices";
import { listDevices } from "../common/xcode/devicectl";
import { listDevicesWithXcdevice } from "../common/xcode/xcdevice";
import { DevicesManager } from "./manager";

// Mock dependencies
jest.mock("../common/exec", () => ({
  exec: jest.fn(),
}));

jest.mock("../common/xcode/devicectl", () => ({
  listDevices: jest.fn(),
}));

jest.mock("../common/xcode/xcdevice", () => ({
  listDevicesWithXcdevice: jest.fn(),
  createNameLookup: jest.fn(),
  createOsVersionLookup: jest.fn(),
  createUdidLookup: jest.fn(),
  getNameForDevice: jest.fn(),
  getOsVersionForDevice: jest.fn(),
  getUdidForDevice: jest.fn(),
}));

// Import mocked modules
import {
  createNameLookup,
  createOsVersionLookup,
  createUdidLookup,
  getNameForDevice,
  getOsVersionForDevice,
  getUdidForDevice,
} from "../common/xcode/xcdevice";

describe("DevicesManager", () => {
  let manager: DevicesManager;
  let mockContext: ReturnType<typeof createMockContext>;

  beforeEach(() => {
    jest.clearAllMocks();
    manager = new DevicesManager();
    mockContext = createMockContext();
    manager.context = mockContext;
  });

  describe("refresh", () => {
    it("fetches devices from both devicectl and xcdevice in parallel", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-17-modern.json"), "utf8"),
      );
      const mockXcdeviceData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/xcdevice-data/xcdevice-ios-devices.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue(mockXcdeviceData);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const devices = await manager.refresh();

      expect(listDevices).toHaveBeenCalledWith(mockContext);
      expect(listDevicesWithXcdevice).toHaveBeenCalledWith(mockContext);
      expect(createNameLookup).toHaveBeenCalledWith(mockXcdeviceData);
      expect(createOsVersionLookup).toHaveBeenCalledWith(mockXcdeviceData);
      expect(createUdidLookup).toHaveBeenCalledWith(mockXcdeviceData);
    });

    it("merges OS version from xcdevice when missing from devicectl", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(
          path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-16-missing-fields.json"),
          "utf8",
        ),
      );
      const mockXcdeviceData = [
        {
          identifier: "00008110-000987654321003E",
          modelCode: "iPhone14,5",
          name: "iPhone 13",
          operatingSystemVersion: "16.5.1",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue(mockXcdeviceData);

      const osVersionMap = new Map([["iPhone14,5", "16.5.1"]]);
      const udidMap = new Map([["iPhone14,5", "00008110-000987654321003E"]]);

      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(osVersionMap);
      (createUdidLookup as jest.Mock).mockReturnValue(udidMap);
      (getNameForDevice as jest.Mock).mockReturnValue(undefined);
      (getOsVersionForDevice as jest.Mock).mockImplementation((map, productType) => map.get(productType));
      (getUdidForDevice as jest.Mock).mockImplementation((map, productType) => map.get(productType));

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].osVersion).toBe("16.5.1");
      expect(devices[0].udid).toBe("00008110-000987654321003E");
    });

    it("merges UDID from xcdevice when missing from devicectl", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(
          path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-16-missing-fields.json"),
          "utf8",
        ),
      );
      const mockXcdeviceData = [
        {
          identifier: "00008110-000987654321003E",
          modelCode: "iPhone14,5",
          name: "John's iPhone",
          operatingSystemVersion: "16.5.1",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue(mockXcdeviceData);

      const udidMap = new Map([["iPhone14,5", "00008110-000987654321003E"]]);
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(udidMap);
      (getOsVersionForDevice as jest.Mock).mockReturnValue(undefined);
      (getUdidForDevice as jest.Mock).mockImplementation((map, productType) => map.get(productType));

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].udid).toBe("00008110-000987654321003E");
    });

    it("merges device name from xcdevice when devicectl returns marketing name", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(
          path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-16-missing-fields.json"),
          "utf8",
        ),
      );
      const mockXcdeviceData = [
        {
          identifier: "00008110-000987654321003E",
          modelCode: "iPhone14,5",
          name: "John's iPhone",
          operatingSystemVersion: "16.5.1",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue(mockXcdeviceData);

      const nameMap = new Map([["iPhone14,5", "John's iPhone"]]);
      (createNameLookup as jest.Mock).mockReturnValue(nameMap);
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());
      (getNameForDevice as jest.Mock).mockImplementation((map, productType) => map.get(productType));
      (getOsVersionForDevice as jest.Mock).mockReturnValue(undefined);
      (getUdidForDevice as jest.Mock).mockReturnValue(undefined);

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].name).toBe("John's iPhone");
    });

    it("uses devicectl name when it differs from marketing name", async () => {
      const mockDevicectlData = {
        result: {
          devices: [
            {
              capabilities: [],
              connectionProperties: {
                tunnelState: "connected",
                pairingState: "paired",
              },
              deviceProperties: {
                name: "John's iPhone 15 Pro",
                osVersionNumber: "17.0",
              },
              hardwareProperties: {
                deviceType: "iPhone",
                marketingName: "iPhone 15 Pro",
                productType: "iPhone16,1",
                udid: "00008110-000123456789001E",
                platform: "iOS",
              },
              identifier: "urn:x-ios-devicectl:device-CS1234567-1234567890123456",
              visibilityClass: "default",
            },
          ],
        },
      };
      const mockXcdeviceData = [
        {
          identifier: "00008110-000987654321001E",
          modelCode: "iPhone16,1",
          name: "Different Name from xcdevice",
          operatingSystemVersion: "17.0",
          platform: "com.apple.platform.iphoneos",
        },
      ];

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue(mockXcdeviceData);

      const nameMap = new Map([["iPhone16,1", "Different Name from xcdevice"]]);
      (createNameLookup as jest.Mock).mockReturnValue(nameMap);
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());
      (getNameForDevice as jest.Mock).mockImplementation((map, productType) => map.get(productType));
      (getOsVersionForDevice as jest.Mock).mockReturnValue(undefined);
      (getUdidForDevice as jest.Mock).mockReturnValue(undefined);

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      // devicectl returns "John's iPhone 15 Pro" which differs from marketing name "iPhone 15 Pro"
      // so we should use the devicectl name, not fall back to xcdevice
      expect(devices[0].name).toBe("John's iPhone 15 Pro");
    });

    it("applies safe defaults for missing data", async () => {
      const mockDevicectlData = {
        result: {
          devices: [
            {
              capabilities: [],
              connectionProperties: {
                tunnelState: "connected",
                pairingState: "paired",
              },
              deviceProperties: {},
              hardwareProperties: {
                deviceType: "iPhone",
                platform: "iOS",
              },
              identifier: "urn:x-ios-devicectl:device-test",
              visibilityClass: "default",
            },
          ],
        },
      };

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].osVersion).toBe("Unknown");
      // When productType is "Unknown", the name falls back to productType value
      expect(devices[0].name).toBe("Unknown");
      expect(devices[0].udid).toBe("urn:x-ios-devicectl:device-test");
    });

    it("filters devices without identifier", async () => {
      const mockDevicectlData = {
        result: {
          devices: [
            {
              capabilities: [],
              connectionProperties: {
                tunnelState: "connected",
                pairingState: "paired",
              },
              deviceProperties: {
                name: "Valid Device",
                osVersionNumber: "17.0",
              },
              hardwareProperties: {
                deviceType: "iPhone",
                marketingName: "iPhone 15 Pro",
                productType: "iPhone16,1",
                platform: "iOS",
              },
              identifier: "urn:x-ios-devicectl:device-valid",
              visibilityClass: "default",
            },
            {
              capabilities: [],
              connectionProperties: {
                tunnelState: "connected",
                pairingState: "paired",
              },
              deviceProperties: {
                name: "Device Without Identifier",
              },
              hardwareProperties: {
                deviceType: "iPhone",
                marketingName: "Invalid Device",
                productType: "iPhone99,9",
                platform: "iOS",
              },
              visibilityClass: "default",
            },
          ],
        },
      };

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].name).toBe("Valid Device");
    });

    it("filters devices without deviceType", async () => {
      const mockDevicectlData = {
        result: {
          devices: [
            {
              capabilities: [],
              connectionProperties: {
                tunnelState: "connected",
                pairingState: "paired",
              },
              deviceProperties: {
                name: "Valid Device",
                osVersionNumber: "17.0",
              },
              hardwareProperties: {
                deviceType: "iPhone",
                marketingName: "iPhone 15 Pro",
                productType: "iPhone16,1",
                platform: "iOS",
              },
              identifier: "urn:x-ios-devicectl:device-valid",
              visibilityClass: "default",
            },
            {
              capabilities: [],
              connectionProperties: {
                tunnelState: "connected",
                pairingState: "paired",
              },
              deviceProperties: {
                name: "Device Without DeviceType",
              },
              hardwareProperties: {
                marketingName: "Invalid Device",
                productType: "iPhone99,9",
                platform: "iOS",
              },
              identifier: "urn:x-ios-devicectl:device-invalid",
              visibilityClass: "default",
            },
          ],
        },
      };

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].name).toBe("Valid Device");
    });

    it("creates correct device type instances", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-multiple-devices.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const devices = await manager.refresh();

      expect(devices).toHaveLength(5);
      expect(devices[0].type).toBe("iOSDevice");
      expect(devices[1].type).toBe("iOSDevice");
      expect(devices[2].type).toBe("watchOSDevice");
      expect(devices[3].type).toBe("tvOSDevice");
      expect(devices[4].type).toBe("visionOSDevice");
    });

    it("handles ENOENT error by setting failed to 'no-devicectl'", async () => {
      const error: any = new Error("devicectl not found");
      error.error = { code: "ENOENT" };
      (listDevices as jest.Mock).mockRejectedValue(error);

      const devices = await manager.refresh();

      expect(devices).toEqual([]);
      expect(manager.failed).toBe("no-devicectl");
    });

    it("handles other errors by setting failed to 'unknown'", async () => {
      (listDevices as jest.Mock).mockRejectedValue(new Error("Unknown error"));

      const devices = await manager.refresh();

      expect(devices).toEqual([]);
      expect(manager.failed).toBe("unknown");
    });

    it("caches device list after refresh", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-17-modern.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      await manager.refresh();

      // Second call should not trigger new fetch
      const devices = await manager.getDevices();

      expect(listDevices).toHaveBeenCalledTimes(1);
      expect(devices).toHaveLength(1);
    });

    it("emits updated event after refresh", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-17-modern.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const updatedListener = jest.fn();
      manager.on("updated", updatedListener);

      await manager.refresh();

      expect(updatedListener).toHaveBeenCalled();
    });
  });

  describe("getDevices", () => {
    it("returns cached devices without refresh", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-17-modern.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      await manager.refresh();
      const devices = await manager.getDevices();

      expect(listDevices).toHaveBeenCalledTimes(1);
      expect(devices).toHaveLength(1);
    });

    it("forces refresh when options.refresh is true", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-17-modern.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      await manager.refresh();
      await manager.getDevices({ refresh: true });

      expect(listDevices).toHaveBeenCalledTimes(2);
    });

    it("fetches devices when cache is empty", async () => {
      const mockDevicectlData = JSON.parse(
        fs.readFileSync(path.join(__dirname, "../../tests/devicectl-data/devicectl-ios-17-modern.json"), "utf8"),
      );

      (listDevices as jest.Mock).mockResolvedValue(mockDevicectlData);
      (listDevicesWithXcdevice as jest.Mock).mockResolvedValue([]);
      (createNameLookup as jest.Mock).mockReturnValue(new Map());
      (createOsVersionLookup as jest.Mock).mockReturnValue(new Map());
      (createUdidLookup as jest.Mock).mockReturnValue(new Map());

      const devices = await manager.getDevices();

      expect(listDevices).toHaveBeenCalledTimes(1);
      expect(devices).toHaveLength(1);
    });
  });

  describe("context property", () => {
    it("throws error when context is not set", () => {
      const newManager = new DevicesManager();

      expect(() => newManager.context).toThrow("Context is not set");
    });

    it("returns context when set", () => {
      const newManager = new DevicesManager();
      newManager.context = mockContext;

      expect(newManager.context).toBe(mockContext);
    });
  });
});
