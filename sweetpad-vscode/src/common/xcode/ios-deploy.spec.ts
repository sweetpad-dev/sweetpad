import { EventEmitter } from "node:events";
/**
 * Unit tests for ios-deploy integration
 */

import type { Mock } from "vitest";

import { createMockContext, createMockTerminal } from "../../__mocks__/devices";
import { exec } from "../exec";
import { tempFilePath } from "../files";
import type { ProcessExit, ProcessGroup, ProcessHandle, ProcessOutputSink, ProcessSpec } from "../tasks/types";
import * as iosDeploy from "./ios-deploy";

// Mock dependencies
vi.mock("../exec", () => ({
  exec: vi.fn(),
}));

vi.mock("../files", () => ({
  tempFilePath: vi.fn(),
}));

vi.mock("../logger", () => ({
  commonLogger: {
    debug: vi.fn(),
    error: vi.fn(),
  },
}));

// Mock child_process.spawn for the tail -f streaming
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

/**
 * Real ios-deploy console output for an iOS 15.6.1 device, from issue #309. Line endings are
 * "\r\n" because the launch path reads it off a pty.
 */
const DEBUG_PHASE_OUTPUT = [
  "[ 95%] GeneratingApplicationMap",
  "[100%] Installed package .build/Build/Products/Debug-iphoneos/MyApp.app/",
  "------ Debug phase ------",
  "Starting debug of MyApp (J96AP, iPad mini 4, iphoneos, arm64, 15.6.1, 19G82) a.k.a. 'iPad mini' connected through USB...",
  "[  0%] Looking up developer disk image",
  "[ 95%] Developer disk image mounted successfully",
  "Symbol Path: /Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
  "[100%] Listening for lldb connections",
  "-------------------------",
  "debugserver port: 12345",
  "App path: /private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/MyApp.app",
  "",
].join("\r\n");

/**
 * ProcessGroup whose spawned handle is driven by hand: "emit" pushes a chunk to the process's
 * data listeners, "exit" settles it. Mirrors the pty contract, where stdout and stderr are
 * merged onto onData.
 */
function createControllableGroup() {
  const spawnedSpecs: ProcessSpec[] = [];
  const written: string[] = [];
  const kill = vi.fn();
  let liveMain = false;

  type Child = {
    sinks: ProcessOutputSink[];
    exit: Promise<ProcessExit>;
    settle: (exit: ProcessExit) => void;
  };
  // One entry per spawn — a retry gets its own exit promise, as it does in the real group.
  const children: Child[] = [];

  function newest(): Child {
    return children[children.length - 1];
  }

  const group = {
    terminal: {
      execute: vi.fn(),
      write: vi.fn((data: string) => written.push(data)),
      runGroup: vi.fn(),
    },
    spawn: (spec: ProcessSpec): ProcessHandle => {
      // Mirrors the real group (tasks/v3.ts): it refuses a second "main" child while the
      // first is still alive, so a retry that spawns before its kill lands fails here.
      if (spec.main && liveMain) {
        throw new Error("Group already has a main process");
      }
      if (spec.main) {
        liveMain = true;
      }
      spawnedSpecs.push(spec);

      // Assigned synchronously by the Promise executor.
      let settle!: (exit: ProcessExit) => void;
      const exit = new Promise<ProcessExit>((resolve) => {
        settle = resolve;
      });
      const child: Child = { sinks: [], exit, settle };
      children.push(child);

      return {
        pid: 4321 + children.length,
        exit,
        kill: vi.fn(() => {
          kill();
          // The real kill is asynchronous — SIGTERM lands, then onExit frees the main slot.
          setTimeout(() => {
            liveMain = false;
            settle({ code: 143, signal: "SIGTERM" });
          }, 10);
        }),
        onData: (listener: ProcessOutputSink) => child.sinks.push(listener),
        onError: () => {},
      };
    },
  } as unknown as ProcessGroup;

  return {
    group,
    spawnedSpecs,
    written,
    kill,
    emit: (chunk: string) => {
      for (const sink of newest().sinks) {
        sink(chunk);
      }
    },
    exitWith: (code: number) => {
      liveMain = false;
      newest().settle({ code, signal: null });
    },
  };
}

