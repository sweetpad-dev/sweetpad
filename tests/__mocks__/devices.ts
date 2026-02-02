/**
 * Mock utilities for device-related tests
 * Provides helper functions to generate mock device objects and test contexts
 */

import type { DeviceCtlDevice } from "../../src/common/xcode/devicectl";
import type { XcdeviceDevice } from "../../src/common/xcode/xcdevice";
import type { ExtensionContext } from "../../src/common/commands";
import type { TaskTerminal } from "../../src/common/tasks";

/**
 * Create a mock DeviceCtlDevice object with optional overrides
 */
export function createMockDevice(overrides: Partial<DeviceCtlDevice> = {}): DeviceCtlDevice {
  return {
    capabilities: [],
    connectionProperties: {
      tunnelState: "connected",
      pairingState: "paired",
    },
    deviceProperties: {
      name: "iPhone 14 Pro",
      osVersionNumber: "17.0",
    },
    hardwareProperties: {
      deviceType: "iPhone",
      marketingName: "iPhone 14 Pro",
      productType: "iPhone15,2",
      udid: "00008110-001234567890001E",
      platform: "iOS",
    },
    identifier: "urn:x-ios-devicectl:device-CS4567890-1234567890123456",
    visibilityClass: "default",
    ...overrides,
  };
}

/**
 * Create a mock XcdeviceDevice object with optional overrides
 */
export function createMockXcdeviceDevice(overrides: Partial<XcdeviceDevice> = {}): XcdeviceDevice {
  return {
    identifier: "00008110-001234567890001E",
    modelCode: "iPhone15,2",
    name: "iPhone 14 Pro",
    operatingSystemVersion: "16.7.12",
    platform: "com.apple.platform.iphoneos",
    ...overrides,
  };
}

/**
 * Create a mock ExtensionContext for testing
 */
export function createMockContext(overrides: Partial<ExtensionContext> = {}): ExtensionContext {
  return {
    startExecutionScope: jest.fn().mockImplementation(async (_scope, callback) => {
      return await callback();
    }),
    updateProgressStatus: jest.fn(),
    getWorkspaceState: jest.fn().mockReturnValue(undefined),
    updateWorkspaceState: jest.fn(),
    buildManager: {} as any,
    destinationsManager: {} as any,
    ...overrides,
  } as unknown as ExtensionContext;
}

/**
 * Create a mock TaskTerminal for testing
 */
export function createMockTerminal(): TaskTerminal {
  return {
    execute: jest.fn().mockResolvedValue(undefined),
    write: jest.fn(),
  } as unknown as TaskTerminal;
}

/**
 * Helper to create a mock device with specific OS version
 */
export function createMockDeviceWithOS(osVersion: string): DeviceCtlDevice {
  return createMockDevice({
    deviceProperties: {
      name: "iPhone Test Device",
      osVersionNumber: osVersion,
    },
  });
}

/**
 * Helper to create a mock device of a specific type
 */
export function createMockDeviceOfType(
  deviceType: "iPhone" | "iPad" | "appleWatch" | "appleTV" | "appleVision",
): DeviceCtlDevice {
  const hardwareProps: Record<string, any> = {
    iPhone: {
      deviceType: "iPhone",
      marketingName: "iPhone 14 Pro",
      productType: "iPhone15,2",
    },
    iPad: {
      deviceType: "iPad",
      marketingName: "iPad Pro 12.9\"",
      productType: "iPad14,5",
    },
    appleWatch: {
      deviceType: "appleWatch",
      marketingName: "Apple Watch Series 9",
      productType: "Watch10,1",
    },
    appleTV: {
      deviceType: "appleTV",
      marketingName: "Apple TV 4K",
      productType: "AppleTV14,1",
    },
    appleVision: {
      deviceType: "appleVision",
      marketingName: "Apple Vision Pro",
      productType: "VisionPro,1",
    },
  };

  return createMockDevice({
    hardwareProperties: {
      ...createMockDevice().hardwareProperties,
      ...hardwareProps[deviceType],
    },
  });
}

/**
 * Create a mock device without OS version (for testing fallback behavior)
 */
export function createMockDeviceWithoutOS(): DeviceCtlDevice {
  return createMockDevice({
    deviceProperties: {
      name: "iPhone Unknown",
    },
    hardwareProperties: {
      ...createMockDevice().hardwareProperties,
      udid: undefined,
    },
  });
}

/**
 * Create a mock device without UDID (for testing fallback behavior)
 */
export function createMockDeviceWithoutUDID(): DeviceCtlDevice {
  return createMockDevice({
    hardwareProperties: {
      ...createMockDevice().hardwareProperties,
      udid: undefined,
    },
  });
}

/**
 * Create a mock device with missing name (for testing fallback behavior)
 */
export function createMockDeviceWithoutName(): DeviceCtlDevice {
  return createMockDevice({
    deviceProperties: {
      osVersionNumber: "17.0",
    },
    hardwareProperties: {
      ...createMockDevice().hardwareProperties,
      marketingName: undefined,
    },
  });
}
