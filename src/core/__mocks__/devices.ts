/**
 * Mock utilities for device-related tests (host-agnostic).
 * Helpers for generating fake device objects and a mock TaskTerminal.
 */

import type { ProcessGroup, ProcessHandle, ProcessSpec, TaskTerminal } from "../tasks/types";
import type { DeviceCtlDevice } from "../xcode/devicectl";
import type { XcdeviceDevice } from "../xcode/xcdevice";

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
    execute: vi.fn().mockResolvedValue(undefined),
    write: vi.fn(),
    runGroup: vi.fn(async (callback: (group: ProcessGroup) => Promise<unknown>) => {
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

export function createMockDeviceWithOS(osVersion: string): DeviceCtlDevice {
  return createMockDevice({
    deviceProperties: {
      name: "iPhone Test Device",
      osVersionNumber: osVersion,
    },
  });
}

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

export function createMockDeviceWithoutUDID(): DeviceCtlDevice {
  return createMockDevice({
    hardwareProperties: {
      ...createMockDevice().hardwareProperties,
      udid: undefined,
    },
  });
}

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
