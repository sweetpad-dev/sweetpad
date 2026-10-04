import vscode from "vscode";

import { CommandExecutionScope, showCommandErrorMessage } from "../common/commands";
import type {
  LastLaunchedAppDeviceContext,
  LastLaunchedAppMacOSContext,
  LastLaunchedAppSimulatorContext,
} from "../common/commands";
import { ExtensionError } from "../common/errors";
import type { ExecutionScopeService } from "../common/execution-scope";
import { commonLogger } from "../common/logger";
import { QuickPickCancelledError } from "../common/quick-pick";
import { getShellEnv } from "../common/tasks/shell-env";
import { checkUnreachable } from "../common/types";
import type { IosDeployDebugserverContext } from "../common/workspace-state";
import { getSweetpadCliStatus } from "./cli";
import {
  DEBUGGING_LAUNCH_TASK_LABEL,
  type DapSelection,
  type DapSelectionDeps,
  buildDapConfig,
  findDebuggingLaunchTask,
  needsDapSelection,
  resolveDapSelection,
} from "./dap-config";
import {
  chooseDebugRoute,
  deviceNeedingCodelldb,
  getDebuggerAdapterSetting,
  isCodelldbInstalled,
  unavailableRouteError,
} from "./route";
import { quoteLldbArgument, quotePythonString, waitForProcessToLaunch } from "./utils";

const LAUNCH_CONFIG: vscode.DebugConfiguration = {
  type: "sweetpad-lldb",
  request: "launch",
  name: "SweetPad: Build and Run",
};

/**
 * Set on a configuration that `sweetpad dap` serves, so the second resolve pass leaves it
 * alone. The CLI ignores fields it doesn't read.
 */
const ROUTE_FIELD = "__sweetpadRoute";

class InitialDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  async provideDebugConfigurations(
    folder: vscode.WorkspaceFolder | undefined,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration[]> {
    return [{ ...LAUNCH_CONFIG }];
  }

  async resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration | undefined> {
    if (Object.keys(config).length === 0) {
      return { ...LAUNCH_CONFIG };
    }
    return config;
  }
}

type DynamicProviderDeps = DapSelectionDeps & {
  execution: ExecutionScopeService;
  vscodeContext: vscode.ExtensionContext;
};

class DynamicDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  private deps: DynamicProviderDeps;

  constructor(deps: DynamicProviderDeps) {
    this.deps = deps;
  }

  async provideDebugConfigurations(
    folder: vscode.WorkspaceFolder | undefined,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration[]> {
    return [{ ...LAUNCH_CONFIG }];
  }

  /**
   * Pick the debugger for the session and shape the configuration for it. This pass runs
   * before the pre-launch task, which is what lets the `sweetpad dap` route drop the
   * "debugging-launch" task. A failure is shown with the buttons that fix it, and the session
   * is cancelled.
   */
  async resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration | undefined> {
    const initial = Object.keys(config).length === 0 ? { ...LAUNCH_CONFIG } : config;
    try {
      return await this.resolveRoute(folder, initial);
    } catch (error) {
      if (error instanceof QuickPickCancelledError) {
        return undefined;
      }
      if (error instanceof ExtensionError) {
        commonLogger.error(error.message, { errorContext: error.options?.context, error: error });
        void showCommandErrorMessage(`SweetPad: ${error.message}`, { actions: error.options?.actions });
        return undefined;
      }
      throw error;
    }
  }

  private async resolveRoute(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
  ): Promise<vscode.DebugConfiguration> {
    const adapter = getDebuggerAdapterSetting();
    const cli = adapter === "codelldb" ? undefined : await getSweetpadCliStatus();

    // The task and the selection matter only when the CLI can take the session. The selection
    // may open a picker, and the CodeLLDB route leaves picking to the "debugging-launch" task.
    const task = cli?.kind === "ready" ? await findDebuggingLaunchTask(config.preLaunchTask) : undefined;
    let selection: DapSelection | undefined;
    if (cli?.kind === "ready" && needsDapSelection(config, task)) {
      const scope = new CommandExecutionScope({ commandName: "sweetpad.debugger.resolveConfiguration" });
      selection = await this.deps.execution.startScope(scope, () => resolveDapSelection(this.deps, config, task));
    }

    const route = chooseDebugRoute({
      adapter: adapter,
      cli: cli,
      codelldbInstalled: isCodelldbInstalled(),
      deviceNeedingCodelldb: selection?.target ? deviceNeedingCodelldb(selection.target) : undefined,
    });
    commonLogger.log("Resolved debug route", { adapter, route });

    switch (route.kind) {
      case "sweetpad": {
        const resolved = buildDapConfig(config, { task: task, selection: selection, folder: folder });
        resolved[ROUTE_FIELD] = "sweetpad";
        return resolved;
      }
      case "codelldb":
        return this.resolveCodelldbRoute(config);
      case "unavailable":
        throw unavailableRouteError(route.reason);
    }
  }

  /**
   * The CodeLLDB route keeps the configuration as written, apart from a `launch` with no
   * pre-launch task, which gets the "debugging-launch" task so it builds and runs the app as it
   * does on the `sweetpad dap` route.
   */
  private resolveCodelldbRoute(config: vscode.DebugConfiguration): vscode.DebugConfiguration {
    if (config.request === "launch" && config.preLaunchTask === undefined) {
      return { ...config, preLaunchTask: DEBUGGING_LAUNCH_TASK_LABEL };
    }
    return config;
  }

  private async resolveMacOSDebugConfiguration(
    config: vscode.DebugConfiguration,
    launchContext: LastLaunchedAppMacOSContext,
  ): Promise<vscode.DebugConfiguration> {
    config.type = "lldb";
    config.waitFor = true;
    config.request = "attach";
    config.program = launchContext.appPath;
    commonLogger.log("Resolved debug configuration", { config: config });
    return config;
  }

  private async resolveSimulatorDebugConfiguration(
    config: vscode.DebugConfiguration,
    launchContext: LastLaunchedAppSimulatorContext,
  ): Promise<vscode.DebugConfiguration> {
    config.type = "lldb";
    config.waitFor = true;
    config.request = "attach";
    config.program = launchContext.appPath;
    commonLogger.log("Resolved debug configuration", { config: config });
    return config;
  }

  /**
   * Devices without CoreDevice (iOS 16 and below), where ios-deploy has installed the app and
   * left a debugserver listening on localhost.
   *
   * LLDB launches the process itself here rather than attaching to a running one, so
   * breakpoints in startup code are already installed when the app begins executing — the
   * devicectl route below can only attach after launch.
   */
  private async resolveDebugserverDeviceConfiguration(
    config: vscode.DebugConfiguration,
    launchContext: LastLaunchedAppDeviceContext,
    debugserver: IosDeployDebugserverContext,
  ): Promise<vscode.DebugConfiguration> {
    const hostAppPath = launchContext.appPath;

    // Without the sysroot LLDB resolves system frames against the host, so anything outside
    // the app's own binary symbolicates wrong. ios-deploy reports the path Xcode extracted
    // when the device was first paired.
    const platformSelect = debugserver.symbolsPath
      ? `platform select remote-ios --sysroot ${quoteLldbArgument(debugserver.symbolsPath)}`
      : "platform select remote-ios";

    // LLDB owns the launch on this route, so args and env are delivered through it rather
    // than through the tool that installed the app.
    const envCommands = Object.entries(debugserver.launchEnv ?? {}).map(
      ([key, value]) => `settings set target.env-vars ${key}=${value}`,
    );
    const launchArgs = debugserver.launchArgs ?? [];
    const launchCommand = launchArgs.length
      ? `process launch -- ${launchArgs.map(quoteLldbArgument).join(" ")}`
      : "process launch";

    config.initCommands = [...(config.initCommands || []), platformSelect, ...envCommands];

    config.targetCreateCommands = [
      ...(config.targetCreateCommands || []),
      `target create ${quoteLldbArgument(hostAppPath)}`,
      // Points the loaded module at where the bundle actually lives on the device, so
      // breakpoints resolve against the remote binary.
      `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec(${quotePythonString(debugserver.deviceAppPath)}))`,
    ];

    config.processCreateCommands = [
      ...(config.processCreateCommands || []),
      `gdb-remote 127.0.0.1:${debugserver.port}`,
      launchCommand,
    ];

    config.postRunCommands = [...(config.postRunCommands || []), `script print("SweetPad: Happy debugging!")`];

    config.type = "lldb";
    config.request = "attach";
    config.program = hostAppPath;

    commonLogger.log("Resolved debug configuration", { config: config });
    return config;
  }

  private async resolveDeviceDebugConfiguration(
    config: vscode.DebugConfiguration,
    launchContext: LastLaunchedAppDeviceContext,
  ): Promise<vscode.DebugConfiguration> {
    if (launchContext.debugserver) {
      return await this.resolveDebugserverDeviceConfiguration(config, launchContext, launchContext.debugserver);
    }

    const deviceUDID = launchContext.destinationId;
    const hostAppPath = launchContext.appPath;
    const appName = launchContext.appName; // Example: "MyApp.app"

    // We need to find the device app path and the process id
    const process = await waitForProcessToLaunch(this.deps.vscodeContext, {
      deviceId: deviceUDID,
      appName: appName,
      timeoutMs: 15000, // wait for 15 seconds before giving up
    });

    // The bundle's decoded path on the device, e.g.
    // "/private/var/containers/Bundle/Application/5045C7CE-DFB9-4C17-BBA9-94D8BCD8F565/Mastodon.app"
    const deviceAppPath = process.appPath;
    const processId = process.pid;

    const continueOnAttach = config.continueOnAttach ?? true;

    // LLDB commands executed upon debugger startup.
    config.initCommands = [
      ...(config.initCommands || []),
      // By default, LLDB runs against the local host platform. This command switches LLDB to a remote
      // iOS environment, necessary for debugging iOS apps on a device.
      "platform select remote-ios",
      // Don't stop after attaching to the process:
      // -n false — Should LLDB print a “stopped with SIGSTOP” message in the UI? Be silent—no notification to you
      // -p true — Should LLDB forward the signal on to your app? Deliver SIGSTOP to the process
      // -s false — Should LLDB pause (break into the debugger) when this signal arrives?  Don’t break; just run LLDB’s signal handler logic
      ...(continueOnAttach ? ["process handle SIGSTOP -p true -s false -n false"] : []),
    ];

    // LLDB commands executed just before launching of attaching to the debuggee.
    config.preRunCommands = [
      ...(config.preRunCommands || []),
      // Adjusts the loaded module’s file specification to point to the actual location of the binary on the remote device.
      // This ensures symbol resolution and breakpoints align correctly with the actual remote binary.
      `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec(${quotePythonString(deviceAppPath)}))`,
    ];

    // LLDB commands executed to create/attach the debuggee process.
    config.processCreateCommands = [
      ...(config.processCreateCommands || []),
      // Tells LLDB which physical iOS device (by UDID) you want to attach to.
      `script lldb.debugger.HandleCommand("device select ${deviceUDID}")`,
      // Attaches LLDB to the already-launched process on that device.
      `script lldb.debugger.HandleCommand("device process attach --continue --pid ${processId}")`,
    ];

    // LLDB commands executed after the debuggee process has been created/attached.
    config.postRunCommands = [...(config.postRunCommands || []), `script print("SweetPad: Happy debugging!")`];

    config.type = "lldb";
    config.request = "attach";
    config.program = hostAppPath;
    config.pid = processId.toString();

    commonLogger.log("Resolved debug configuration", { config: config });
    return config;
  }

  /*
   * We use this method because it runs after "preLaunchTask" is completed, "resolveDebugConfiguration"
   * runs before "preLaunchTask" so it's not suitable for our use case without some hacks.
   */
  async resolveDebugConfigurationWithSubstitutedVariables(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration> {
    // `sweetpad dap` builds and launches the app itself, so there is no launched app to read.
    if (config[ROUTE_FIELD] === "sweetpad") {
      const { [ROUTE_FIELD]: _route, ...rest } = config;
      return rest as vscode.DebugConfiguration;
    }

    const launchContext = this.deps.workspaceState.get("build.lastLaunchedApp");
    if (!launchContext) {
      throw new Error("No last launched app found, please launch the app first using the SweetPad extension");
    }

    // Pass the "codelldbAttributes" to the lldb debugger
    const codelldbAttributes = config.codelldbAttributes || {};
    for (const [key, value] of Object.entries(codelldbAttributes)) {
      config[key] = value;
    }
    config.codelldbAttributes = undefined;

    if (launchContext.type === "macos") {
      return await this.resolveMacOSDebugConfiguration(config, launchContext);
    }

    if (launchContext.type === "simulator") {
      return await this.resolveSimulatorDebugConfiguration(config, launchContext);
    }

    if (launchContext.type === "device") {
      return await this.resolveDeviceDebugConfiguration(config, launchContext);
    }

    checkUnreachable(launchContext);
    return config;
  }

  dispose(): void {
    // LogStreamManager is a singleton, disposed separately
  }
}

/**
 * Starts `sweetpad dap` for the sessions the first resolve pass gave to the CLI. CodeLLDB
 * sessions leave that pass as type "lldb" and never reach this factory.
 */
class SweetpadDebugAdapterFactory implements vscode.DebugAdapterDescriptorFactory {
  async createDebugAdapterDescriptor(
    session: vscode.DebugSession,
    executable: vscode.DebugAdapterExecutable | undefined,
  ): Promise<vscode.DebugAdapterDescriptor> {
    const cli = await getSweetpadCliStatus();
    if (cli.kind !== "ready") {
      throw new Error("SweetPad: No SweetPad CLI with 'sweetpad dap' was found. Start the debug session again.");
    }
    const configuredCwd = session.configuration.cwd;
    const cwd = session.workspaceFolder?.uri.fsPath ?? (typeof configuredCwd === "string" ? configuredCwd : undefined);

    // The login shell's environment, as every tool the extension runs gets: a VS Code started
    // from the Dock has a short PATH and no DEVELOPER_DIR from the dotfiles.
    const env: { [key: string]: string } = {};
    for (const [key, value] of Object.entries(await getShellEnv(cwd ?? null))) {
      if (value !== undefined) {
        env[key] = value;
      }
    }

    commonLogger.log("Starting sweetpad dap", { cliPath: cli.path, cwd });
    return new vscode.DebugAdapterExecutable(cli.path, ["dap"], { cwd: cwd, env: env });
  }
}

export function registerDebugConfigurationProvider(options: DynamicProviderDeps) {
  const dynamicProvider = new DynamicDebugConfigurationProvider(options);
  const initialProvider = new InitialDebugConfigurationProvider();
  const disposable1 = vscode.debug.registerDebugConfigurationProvider(
    "sweetpad-lldb",
    initialProvider,
    vscode.DebugConfigurationProviderTriggerKind.Initial,
  );
  const disposable2 = vscode.debug.registerDebugConfigurationProvider(
    "sweetpad-lldb",
    dynamicProvider,
    vscode.DebugConfigurationProviderTriggerKind.Dynamic,
  );
  const disposable3 = vscode.debug.registerDebugAdapterDescriptorFactory(
    "sweetpad-lldb",
    new SweetpadDebugAdapterFactory(),
  );

  return {
    dispose() {
      disposable1.dispose();
      disposable2.dispose();
      disposable3.dispose();
      dynamicProvider.dispose();
    },
  };
}
