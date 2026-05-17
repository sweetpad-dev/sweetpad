import * as vscode from "vscode";

import type { ConfigKey, ConfigProvider, ConfigSchema } from "../../core/config/types";

/**
 * VS Code-backed ConfigProvider. Reads from `sweetpad.*` workspace configuration
 * and recursively expands `${env:NAME}` placeholders in string values.
 */
export class VsCodeConfigProvider implements ConfigProvider {
  get<K extends ConfigKey>(key: K): ConfigSchema[K] | undefined {
    const config = vscode.workspace.getConfiguration("sweetpad");
    const value = config.get<ConfigSchema[K]>(key);
    return expandConfigEnvVars(value);
  }

  isDefined<K extends ConfigKey>(key: K): boolean {
    return this.get(key) !== undefined;
  }

  async update<K extends ConfigKey>(key: K, value: ConfigSchema[K]): Promise<void> {
    const config = vscode.workspace.getConfiguration("sweetpad");
    await config.update(key, value, vscode.ConfigurationTarget.Workspace);
  }
}

/**
 * Recursively expands environment variables inside string values.
 *
 * Example: `"Hello ${env:USER}"` → `"Hello john"`.
 *
 * Strings are scanned; arrays and objects are processed recursively; everything
 * else passes through unchanged.
 */
function expandConfigEnvVars<T>(value: T): T {
  if (typeof value === "string") {
    return value.replace(
      // Match ${env:VAR_NAME}; no fallback or recursive expansion.
      /\${env:([A-Za-z0-9_]+)}/g,
      (match: string, envName: string) => {
        const envValue = process.env[envName];
        // Empty string is a valid value, so ENV_VAR_NAME="" resolves to empty string.
        if (envValue !== undefined) {
          return envValue;
        }
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

  return value;
}
