import vscode from "vscode";
import type {
  ExtensionContext,
  LastLaunchedAppDeviceContext,
  LastLaunchedAppMacOSContext,
  LastLaunchedAppSimulatorContext,
} from "../common/context";
import { commonLogger } from "../common/logger";
import { checkUnreachable } from "../common/types";
import { waitForProcessToLaunch } from "./utils";

const ATTACH_CONFIG: vscode.DebugConfiguration = {
  type: "sweetpad-lldb",
  request: "attach",
  name: "SweetPad: Build and Run (Wait for debugger)",
  preLaunchTask: "sweetpad: debugging-launch",
};

class InitialDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  async provideDebugConfigurations(
    folder: vscode.WorkspaceFolder | undefined,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration[]> {
    return [ATTACH_CONFIG];
  }

  async resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration | undefined> {
    if (Object.keys(config).length === 0) {
      return ATTACH_CONFIG;
    }
    return config;
  }
}

class DynamicDebugConfigurationProvider implements vscode.DebugConfigurationProvider {
  context: ExtensionContext;
  constructor(options: { context: ExtensionContext }) {
    this.context = options.context;
  }

  async provideDebugConfigurations(
    folder: vscode.WorkspaceFolder | undefined,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration[]> {
    return [ATTACH_CONFIG];
  }

  async resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration | undefined> {
    if (Object.keys(config).length === 0) {
      return ATTACH_CONFIG;
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

  private async resolveDeviceDebugConfiguration(
    config: vscode.DebugConfiguration,
    launchContext: LastLaunchedAppDeviceContext,
  ): Promise<vscode.DebugConfiguration> {
    const deviceUDID = launchContext.destinationId;
    const hostAppPath = launchContext.appPath;
    const appName = launchContext.appName; // Example: "MyApp.app"

    // We need to find the device app path and the process id
    const process = await waitForProcessToLaunch(this.context, {
      deviceId: deviceUDID,
      appName: appName,
      timeoutMs: 15000, // wait for 15 seconds before giving up
    });

    const deviceExecutableURL = process.executable;
    if (!deviceExecutableURL) {
      throw new Error("No device app path found");
    }

    // Remove the "file://" prefix and remove everything after the app name
    // Result should be something like:
    //  - "/private/var/containers/Bundle/Application/5045C7CE-DFB9-4C17-BBA9-94D8BCD8F565/Mastodon.app"
    const deviceAppPath = deviceExecutableURL.match(/^file:\/\/(.*\.app)/)?.[1];
    const processId = process.processIdentifier;

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
      `script lldb.target.module[0].SetPlatformFileSpec(lldb.SBFileSpec('${deviceAppPath}'))`,
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
    const launchContext = this.context.getWorkspaceState("build.lastLaunchedApp");
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
}

export function registerDebugConfigurationProvider(context: ExtensionContext) {
  const dynamicProvider = new DynamicDebugConfigurationProvider({ context });
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

  return {
    dispose() {
      disposable1.dispose();
      disposable2.dispose();
    },
  };
}
