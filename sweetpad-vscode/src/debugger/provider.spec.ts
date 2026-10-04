/**
 * Unit tests for the "sweetpad-lldb" debug configuration provider.
 *
 * The first resolve pass picks the debugger: `sweetpad dap` or CodeLLDB. On the `sweetpad dap`
 * route the configuration becomes a request the CLI reads, filled from SweetPad's selection.
 * On the CodeLLDB route the second pass rewrites it into a CodeLLDB attach, and its device path
 * has two routes that must not drift into each other: devicectl (iOS 17+, attach by pid) and
 * ios-deploy's debugserver (iOS 16 and below, connect over gdb-remote). These assert the exact
 * LLDB command sequence each one emits.
 */

import type { Mock } from "vitest";
import * as vscode from "vscode";

import type { BuildManager } from "../build/manager";
import {
  askConfiguration,
  askDestinationToRunOn,
  askSchemeForBuild,
  askXcodeWorkspacePath,
  getWorkspaceRoot,
} from "../build/utils";
import { ExecutionScopeService } from "../common/execution-scope";
import { QuickPickCancelledError } from "../common/quick-pick";
import type { WorkspaceContextService } from "../common/workspace-context";
import type {
  IosDeployDebugserverContext,
  LastLaunchedAppContext,
  LastLaunchedAppDeviceContext,
  WorkspaceStateService,
} from "../common/workspace-state";
import { getRunningProcessesJson } from "../common/xcode/devicectl";
import type { DestinationsManager } from "../destination/manager";
import type { Destination } from "../destination/types";
import type { ProgressStatusBar } from "../system/status-bar";
import { type SweetpadCliStatus, getSweetpadCliStatus } from "./cli";
import { registerDebugConfigurationProvider } from "./provider";

// Only the spawning entry point is replaced; the addon reads the JSON it returns.
vi.mock("../common/xcode/devicectl", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../common/xcode/devicectl")>()),
  getRunningProcessesJson: vi.fn(),
}));

vi.mock("../common/logger", () => ({
  commonLogger: {
    log: vi.fn(),
    debug: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
  },
}));

vi.mock("./cli", () => ({
  getSweetpadCliStatus: vi.fn(),
}));

vi.mock("../common/tasks/shell-env", () => ({
  getShellEnv: vi.fn(async () => ({ PATH: "/usr/bin:/opt/homebrew/bin", DEVELOPER_DIR: undefined })),
}));

// The pickers are what the "debugging-launch" task uses; here they stand for SweetPad's
// current selection, and a test that expects no prompt asserts they were not called.
vi.mock("../build/utils", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../build/utils")>()),
  askXcodeWorkspacePath: vi.fn(),
  askSchemeForBuild: vi.fn(),
  askConfiguration: vi.fn(),
  askDestinationToRunOn: vi.fn(),
  getWorkspaceRoot: vi.fn(),
}));

const CLI_READY: SweetpadCliStatus = { kind: "ready", path: "/opt/homebrew/bin/sweetpad" };
const CLI_MISSING: SweetpadCliStatus = { kind: "missing", configuredPath: undefined };

const WORKSPACE_ROOT = "/Users/me/MyApp";
const XCWORKSPACE = `${WORKSPACE_ROOT}/MyApp.xcworkspace`;

const SIMULATOR = {
  type: "iOSSimulator",
  udid: "11111111-2222-3333-4444-555555555555",
  name: "iPhone 17",
} as unknown as Destination;

const NEW_DEVICE = {
  type: "iOSDevice",
  udid: "00008110-001234567890001E",
  name: "My iPhone",
  osVersion: "18.1",
  supportsDevicectl: true,
} as unknown as Destination;

const OLD_DEVICE = {
  type: "iOSDevice",
  udid: "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678",
  name: "Old iPhone",
  osVersion: "16.7",
  supportsDevicectl: false,
} as unknown as Destination;

const MAC = { type: "macOS", name: "My Mac", arch: "arm64" } as unknown as Destination;

