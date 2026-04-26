/**
 * Mock utilities for device-related tests
 * Provides helper functions to generate mock device objects and test contexts
 */

import type { ExtensionContext } from "../common/commands";
import type { ProcessGroup, ProcessHandle, ProcessSpec, TaskTerminal } from "../common/tasks/types";
import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";

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
    tunnelManager: { autoStart: jest.fn().mockResolvedValue(undefined) } as any,
    ...overrides,
  } as unknown as ExtensionContext;
}

/**
 * Mock TaskTerminal for tests. `spawnedSpecs` captures every ProcessSpec passed
 * to `runGroup`'s group.spawn — use it to assert against the launch path
 * (which now goes through runGroup/spawn, not execute).
 *
 * Each spawned process resolves immediately with code: 0; tests that need a
 * different exit code can override per-call by inspecting `spawnedSpecs` after
 * the fact (the assertions don't depend on real exit codes).
 */
export type MockTaskTerminal = TaskTerminal & {
  spawnedSpecs: ProcessSpec[];
};

export function createMockTerminal(): MockTaskTerminal {
  const spawnedSpecs: ProcessSpec[] = [];
  const terminal = {
    execute: jest.fn().mockResolvedValue(undefined),
    write: jest.fn(),
    runGroup: jest.fn(async (callback: (group: ProcessGroup) => Promise<unknown>) => {
      const group: ProcessGroup = {
        terminal: terminal as TaskTerminal,
        spawn: (spec: ProcessSpec): ProcessHandle => {
          spawnedSpecs.push(spec);
          return {
            pid: 1234,
            exit: Promise.resolve({ code: 0, signal: null }),
            kill: () => {},
            onData: () => {},
            onError: () => {},
          };
        },
      };
      return await callback(group);
    }),
    spawnedSpecs,
  };
  return terminal as unknown as MockTaskTerminal;
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
      marketingName: 'iPad Pro 12.9"',
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
