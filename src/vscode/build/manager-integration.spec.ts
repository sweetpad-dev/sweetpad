import type { Mock } from "vitest";
/**
 * Integration tests for build manager deployment logic
 */

import {
  createMockDevice,
  createMockDeviceOfType,
  createMockDeviceWithOS,
  createMockTerminal,
} from "../../core/__mocks__/devices";
import type { UserAsker } from "../../core/asker/types";
import { BuildManager } from "../../core/build/manager";
import { getBuildSettingsToLaunch, getXcodeVersionInstalled } from "../../core/cli/scripts";
import type { ConfigProvider } from "../../core/config/types";
import {
  type DeviceDestination,
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "../../core/devices/types";
import { ExecutionScopeService } from "../../core/execution-scope";
import { isFileExists, readJsonFile, tempFilePath } from "../../core/files";
import { noopLogger } from "../../core/logger/types";
import { noopLspRefresher } from "../../core/lsp/types";
import { noopNotifier } from "../../core/notifier/types";
import type { WorkspaceState } from "../../core/state/types";
import type { TaskRunner } from "../../core/tasks/types";
import * as iosDeploy from "../../core/xcode/ios-deploy";

// Mock dependencies
vi.mock("../../core/exec", () => ({
  exec: vi.fn(),
}));

vi.mock("../../core/cli/scripts", () => ({
  getXcodeVersionInstalled: vi.fn(),
  getBuildSettingsToLaunch: vi.fn(),
  getIsXcbeautifyInstalled: vi.fn(),
  getIsXcodeBuildServerInstalled: vi.fn(),
  generateBuildServerConfig: vi.fn(),
  getSchemes: vi.fn(),
  getBasicProjectInfo: vi.fn(),
  detectWorkspaceType: vi.fn().mockReturnValue("xcode"),
  getSwiftPMDirectory: vi.fn().mockReturnValue("/tmp"),
  getXcodeBuildCommand: vi.fn().mockReturnValue("xcodebuild"),
  getSwiftCommand: vi.fn().mockReturnValue("swift"),
}));

vi.mock("../../core/files", () => ({
  tempFilePath: vi.fn(),
  isFileExists: vi.fn(),
  readJsonFile: vi.fn(),
}));

vi.mock("../../core/xcode/ios-deploy", () => ({
  installAndLaunchApp: vi.fn(),
  isIosDeployInstalled: vi.fn(),
}));

vi.mock("../../core/devices/manager", () => ({
  DevicesManager: vi.fn().mockImplementation(() => ({
    getDevices: vi.fn().mockResolvedValue([]),
  })),
}));

describe("BuildManager - iOS Device Deployment Integration", () => {
  let buildManager: BuildManager;
  let mockTerminal: ReturnType<typeof createMockTerminal>;

  beforeEach(() => {
    vi.clearAllMocks();
    const mockState = {
      get: vi.fn().mockReturnValue(undefined),
      update: vi.fn(),
      reset: vi.fn(),
    } as unknown as WorkspaceState;
    const mockProgress = { updateText: vi.fn() };
    const execution = new ExecutionScopeService();
    void execution;
    const mockDestinations = {
      refreshSimulators: vi.fn().mockResolvedValue([]),
      getDestinations: vi.fn().mockResolvedValue([]),
    } as any;
    const mockDiagnostics = {
      beginBuild: vi.fn().mockReturnValue({
        recordLine: vi.fn(),
        flush: vi.fn(),
      }),
    } as any;
    const mockConfig = {
      get: vi.fn().mockReturnValue(undefined),
      isDefined: vi.fn().mockReturnValue(false),
      update: vi.fn(),
    } as unknown as ConfigProvider;
    const mockAsker = {
      pick: vi.fn(),
      input: vi.fn(),
    } as unknown as UserAsker;
    const mockTaskRunner = {
      run: vi.fn().mockImplementation(async (opts) => opts.callback(mockTerminal)),
      stopMatching: vi.fn(),
    } as unknown as TaskRunner;
    buildManager = new BuildManager({
      logger: noopLogger,
      config: mockConfig,
      state: mockState,
      asker: mockAsker,
      progress: mockProgress,
      taskRunner: mockTaskRunner,
      notifier: noopNotifier,
      lsp: noopLspRefresher,
      destinations: mockDestinations,
      diagnostics: mockDiagnostics,
      workspaceRoot: {
        getPath: () => "/tmp/sweetpad-test-cwd",
        getStoragePath: async () => "/tmp/sweetpad-test",
        getRelativePath: (p) => p,
      },
    });
    mockTerminal = createMockTerminal();

    // Setup common mocks
    (getXcodeVersionInstalled as Mock).mockResolvedValue({ major: 16, minor: 0, patch: 0 });
    (tempFilePath as Mock).mockResolvedValue({
      path: "/tmp/test-output",
      [Symbol.asyncDispose]: vi.fn().mockResolvedValue(undefined),
    });
    // Mock file existence checks to return true
    (isFileExists as Mock).mockResolvedValue(true);
    // Mock readJsonFile for devicectl launch output
    (readJsonFile as Mock).mockResolvedValue({
      info: {
        outcome: "success",
      },
      result: {
        process: {
          processIdentifier: 12345,
        },
      },
    });
  });

  describe("runOniOSDevice", () => {
    const baseOptions = {
      scheme: "TestApp",
      configuration: "Debug",
      sdk: "iphoneos",
      xcworkspace: "/path/to/workspace.xcworkspace",
      watchMarker: false,
      launchArgs: [],
      launchEnv: {},
    };

    beforeEach(() => {
      // Mock build settings
      const mockBuildSettings = {
        executablePath: "/path/to/executable",
        appPath: "/path/to/TestApp.app",
        bundleIdentifier: "com.example.testapp",
        appName: "TestApp",
        executableName: "TestApp",
      };
      (getBuildSettingsToLaunch as Mock).mockResolvedValue(mockBuildSettings);
    });

    describe("with iOS 17+ device (uses devicectl)", () => {
      let modernDevice: DeviceDestination;

      beforeEach(() => {
        const device = createMockDeviceWithOS("17.0");
        modernDevice = new iOSDeviceDestination({ devicectl: device });
      });

      it("uses devicectl for deployment", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
        });

        // Should call execute with devicectl install command
        expect(mockTerminal.execute).toHaveBeenCalledWith(
          expect.objectContaining({
            command: "xcrun",
            args: expect.arrayContaining(["devicectl", "device", "install", "app"]),
          }),
        );
      });

      it("uses devicectlId for device identifier", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
        });

        const installCall = (mockTerminal.execute as Mock).mock.calls.find((call) => call[0].args?.includes("install"));
        expect(installCall).toBeDefined();
        const args = installCall![0].args;
        const deviceId = args[args.indexOf("--device") + 1];

        expect(deviceId).toBe(modernDevice.devicectlId);
      });

      it("passes launch arguments via devicectl", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
          launchArgs: ["--arg1", "value1"],
        });

        const launchSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("launch"));
        expect(launchSpec?.args).toContain("--arg1");
        expect(launchSpec?.args).toContain("value1");
      });

      it("passes environment variables with DEVICECTL_CHILD_ prefix", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
          launchEnv: {
            TEST_VAR: "test_value",
          },
        });

        const launchSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("launch"));
        expect(launchSpec?.env).toEqual({
          DEVICECTL_CHILD_TEST_VAR: "test_value",
        });
      });

      it("launches app with devicectl after install", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
        });

        const installCall = (mockTerminal.execute as Mock).mock.calls.find((call) => call[0].args?.includes("install"));
        const launchSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("launch"));

        expect(installCall).toBeDefined();
        expect(launchSpec).toBeDefined();
      });
    });

    describe("with iOS < 17 device (uses ios-deploy)", () => {
      let legacyDevice: DeviceDestination;

      beforeEach(() => {
        const device = createMockDeviceWithOS("16.7");
        legacyDevice = new iOSDeviceDestination({ devicectl: device });
        (iosDeploy.isIosDeployInstalled as Mock).mockResolvedValue(true);
      });

      it("uses ios-deploy for deployment", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
        });

        expect(iosDeploy.installAndLaunchApp).toHaveBeenCalledWith(
          mockTerminal,
          expect.objectContaining({
            deviceId: legacyDevice.udid,
            appPath: "/path/to/TestApp.app",
            bundleId: "com.example.testapp",
          }),
        );
      });

      it("uses udid for device identifier", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
        });

        const callArgs = (iosDeploy.installAndLaunchApp as Mock).mock.calls[0][1];
        expect(callArgs.deviceId).toBe(legacyDevice.udid);
      });

      it("passes launch arguments to ios-deploy", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
          launchArgs: ["--arg1", "value1", "--arg2"],
        });

        const callArgs = (iosDeploy.installAndLaunchApp as Mock).mock.calls[0][1];
        expect(callArgs.launchArgs).toEqual(["--arg1", "value1", "--arg2"]);
      });

      it("passes environment variables to ios-deploy", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
          launchEnv: {
            TEST_VAR: "test_value",
          },
        });

        const callArgs = (iosDeploy.installAndLaunchApp as Mock).mock.calls[0][1];
        expect(callArgs.launchEnv).toEqual({
          TEST_VAR: "test_value",
        });
      });

      it("throws error when ios-deploy is not installed", async () => {
        (iosDeploy.isIosDeployInstalled as Mock).mockResolvedValue(false);

        await expect(
          buildManager.runOniOSDevice(mockTerminal, {
            ...baseOptions,
            destination: legacyDevice,
          }),
        ).rejects.toThrow("ios-deploy is required");
      });
    });

    describe("error handling", () => {
      it("throws error when deviceId is missing", async () => {
        const device = createMockDevice();
        const destination = new iOSDeviceDestination({ devicectl: device });
        // Manually set properties to simulate missing deviceId
        Object.defineProperty(device, "identifier", { get: () => undefined });
        Object.defineProperty(device.hardwareProperties, "udid", { get: () => undefined });

        await expect(
          buildManager.runOniOSDevice(mockTerminal, {
            ...baseOptions,
            destination,
          }),
        ).rejects.toThrow("Could not determine device ID");
      });

      it("throws error with device name in message", async () => {
        const device = createMockDevice({
          deviceProperties: {
            name: "Test iPhone",
            osVersionNumber: "17.0",
          },
        });
        const destination = new iOSDeviceDestination({ devicectl: device });
        Object.defineProperty(device, "identifier", { get: () => undefined });
        Object.defineProperty(device.hardwareProperties, "udid", { get: () => undefined });

        await expect(
          buildManager.runOniOSDevice(mockTerminal, {
            ...baseOptions,
            destination,
          }),
        ).rejects.toThrow("Test iPhone");
      });
    });

    describe("device type support", () => {
      it("supports watchOS devices with appropriate version check", async () => {
        const device = createMockDeviceOfType("appleWatch");
        device.deviceProperties.osVersionNumber = "10.0";
        const watchDevice = new watchOSDeviceDestination({ devicectl: device });

        // Should use devicectl for watchOS 10+
        expect(watchDevice.supportsDevicectl).toBe(true);
      });

      it("supports tvOS devices with appropriate version check", async () => {
        const device = createMockDeviceOfType("appleTV");
        device.deviceProperties.osVersionNumber = "17.0";
        const tvDevice = new tvOSDeviceDestination({ devicectl: device });

        // Should use devicectl for tvOS 17+
        expect(tvDevice.supportsDevicectl).toBe(true);
      });

      it("supports visionOS devices with appropriate version check", async () => {
        const device = createMockDeviceOfType("appleVision");
        device.deviceProperties.osVersionNumber = "1.0";
        const visionDevice = new visionOSDeviceDestination({ devicectl: device });

        // Should use devicectl for visionOS 1+
        expect(visionDevice.supportsDevicectl).toBe(true);
      });
    });
  });

  describe("deployment method selection", () => {
    it("selects devicectl for iOS 17+ devices", async () => {
      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination({ devicectl: device });

      expect(destination.supportsDevicectl).toBe(true);
      expect(destination.devicectlId).toBeDefined();
    });

    it("selects ios-deploy for iOS < 17 devices", async () => {
      const device = createMockDeviceWithOS("16.7");
      const destination = new iOSDeviceDestination({ devicectl: device });

      expect(destination.supportsDevicectl).toBe(false);
      expect(destination.udid).toBeDefined();
    });

    it("selects devicectl for iOS 18+ devices", async () => {
      const device = createMockDeviceWithOS("18.0");
      const destination = new iOSDeviceDestination({ devicectl: device });

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("selects ios-deploy for iOS 15.x devices", async () => {
      const device = createMockDeviceWithOS("15.5");
      const destination = new iOSDeviceDestination({ devicectl: device });

      expect(destination.supportsDevicectl).toBe(false);
    });
  });

  describe("Xcode version handling", () => {
    it("uses --console option for Xcode 16+", async () => {
      (getXcodeVersionInstalled as Mock).mockResolvedValue({ major: 16, minor: 0, patch: 0 });

      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination({ devicectl: device });

      await buildManager.runOniOSDevice(mockTerminal, {
        scheme: "TestApp",
        configuration: "Debug",
        destination,
        sdk: "iphoneos",
        xcworkspace: "/path/to/workspace.xcworkspace",
        watchMarker: false,
        launchArgs: [],
        launchEnv: {},
      });

      const launchSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("launch"));
      expect(launchSpec?.args).toContain("--console");
    });

    it("does not use --console option for Xcode < 16", async () => {
      (getXcodeVersionInstalled as Mock).mockResolvedValue({ major: 15, minor: 0, patch: 0 });

      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination({ devicectl: device });

      await buildManager.runOniOSDevice(mockTerminal, {
        scheme: "TestApp",
        configuration: "Debug",
        destination,
        sdk: "iphoneos",
        xcworkspace: "/path/to/workspace.xcworkspace",
        watchMarker: false,
        launchArgs: [],
        launchEnv: {},
      });

      const launchSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("launch"));
      expect(launchSpec?.args).not.toContain("--console");
    });
  });
});
