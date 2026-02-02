/**
 * Unit tests for device destination classes
 */

import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import {
  iOSDeviceDestination,
  watchOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
} from "./types";
import {
  createMockDevice,
  createMockDeviceWithOS,
  createMockDeviceOfType,
  createMockDeviceWithoutName,
  createMockDeviceWithoutOS,
  createMockDeviceWithoutUDID,
} from "../../tests/__mocks__/devices";

describe("iOSDeviceDestination", () => {
  describe("type and platform", () => {
    it("has correct type and platform", () => {
      const device = createMockDevice();
      const destination = new iOSDeviceDestination(device);

      expect(destination.type).toBe("iOSDevice");
      expect(destination.typeLabel).toBe("iOS Device");
      expect(destination.platform).toBe("iphoneos");
    });
  });

  describe("udid property", () => {
    it("returns hardwareProperties.udid when present", () => {
      const device = createMockDevice({
        hardwareProperties: {
          ...createMockDevice().hardwareProperties,
          udid: "00008110-001234567890001E",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.udid).toBe("00008110-001234567890001E");
    });

    it("falls back to identifier when udid is missing", () => {
      const device = createMockDeviceWithoutUDID();
      const destination = new iOSDeviceDestination(device);

      expect(destination.udid).toBe(device.identifier);
    });

    it("falls back to identifier when udid is undefined", () => {
      const device = createMockDevice({
        hardwareProperties: {
          ...createMockDevice().hardwareProperties,
          udid: undefined,
        },
        identifier: "fallback-identifier",
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.udid).toBe("fallback-identifier");
    });
  });

  describe("devicectlId property", () => {
    it("always returns the identifier", () => {
      const device = createMockDevice({
        identifier: "urn:x-ios-devicectl:device-test",
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.devicectlId).toBe("urn:x-ios-devicectl:device-test");
    });
  });

  describe("name property", () => {
    it("returns deviceProperties.name when present", () => {
      const device = createMockDevice({
        deviceProperties: {
          name: "My iPhone",
          osVersionNumber: "17.0",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.name).toBe("My iPhone");
    });

    it("falls back to marketingName when name is missing", () => {
      const device = createMockDevice({
        deviceProperties: {
          osVersionNumber: "17.0",
        },
        hardwareProperties: {
          ...createMockDevice().hardwareProperties,
          marketingName: "iPhone 14 Pro",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.name).toBe("iPhone 14 Pro");
    });

    it("falls back to productType when name and marketingName are missing", () => {
      const device = createMockDevice({
        deviceProperties: {
          osVersionNumber: "17.0",
        },
        hardwareProperties: {
          ...createMockDevice().hardwareProperties,
          marketingName: undefined,
          productType: "iPhone15,2",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.name).toBe("iPhone15,2");
    });

    it("returns 'Unknown Device' when all name sources are missing", () => {
      const device = createMockDevice({
        deviceProperties: {
          osVersionNumber: "17.0",
        },
        hardwareProperties: {
          ...createMockDevice().hardwareProperties,
          marketingName: undefined,
          productType: undefined,
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.name).toBe("Unknown Device");
    });
  });

  describe("osVersion property", () => {
    it("returns osVersionNumber when present", () => {
      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination(device);

      expect(destination.osVersion).toBe("17.0");
    });

    it("returns 'Unknown' when osVersionNumber is missing", () => {
      const device = createMockDeviceWithoutOS();
      const destination = new iOSDeviceDestination(device);

      expect(destination.osVersion).toBe("Unknown");
    });

    it("returns 'Unknown' when osVersionNumber is undefined", () => {
      const device = createMockDevice({
        deviceProperties: {
          osVersionNumber: undefined,
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.osVersion).toBe("Unknown");
    });
  });

  describe("label property", () => {
    it("includes OS version when known", () => {
      const device = createMockDevice({
        deviceProperties: {
          name: "iPhone 14 Pro",
          osVersionNumber: "17.0",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.label).toBe("iPhone 14 Pro (17.0)");
    });

    it("excludes OS version when unknown", () => {
      const device = createMockDevice({
        deviceProperties: {
          name: "iPhone 14 Pro",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.label).toBe("iPhone 14 Pro");
    });
  });

  describe("supportsDevicectl property", () => {
    it("returns true for iOS 17.0", () => {
      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("returns true for iOS 18.0", () => {
      const device = createMockDeviceWithOS("18.0");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("returns false for iOS 16.7", () => {
      const device = createMockDeviceWithOS("16.7");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });

    it("returns false for iOS 15.0", () => {
      const device = createMockDeviceWithOS("15.0");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });

    it("returns false when OS version is unknown", () => {
      const device = createMockDeviceWithoutOS();
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });
  });

  describe("icon property", () => {
    it("returns iPad icon when deviceType is iPad", () => {
      const device = createMockDeviceOfType("iPad");
      const destination = new iOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-ipad");
    });

    it("returns connected iPhone icon when deviceType is iPhone and connected", () => {
      const device = createMockDeviceOfType("iPhone");
      const destination = new iOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-mobile");
    });

    it("returns disconnected iPhone icon when deviceType is iPhone and disconnected", () => {
      const device = createMockDeviceOfType("iPhone");
      device.connectionProperties.tunnelState = "disconnected";
      const destination = new iOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-mobile-x");
    });

    it("returns default icon for other device types", () => {
      const device = createMockDevice({
        hardwareProperties: {
          ...createMockDevice().hardwareProperties,
          deviceType: "iPhone",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-mobile");
    });
  });

  describe("isConnected property", () => {
    it("returns true when tunnelState is connected", () => {
      const device = createMockDevice();
      const destination = new iOSDeviceDestination(device);

      expect(destination.isConnected).toBe(true);
    });

    it("returns false when tunnelState is disconnected", () => {
      const device = createMockDevice();
      device.connectionProperties.tunnelState = "disconnected";
      const destination = new iOSDeviceDestination(device);

      expect(destination.isConnected).toBe(false);
    });

    it("returns false when tunnelState is unavailable", () => {
      const device = createMockDevice();
      device.connectionProperties.tunnelState = "unavailable";
      const destination = new iOSDeviceDestination(device);

      expect(destination.isConnected).toBe(false);
    });
  });

  describe("id property", () => {
    it("returns id with iosdevice prefix and udid", () => {
      const device = createMockDevice();
      const destination = new iOSDeviceDestination(device);

      expect(destination.id).toBe("iosdevice-00008110-001234567890001E");
    });
  });

  describe("quickPickDetails property", () => {
    it("returns detailed info string", () => {
      const device = createMockDevice({
        deviceProperties: {
          name: "iPhone 14 Pro",
          osVersionNumber: "17.0",
        },
      });
      const destination = new iOSDeviceDestination(device);

      expect(destination.quickPickDetails).toBe(
        "Type: iOS Device, Version: 17.0, ID: 00008110-001234567890001e",
      );
    });
  });
});

describe("watchOSDeviceDestination", () => {
  it("has correct type and platform", () => {
    const device = createMockDeviceOfType("appleWatch");
    const destination = new watchOSDeviceDestination(device);

    expect(destination.type).toBe("watchOSDevice");
    expect(destination.typeLabel).toBe("watchOS Device");
    expect(destination.platform).toBe("watchos");
  });

  describe("supportsDevicectl property", () => {
    it("returns true for watchOS 10.0", () => {
      const device = createMockDeviceOfType("appleWatch");
      device.deviceProperties.osVersionNumber = "10.0";
      const destination = new watchOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("returns false for watchOS 9.5", () => {
      const device = createMockDeviceOfType("appleWatch");
      device.deviceProperties.osVersionNumber = "9.5";
      const destination = new watchOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });
  });

  describe("icon property", () => {
    it("returns connected icon when connected", () => {
      const device = createMockDeviceOfType("appleWatch");
      const destination = new watchOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-watch");
    });

    it("returns disconnected icon when disconnected", () => {
      const device = createMockDeviceOfType("appleWatch");
      device.connectionProperties.tunnelState = "disconnected";
      const destination = new watchOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-watch-pause");
    });
  });
});

describe("tvOSDeviceDestination", () => {
  it("has correct type and platform", () => {
    const device = createMockDeviceOfType("appleTV");
    const destination = new tvOSDeviceDestination(device);

    expect(destination.type).toBe("tvOSDevice");
    expect(destination.typeLabel).toBe("tvOS Device");
    expect(destination.platform).toBe("appletvos");
  });

  describe("supportsDevicectl property", () => {
    it("returns true for tvOS 17.0", () => {
      const device = createMockDeviceOfType("appleTV");
      device.deviceProperties.osVersionNumber = "17.0";
      const destination = new tvOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("returns false for tvOS 16.5", () => {
      const device = createMockDeviceOfType("appleTV");
      device.deviceProperties.osVersionNumber = "16.5";
      const destination = new tvOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });
  });

  describe("icon property", () => {
    it("returns TV icon", () => {
      const device = createMockDeviceOfType("appleTV");
      const destination = new tvOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-device-tv-old");
    });
  });
});

describe("visionOSDeviceDestination", () => {
  it("has correct type and platform", () => {
    const device = createMockDeviceOfType("appleVision");
    const destination = new visionOSDeviceDestination(device);

    expect(destination.type).toBe("visionOSDevice");
    expect(destination.typeLabel).toBe("visionOS Device");
    expect(destination.platform).toBe("xros");
  });

  describe("supportsDevicectl property", () => {
    it("returns true for visionOS 1.0", () => {
      const device = createMockDeviceOfType("appleVision");
      device.deviceProperties.osVersionNumber = "1.0";
      const destination = new visionOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("returns true for visionOS 2.0", () => {
      const device = createMockDeviceOfType("appleVision");
      device.deviceProperties.osVersionNumber = "2.0";
      const destination = new visionOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("returns false when OS version is unknown", () => {
      const device = createMockDeviceOfType("appleVision");
      device.deviceProperties.osVersionNumber = undefined;
      const destination = new visionOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });
  });

  describe("icon property", () => {
    it("returns vision icon", () => {
      const device = createMockDeviceOfType("appleVision");
      const destination = new visionOSDeviceDestination(device);

      expect(destination.icon).toBe("sweetpad-cardboards");
    });
  });
});

describe("Common device destination behavior", () => {
  describe("udid fallback behavior", () => {
    it("all device types fall back to identifier when udid is missing", () => {
      const device = createMockDeviceWithoutUDID();

      const iOSDest = new iOSDeviceDestination(device);
      const watchOSDest = new watchOSDeviceDestination(device);
      const tvOSDest = new tvOSDeviceDestination(device);
      const visionOSDest = new visionOSDeviceDestination(device);

      expect(iOSDest.udid).toBe(device.identifier);
      expect(watchOSDest.udid).toBe(device.identifier);
      expect(tvOSDest.udid).toBe(device.identifier);
      expect(visionOSDest.udid).toBe(device.identifier);
    });
  });

  describe("name fallback behavior", () => {
    it("all device types fall back through name -> marketingName -> productType -> Unknown", () => {
      const device = createMockDeviceWithoutName();

      const iOSDest = new iOSDeviceDestination(device);
      expect(iOSDest.name).toBe("iPhone15,2");

      device.hardwareProperties.marketingName = undefined;
      device.hardwareProperties.productType = undefined;
      const iOSDest2 = new iOSDeviceDestination(device);
      expect(iOSDest2.name).toBe("Unknown Device");
    });
  });

  describe("osVersion fallback behavior", () => {
    it("all device types return 'Unknown' when osVersionNumber is missing", () => {
      const device = createMockDeviceWithoutOS();

      const iOSDest = new iOSDeviceDestination(device);
      const watchOSDest = new watchOSDeviceDestination(device);
      const tvOSDest = new tvOSDeviceDestination(device);
      const visionOSDest = new visionOSDeviceDestination(device);

      expect(iOSDest.osVersion).toBe("Unknown");
      expect(watchOSDest.osVersion).toBe("Unknown");
      expect(tvOSDest.osVersion).toBe("Unknown");
      expect(visionOSDest.osVersion).toBe("Unknown");
    });
  });

  describe("label behavior with unknown OS", () => {
    it("all device types exclude OS version from label when unknown", () => {
      const device = createMockDevice({
        deviceProperties: {
          name: "Test Device",
        },
      });

      const iOSDest = new iOSDeviceDestination(device);
      const watchOSDest = new watchOSDeviceDestination(device);
      const tvOSDest = new tvOSDeviceDestination(device);
      const visionOSDest = new visionOSDeviceDestination(device);

      expect(iOSDest.label).toBe("Test Device");
      expect(watchOSDest.label).toBe("Test Device");
      expect(tvOSDest.label).toBe("Test Device");
      expect(visionOSDest.label).toBe("Test Device");
    });
  });
});
