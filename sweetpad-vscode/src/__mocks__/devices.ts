/**
 * Mock utilities for device-related tests
 * Provides helper functions to generate mock device objects and test contexts
 */

import type { AppDeps } from "../common/commands";
import type { ProcessGroup, ProcessHandle, ProcessSpec, TaskTerminal } from "../common/tasks/types";
import type { DevicectlDevice } from "../common/xcode/devicectl";
import type { XcdeviceDevice } from "../common/xcode/xcdevice";

/**
 * Create a mock devicectl device, as the addon's "parseDevicectlDevices" returns it, with
 * optional overrides. Pass `undefined` for a field to model devicectl leaving it out.
 */
export function createMockDevice(overrides: Partial<DevicectlDevice> = {}): DevicectlDevice {
  return {
    identifier: "urn:x-ios-devicectl:device-CS4567890-1234567890123456",
    udid: "00008110-001234567890001E",
    name: "iPhone 14 Pro",
    marketingName: "iPhone 14 Pro",
    productType: "iPhone15,2",
    deviceType: "iPhone",
    platform: "iOS",
    osVersion: "17.0",
    connection: "connected",
    pairing: "paired",
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
export function createMockDeviceWithOS(osVersion: string): DevicectlDevice {
  return createMockDevice({ name: "iPhone Test Device", osVersion: osVersion });
}

/**
 * Helper to create a mock device of a specific type
 */
export function createMockDeviceOfType(
  deviceType: "iPhone" | "iPad" | "appleWatch" | "appleTV" | "appleVision",
): DevicectlDevice {
  const hardware: Record<string, Partial<DevicectlDevice>> = {
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

  return createMockDevice(hardware[deviceType]);
}

/**
 * Create a mock device without OS version (for testing fallback behavior)
 */
export function createMockDeviceWithoutOS(): DevicectlDevice {
  return createMockDevice({ name: "iPhone Unknown", osVersion: undefined, udid: undefined });
}

/**
 * Create a mock device without UDID (for testing fallback behavior)
 */
export function createMockDeviceWithoutUDID(): DevicectlDevice {
  return createMockDevice({ udid: undefined });
}

/**
 * Create a mock device with missing name (for testing fallback behavior)
 */
export function createMockDeviceWithoutName(): DevicectlDevice {
  return createMockDevice({ name: undefined, marketingName: undefined });
}
