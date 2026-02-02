/**
 * Integration tests for build manager deployment logic
 */

import { exec } from "../common/exec";
import { getXcodeVersionInstalled } from "../common/cli/scripts";
import { tempFilePath } from "../common/files";
import * as iosDeploy from "../common/xcode/ios-deploy";
import { BuildManager } from "./manager";
import type { DeviceDestination } from "../devices/types";
import { iOSDeviceDestination } from "../devices/types";
import {
  createMockDevice,
  createMockDeviceWithOS,
  createMockDeviceOfType,
  createMockContext,
  createMockTerminal,
} from "../../tests/__mocks__/devices";

// Mock dependencies
jest.mock("../common/exec", () => ({
  exec: jest.fn(),
}));

jest.mock("../common/cli/scripts", () => ({
  getXcodeVersionInstalled: jest.fn(),
  getBuildSettingsToLaunch: jest.fn(),
  getIsXcbeautifyInstalled: jest.fn(),
  getIsXcodeBuildServerInstalled: jest.fn(),
  generateBuildServerConfig: jest.fn(),
  getSchemes: jest.fn(),
  getBasicProjectInfo: jest.fn(),
}));

jest.mock("../common/files", () => ({
  tempFilePath: jest.fn(),
  isFileExists: jest.fn(),
  readJsonFile: jest.fn(),
}));

jest.mock("../common/xcode/ios-deploy", () => ({
  installAndLaunchApp: jest.fn(),
  isIosDeployInstalled: jest.fn(),
}));

jest.mock("../devices/manager", () => ({
  DevicesManager: jest.fn().mockImplementation(() => ({
    getDevices: jest.fn().mockResolvedValue([]),
  })),
}));

