import { EventEmitter } from "node:events";

import type { Mock } from "vitest";

import { createMockTerminal } from "../__mocks__/devices";
import { exec } from "../exec";
import { tempFilePath } from "../files";
import { noopLogger } from "../logger/types";
import * as iosDeploy from "./ios-deploy";

vi.mock("../exec", () => ({
  exec: vi.fn(),
}));

vi.mock("../files", () => ({
  tempFilePath: vi.fn(),
}));

const mockSpawn = vi.fn();
vi.mock("node:child_process", () => ({
  spawn: (...args: any[]) => mockSpawn(...args),
}));

function createMockChildProcess() {
  const proc = new EventEmitter() as any;
  proc.stdout = new EventEmitter();
  proc.stderr = new EventEmitter();
  proc.kill = vi.fn();
  return proc;
}

const STORAGE_PATH = "/tmp/sweetpad-test";

describe("ios-deploy", () => {
  describe("isIosDeployInstalled", () => {
    beforeEach(() => {
      vi.clearAllMocks();
    });

    it("returns true when ios-deploy is installed", async () => {
      (exec as Mock).mockResolvedValue("1.12.0\n");

      const result = await iosDeploy.isIosDeployInstalled({ cwd: STORAGE_PATH, logger: noopLogger });

      expect(result).toBe(true);
      expect(exec).toHaveBeenCalledWith({
        command: "ios-deploy",
        args: ["--version"],
        cwd: STORAGE_PATH,
        logger: noopLogger,
      });
    });

    it("returns false when ios-deploy is not installed", async () => {
      (exec as Mock).mockRejectedValue(new Error("Command not found"));

      const result = await iosDeploy.isIosDeployInstalled({ cwd: STORAGE_PATH, logger: noopLogger });

      expect(result).toBe(false);
    });

    it("returns false when ios-deploy command fails", async () => {
      (exec as Mock).mockRejectedValue(new Error("ENOENT"));

      const result = await iosDeploy.isIosDeployInstalled({ cwd: STORAGE_PATH, logger: noopLogger });

      expect(result).toBe(false);
    });
  });

  describe("installAndLaunchApp", () => {
    const mockTerminal = createMockTerminal();
    let iosDeployExecuteCalls: any[] = [];

    function setupExecuteMock() {
      iosDeployExecuteCalls = [];
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          iosDeployExecuteCalls.push(options);
          return Promise.resolve();
        }
        if (options.command === "tail") {
          return Promise.resolve();
        }
        return Promise.resolve();
      });
    }

    beforeEach(() => {
      vi.clearAllMocks();
      setupExecuteMock();

      mockSpawn.mockReturnValue(createMockChildProcess());

      (tempFilePath as Mock).mockImplementation(async () => {
        return {
          path: "/tmp/test-file",
          [Symbol.asyncDispose]: vi.fn().mockResolvedValue(undefined),
        };
      });
    });

    it("installs and launches app with correct arguments", async () => {
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        logger: noopLogger,
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
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchArgs: ["--arg1", "value1", "--arg2", "value2"],
        logger: noopLogger,
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
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchEnv: {
          ENV_VAR1: "value1",
          ENV_VAR2: "value2",
        },
        logger: noopLogger,
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      const args = iosDeployExecuteCalls[0].args;
      expect(args).toContain("--env");
      expect(args).toContain("ENV_VAR1=value1");
      expect(args).toContain("--env");
      expect(args).toContain("ENV_VAR2=value2");
    });

    it("adds both launch arguments and environment variables", async () => {
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchArgs: ["--debug"],
        launchEnv: { DEBUG_MODE: "1" },
        logger: noopLogger,
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      const args = iosDeployExecuteCalls[0].args;
      expect(args).toContain("--args");
      expect(args).toContain("--debug");
      expect(args).toContain("--env");
      expect(args).toContain("DEBUG_MODE=1");
    });

    it("throws error when command not found (exit code 127)", async () => {
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Command not found");
          error.exitCode = 127;
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockTerminal, {
          storagePath: STORAGE_PATH,
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
          logger: noopLogger,
        }),
      ).rejects.toThrow("Command not found");
    });

    it("throws error when device not found", async () => {
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Could not connect to device");
          error.exitCode = 1;
          error.stderr = "Error: no device found";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockTerminal, {
          storagePath: STORAGE_PATH,
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
          logger: noopLogger,
        }),
      ).rejects.toThrow("Could not connect to device");
    });

    it("throws error when stderr contains device not found message", async () => {
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Device not found");
          error.exitCode = 255;
          error.stderr = "ERROR: Device not found, check connection";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockTerminal, {
          storagePath: STORAGE_PATH,
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
          logger: noopLogger,
        }),
      ).rejects.toThrow("Device not found");
    });

    it("ignores non-zero exit code from safequit", async () => {
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
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
        iosDeploy.installAndLaunchApp(mockTerminal, {
          storagePath: STORAGE_PATH,
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
          logger: noopLogger,
        }),
      ).resolves.not.toThrow();

      expect(iosDeployExecuteCalls).toHaveLength(1);
    });

    it("throws error when process is interrupted by signal (exit code 130)", async () => {
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Process interrupted");
          error.exitCode = 130;
          error.stderr = "User interrupted the process";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockTerminal, {
          storagePath: STORAGE_PATH,
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
          logger: noopLogger,
        }),
      ).rejects.toThrow("Process interrupted");
    });

    it("throws error when process is killed by SIGTERM (exit code 143)", async () => {
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Process terminated");
          error.exitCode = 143;
          error.stderr = "";
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockTerminal, {
          storagePath: STORAGE_PATH,
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
          logger: noopLogger,
        }),
      ).rejects.toThrow("Process terminated");
    });

    it("streams log file using spawn instead of terminal.execute", async () => {
      // streamLogFile uses child_process.spawn directly instead of terminal.execute,
      // so it won't appear as a terminal.execute call for tail
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        logger: noopLogger,
      });

      expect(iosDeployExecuteCalls).toHaveLength(1);
      const allExecuteCalls = (mockTerminal.execute as Mock).mock.calls;
      const tailCalls = allExecuteCalls.filter((call: any) => call[0]?.command === "tail");
      expect(tailCalls).toHaveLength(0);
    });

    it("handles empty launch arguments array", async () => {
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchArgs: [],
        logger: noopLogger,
      });

      const args = iosDeployExecuteCalls[0].args;
      // Should not include --args when array is empty
      expect(args).not.toContain("--args");
    });

    it("handles empty launch env object", async () => {
      await iosDeploy.installAndLaunchApp(mockTerminal, {
        storagePath: STORAGE_PATH,
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
        launchEnv: {},
        logger: noopLogger,
      });

      const args = iosDeployExecuteCalls[0].args;
      // Should not include --env when object is empty
      expect(args).not.toContain("--env");
    });
  });
});
