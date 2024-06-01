import * as vscode from "vscode";

type ConfigKey =
  | "format.path"
  | "format.args"
  | "build.xcbeautifyEnabled"
  | "system.taskExecutor"
  | "system.logLevel"
  | "xcodegen.autogenerate";

export function getWorkspaceConfig<T = any>(key: ConfigKey): T | undefined {
  const config = vscode.workspace.getConfiguration("sweetpad");
  return config.get(key);
}