describe("BuildManager - iOS Device Deployment Integration", () => {
  let buildManager: BuildManager;
  let mockContext: ReturnType<typeof createMockContext>;
  let mockTerminal: ReturnType<typeof createMockTerminal>;

  beforeEach(() => {
    jest.clearAllMocks();
    buildManager = new BuildManager();
    mockContext = createMockContext();
    mockTerminal = createMockTerminal();
    buildManager.context = mockContext;

    // Setup common mocks
    (getXcodeVersionInstalled as jest.Mock).mockResolvedValue({ major: 16, minor: 0, patch: 0 });
    (tempFilePath as jest.Mock).mockResolvedValue({
      path: "/tmp/test-output",
      [Symbol.asyncDispose]: jest.fn().mockResolvedValue(undefined),
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
      };
      jest.resetModules();
      jest.unmock("../common/cli/scripts");
      const scripts = require("../common/cli/scripts");
      scripts.getBuildSettingsToLaunch = jest.fn().mockResolvedValue(mockBuildSettings);
    });

    describe("with iOS 17+ device (uses devicectl)", () => {
      let modernDevice: DeviceDestination;

      beforeEach(() => {
        const device = createMockDeviceWithOS("17.0");
        modernDevice = new iOSDeviceDestination(device);
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

        const installCall = (mockTerminal.execute as jest.Mock).mock.calls.find((call) =>
          call[0].args?.includes("install"),
        );
        const deviceId = installCall[0].args[installCall[0].args.indexOf("--device") + 1];

        expect(deviceId).toBe(modernDevice.devicectlId);
      });

      it("passes launch arguments via devicectl", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
          launchArgs: ["--arg1", "value1"],
        });

        const launchCall = (mockTerminal.execute as jest.Mock).mock.calls.find((call) =>
          call[0].args?.includes("launch"),
        );
        const args = launchCall[0].args;

        expect(args).toContain("--arg1");
        expect(args).toContain("value1");
      });

      it("passes environment variables with DEVICECTL_CHILD_ prefix", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
          launchEnv: {
            TEST_VAR: "test_value",
          },
        });

        const launchCall = (mockTerminal.execute as jest.Mock).mock.calls.find((call) =>
          call[0].args?.includes("launch"),
        );
        const env = launchCall[0].env;

        expect(env).toEqual({
          DEVICECTL_CHILD_TEST_VAR: "test_value",
        });
      });

      it("launches app with devicectl after install", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: modernDevice,
        });

        const calls = (mockTerminal.execute as jest.Mock).mock.calls;
        const installCall = calls.find((call) => call[0].args?.includes("install"));
        const launchCall = calls.find((call) => call[0].args?.includes("launch"));

        expect(installCall).toBeDefined();
        expect(launchCall).toBeDefined();
      });
    });

    describe("with iOS < 17 device (uses ios-deploy)", () => {
      let legacyDevice: DeviceDestination;

      beforeEach(() => {
        const device = createMockDeviceWithOS("16.7");
        legacyDevice = new iOSDeviceDestination(device);
        (iosDeploy.isIosDeployInstalled as jest.Mock).mockResolvedValue(true);
      });

      it("uses ios-deploy for deployment", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
        });

        expect(iosDeploy.installAndLaunchApp).toHaveBeenCalledWith(
          mockContext,
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

        const callArgs = (iosDeploy.installAndLaunchApp as jest.Mock).mock.calls[0][2];
        expect(callArgs.deviceId).toBe(legacyDevice.udid);
      });

      it("passes launch arguments to ios-deploy", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
          launchArgs: ["--arg1", "value1", "--arg2"],
        });

        const callArgs = (iosDeploy.installAndLaunchApp as jest.Mock).mock.calls[0][2];
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

        const callArgs = (iosDeploy.installAndLaunchApp as jest.Mock).mock.calls[0][2];
        expect(callArgs.launchEnv).toEqual({
          TEST_VAR: "test_value",
        });
      });

      it("throws error when ios-deploy is not installed", async () => {
        (iosDeploy.isIosDeployInstalled as jest.Mock).mockResolvedValue(false);

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
        const destination = new iOSDeviceDestination(device);
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
        const destination = new iOSDeviceDestination(device);
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
        const destination = require("../devices/types").watchOSDeviceDestination;
        const watchDevice = new destination(device);

        // Should use devicectl for watchOS 10+
        expect(watchDevice.supportsDevicectl).toBe(true);
      });

      it("supports tvOS devices with appropriate version check", async () => {
        const device = createMockDeviceOfType("appleTV");
        device.deviceProperties.osVersionNumber = "17.0";
        const destination = require("../devices/types").tvOSDeviceDestination;
        const tvDevice = new destination(device);

        // Should use devicectl for tvOS 17+
        expect(tvDevice.supportsDevicectl).toBe(true);
      });

      it("supports visionOS devices with appropriate version check", async () => {
        const device = createMockDeviceOfType("appleVision");
        device.deviceProperties.osVersionNumber = "1.0";
        const destination = require("../devices/types").visionOSDeviceDestination;
        const visionDevice = new destination(device);

        // Should use devicectl for visionOS 1+
        expect(visionDevice.supportsDevicectl).toBe(true);
      });
    });
  });

  describe("deployment method selection", () => {
    it("selects devicectl for iOS 17+ devices", async () => {
      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
      expect(destination.devicectlId).toBeDefined();
    });

    it("selects ios-deploy for iOS < 17 devices", async () => {
      const device = createMockDeviceWithOS("16.7");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
      expect(destination.udid).toBeDefined();
    });

    it("selects devicectl for iOS 18+ devices", async () => {
      const device = createMockDeviceWithOS("18.0");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(true);
    });

    it("selects ios-deploy for iOS 15.x devices", async () => {
      const device = createMockDeviceWithOS("15.5");
      const destination = new iOSDeviceDestination(device);

      expect(destination.supportsDevicectl).toBe(false);
    });
  });

  describe("Xcode version handling", () => {
    it("uses --console option for Xcode 16+", async () => {
      (getXcodeVersionInstalled as jest.Mock).mockResolvedValue({ major: 16, minor: 0, patch: 0 });

      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination(device);

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

      const launchCall = (mockTerminal.execute as jest.Mock).mock.calls.find((call) =>
        call[0].args?.includes("launch"),
      );
      expect(launchCall[0].args).toContain("--console");
    });

    it("does not use --console option for Xcode < 16", async () => {
      (getXcodeVersionInstalled as jest.Mock).mockResolvedValue({ major: 15, minor: 0, patch: 0 });

      const device = createMockDeviceWithOS("17.0");
      const destination = new iOSDeviceDestination(device);

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

      const launchCall = (mockTerminal.execute as jest.Mock).mock.calls.find((call) =>
        call[0].args?.includes("launch"),
      );
      expect(launchCall[0].args).not.toContain("--console");
    });
  });
});
