import type { Mock } from "vitest";
/**
 * Integration tests for build manager deployment logic
 */
import type * as vscode from "vscode";

import {
  createMockDevice,
  createMockDeviceOfType,
  createMockDeviceWithOS,
  createMockTerminal,
} from "../__mocks__/devices";
import { getBuildSettingsToLaunch, getXcodeVersionInstalled } from "../common/cli/scripts";
import { ExecutionScopeService } from "../common/execution-scope";
import { isFileExists, readJsonFile, tempFilePath } from "../common/files";
import type { WorkspaceStateService } from "../common/workspace-state";
import * as iosDeploy from "../common/xcode/ios-deploy";
import type { TunnelManager } from "../devices/tunnel";
import type { DeviceDestination } from "../devices/types";
import {
  iOSDeviceDestination,
  tvOSDeviceDestination,
  visionOSDeviceDestination,
  watchOSDeviceDestination,
} from "../devices/types";
import type { ProgressStatusBar } from "../system/status-bar";
import { BuildManager } from "./manager";

// Mock dependencies
vi.mock("../common/exec", () => ({
  exec: vi.fn(),
}));

vi.mock("../common/cli/scripts", () => ({
  getXcodeVersionInstalled: vi.fn(),
  getBuildSettingsToLaunch: vi.fn(),
  getIsXcbeautifyInstalled: vi.fn(),
  getIsXcodeBuildServerInstalled: vi.fn(),
  generateBuildServerConfig: vi.fn(),
  getSchemes: vi.fn(),
  getBasicProjectInfo: vi.fn(),
}));

vi.mock("../common/files", () => ({
  tempFilePath: vi.fn(),
  isFileExists: vi.fn(),
  readJsonFile: vi.fn(),
}));

vi.mock("../common/xcode/ios-deploy", () => ({
  installAndLaunchApp: vi.fn(),
  isIosDeployInstalled: vi.fn(),
  launchDebugserver: vi.fn(),
}));

vi.mock("../devices/manager", () => ({
  DevicesManager: vi.fn().mockImplementation(() => ({
    getDevices: vi.fn().mockResolvedValue([]),
  })),
}));

