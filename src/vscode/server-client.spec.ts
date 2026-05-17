import { describe, expect, it } from "vitest";

import type { ConfigKey, ConfigProvider, ConfigSchema } from "../core/config/types";
import { isServerModeEnabled } from "./server-client";

function makeConfig(overrides: Partial<ConfigSchema>): ConfigProvider {
  return {
    get<K extends ConfigKey>(key: K): ConfigSchema[K] | undefined {
      return overrides[key];
    },
    isDefined<K extends ConfigKey>(key: K): boolean {
      return overrides[key] !== undefined;
    },
    async update() {
      // no-op; tests don't mutate config
    },
  };
}

describe("isServerModeEnabled", () => {
  it("reads system.experimental.serverMode", () => {
    expect(isServerModeEnabled(makeConfig({ "system.experimental.serverMode": true }))).toBe(true);
  });

  it("defaults to false when unset (Phase 3 is opt-in)", () => {
    expect(isServerModeEnabled(makeConfig({}))).toBe(false);
  });

  it("treats falsy non-undefined values as off", () => {
    expect(isServerModeEnabled(makeConfig({ "system.experimental.serverMode": false }))).toBe(false);
  });
});
