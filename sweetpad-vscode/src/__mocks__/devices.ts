/**
 * Mock utilities for device-related tests
 * Provides helper functions to generate mock device objects and test contexts
 */

import type { AppDeps } from "../common/commands";
import type { ProcessGroup, ProcessHandle, ProcessSpec, TaskTerminal } from "../common/tasks/types";
import type { DeviceCtlDevice } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";

/**
 * A device in devicectl's "jsonVersion" 4 shape, where the three deprecated
 * property bags are always present. Spelled out so specs can reach into them
 * without a null check; "createMockDeviceV5" covers the shape that replaces it.
 */
export type MockLegacyDevice = DeviceCtlDevice &
  Required<Pick<DeviceCtlDevice, "connectionProperties" | "deviceProperties" | "hardwareProperties">>;

/**
 * Create a mock DeviceCtlDevice object with optional overrides
 */
export function createMockDevice(overrides: Partial<DeviceCtlDevice> = {}): MockLegacyDevice {
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
 * The same device as "createMockDevice", in devicectl's "jsonVersion" 5 shape:
 * one "properties" dictionary and none of the deprecated bags.
 */
export function createMockDeviceV5(overrides: Partial<DeviceCtlDevice> = {}): DeviceCtlDevice {
  return {
    capabilities: [],
    properties: {
      connection: {
        pairingState: "paired",
        state: "connected",
        transportType: "wired",
      },
      hardware: {
        deviceType: "iPhone",
        marketingName: "iPhone 14 Pro",
        platform: "iOS",
        productType: "iPhone15,2",
        udid: "00008110-001234567890001E",
      },
      software: {
        osVersionNumber: { components: [17, 0, 0, 0, 0], stringValue: "17.0" },
      },
      state: {
        bootState: "booted",
        name: "iPhone 14 Pro",
      },
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
 * Create a mock AppDeps for testing
 */
export function createMockContext(overrides: Partial<AppDeps> = {}): AppDeps {
  return {
    workspace: {
      get: vi.fn().mockReturnValue(undefined),
      update: vi.fn(),
      reset: vi.fn(),
    },
    execution: {
      startScope: vi.fn().mockImplementation(async (_scope, callback) => callback()),
      setScope: vi.fn().mockImplementation(async (_scope, callback) => callback()),
      getScope: vi.fn().mockReturnValue(undefined),
      getScopeId: vi.fn().mockReturnValue(undefined),
      onClosed: vi.fn(),
    },
    progressStatusBar: { updateText: vi.fn() },
    buildManager: {} as any,
    destinationsManager: {} as any,
    tunnelManager: { autoConnect: vi.fn().mockResolvedValue(undefined) } as any,
    vscodeContext: createMockVscodeContext(),
    ...overrides,
  } as unknown as AppDeps;
}

/**
 * Minimal stand-in for vscode.ExtensionContext used in tests that call helpers
 * which need storage paths (tempFilePath etc.). Most fields are stubs.
 */
export function createMockVscodeContext(): any {
  return {
    storageUri: { fsPath: "/tmp/sweetpad-test" },
    extensionPath: "/tmp/sweetpad-ext",
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

/**
 * Helper to create a mock device with specific OS version
 */
export function createMockDeviceWithOS(osVersion: string): MockLegacyDevice {
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
): MockLegacyDevice {
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
export function createMockDeviceWithoutOS(): MockLegacyDevice {
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
export function createMockDeviceWithoutUDID(): MockLegacyDevice {
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
export function createMockDeviceWithoutName(): MockLegacyDevice {
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