/** Settings the provider reads, as `vscode.workspace.getConfiguration("sweetpad")` returns them. */
function useSettings(settings: Record<string, unknown>) {
  (vscode.workspace.getConfiguration as Mock).mockImplementation(() => ({
    get: vi.fn((key: string) => settings[key]),
    inspect: vi.fn(),
  }));
}

function useCodelldb(installed: boolean) {
  (vscode.extensions.getExtension as Mock).mockImplementation((id: string) =>
    installed && id === "vadimcn.vscode-lldb" ? { id } : undefined,
  );
}

/** SweetPad's selection: what the pickers answer without prompting. */
function useSelection(selection: { scheme?: string; configuration?: string; destination?: Destination }) {
  (askXcodeWorkspacePath as Mock).mockResolvedValue(XCWORKSPACE);
  (getWorkspaceRoot as Mock).mockReturnValue(WORKSPACE_ROOT);
  (askSchemeForBuild as Mock).mockResolvedValue(selection.scheme ?? "MyApp");
  (askConfiguration as Mock).mockResolvedValue(selection.configuration ?? "Debug");
  (askDestinationToRunOn as Mock).mockResolvedValue(selection.destination ?? SIMULATOR);
}

const FOLDER = { uri: { fsPath: WORKSPACE_ROOT }, name: "MyApp", index: 0 } as unknown as vscode.WorkspaceFolder;

/**
 * The providers are only reachable through the registration helper, so grab what it hands to
 * vscode.debug and drive them the way a debug session would: the first pass of every provider,
 * then the second pass of the dynamic one, which is the only one that has it.
 */
function createProvider(launchContext: LastLaunchedAppContext | undefined) {
  const workspaceState = {
    get: vi.fn((key: string) => (key === "build.lastLaunchedApp" ? launchContext : undefined)),
    update: vi.fn(),
    reset: vi.fn(),
  } as unknown as WorkspaceStateService;

  const vscodeContext = { storageUri: { fsPath: "/tmp/sweetpad-test" } } as unknown as vscode.ExtensionContext;
  const progressStatusBar = { updateText: vi.fn() } as unknown as ProgressStatusBar;

  const registered: any[] = [];
  (vscode.debug.registerDebugConfigurationProvider as unknown as Mock).mockImplementation(
    (_type: string, provider: any) => {
      registered.push(provider);
      return { dispose: vi.fn() };
    },
  );
  let factory: any;
  (vscode.debug.registerDebugAdapterDescriptorFactory as unknown as Mock).mockImplementation(
    (_type: string, registeredFactory: any) => {
      factory = registeredFactory;
      return { dispose: vi.fn() };
    },
  );

  registerDebugConfigurationProvider({
    workspaceState: workspaceState,
    workspaceContext: {} as WorkspaceContextService,
    buildManager: {} as BuildManager,
    destinationsManager: {
      getDestinations: vi.fn(async () => [SIMULATOR, NEW_DEVICE, OLD_DEVICE, MAC]),
    } as unknown as DestinationsManager,
    progressStatusBar: progressStatusBar,
    execution: new ExecutionScopeService(),
    vscodeContext: vscodeContext,
  });

  // [initial, dynamic] — the dynamic one is what resolves against the launch context.
  const [initial, dynamic] = registered;
  const firstPass = async (config: vscode.DebugConfiguration) => {
    const afterInitial = await initial.resolveDebugConfiguration(FOLDER, config, undefined);
    return await dynamic.resolveDebugConfiguration(FOLDER, afterInitial, undefined);
  };
  return {
    workspaceState: workspaceState,
    factory: () => factory,
    resolve: (config: vscode.DebugConfiguration = {} as vscode.DebugConfiguration) =>
      dynamic.resolveDebugConfigurationWithSubstitutedVariables(undefined, config, undefined),
    firstPass: firstPass,
    /** Both passes, as VS Code runs them around the pre-launch task. */
    session: async (config: vscode.DebugConfiguration) => {
      const resolved = await firstPass(config);
      if (resolved === undefined) {
        return undefined;
      }
      return await dynamic.resolveDebugConfigurationWithSubstitutedVariables(FOLDER, resolved, undefined);
    },
  };
}

