/**
 * Unit tests for the "sweetpad-lldb" debug configuration provider.
 *
 * The device path has two routes that must not drift into each other: devicectl (iOS 17+,
 * attach by pid) and ios-deploy's debugserver (iOS 16 and below, connect over gdb-remote).
 * These assert the exact LLDB command sequence each one emits.
 */

import type { Mock } from "vitest";
import * as vscode from "vscode";

import type {
  IosDeployDebugserverContext,
  LastLaunchedAppContext,
  LastLaunchedAppDeviceContext,
  WorkspaceStateService,
} from "../common/workspace-state";
import { getRunningProcesses } from "../common/xcode/devicectl";
import { registerDebugConfigurationProvider } from "./provider";

vi.mock("../common/xcode/devicectl", () => ({
  getRunningProcesses: vi.fn(),
}));

vi.mock("../common/logger", () => ({
  commonLogger: {
    log: vi.fn(),
    debug: vi.fn(),
    error: vi.fn(),
  },
}));

/**
 * The provider is only reachable through the registration helper, so grab the dynamic one
 * out of what it hands to vscode.debug and drive it the way the debug session would.
 */
function createProvider(launchContext: LastLaunchedAppContext | undefined) {
  const workspaceState = {
    get: vi.fn((key: string) => (key === "build.lastLaunchedApp" ? launchContext : undefined)),
    update: vi.fn(),
    reset: vi.fn(),
  } as unknown as WorkspaceStateService;

  const vscodeContext = { storageUri: { fsPath: "/tmp/sweetpad-test" } } as unknown as vscode.ExtensionContext;

  const registered: any[] = [];
  (vscode.debug.registerDebugConfigurationProvider as unknown as Mock).mockImplementation(
    (_type: string, provider: any) => {
      registered.push(provider);
      return { dispose: vi.fn() };
    },
  );

  registerDebugConfigurationProvider({ workspaceState, vscodeContext });

  // [initial, dynamic] — the dynamic one is what resolves against the launch context.
  const dynamic = registered[1];
  return {
    resolve: (config: vscode.DebugConfiguration = {} as vscode.DebugConfiguration) =>
      dynamic.resolveDebugConfigurationWithSubstitutedVariables(undefined, config, undefined),
  };
}

const DEVICE_CONTEXT: LastLaunchedAppDeviceContext = {
  type: "device",
  appPath: "/Users/me/Library/Developer/Xcode/DerivedData/MyApp-abc/Build/Products/Debug-iphoneos/MyApp.app",
  appName: "MyApp.app",
  executableName: "MyApp",
  bundleIdentifier: "com.example.MyApp",
  destinationId: "00008110-001234567890001E",
  destinationType: "iOSDevice",
};

const DEBUGSERVER: IosDeployDebugserverContext = {
  port: 12345,
  deviceAppPath: "/private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/MyApp.app",
  symbolsPath: "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
};

