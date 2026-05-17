import { type Mock, beforeEach, describe, expect, it, vi } from "vitest";

import type { BuildManager } from "../../core/build/manager";
import * as buildUtils from "../../core/build/utils";
import type { ConfigProvider } from "../../core/config/types";
import type { WorkspaceState } from "../../core/state/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { createSchemesListMethod } from "./schemes-list";

vi.mock("../../core/build/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../core/build/utils")>();
  return {
    ...actual,
    findXcodeWorkspaceInDirectory: vi.fn(),
  };
});

function makeHarness(options?: { xcworkspaceAuto?: string | undefined; schemes?: Array<{ name: string }> }) {
  const autoValue = options && "xcworkspaceAuto" in options ? options.xcworkspaceAuto : "/fixture/MyApp.xcworkspace";
  (buildUtils.findXcodeWorkspaceInDirectory as Mock).mockResolvedValue(autoValue);

  const stateValues: Record<string, unknown> = {};
  const state = {
    get: vi.fn((key: string) => stateValues[key]),
    update: vi.fn((key: string, value: unknown) => {
      if (value === undefined) delete stateValues[key];
      else stateValues[key] = value;
    }),
    reset: vi.fn(),
    _values: stateValues,
  };

  const config: ConfigProvider = {
    get: vi.fn().mockReturnValue(undefined),
    isDefined: vi.fn().mockReturnValue(false),
    update: vi.fn(),
  };

  const workspaceRoot: WorkspaceRoot = {
    getPath: () => "/fixture",
    getStoragePath: async () => "/fixture/.sweetpad/storage",
    getRelativePath: (p) => p,
  };

  const buildManager = {
    getSchemes: vi.fn().mockResolvedValue(options?.schemes ?? [{ name: "App" }, { name: "AppTests" }]),
  };

  const method = createSchemesListMethod({
    buildManager: buildManager as unknown as BuildManager,
    workspaceRoot,
    config,
    state: state as unknown as WorkspaceState,
  });

  return { method, state, buildManager };
}

describe("schemes.list method", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns the scheme list, mapped to wire shape", async () => {
    const h = makeHarness();
    const result = await h.method({});

    expect(result.schemes).toEqual([{ name: "App" }, { name: "AppTests" }]);
    expect(result.xcworkspace).toBe("/fixture/MyApp.xcworkspace");
    expect(h.state._values["build.xcodeWorkspacePath"]).toBe("/fixture/MyApp.xcworkspace");
  });

  it("respects the explicit xcworkspace override", async () => {
    const h = makeHarness();
    const result = await h.method({ xcworkspace: "/custom/Other.xcworkspace" });

    expect(result.xcworkspace).toBe("/custom/Other.xcworkspace");
    expect(h.state._values["build.xcodeWorkspacePath"]).toBe("/custom/Other.xcworkspace");
  });

  it("accepts undefined params (CLI may omit them)", async () => {
    const h = makeHarness();
    await expect(h.method(undefined)).resolves.toMatchObject({ xcworkspace: expect.any(String) });
  });

  it("rejects non-object params", async () => {
    const h = makeHarness();
    await expect(h.method("nope")).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects non-string xcworkspace", async () => {
    const h = makeHarness();
    await expect(h.method({ xcworkspace: 42 })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("throws WORKSPACE_NOT_DETECTED when nothing is auto-detected", async () => {
    const h = makeHarness({ xcworkspaceAuto: undefined });
    await expect(h.method({})).rejects.toMatchObject({ code: "WORKSPACE_NOT_DETECTED" });
  });
});