const ATTACH_WITH_TASK = {
  type: "sweetpad-lldb",
  request: "attach",
  name: "SweetPad: Build and Run (Wait for debugger)",
  preLaunchTask: "sweetpad: debugging-launch",
} as vscode.DebugConfiguration;

const DEVICE_CONTEXT: LastLaunchedAppDeviceContext = {
  type: "device",
  appPath: "/Users/me/Library/Developer/Xcode/DerivedData/MyApp-abc/Build/Products/Debug-iphoneos/MyApp.app",
  appName: "MyApp.app",
  executableName: "MyApp",
  bundleIdentifier: "com.example.MyApp",
  destinationId: "00008110-001234567890001E",
  destinationType: "iOSDevice",
};

/** "devicectl device info processes --json-output" JSON listing these processes. */
function processList(runningProcesses: { executable: string; processIdentifier: number }[]): string {
  return JSON.stringify({ result: { runningProcesses } });
}

const DEBUGSERVER: IosDeployDebugserverContext = {
  port: 12345,
  deviceAppPath: "/private/var/containers/Bundle/Application/C82BF61B-1E77-49F4-B17C-71A0F6520873/MyApp.app",
  symbolsPath: "/Users/me/Library/Developer/Xcode/iOS DeviceSupport/iPad5,1 15.6.1 (19G82)/Symbols",
};

beforeEach(() => {
  vi.clearAllMocks();
  useSettings({});
  useCodelldb(true);
  (vscode.tasks.fetchTasks as Mock).mockResolvedValue([]);
});

describe("DynamicDebugConfigurationProvider", () => {
  describe("device with an ios-deploy debugserver (iOS <= 16)", () => {
    const context: LastLaunchedAppDeviceContext = { ...DEVICE_CONTEXT, debugserver: DEBUGSERVER };

    it("connects over gdb-remote and launches, without consulting devicectl", async () => {
      const config = await createProvider(context).resolve();

      expect(config.processCreateCommands).toEqual(["gdb-remote 127.0.0.1:12345", "process launch"]);
      expect(getRunningProcessesJson).not.toHaveBeenCalled();
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
      (getRunningProcessesJson as Mock).mockResolvedValue(
        processList([{ executable: `file://${DEBUGSERVER.deviceAppPath}/MyApp`, processIdentifier: 19350 }]),
      );
    });

    it("attaches to the app it launched, not one whose name ends the same way", async () => {
      // "App.app" is a suffix of "MyApp.app", and a widget extension runs out of the
      // app's own bundle; both are listed ahead of the app's executable.
      const bundles = "file:///private/var/containers/Bundle/Application";
      (getRunningProcessesJson as Mock).mockResolvedValue(
        processList([
          { executable: `${bundles}/AAAA/MyApp.app/MyApp`, processIdentifier: 100 },
          { executable: `${bundles}/BBBB/App.app/PlugIns/Widget.appex/Widget`, processIdentifier: 200 },
          { executable: `${bundles}/BBBB/App.app/App`, processIdentifier: 300 },
        ]),
      );

      const config = await createProvider({ ...DEVICE_CONTEXT, appName: "App.app", executableName: "App" }).resolve();

      expect(config.pid).toBe("300");
      expect(config.preRunCommands).toEqual([
        `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec('/private/var/containers/Bundle/Application/BBBB/App.app'))`,
      ]);
    });

    it("finds an app whose bundle name has a space, which the executable URL encodes", async () => {
      (getRunningProcessesJson as Mock).mockResolvedValue(
        processList([
          {
            executable: "file:///private/var/containers/Bundle/Application/CCCC/My%20App.app/My%20App",
            processIdentifier: 400,
          },
        ]),
      );

      const config = await createProvider({
        ...DEVICE_CONTEXT,
        appName: "My App.app",
        executableName: "My App",
      }).resolve();

      expect(config.pid).toBe("400");
      expect(config.preRunCommands).toEqual([
        `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec('/private/var/containers/Bundle/Application/CCCC/My App.app'))`,
      ]);
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

describe("route selection", () => {
  it("serves the session through sweetpad dap when the CLI has it", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_READY);
    useSelection({});

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK });

    expect(config).toMatchObject({ type: "sweetpad-lldb", request: "launch" });
    expect(config.preLaunchTask).toBeUndefined();
  });

  it("falls back to CodeLLDB unchanged when the CLI is missing", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_MISSING);

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toEqual(ATTACH_WITH_TASK);
    expect(askXcodeWorkspacePath).not.toHaveBeenCalled();
  });

  it("falls back to CodeLLDB when the CLI is too old for dap", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue({ kind: "no-dap", path: "/usr/local/bin/sweetpad" });

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toEqual(ATTACH_WITH_TASK);
  });

  it("keeps a device without devicectl on CodeLLDB, after the device is picked", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_READY);
    useSelection({ destination: OLD_DEVICE });

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toEqual(ATTACH_WITH_TASK);
    expect(askDestinationToRunOn).toHaveBeenCalledTimes(1);
  });

  it("cancels the session and explains both options when neither is installed", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_MISSING);
    useCodelldb(false);

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toBeUndefined();
    const [message, ...buttons] = (vscode.window.showErrorMessage as Mock).mock.calls[0];
    expect(message).toContain("SweetPad CLI");
    expect(message).toContain("CodeLLDB");
    expect(buttons).toEqual(["Install SweetPad CLI", "Install CodeLLDB", "Close"]);
  });

  it("cancels the session when an old device needs CodeLLDB and it is missing", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_READY);
    useCodelldb(false);
    useSelection({ destination: OLD_DEVICE });

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toBeUndefined();
    expect((vscode.window.showErrorMessage as Mock).mock.calls[0][0]).toContain("Old iPhone (16.7)");
  });

  it("does not look for the CLI when the setting picks CodeLLDB", async () => {
    useSettings({ "debugger.adapter": "codelldb" });

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toEqual(ATTACH_WITH_TASK);
    expect(getSweetpadCliStatus).not.toHaveBeenCalled();
  });

  it("fails instead of falling back when the setting picks the CLI and it is missing", async () => {
    useSettings({ "debugger.adapter": "sweetpad" });
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_MISSING);

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toBeUndefined();
    expect((vscode.window.showErrorMessage as Mock).mock.calls[0][0]).toContain("sweetpad.debugger.adapter");
  });

  it("cancels quietly when a picker is dismissed", async () => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_READY);
    useSelection({});
    (askSchemeForBuild as Mock).mockRejectedValue(new QuickPickCancelledError());

    const config = await createProvider(undefined).firstPass({ ...ATTACH_WITH_TASK });

    expect(config).toBeUndefined();
    expect(vscode.window.showErrorMessage).not.toHaveBeenCalled();
  });
});

