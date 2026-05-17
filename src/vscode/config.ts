import type { ConfigKey, ConfigSchema } from "../core/config/types";
import { VsCodeConfigProvider } from "./adapters/config";

const provider = new VsCodeConfigProvider();

/**
 * Module-level convenience for the VS Code side, where it's natural to read
 * config without threading a `ConfigProvider` through every layer. Core code
 * must use the injected `ConfigProvider` instead.
 */
export const workspaceConfig = provider;

export function getWorkspaceConfig<K extends ConfigKey>(key: K): ConfigSchema[K] | undefined {
  return provider.get(key);
}

export function isWorkspaceConfigIsDefined<K extends ConfigKey>(key: K): boolean {
  return provider.isDefined(key);
}

export async function updateWorkspaceConfig<K extends ConfigKey>(key: K, value: ConfigSchema[K]): Promise<void> {
  return await provider.update(key, value);
}
