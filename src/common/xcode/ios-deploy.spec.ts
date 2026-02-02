/**
 * Unit tests for ios-deploy integration
 */

import { exec } from "../exec";
import { tempFilePath } from "../files";
import * as iosDeploy from "./ios-deploy";
import { createMockContext, createMockTerminal } from "../../../tests/__mocks__/devices";

// Mock dependencies
jest.mock("../exec", () => ({
  exec: jest.fn(),
}));

jest.mock("../files", () => ({
  tempFilePath: jest.fn(),
}));

jest.mock("../logger", () => ({
  commonLogger: {
    debug: jest.fn(),
    error: jest.fn(),
  },
}));

describe("ios-deploy", () => {
  describe("isIosDeployInstalled", () => {
    beforeEach(() => {
      jest.clearAllMocks();
    });

    it("returns true when ios-deploy is installed", async () => {
      (exec as jest.Mock).mockResolvedValue("1.12.0\n");

      const result = await iosDeploy.isIosDeployInstalled();

      expect(result).toBe(true);
      expect(exec).toHaveBeenCalledWith({
        command: "ios-deploy",
        args: ["--version"],
      });
    });

    it("returns false when ios-deploy is not installed", async () => {
      (exec as jest.Mock).mockRejectedValue(new Error("Command not found"));

      const result = await iosDeploy.isIosDeployInstalled();

      expect(result).toBe(false);
    });

    it("returns false when ios-deploy command fails", async () => {
      (exec as jest.Mock).mockRejectedValue(new Error("ENOENT"));

      const result = await iosDeploy.isIosDeployInstalled();

      expect(result).toBe(false);
    });
  });

  describe("installAndLaunchApp", () => {
    const mockContext = createMockContext();
    const mockTerminal = createMockTerminal();
    let iosDeployExecuteCalls: any[] = [];

    function setupExecuteMock() {
      iosDeployExecuteCalls = [];
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          iosDeployExecuteCalls.push(options);
          return Promise.resolve();
        } else if (options.command === "tail") {
          return Promise.resolve();
        }
        return Promise.resolve();
      });
    }

    beforeEach(() => {
      jest.clearAllMocks();
      setupExecuteMock();

      // Setup tempFilePath mock to return disposable objects
      (tempFilePath as jest.Mock).mockImplementation(async () => {
        return {
          path: "/tmp/test-file",
          [Symbol.asyncDispose]: jest.fn().mockResolvedValue(undefined),
        };
      });
    });

    it("installs and launches app with correct arguments", async () => {
      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      expect(iosDeployExecuteCalls[0]).toEqual({
        command: "ios-deploy",
        args: [
          "--id",
          "00008110-001234567890001E",
          "--bundle",
          "/path/to/app.app",
          "--debug",
          "--unbuffered",
          "--output",
          "/tmp/test-file",
          "--error_output",
          "/tmp/test-file",
        ],
      });
    });

    it("adds launch arguments when provided", async () => {
      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchArgs: ["--arg1", "value1", "--arg2", "value2"],
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      const args = iosDeployExecuteCalls[0].args;
      expect(args).toContain("--args");
      expect(args).toContain("--arg1");
      expect(args).toContain("value1");
      expect(args).toContain("--arg2");
      expect(args).toContain("value2");
    });

    it("adds environment variables when provided", async () => {
      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchEnv: {
          ENV_VAR1: "value1",
          ENV_VAR2: "value2",
        },
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      const args = iosDeployExecuteCalls[0].args;
      expect(args).toContain("--env");
      expect(args).toContain("ENV_VAR1=value1");
      expect(args).toContain("--env");
      expect(args).toContain("ENV_VAR2=value2");
    });

    it("adds both launch arguments and environment variables", async () => {
      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchArgs: ["--debug"],
        launchEnv: {
          DEBUG_MODE: "1",
        },
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      const args = iosDeployExecuteCalls[0].args;
      expect(args).toContain("--args");
      expect(args).toContain("--debug");
      expect(args).toContain("--env");
      expect(args).toContain("DEBUG_MODE=1");
    });

    it("throws error when command not found (exit code 127)", async () => {
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Command not found");
          error.exitCode = 127;
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
        }),
      ).rejects.toThrow("Command not found");
    });

    it("throws error when device not found", async () => {
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Could not connect to device");
          error.exitCode = 1;
          error.stderr = "Error: no device found";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
        }),
      ).rejects.toThrow("Could not connect to device");
    });

    it("throws error when stderr contains device not found message", async () => {
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Device not found");
          error.exitCode = 255;
          error.stderr = "ERROR: Device not found, check connection";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
        }),
      ).rejects.toThrow("Device not found");
    });

    it("ignores non-zero exit code from safequit", async () => {
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          iosDeployExecuteCalls.push(options);
          const error: any = new Error("ios-deploy exited with code 255");
          error.exitCode = 255;
          error.stderr = "Application quit with safequit";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
        }),
      ).resolves.not.toThrow();

      expect(iosDeployExecuteCalls).toHaveLength(1);
    });

    it("ignores exit code when stderr does not contain device errors", async () => {
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Process interrupted");
          error.exitCode = 130;
          error.stderr = "User interrupted the process";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
        }),
      ).resolves.not.toThrow();
    });

    it("starts log file streaming in background", async () => {
      let tailCalled = false;
      (mockTerminal.execute as jest.Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          iosDeployExecuteCalls.push(options);
          return Promise.resolve();
        } else if (options.command === "tail") {
          tailCalled = true;
          return Promise.resolve();
        }
        return Promise.resolve();
      });

      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
      });

      // Should have ios-deploy call and tail -f call
      expect(iosDeployExecuteCalls).toHaveLength(1);
      expect(tailCalled).toBe(true);
    });

    it("handles empty launch arguments array", async () => {
      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchArgs: [],
      });

      const args = iosDeployExecuteCalls[0].args;
      // Should not include --args when array is empty
      expect(args).not.toContain("--args");
    });

    it("handles empty launch env object", async () => {
      await iosDeploy.installAndLaunchApp(mockContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchEnv: {},
      });

      const args = iosDeployExecuteCalls[0].args;
      // Should not include --env when object is empty
      expect(args).not.toContain("--env");
    });
  });
});
