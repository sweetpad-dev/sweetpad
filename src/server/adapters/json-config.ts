import type { ConfigKey, ConfigProvider, ConfigSchema } from "../../core/config/types";

/**
 * Headless `ConfigProvider` for the CLI/server. v1 holds no on-disk config
 * file (deferred per `docs/dev/agent-cli.md` §9): every key returns
 * `undefined`, so the engine falls back to its built-in defaults. Critical
 * knobs come from CLI flags via the `build` method's params, not config.
 */
export class JsonConfigProvider implements ConfigProvider {
  get<K extends ConfigKey>(_key: K): ConfigSchema[K] | undefined {
    return undefined;
  }

  isDefined<K extends ConfigKey>(_key: K): boolean {
    return false;
  }

  async update<K extends ConfigKey>(_key: K, _value: ConfigSchema[K]): Promise<void> {
    // No persistent CLI config in v1 — silently ignore. Future iterations
    // may surface this as an error if a code path tries to mutate config.
  }
}
