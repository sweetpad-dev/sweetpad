import * as vscode from "vscode";

type ConfigKey = "format.path" | "build.xcbeautifyEnabled" | "system.taskExecutor";

export function getWorkspaceConfig<T = any>(key: ConfigKey): T | undefined {
  const config = vscode.workspace.getConfiguration("sweetpad");
  return config.get(key);
}