describe("sweetpad dap route", () => {
  beforeEach(() => {
    (getSweetpadCliStatus as Mock).mockResolvedValue(CLI_READY);
  });

  it("turns the build-and-attach config into a launch filled from SweetPad's selection", async () => {
    useSelection({ scheme: "MyApp", configuration: "Debug", destination: SIMULATOR });

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK });

    expect(config).toEqual({
      type: "sweetpad-lldb",
      request: "launch",
      name: "SweetPad: Build and Run (Wait for debugger)",
      workspace: XCWORKSPACE,
      cwd: WORKSPACE_ROOT,
      scheme: "MyApp",
      configuration: "Debug",
      destination: "11111111-2222-3333-4444-555555555555",
      xcodebuildArgs: ["-allowProvisioningUpdates"],
    });
  });

  it("never reads the last launched app", async () => {
    useSelection({});
    const provider = createProvider(undefined);

    await expect(provider.session({ ...ATTACH_WITH_TASK })).resolves.toBeDefined();
    expect(provider.workspaceState.get).not.toHaveBeenCalledWith("build.lastLaunchedApp");
  });

  it("builds from an empty configuration, as F5 without a launch.json sends", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({} as vscode.DebugConfiguration);

    expect(config).toMatchObject({ type: "sweetpad-lldb", request: "launch", scheme: "MyApp" });
  });

  it("names a macOS destination as mac and a device by its UDID", async () => {
    useSelection({ destination: MAC });
    expect((await createProvider(undefined).session({ ...ATTACH_WITH_TASK })).destination).toBe("mac");

    useSelection({ destination: NEW_DEVICE });
    expect((await createProvider(undefined).session({ ...ATTACH_WITH_TASK })).destination).toBe(
      "00008110-001234567890001E",
    );
  });

  it("keeps the values written in launch.json over SweetPad's selection", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({
      ...ATTACH_WITH_TASK,
      scheme: "Other",
      configuration: "Release",
      destination: "booted",
      args: ["-launch.json"],
      env: { FROM: "launch.json" },
      xcodebuildArgs: ["-quiet"],
    });

    expect(config).toMatchObject({
      scheme: "Other",
      configuration: "Release",
      destination: "booted",
      args: ["-launch.json"],
      env: { FROM: "launch.json" },
      xcodebuildArgs: ["-quiet"],
    });
    expect(askSchemeForBuild).not.toHaveBeenCalled();
    expect(askConfiguration).not.toHaveBeenCalled();
    expect(askDestinationToRunOn).not.toHaveBeenCalled();
  });

  it("leaves the container to a cwd written in launch.json", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK, cwd: "/elsewhere" });

    expect(config.cwd).toBe("/elsewhere");
    expect(config.workspace).toBeUndefined();
    expect(config.project).toBeUndefined();
  });

  it("names a project's embedded workspace by its project", async () => {
    useSelection({});
    (askXcodeWorkspacePath as Mock).mockResolvedValue(`${WORKSPACE_ROOT}/MyApp.xcodeproj/project.xcworkspace`);

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK });

    expect(config.project).toBe(`${WORKSPACE_ROOT}/MyApp.xcodeproj`);
    expect(config.workspace).toBeUndefined();
  });

  it("resolves a Swift package from its own directory", async () => {
    useSelection({});
    (askXcodeWorkspacePath as Mock).mockResolvedValue(`${WORKSPACE_ROOT}/Packages/Kit/Package.swift`);

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK });

    expect(config.cwd).toBe(`${WORKSPACE_ROOT}/Packages/Kit`);
    expect(config.workspace).toBeUndefined();
    expect(config.project).toBeUndefined();
  });

  it("passes the extension's launch and build settings", async () => {
    useSelection({});
    useSettings({
      "build.launchArgs": ["-FromSettings"],
      "build.launchEnv": { API: "staging" },
      "build.args": ["-quiet", "-derivedDataPath", "/dd/old", "-derivedDataPath", "/dd/new"],
      "build.allowProvisioningUpdates": false,
    });

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK });

    expect(config.args).toEqual(["-FromSettings"]);
    expect(config.env).toEqual({ API: "staging" });
    expect(config.xcodebuildArgs).toEqual(["-quiet", "-derivedDataPath", "/dd/new"]);
  });

  it("drops codelldbAttributes and passes lldb through", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({
      ...ATTACH_WITH_TASK,
      codelldbAttributes: { initCommands: ["codelldb only"] },
      lldb: { initCommands: ["lldb-dap"], sourceMap: [["/build", "/src"]] },
    });

    expect(config.codelldbAttributes).toBeUndefined();
    expect(config.initCommands).toBeUndefined();
    expect(config.lldb).toEqual({ initCommands: ["lldb-dap"], sourceMap: [["/build", "/src"]] });
  });

  it("keeps an attach without the pre-launch task as an attach to the running app", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({
      type: "sweetpad-lldb",
      request: "attach",
      name: "Attach",
    });

    expect(config).toMatchObject({
      request: "attach",
      scheme: "MyApp",
      destination: "11111111-2222-3333-4444-555555555555",
    });
  });

  it("attaches by pid alone without asking for a scheme or destination", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({
      type: "sweetpad-lldb",
      request: "attach",
      name: "Attach to pid",
      pid: 4242,
    });

    expect(config).toEqual({
      type: "sweetpad-lldb",
      request: "attach",
      name: "Attach to pid",
      pid: 4242,
      cwd: WORKSPACE_ROOT,
    });
    expect(askXcodeWorkspacePath).not.toHaveBeenCalled();
  });

  it("forwards a plain lldb-dap config untouched", async () => {
    const config = await createProvider(undefined).session({
      type: "sweetpad-lldb",
      request: "launch",
      name: "Tool",
      program: "/path/to/tool",
      cwd: "/work",
    });

    expect(config).toEqual({
      type: "sweetpad-lldb",
      request: "launch",
      name: "Tool",
      program: "/path/to/tool",
      cwd: "/work",
    });
    expect(askXcodeWorkspacePath).not.toHaveBeenCalled();
  });

  it("recognizes a tasks.json copy of the debugging-launch task and keeps its fields", async () => {
    useSelection({});
    (vscode.tasks.fetchTasks as Mock).mockResolvedValue([
      { name: "debugging-launch", source: "sweetpad", definition: { type: "sweetpad", action: "debugging-launch" } },
      {
        name: "Debug MyApp",
        source: "Workspace",
        definition: {
          type: "sweetpad",
          action: "debugging-launch",
          scheme: "FromTask",
          destinationId: "00008110-001234567890001E",
          launchArgs: ["-FromTask"],
        },
      },
    ]);

    const config = await createProvider(undefined).session({ ...ATTACH_WITH_TASK, preLaunchTask: "Debug MyApp" });

    expect(config).toMatchObject({
      request: "launch",
      scheme: "FromTask",
      destination: "00008110-001234567890001E",
      args: ["-FromTask"],
    });
    expect(config.preLaunchTask).toBeUndefined();
    expect(askSchemeForBuild).not.toHaveBeenCalled();
    expect(askDestinationToRunOn).not.toHaveBeenCalled();
  });

  it("keeps a pre-launch task that is not the debugging-launch task", async () => {
    useSelection({});

    const config = await createProvider(undefined).session({
      type: "sweetpad-lldb",
      request: "launch",
      name: "Generate, then debug",
      preLaunchTask: "generate code",
    });

    expect(config).toMatchObject({ request: "launch", preLaunchTask: "generate code", scheme: "MyApp" });
  });

  it("starts sweetpad dap in the workspace folder with the login shell's environment", async () => {
    const provider = createProvider(undefined);

    const descriptor = await provider
      .factory()
      .createDebugAdapterDescriptor({ workspaceFolder: FOLDER, configuration: { cwd: "/other" } }, undefined);

    expect(descriptor).toBeInstanceOf(vscode.DebugAdapterExecutable);
    expect(descriptor.command).toBe("/opt/homebrew/bin/sweetpad");
    expect(descriptor.args).toEqual(["dap"]);
    expect(descriptor.options).toEqual({ cwd: WORKSPACE_ROOT, env: { PATH: "/usr/bin:/opt/homebrew/bin" } });
  });
});

