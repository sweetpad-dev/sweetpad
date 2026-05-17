import { describe, expect, it } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { MethodDispatcher } from "../dispatcher";
import { createUsageMethod } from "./usage";

/**
 * The dispatcher's `register` is statically typed against the production
 * `MethodMap`. The test uses a synthetic method name to keep this test
 * decoupled from whatever real methods happen to be wired up — the cast is
 * an intentional test-only escape from the static contract.
 */
type AnyDispatcher = {
  register: (
    method: string,
    options: { description: string; handler: (params: unknown) => Promise<unknown> },
  ) => void;
};

describe("usage method", () => {
  it("returns every registered method's name + description", async () => {
    const dispatcher = new MethodDispatcher(noopLogger);
    (dispatcher as unknown as AnyDispatcher).register("alpha", {
      description: "first method",
      handler: async () => ({}),
    });
    (dispatcher as unknown as AnyDispatcher).register("beta", {
      description: "second method",
      handler: async () => ({}),
    });

    const method = createUsageMethod({ dispatcher });
    const result = await method();

    expect(result.methods).toEqual([
      { name: "alpha", description: "first method" },
      { name: "beta", description: "second method" },
    ]);
    expect(result.schemaVersion).toBe("1.0");
  });

  it("returns an empty list when nothing is registered", async () => {
    const dispatcher = new MethodDispatcher(noopLogger);
    const method = createUsageMethod({ dispatcher });
    const result = await method();
    expect(result.methods).toEqual([]);
  });
});