describe("DynamicDebugConfigurationProvider", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("device with an ios-deploy debugserver (iOS <= 16)", () => {
    const context: LastLaunchedAppDeviceContext = { ...DEVICE_CONTEXT, debugserver: DEBUGSERVER };

    it("connects over gdb-remote and launches, without consulting devicectl", async () => {
      const config = await createProvider(context).resolve();

      expect(config.processCreateCommands).toEqual(["gdb-remote 127.0.0.1:12345", "process launch"]);
      expect(getRunningProcesses).not.toHaveBeenCalled();
    });

    it("selects the remote-ios platform with the device's symbols as sysroot", async () => {
      const config = await createProvider(context).resolve();

      expect(config.initCommands).toEqual([
        `platform select remote-ios --sysroot "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols"`,
      ]);
    });

    it("falls back to a bare platform select when no symbols path was reported", async () => {
      const config = await createProvider({
        ...context,
        debugserver: { ...DEBUGSERVER, symbolsPath: undefined },
      }).resolve();

      expect(config.initCommands).toEqual(["platform select remote-ios"]);
    });

    it("creates the target from the host bundle and repoints it at the device bundle", async () => {
      const config = await createProvider(context).resolve();

      expect(config.targetCreateCommands).toEqual([
        `target create "${DEVICE_CONTEXT.appPath}"`,
        `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec('${DEBUGSERVER.deviceAppPath}'))`,
      ]);
    });

    it("passes launch arguments through the LLDB launch, not the install tool", async () => {
      const config = await createProvider({
        ...context,
        debugserver: { ...DEBUGSERVER, launchArgs: ["-AppleLanguages", "(de)", "--flag with space"] },
      }).resolve();

      expect(config.processCreateCommands?.[1]).toBe(`process launch -- "-AppleLanguages" "(de)" "--flag with space"`);
    });

    it("sets launch environment as target env-vars", async () => {
      const config = await createProvider({
        ...context,
        debugserver: { ...DEBUGSERVER, launchEnv: { API_HOST: "staging.example.com" } },
      }).resolve();

      expect(config.initCommands).toContain("settings set target.env-vars API_HOST=staging.example.com");
    });

    it("attaches as an lldb session against the host bundle without a pid", async () => {
      const config = await createProvider(context).resolve();

      expect(config.type).toBe("lldb");
      expect(config.request).toBe("attach");
      expect(config.program).toBe(DEVICE_CONTEXT.appPath);
      expect(config.pid).toBeUndefined();
    });

    it("preserves user-supplied commands ahead of the generated ones", async () => {
      const config = await createProvider(context).resolve({
        initCommands: ["command script import ~/custom.py"],
        processCreateCommands: ["script print('before')"],
      } as unknown as vscode.DebugConfiguration);

      expect(config.initCommands?.[0]).toBe("command script import ~/custom.py");
      expect(config.processCreateCommands?.[0]).toBe("script print('before')");
      expect(config.processCreateCommands?.[1]).toBe("gdb-remote 127.0.0.1:12345");
    });

    it("escapes quotes in paths so the LLDB command stays one argument", async () => {
      const config = await createProvider({
        ...context,
        appPath: '/tmp/we"ird/MyApp.app',
        debugserver: { ...DEBUGSERVER, deviceAppPath: "/private/var/it's/MyApp.app" },
      }).resolve();

      expect(config.targetCreateCommands?.[0]).toBe(`target create "/tmp/we\\"ird/MyApp.app"`);
      expect(config.targetCreateCommands?.[1]).toBe(
        `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec('/private/var/it\\'s/MyApp.app'))`,
      );
    });
  });

  describe("device without a debugserver (iOS 17+, devicectl)", () => {
    beforeEach(() => {
      (getRunningProcesses as Mock).mockResolvedValue({
        result: {
          runningProcesses: [
            {
              executable: `file://${DEBUGSERVER.deviceAppPath}/MyApp`,
              processIdentifier: 19350,
            },
          ],
        },
      });
    });

    it("attaches to the running process by pid", async () => {
      const config = await createProvider(DEVICE_CONTEXT).resolve();

      expect(config.pid).toBe("19350");
      expect(config.processCreateCommands).toEqual([
        `script lldb.debugger.HandleCommand("device select ${DEVICE_CONTEXT.destinationId}")`,
        `script lldb.debugger.HandleCommand("device process attach --continue --pid 19350")`,
      ]);
    });

    it("selects the remote-ios platform and keeps the process running after attach", async () => {
      const config = await createProvider(DEVICE_CONTEXT).resolve();

      expect(config.initCommands).toEqual([
        "platform select remote-ios",
        "process handle SIGSTOP -p true -s false -n false",
      ]);
    });

    it("repoints the module at the device bundle via preRunCommands", async () => {
      const config = await createProvider(DEVICE_CONTEXT).resolve();

      expect(config.preRunCommands).toEqual([
        `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec('${DEBUGSERVER.deviceAppPath}'))`,
      ]);
      expect(config.targetCreateCommands).toBeUndefined();
    });

    it("does not emit the gdb-remote connect used by the debugserver route", async () => {
      const config = await createProvider(DEVICE_CONTEXT).resolve();

      expect(JSON.stringify(config)).not.toContain("gdb-remote");
    });
  });

  describe("simulator and macOS", () => {
    it("waits for the process on the simulator", async () => {
      const config = await createProvider({
        type: "simulator",
        appPath: "/path/to/MyApp.app",
        bundleIdentifier: "com.example.MyApp",
        simulatorUdid: "00000000-0000-0000-0000-000000000000",
      }).resolve();

      expect(config).toMatchObject({ type: "lldb", request: "attach", waitFor: true, program: "/path/to/MyApp.app" });
    });

    it("waits for the process on macOS", async () => {
      const config = await createProvider({
        type: "macos",
        appPath: "/path/to/MyApp",
        bundleIdentifier: "com.example.MyApp",
      }).resolve();

      expect(config).toMatchObject({ type: "lldb", request: "attach", waitFor: true, program: "/path/to/MyApp" });
    });
  });

  it("throws when nothing has been launched yet", async () => {
    await expect(createProvider(undefined).resolve()).rejects.toThrow("No last launched app found");
  });
});
