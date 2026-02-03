import * as vscode from "vscode";

type Config = {
  "format.path": string;
  "format.args": string[] | null;
  "format.selectionArgs": string[] | null;
  "build.xcbeautifyEnabled": boolean;
  "build.xcodeWorkspacePath": string;
  "build.xcodebuildCommand": string;
  "build.derivedDataPath": string;
  "build.configuration": string;
  "build.arch": "x86_64" | "arm64";
  "build.allowProvisioningUpdates": boolean;
  "build.args": string[];
  "build.env": { [key: string]: string | null };
  "build.launchArgs": string[];
  "build.launchEnv": { [key: string]: string };
  "build.rosettaDestination": boolean;
  "build.bringSimulatorToForeground": boolean;
  "build.autoRefreshSchemes": boolean;
  "build.autoRefreshSchemesDelay": number;
  "system.taskExecutor": "v1" | "v2";
  "system.logLevel": "debug" | "info" | "warn" | "error";
  "system.enableSentry": boolean;
  "system.autoRevealTerminal": boolean;
  "system.showProgressStatusBar": boolean;
  "system.customXcodeWorkspaceParser": boolean;
  "xcodegen.autogenerate": boolean;
  "xcodebuildserver.autogenerate": boolean;
  "xcodebuildserver.path": string;
  "tuist.autogenerate": boolean;
  "tuist.generate.env": { [key: string]: string | null };
  "testing.configuration": string;
};

type ConfigKey = keyof Config;

export function getWorkspaceConfig<K extends ConfigKey>(key: K): Config[K] | undefined {
  const config = vscode.workspace.getConfiguration("sweetpad");
  const value = config.get<Config[K]>(key);

  return expandConfigEnvVars(value);
}

export function isWorkspaceConfigIsDefined<K extends ConfigKey>(key: K): boolean {
  return getWorkspaceConfig(key) !== undefined;
}

export async function updateWorkspaceConfig<K extends ConfigKey>(key: K, value: Config[K]): Promise<void> {
  const config = vscode.workspace.getConfiguration("sweetpad");
  return await config.update(key, value, vscode.ConfigurationTarget.Workspace);
}

/**
 * Recursively expands environment variables in the config object
 *
 * Example: { key: "Hello ${env:USER}" } -> { key: "Hello john" }
 *
 * We expand only strings. Array and objects are processed recursively. Other types are
 * returned as is.
 */
function expandConfigEnvVars<T>(value: T): T {
  if (typeof value === "string") {
    return value.replace(
      // Match ${env:VAR_NAME}, fallback and recursive expansion is not supported yet
      /\${env:([A-Za-z0-9_]+)}/g,
      (match: string, envName: string) => {
        const envValue = process.env[envName];
        // note: empty string is a valid value, so ENV_VAR_NAME="" will be resolved to empty string
        if (envValue !== undefined) {
          return envValue;
        }

        // if no env value is found, return the original ${env:VAR_NAME} string
        return match;
      },
    ) as T;
  }

  if (Array.isArray(value)) {
    return value.map((v) => expandConfigEnvVars(v)) as T;
  }

  if (value && typeof value === "object") {
    const result: Record<string, any> = {};
    for (const [k, v] of Object.entries(value)) {
      result[k] = expandConfigEnvVars(v);
    }
    return result as T;
  }

  // numbers, booleans, null, undefined, etc. are returned as is
  return value;
}