describe("ios-deploy", () => {
  describe("parseDebugserverOutput", () => {
    it("extracts port, device app path and symbols path from the debug phase", () => {
      expect(iosDeploy.parseDebugserverOutput(DEBUG_PHASE_OUTPUT)).toEqual({
        port: 12345,
        deviceAppPath: "/private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/MyApp.app",
        symbolsPath: "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
      });
    });

    it("reports nothing while the install phase is still running", () => {
      expect(iosDeploy.parseDebugserverOutput("[ 95%] GeneratingApplicationMap\r\n")).toEqual({
        port: undefined,
        deviceAppPath: undefined,
        symbolsPath: undefined,
      });
    });

    it("reports the symbols path before the port arrives", () => {
      const partial = DEBUG_PHASE_OUTPUT.slice(0, DEBUG_PHASE_OUTPUT.indexOf("debugserver port"));

      const parsed = iosDeploy.parseDebugserverOutput(partial);
      expect(parsed.symbolsPath).toContain("iOS DeviceSupport");
      expect(parsed.port).toBeUndefined();
      expect(parsed.deviceAppPath).toBeUndefined();
    });

    it("parses output without carriage returns", () => {
      const parsed = iosDeploy.parseDebugserverOutput(DEBUG_PHASE_OUTPUT.replace(/\r/g, ""));

      expect(parsed.port).toBe(12345);
      expect(parsed.deviceAppPath).toBe(
        "/private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/MyApp.app",
      );
    });

    it("does not mistake a similar line elsewhere in the stream for the port", () => {
      const parsed = iosDeploy.parseDebugserverOutput("note: debugserver port: not a number\r\n");

      expect(parsed.port).toBeUndefined();
    });
  });

  describe("launchDebugserver", () => {
    beforeEach(() => {
      vi.clearAllMocks();
    });

    it("invokes ios-deploy with --nolldb so LLDB owns the launch", async () => {
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/MyApp.app",
      });
      harness.emit(DEBUG_PHASE_OUTPUT);
      await sessionPromise;

      expect(harness.spawnedSpecs).toHaveLength(1);
      expect(harness.spawnedSpecs[0]).toEqual({
        command: "ios-deploy",
        args: ["--id", "00008110-001234567890001E", "--bundle", "/path/to/MyApp.app", "--nolldb", "--unbuffered"],
        pty: true,
        main: true,
      });
      // --debug would start ios-deploy's own LLDB and take the process CodeLLDB wants.
      expect(harness.spawnedSpecs[0].args).not.toContain("--debug");
    });

    it("resolves with the parsed debugserver coordinates", async () => {
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
      });
      harness.emit(DEBUG_PHASE_OUTPUT);
      const session = await sessionPromise;

      expect(session.debugserver).toEqual({
        port: 12345,
        deviceAppPath: "/private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/MyApp.app",
        symbolsPath: "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
      });
    });

    it("waits for the whole debug phase when output arrives in fragments", async () => {
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
      });

      let resolved = false;
      void sessionPromise.then(() => {
        resolved = true;
      });

      // The port alone is not enough — the device app path is what LLDB repoints the module at.
      harness.emit("[100%] Listening for lldb connections\r\ndebugserver port: 12345\r\n");
      await Promise.resolve();
      expect(resolved).toBe(false);

      harness.emit("App path: /private/var/containers/Bundle/Application/ABC/MyApp.app\r\n");
      const session = await sessionPromise;
      expect(session.debugserver.port).toBe(12345);
      expect(session.debugserver.symbolsPath).toBeUndefined();
    });

    it("mirrors ios-deploy output to the terminal", async () => {
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
      });
      harness.emit(DEBUG_PHASE_OUTPUT);
      await sessionPromise;

      expect(harness.written.join("")).toContain("Developer disk image mounted successfully");
    });

    it("keeps the session open until ios-deploy exits", async () => {
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
      });
      harness.emit(DEBUG_PHASE_OUTPUT);
      const session = await sessionPromise;

      let finished = false;
      void session.wait().then(() => {
        finished = true;
      });
      await Promise.resolve();
      expect(finished).toBe(false);

      harness.exitWith(0);
      await session.wait();
      expect(finished).toBe(true);
    });

    it("retries once when ios-deploy dies before the debug phase", async () => {
      const first = createControllableGroup();
      let attempt = 0;
      const spawnedArgs: ProcessSpec[] = [];
      const group = {
        terminal: first.group.terminal,
        spawn: (spec: ProcessSpec) => {
          spawnedArgs.push(spec);
          attempt += 1;
          if (attempt === 1) {
            // Dies immediately, the way a first connection to an older device does.
            return {
              pid: 1,
              exit: Promise.resolve({ code: 253, signal: null } as ProcessExit),
              kill: vi.fn(),
              onData: (sink: ProcessOutputSink) => sink("[  0%] Looking up developer disk image\r\n"),
              onError: () => {},
            };
          }
          return {
            pid: 2,
            exit: new Promise<ProcessExit>(() => {}),
            kill: vi.fn(),
            onData: (sink: ProcessOutputSink) => sink(DEBUG_PHASE_OUTPUT),
            onError: () => {},
          };
        },
      } as unknown as ProcessGroup;

      const session = await iosDeploy.launchDebugserver(group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
      });

      expect(spawnedArgs).toHaveLength(2);
      expect(session.debugserver.port).toBe(12345);
    });

    it("surfaces ios-deploy's own output when every attempt fails", async () => {
      const group = {
        terminal: { execute: vi.fn(), write: vi.fn(), runGroup: vi.fn() },
        spawn: () => ({
          pid: 1,
          exit: Promise.resolve({ code: 253, signal: null } as ProcessExit),
          kill: vi.fn(),
          onData: (sink: ProcessOutputSink) =>
            sink("[ 95%] Developer disk image mounted successfully\r\nerror: could not start device support\r\n"),
          onError: () => {},
        }),
      } as unknown as ProcessGroup;

      await expect(
        iosDeploy.launchDebugserver(group, { deviceId: "device-id", appPath: "/path/to/MyApp.app" }),
      ).rejects.toThrow("could not start device support");
    });

    it("gives up and kills ios-deploy when the debug phase never arrives", async () => {
      vi.useFakeTimers();
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
        timeoutMs: 1000,
        attempts: 1,
      });
      // Captured rather than awaited inline: the timers have to advance before it settles.
      const settled: Promise<Error> = sessionPromise.then(
        () => new Error("launchDebugserver resolved instead of timing out"),
        (error: Error) => error,
      );

      harness.emit("[  0%] Looking up developer disk image\r\n");
      // Past the timeout, then past the kill landing — the rejection waits for the process
      // to actually exit.
      await vi.advanceTimersByTimeAsync(1001);
      await vi.advanceTimersByTimeAsync(50);

      expect((await settled).message).toContain("Timed out waiting for ios-deploy");
      expect(harness.kill).toHaveBeenCalled();
      vi.useRealTimers();
    });

    it("retries after a timeout without tripping the group's single-main rule", async () => {
      vi.useFakeTimers();
      const harness = createControllableGroup();

      const sessionPromise = iosDeploy.launchDebugserver(harness.group, {
        deviceId: "device-id",
        appPath: "/path/to/MyApp.app",
        timeoutMs: 1000,
        attempts: 2,
      });
      const settled: Promise<Error> = sessionPromise.then(
        () => new Error("launchDebugserver resolved instead of timing out"),
        (error: Error) => error,
      );

      // First attempt stalls and is killed. The retry must wait for that kill to land:
      // spawning a second "main" child while the first is alive throws in the real group.
      await vi.advanceTimersByTimeAsync(1001);
      await vi.advanceTimersByTimeAsync(50);
      expect(harness.spawnedSpecs).toHaveLength(2);

      await vi.advanceTimersByTimeAsync(1001);
      await vi.advanceTimersByTimeAsync(50);

      // The surviving error is the timeout, not a spawn failure from the retry.
      expect((await settled).message).toContain("Timed out waiting for ios-deploy");
      vi.useRealTimers();
    });
  });

  describe("isIosDeployInstalled", () => {
    beforeEach(() => {
      vi.clearAllMocks();
    });

    it("returns true when ios-deploy is installed", async () => {
      (exec as Mock).mockResolvedValue("1.12.0\n");

      const result = await iosDeploy.isIosDeployInstalled();

      expect(result).toBe(true);
      expect(exec).toHaveBeenCalledWith({
        command: "ios-deploy",
        args: ["--version"],
      });
    });

    it("returns false when ios-deploy is not installed", async () => {
      (exec as Mock).mockRejectedValue(new Error("Command not found"));

      const result = await iosDeploy.isIosDeployInstalled();

      expect(result).toBe(false);
    });

    it("returns false when ios-deploy command fails", async () => {
      (exec as Mock).mockRejectedValue(new Error("ENOENT"));

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

      // Setup spawn mock for tail -f streaming
      mockSpawn.mockReturnValue(createMockChildProcess());

      // Setup tempFilePath mock to return disposable objects
      (tempFilePath as Mock).mockImplementation(async () => {
        return {
          path: "/tmp/test-file",
          [Symbol.asyncDispose]: vi.fn().mockResolvedValue(undefined),
        };
      });
    });

    it("installs and launches app with correct arguments", async () => {
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
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
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
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
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
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
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
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
      (mockTerminal.execute as Mock).mockImplementation(async (options: any) => {
        if (options.command === "ios-deploy") {
          const error: any = new Error("Command not found");
          error.exitCode = 127;
          throw error;
        }
        return Promise.resolve();
      });

      await expect(
        iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
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
        iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
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
        iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
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
        iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
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
        iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
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
        iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
          deviceId: "00008110-001234567890001E",
          appPath: "/path/to/app.app",
          bundleId: "com.example.app",
        }),
      ).rejects.toThrow("Process terminated");
    });

    it("streams log file using spawn instead of terminal.execute", async () => {
      // streamLogFile now uses child_process.spawn directly instead of terminal.execute
      // so it won't appear as a terminal.execute call for tail
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
        deviceId: "00008110-001234567890001E",
        appPath: "/path/to/app.app",
        bundleId: "com.example.app",
      });

      // Only ios-deploy should be called through terminal.execute, not tail
      expect(iosDeployExecuteCalls).toHaveLength(1);
      const allExecuteCalls = (mockTerminal.execute as Mock).mock.calls;
      const tailCalls = allExecuteCalls.filter((call: any) => call[0]?.command === "tail");
      expect(tailCalls).toHaveLength(0);
    });

    it("handles empty launch arguments array", async () => {
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
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
      await iosDeploy.installAndLaunchApp(mockContext.vscodeContext, mockTerminal, {
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