describe("BuildManager - iOS Device Deployment Integration", () => {
  let buildManager: BuildManager;
  let mockTerminal: ReturnType<typeof createMockTerminal>;
  let mockVscodeContext: vscode.ExtensionContext;
  let mockWorkspace: WorkspaceStateService;
  let mockTunnel: TunnelManager;

  beforeEach(() => {
    vi.clearAllMocks();
    mockWorkspace = {
      get: vi.fn().mockReturnValue(undefined),
      update: vi.fn(),
      reset: vi.fn(),
    } as unknown as WorkspaceStateService;
    const mockProgress = { updateText: vi.fn() } as unknown as ProgressStatusBar;
    const execution = new ExecutionScopeService();
    mockTunnel = { autoConnect: vi.fn().mockResolvedValue(undefined) } as unknown as TunnelManager;
    mockVscodeContext = {
      storageUri: { fsPath: "/tmp/sweetpad-test" },
      extensionPath: "/tmp/sweetpad-ext",
    } as unknown as vscode.ExtensionContext;
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
    buildManager = new BuildManager({
      workspaceState: mockWorkspace,
      progress: mockProgress,
      execution,
      tunnel: mockTunnel,
      vscodeContext: mockVscodeContext,
      destinations: mockDestinations,
      diagnostics: mockDiagnostics,
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
      debug: false,
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
          mockVscodeContext,
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

        const callArgs = (iosDeploy.installAndLaunchApp as Mock).mock.calls[0][2];
        expect(callArgs.deviceId).toBe(legacyDevice.udid);
      });

      it("passes launch arguments to ios-deploy", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
          launchArgs: ["--arg1", "value1", "--arg2"],
        });

        const callArgs = (iosDeploy.installAndLaunchApp as Mock).mock.calls[0][2];
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

        const callArgs = (iosDeploy.installAndLaunchApp as Mock).mock.calls[0][2];
        expect(callArgs.launchEnv).toEqual({
          TEST_VAR: "test_value",
        });
      });

      it("streams os_log output the ios-deploy relay does not carry", async () => {
        await buildManager.runOniOSDevice(mockTerminal, {
          ...baseOptions,
          destination: legacyDevice,
        });

        const syslogSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("syslog"));
        expect(syslogSpec?.args).toEqual(expect.arrayContaining(["syslog", "live", "--process-name", "TestApp"]));
        // ios-deploy still owns the foreground so Ctrl+C reaches it.
        expect(iosDeploy.installAndLaunchApp).toHaveBeenCalled();
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

      describe("when debugging", () => {
        const debugOptions = { ...baseOptions, debug: true };

        beforeEach(() => {
          (iosDeploy.launchDebugserver as Mock).mockResolvedValue({
            debugserver: {
              port: 12345,
              deviceAppPath: "/private/var/containers/Bundle/Application/ABC/TestApp.app",
              symbolsPath: "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
            },
            wait: vi.fn().mockResolvedValue(undefined),
          });
        });

        it("starts a debugserver instead of launching the app under ios-deploy's own LLDB", async () => {
          await buildManager.runOniOSDevice(mockTerminal, {
            ...debugOptions,
            destination: legacyDevice,
          });

          expect(iosDeploy.launchDebugserver).toHaveBeenCalledWith(
            expect.anything(),
            expect.objectContaining({
              deviceId: legacyDevice.udid,
              appPath: "/path/to/TestApp.app",
            }),
          );
          expect(iosDeploy.installAndLaunchApp).not.toHaveBeenCalled();
        });

        it("records the debugserver coordinates for the debug configuration provider", async () => {
          await buildManager.runOniOSDevice(mockTerminal, {
            ...debugOptions,
            destination: legacyDevice,
            launchArgs: ["--arg1"],
            launchEnv: { TEST_VAR: "test_value" },
          });

          const [key, value] = (mockWorkspace.update as Mock).mock.calls.at(-1)!;
          expect(key).toBe("build.lastLaunchedApp");
          expect(value).toMatchObject({
            type: "device",
            appPath: "/path/to/TestApp.app",
            destinationId: legacyDevice.udid,
            debugserver: {
              port: 12345,
              deviceAppPath: "/private/var/containers/Bundle/Application/ABC/TestApp.app",
              symbolsPath: "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
              launchArgs: ["--arg1"],
              launchEnv: { TEST_VAR: "test_value" },
            },
          });
        });

        it("holds the watch marker until the debugserver is listening", async () => {
          const writes: string[] = [];
          (mockTerminal.write as Mock).mockImplementation((data: string) => writes.push(data));
          (iosDeploy.launchDebugserver as Mock).mockImplementation(async () => {
            // Nothing may signal readiness while ios-deploy is still working: the marker is
            // what releases the pre-launch task and starts the debug session.
            expect(writes.join("")).not.toContain("watch marker");
            return {
              debugserver: { port: 12345, deviceAppPath: "/private/var/.../TestApp.app" },
              wait: vi.fn().mockResolvedValue(undefined),
            };
          });

          await buildManager.runOniOSDevice(mockTerminal, {
            ...debugOptions,
            destination: legacyDevice,
            watchMarker: true,
          });

          expect(writes.join("")).toContain("watch marker");
        });

        it("streams device logs to the terminal alongside the Debug Console", async () => {
          await buildManager.runOniOSDevice(mockTerminal, {
            ...debugOptions,
            destination: legacyDevice,
          });

          // The app's own stdout/stderr go to CodeLLDB over debugserver; os_log output only
          // reaches the terminal if the sidecar runs.
          const syslogSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("syslog"));
          expect(syslogSpec?.args).toEqual(expect.arrayContaining(["syslog", "live", "--process-name", "TestApp"]));
        });

        it("does not open a tunnel, which only iOS 17+ RemoteXPC needs", async () => {
          await buildManager.runOniOSDevice(mockTerminal, {
            ...debugOptions,
            destination: legacyDevice,
          });

          expect(mockTunnel.autoConnect).not.toHaveBeenCalled();
        });

        it("still writes the marker up front on the devicectl route", async () => {
          const writes: string[] = [];
          (mockTerminal.write as Mock).mockImplementation((data: string) => writes.push(data));

          await buildManager.runOniOSDevice(mockTerminal, {
            ...debugOptions,
            destination: new iOSDeviceDestination({ devicectl: createMockDeviceWithOS("17.0") }),
            watchMarker: true,
          });

          expect(writes.join("")).toContain("watch marker");
          expect(iosDeploy.launchDebugserver).not.toHaveBeenCalled();
        });
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
        debug: false,
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
        debug: false,
      });

      const launchSpec = mockTerminal.spawnedSpecs.find((s) => s.args?.includes("launch"));
      expect(launchSpec?.args).not.toContain("--console");
    });
  });
});
