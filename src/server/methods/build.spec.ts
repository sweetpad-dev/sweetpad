import { type Mock, beforeEach, describe, expect, it, vi } from "vitest";

import type { BuildManager } from "../../core/build/manager";
import * as buildUtils from "../../core/build/utils";
import type { ConfigProvider } from "../../core/config/types";
import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination } from "../../core/destination/types";
import type { WorkspaceState } from "../../core/state/types";
import { ExecuteTaskError } from "../../core/tasks/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import type { BuildRequestParams } from "../../protocol/types";
import { JsonDiagnosticsCollector } from "../adapters/json-diagnostics";
import { BuildRegistry } from "../registry";
import { createBuildMethod } from "./build";

// `findXcodeWorkspaceInDirectory` walks the filesystem; the rest of utils are
// pure helpers we don't touch from the build method.
vi.mock("../../core/build/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../core/build/utils")>();
  return {
    ...actual,
    findXcodeWorkspaceInDirectory: vi.fn(),
  };
});

type Harness = {
  buildMethod: ReturnType<typeof createBuildMethod>;
  registry: BuildRegistry;
  diagnostics: JsonDiagnosticsCollector;
  buildManager: { buildExplicit: Mock; getSchemes: Mock };
  state: {
    get: Mock;
    update: Mock;
    reset: Mock;
    _values: Record<string, unknown>;
  };
};

const SIM_15 = {
  id: "ios-simulator-A",
  label: "iPhone 15",
  type: "iOSSimulator" as const,
  platform: "iphonesimulator" as const,
} as unknown as Destination;

const SIM_15_DUPLICATE = {
  id: "ios-simulator-B",
  label: "iPhone 15",
  type: "iOSSimulator" as const,
  platform: "iphonesimulator" as const,
} as unknown as Destination;

const SIM_16 = {
  id: "ios-simulator-C",
  label: "iPhone 16",
  type: "iOSSimulator" as const,
  platform: "iphonesimulator" as const,
} as unknown as Destination;

const SCHEMES = [{ name: "MyApp", isTestable: false, configurations: ["Debug", "Release"] }];

