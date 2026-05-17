import * as fs from "node:fs";
import * as path from "node:path";

import type { Mock } from "vitest";

import { noopLogger } from "../logger/types";
import { listDevices } from "../xcode/devicectl";
import { listDevicesWithXcdevice } from "../xcode/xcdevice";
import { DevicesManager } from "./manager";

vi.mock("../exec", () => ({
  exec: vi.fn(),
}));

vi.mock("../xcode/devicectl", () => ({
  listDevices: vi.fn(),
}));

vi.mock("../xcode/xcdevice", () => ({
  listDevicesWithXcdevice: vi.fn(),
}));

function loadFixture(relativePath: string): any {
  return JSON.parse(fs.readFileSync(path.join(__dirname, "../../..", relativePath), "utf8"));
}

const TEST_CWD = "/tmp/sweetpad-test-cwd";
const TEST_STORAGE = "/tmp/sweetpad-test";

const TEST_WORKSPACE_ROOT = {
  getPath: () => TEST_CWD,
  getStoragePath: async () => TEST_STORAGE,
  getRelativePath: (p: string) => p,
};

const TEST_DEPS = {
  logger: noopLogger,
  workspaceRoot: TEST_WORKSPACE_ROOT,
};

describe("DevicesManager", () => {
  let manager: DevicesManager;

  beforeEach(() => {
    vi.clearAllMocks();
    manager = new DevicesManager(TEST_DEPS);
  });

  describe("refresh", () => {
    it("fetches devices from both devicectl and xcdevice in parallel", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-ios-17-modern.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue(loadFixture("tests/xcdevice-data/xcdevice-ios-devices.json"));

      await manager.refresh();

      expect(listDevices).toHaveBeenCalledWith({
        storagePath: TEST_STORAGE,
        cwd: TEST_CWD,
        logger: TEST_DEPS.logger,
      });
      expect(listDevicesWithXcdevice).toHaveBeenCalledWith({ cwd: TEST_CWD, logger: TEST_DEPS.logger });
    });

    it("wraps iOS 17+ devicectl device when xcdevice is empty", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-ios-17-modern.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].type).toBe("iOSDevice");
      expect(devices[0].supportsDevicectl).toBe(true);
    });

    it("deduplicates when both sources report the same UDID", async () => {
      const devicectlData = {
        result: {
          devices: [
            {
              capabilities: [],
              connectionProperties: { tunnelState: "connected", pairingState: "paired" },
              deviceProperties: { name: "John's iPhone", osVersionNumber: "17.0" },
              hardwareProperties: {
                deviceType: "iPhone",
                marketingName: "iPhone 15 Pro",
                productType: "iPhone16,1",
                udid: "00008110-DEDUP000001",
                platform: "iOS",
              },
              identifier: "urn:x-ios-devicectl:device-DEDUP",
              visibilityClass: "default",
            },
          ],
        },
      };
      const xcdeviceData = [
        {
          simulator: false,
          available: true,
          platform: "com.apple.platform.iphoneos",
          modelCode: "iPhone16,1",
          identifier: "00008110-DEDUP000001",
          operatingSystemVersion: "17.0",
          name: "John's iPhone",
        },
      ];

      (listDevices as Mock).mockResolvedValue(devicectlData);
      (listDevicesWithXcdevice as Mock).mockResolvedValue(xcdeviceData);

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].devicectlId).toBe("urn:x-ios-devicectl:device-DEDUP");
    });

    it("recovers iOS 16 Wi-Fi device when devicectl returns no devices", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-no-devices.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue(loadFixture("tests/xcdevice-data/xcdevice-ios-16-wifi.json"));

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].type).toBe("iOSDevice");
      expect(devices[0].name).toBe("My iPhone");
      expect(devices[0].osVersion).toBe("16.4.1");
      expect(devices[0].udid).toBe("00008110-000AABBCCDD11111");
      expect(devices[0].devicectlId).toBeNull();
      expect(devices[0].supportsDevicectl).toBe(false);
      expect(devices[0].isConnected).toBe(true);
    });

    it("recovers iOS 16 USB device when devicectl returns empty hardwareProperties", async () => {
      (listDevices as Mock).mockResolvedValue(
        loadFixture("tests/devicectl-data/devicectl-ios-16-usb-empty-hardware.json"),
      );
      (listDevicesWithXcdevice as Mock).mockResolvedValue(
        loadFixture("tests/xcdevice-data/xcdevice-ios-16-usb-match.json"),
      );

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].type).toBe("iOSDevice");
      expect(devices[0].name).toBe("Nixuge iPhone");
      expect(devices[0].udid).toBe("00008110-000AABBCCDD22222");
      expect(devices[0].supportsDevicectl).toBe(false);
    });

    it("shows an unavailable xcdevice entry as disconnected", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-no-devices.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue(loadFixture("tests/xcdevice-data/xcdevice-unavailable.json"));

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].isConnected).toBe(false);
      expect(devices[0].state).toBe("unavailable");
    });

    it("drops devicectl entries with empty hardwareProperties and no xcdevice match", async () => {
      (listDevices as Mock).mockResolvedValue(
        loadFixture("tests/devicectl-data/devicectl-ios-16-usb-empty-hardware.json"),
      );
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const devices = await manager.refresh();

      expect(devices).toEqual([]);
    });

    it("filters devices without identifier", async () => {
      const devicectlData = {
        result: {
          devices: [
            {
              capabilities: [],
              connectionProperties: { tunnelState: "connected", pairingState: "paired" },
              deviceProperties: { name: "Valid Device", osVersionNumber: "17.0" },
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
              connectionProperties: { tunnelState: "connected", pairingState: "paired" },
              deviceProperties: { name: "Device Without Identifier" },
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

      (listDevices as Mock).mockResolvedValue(devicectlData);
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const devices = await manager.refresh();

      expect(devices).toHaveLength(1);
      expect(devices[0].name).toBe("Valid Device");
    });

    it("creates correct device type instances from mixed xcdevice platforms", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-no-devices.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue(
        loadFixture("tests/xcdevice-data/xcdevice-mixed-platforms.json"),
      );

      const devices = await manager.refresh();

      const byType = devices.map((d) => d.type).toSorted();
      // iPhone, iPad → both iOSDevice; watch, tv → watchOSDevice, tvOSDevice.
      // Simulator filtering lives in listDevicesWithXcdevice, so here we get all non-simulator entries.
      expect(byType).toEqual(["iOSDevice", "iOSDevice", "tvOSDevice", "watchOSDevice", "iOSDevice"].toSorted());
    });

    it("creates correct device type instances from devicectl fixtures", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-multiple-devices.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const devices = await manager.refresh();

      expect(devices).toHaveLength(5);
      expect(devices.map((d) => d.type)).toEqual([
        "iOSDevice",
        "iOSDevice",
        "watchOSDevice",
        "tvOSDevice",
        "visionOSDevice",
      ]);
    });

    it("handles ENOENT error by setting failed to 'no-devicectl'", async () => {
      const error: any = new Error("devicectl not found");
      error.error = { code: "ENOENT" };
      (listDevices as Mock).mockRejectedValue(error);
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const devices = await manager.refresh();

      expect(devices).toEqual([]);
      expect(manager.failed).toBe("no-devicectl");
    });

    it("handles other errors by setting failed to 'unknown'", async () => {
      (listDevices as Mock).mockRejectedValue(new Error("Unknown error"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const devices = await manager.refresh();

      expect(devices).toEqual([]);
      expect(manager.failed).toBe("unknown");
    });

    it("caches device list after refresh", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-ios-17-modern.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      await manager.refresh();
      await manager.getDevices();

      expect(listDevices).toHaveBeenCalledTimes(1);
    });

    it("emits updated event after refresh", async () => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-ios-17-modern.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);

      const listener = vi.fn();
      manager.on("updated", listener);

      await manager.refresh();

      expect(listener).toHaveBeenCalled();
    });
  });

  describe("getDevices", () => {
    beforeEach(() => {
      (listDevices as Mock).mockResolvedValue(loadFixture("tests/devicectl-data/devicectl-ios-17-modern.json"));
      (listDevicesWithXcdevice as Mock).mockResolvedValue([]);
    });

    it("returns cached devices without refresh", async () => {
      await manager.refresh();
      await manager.getDevices();
      expect(listDevices).toHaveBeenCalledTimes(1);
    });

    it("forces refresh when options.refresh is true", async () => {
      await manager.refresh();
      await manager.getDevices({ refresh: true });
      expect(listDevices).toHaveBeenCalledTimes(2);
    });

    it("fetches devices when cache is empty", async () => {
      await manager.getDevices();
      expect(listDevices).toHaveBeenCalledTimes(1);
    });
  });
});
