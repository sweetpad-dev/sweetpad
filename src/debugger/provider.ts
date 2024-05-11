import vscode from "vscode";
import { ExtensionContext } from "../common/commands";

const ATTACH_CONFIG: vscode.DebugConfiguration = {
  type: "lldb",
  request: "attach",
  name: "Attach to iOS Simulator (SweetPad)",
  waitFor: true,
  program: "${command:sweetpad.debugger.getAppPath}",
};

export class DebuggerConfigurationProvider implements vscode.DebugConfigurationProvider {
  async provideDebugConfigurations(
    folder: vscode.WorkspaceFolder | undefined,
    token?: vscode.CancellationToken | undefined
  ): Promise<vscode.DebugConfiguration[]> {
    return [ATTACH_CONFIG];
  }

  async resolveDebugConfiguration(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined
  ): Promise<vscode.DebugConfiguration> {
    // currently doing nothing useful here, but leave it for future extension
    return config;
  }

  async resolveDebugConfigurationWithSubstitutedVariables(
    folder: vscode.WorkspaceFolder | undefined,
    config: vscode.DebugConfiguration,
    token?: vscode.CancellationToken | undefined
  ): Promise<vscode.DebugConfiguration> {
    // currently doing nothing useful here, but leave it for future extension
    return config;
  }
}

export function registerDebugConfigurationProvider(context: ExtensionContext) {
  vscode.debug.registerDebugConfigurationProvider(
    "lldb",
    new DebuggerConfigurationProvider(),
    vscode.DebugConfigurationProviderTriggerKind.Initial
  );
  return vscode.debug.registerDebugConfigurationProvider(
    "lldb",
    new DebuggerConfigurationProvider(),
    vscode.DebugConfigurationProviderTriggerKind.Dynamic
  );
}