function makeHarness(options?: {
  destinations?: Destination[];
  xcworkspaceAuto?: string | undefined;
  buildShouldFail?: ExecuteTaskError | Error;
}): Harness {
  (buildUtils.findXcodeWorkspaceInDirectory as Mock).mockResolvedValue(
    options?.xcworkspaceAuto ?? "/fixture/MyApp.xcworkspace",
  );

  const stateValues: Record<string, unknown> = {};
  const state = {
    get: vi.fn((key: string) => stateValues[key]),
    update: vi.fn((key: string, value: unknown) => {
      if (value === undefined) delete stateValues[key];
      else stateValues[key] = value;
    }),
    reset: vi.fn(() => {
      for (const key of Object.keys(stateValues)) delete stateValues[key];
    }),
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

  const destinationsManager = {
    getDestinations: vi.fn().mockResolvedValue(options?.destinations ?? [SIM_15, SIM_16]),
  } as unknown as DestinationsManager;

  const buildManager = {
    getSchemes: vi.fn().mockResolvedValue(SCHEMES),
    buildExplicit: vi.fn(async () => {
      if (options?.buildShouldFail) throw options.buildShouldFail;
    }),
  };

  const diagnostics = new JsonDiagnosticsCollector();
  const registry = new BuildRegistry();

  const buildMethod = createBuildMethod({
    buildManager: buildManager as unknown as BuildManager,
    destinationsManager,
    registry,
    diagnostics,
    workspaceRoot,
    config,
    state: state as unknown as WorkspaceState,
  });

  return { buildMethod, registry, diagnostics, buildManager, state };
}

const VALID_PARAMS: BuildRequestParams = {
  scheme: "MyApp",
  destination: "iPhone 15",
  configuration: "Debug",
};

describe("build method", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("succeeds: resolves scheme + destination + xcworkspace, returns Build envelope", async () => {
    const h = makeHarness();
    const build = await h.buildMethod({ ...VALID_PARAMS });

    expect(build.status).toBe("succeeded");
    expect(build.exitCode).toBe(0);
    expect(build.scheme).toBe("MyApp");
    expect(build.destination).toBe("iPhone 15");
    expect(build.config).toBe("Debug");
    expect(build.command).toBe("build");
    expect(build.originator).toBe("cli");
    expect(build.errorCount).toBe(0);
    expect(build.warningCount).toBe(0);

    // Workspace persisted to state so subsequent flows find it.
    expect(h.state._values["build.xcodeWorkspacePath"]).toBe("/fixture/MyApp.xcworkspace");
    // buildExplicit invoked with the resolved params.
    expect(h.buildManager.buildExplicit).toHaveBeenCalledOnce();
    expect(h.buildManager.buildExplicit.mock.calls[0][0]).toMatchObject({
      scheme: "MyApp",
      configuration: "Debug",
      xcworkspace: "/fixture/MyApp.xcworkspace",
      debug: false,
    });
  });

  it("uses an explicit xcworkspace override when provided", async () => {
    const h = makeHarness();
    await h.buildMethod({ ...VALID_PARAMS, xcworkspace: "/custom/path.xcworkspace" });
    expect(h.state._values["build.xcodeWorkspacePath"]).toBe("/custom/path.xcworkspace");
  });

  it("resolves destination by id when label has duplicates", async () => {
    const h = makeHarness({
      destinations: [SIM_15, SIM_15_DUPLICATE, SIM_16],
    });
    const build = await h.buildMethod({ ...VALID_PARAMS, destination: "ios-simulator-A" });
    expect(build.status).toBe("succeeded");
  });

  it("fails with SCHEME_NOT_FOUND when the scheme isn't in the project", async () => {
    const h = makeHarness();
    await expect(h.buildMethod({ ...VALID_PARAMS, scheme: "Ghost" })).rejects.toMatchObject({
      code: "SCHEME_NOT_FOUND",
    });
  });

  it("fails with DESTINATION_NOT_FOUND when the identifier doesn't match anything", async () => {
    const h = makeHarness();
    await expect(h.buildMethod({ ...VALID_PARAMS, destination: "Ghost" })).rejects.toMatchObject({
      code: "DESTINATION_NOT_FOUND",
    });
  });

  it("fails with DESTINATION_AMBIGUOUS when the label matches multiple destinations", async () => {
    const h = makeHarness({
      destinations: [SIM_15, SIM_15_DUPLICATE],
    });
    await expect(h.buildMethod({ ...VALID_PARAMS, destination: "iPhone 15" })).rejects.toMatchObject({
      code: "DESTINATION_AMBIGUOUS",
    });
  });

  it("fails with WORKSPACE_NOT_DETECTED when no xcworkspace is found", async () => {
    (buildUtils.findXcodeWorkspaceInDirectory as Mock).mockResolvedValueOnce(undefined);
    const h = makeHarness({ xcworkspaceAuto: undefined });
    await expect(h.buildMethod({ ...VALID_PARAMS })).rejects.toMatchObject({
      code: "WORKSPACE_NOT_DETECTED",
    });
  });

  it("fails with BUILD_IN_PROGRESS when another build is already running", async () => {
    const h = makeHarness();
    h.registry.start({
      scheme: "Other",
      destination: "iPhone 15",
      configuration: "Debug",
      originator: "vscode",
    });
    await expect(h.buildMethod({ ...VALID_PARAMS })).rejects.toMatchObject({
      code: "BUILD_IN_PROGRESS",
    });
  });

  it("returns status=failed when buildExplicit throws ExecuteTaskError", async () => {
    const h = makeHarness({
      buildShouldFail: new ExecuteTaskError("xcodebuild failed", {
        command: "xcodebuild ...",
        errorCode: 65,
      }),
    });
    const build = await h.buildMethod({ ...VALID_PARAMS });
    expect(build.status).toBe("failed");
    expect(build.exitCode).toBe(65);
  });

  it("validates required params (INVALID_ARGUMENT)", async () => {
    const h = makeHarness();
    await expect(h.buildMethod({ scheme: "MyApp" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
    await expect(h.buildMethod(null)).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("each invocation gets a new build id (b1, b2, ...)", async () => {
    const h = makeHarness();
    const first = await h.buildMethod({ ...VALID_PARAMS });
    const second = await h.buildMethod({ ...VALID_PARAMS });
    expect(first.buildId).toBe("b1");
    expect(second.buildId).toBe("b2");
  });

  it("inlines diagnostics from the collector into the response", async () => {
    const h = makeHarness();
    // Simulate a build that recorded diagnostics by feeding the collector
    // directly — the real engine does this via the accumulator returned from
    // beginBuild during xcodebuild execution.
    const acc = h.diagnostics.beginBuild({ mode: "xcodebuild" });
    acc.recordLine("/x/MyApp/Foo.swift:10:5: error: cannot find 'bar' in scope");
    acc.recordLine("/x/MyApp/Foo.swift:12:1: warning: variable 'x' was never used");
    acc.flush();

    const build = await h.buildMethod({ ...VALID_PARAMS });
    expect(build.errorCount).toBe(1);
    expect(build.warningCount).toBe(1);
    expect(build.diagnostics).toHaveLength(2);
  });
});