describe("CodeLLDB route", () => {
  beforeEach(() => {
    useSettings({ "debugger.adapter": "codelldb" });
  });

  it("gives a launch without a pre-launch task the debugging-launch task", async () => {
    const config = await createProvider(undefined).firstPass({
      type: "sweetpad-lldb",
      request: "launch",
      name: "SweetPad: Build and Run",
    });

    expect(config).toEqual({
      type: "sweetpad-lldb",
      request: "launch",
      name: "SweetPad: Build and Run",
      preLaunchTask: "sweetpad: debugging-launch",
    });
  });

  it("leaves an attach without a pre-launch task alone", async () => {
    const attach = { type: "sweetpad-lldb", request: "attach", name: "Attach" };

    expect(await createProvider(undefined).firstPass({ ...attach })).toEqual(attach);
  });

  it("rewrites the session into a CodeLLDB attach after the pre-launch task", async () => {
    const config = await createProvider({
      type: "simulator",
      appPath: "/path/to/MyApp.app",
      bundleIdentifier: "com.example.MyApp",
      simulatorUdid: "00000000-0000-0000-0000-000000000000",
    }).session({ ...ATTACH_WITH_TASK, codelldbAttributes: { stopOnEntry: true } });

    expect(config).toMatchObject({
      type: "lldb",
      request: "attach",
      waitFor: true,
      program: "/path/to/MyApp.app",
      preLaunchTask: "sweetpad: debugging-launch",
      stopOnEntry: true,
    });
  });
});
