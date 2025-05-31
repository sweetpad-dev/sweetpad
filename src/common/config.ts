import * as vscode from "vscode";

type Config = {
  "format.path": string;
  "format.args": string[] | null;
  "format.selectionArgs": string[] | null;
  "build.xcbeautifyEnabled": boolean;
  "build.xcodeWorkspacePath": string;
  "build.derivedDataPath": string;
  "build.configuration": string;
  "build.arch": "x86_64" | "arm64";
  "build.allowProvisioningUpdates": boolean;
  "build.args": string[];
  "build.env": Record<string, string | null>;
  "build.launchArgs": string[];
  "build.launchEnv": { [key: string]: string };
  "build.autoRefreshSchemes": boolean;
  "build.autoRefreshSchemesDelay": number;
  "system.taskExecutor": "v1" | "v2";
  "system.logLevel": "debug" | "info" | "warn" | "error";
  "system.enableSentry": boolean;
  "system.autoRevealTerminal": boolean;
  "system.showProgressStatusBar": boolean;
  "xcodegen.autogenerate": boolean;
  "xcodebuildserver.autogenerate": boolean;
  "tuist.autogenerate": boolean;
  "tuist.generate.env": { [key: string]: string | null };
  "testing.configuration": string;
};

type ConfigKey = keyof Config;

export function getWorkspaceConfig<K extends ConfigKey>(key: K): Config[K] | undefined {
  const config = vscode.workspace.getConfiguration("sweetpad");
  return config.get(key);
}

export function isWorkspaceConfigIsDefined<K extends ConfigKey>(key: K): boolean {
  return getWorkspaceConfig(key) !== undefined;
}

export async function updateWorkspaceConfig<K extends ConfigKey>(key: K, value: Config[K]): Promise<void> {
  const config = vscode.workspace.getConfiguration("sweetpad");
  return await config.update(key, value, vscode.ConfigurationTarget.Workspace);
}
