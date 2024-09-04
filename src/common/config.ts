import * as vscode from "vscode";

type Config = {
  "format.path": string;
  "format.args": string[];
  "build.xcbeautifyEnabled": boolean;
  "build.xcodeWorkspacePath": string;
  "build.derivedDataPath": string;
  "build.arch": "x86_64" | "arm64";
  "build.args": string[];
  "system.taskExecutor": "v1" | "v2";
  "system.logLevel": "debug" | "info" | "warn" | "error";
  "xcodegen.autogenerate": boolean;
  "tuist.autogenerate": boolean;
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
