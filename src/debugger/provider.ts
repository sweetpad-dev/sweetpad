import vscode from "vscode";
import { ExtensionContext } from "../common/commands";

const ATTACH_CONFIG: vscode.DebugConfiguration = {
  type: "sweetpad-lldb",
  request: "launch",
  name: "Attach to iOS app (SweetPad)",
  preLaunchTask: "sweetpad: launch",
};

class DebuggerConfigurationProvider implements vscode.DebugConfigurationProvider {
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
  ): Promise<vscode.DebugConfiguration> {
    if (Object.keys(config).length === 0) {
      return ATTACH_CONFIG;
    }
    return config;
  }

  async resolveDebugConfigurationWithSubstitutedVariables(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined,
  ): Promise<vscode.DebugConfiguration> {
    config.type = "lldb";
    config.waitFor = true;
    config.request = "attach";
    const appPath = this.context.getWorkspaceState("build.lastLaunchedAppPath");
    if (!appPath) {
      throw new Error("No last launched app path found, please launch the app first using the extension");
    }
    config.program = appPath;

    return config;
  }
}

export function registerDebugConfigurationProvider(context: ExtensionContext) {
  const provider = new DebuggerConfigurationProvider({ context });
  const disposable1 = vscode.debug.registerDebugConfigurationProvider(
    "sweetpad-lldb",
    provider,
    vscode.DebugConfigurationProviderTriggerKind.Initial,
  );
  const disposable2 = vscode.debug.registerDebugConfigurationProvider(
    "sweetpad-lldb",
    provider,
    vscode.DebugConfigurationProviderTriggerKind.Dynamic,
  );

  return {
    dispose() {
      disposable1.dispose();
      disposable2.dispose();
    },
  };
}
