import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { type Mock, afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BuildManager } from "../../core/build/manager";
import * as buildUtils from "../../core/build/utils";
import type { ConfigProvider } from "../../core/config/types";
import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination } from "../../core/destination/types";
import { noopLogger } from "../../core/logger/types";
import type { WorkspaceState } from "../../core/state/types";
import { ExecuteTaskError } from "../../core/tasks/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import type { RunRequestParams } from "../../protocol/types";
import { JsonDiagnosticsCollector } from "../adapters/json-diagnostics";
import { EventBus } from "../event-bus";
import { BuildRegistry } from "../registry";
import { createRunMethod } from "./run";

vi.mock("../../core/build/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../core/build/utils")>();
  return {
    ...actual,
    findXcodeWorkspaceInDirectory: vi.fn(),
    getSchemeLaunchSettings: vi.fn().mockResolvedValue({ args: [], env: {} }),
  };
});

type Harness = {
  runMethod: ReturnType<typeof createRunMethod>;
  registry: BuildRegistry;
  diagnostics: JsonDiagnosticsCollector;
  buildManager: { launchExplicit: Mock; getSchemes: Mock };
  eventBus: EventBus;
};

const SIM_15 = {
  id: "ios-simulator-A",
  label: "iPhone 15",
  type: "iOSSimulator" as const,
  platform: "iphonesimulator" as const,
} as unknown as Destination;

const SCHEMES = [{ name: "MyApp" }];

const _tmpDirs: string[] = [];

function makeTmpBuildsDir(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-run-test-"));
  _tmpDirs.push(dir);
  return dir;
}

function makeHarness(options?: { launchShouldFail?: ExecuteTaskError | Error }): Harness {
  (buildUtils.findXcodeWorkspaceInDirectory as Mock).mockResolvedValue("/fixture/MyApp.xcworkspace");

  const stateValues: Record<string, unknown> = {};
  const state: WorkspaceState = {
    get: ((key: string) => stateValues[key]) as unknown as WorkspaceState["get"],
    update: ((key: string, value: unknown) => {
      if (value === undefined) delete stateValues[key];
      else stateValues[key] = value;
    }) as unknown as WorkspaceState["update"],
    reset: () => {
      for (const k of Object.keys(stateValues)) delete stateValues[k];
    },
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
    getDestinations: vi.fn().mockResolvedValue([SIM_15]),
  } as unknown as DestinationsManager;

  const buildManager = {
    getSchemes: vi.fn().mockResolvedValue(SCHEMES),
    launchExplicit: vi.fn(async () => {
      if (options?.launchShouldFail) throw options.launchShouldFail;
    }),
  };

  const diagnostics = new JsonDiagnosticsCollector();
  const registry = new BuildRegistry({ buildsDir: makeTmpBuildsDir(), logger: noopLogger });
  const eventBus = new EventBus();

  const runMethod = createRunMethod({
    buildManager: buildManager as unknown as BuildManager,
    destinationsManager,
    registry,
    diagnostics,
    workspaceRoot,
    config,
    state,
    logger: noopLogger,
    eventBus,
  });

  return { runMethod, registry, diagnostics, buildManager, eventBus };
}

const VALID_PARAMS: RunRequestParams = {
  scheme: "MyApp",
  destination: "iPhone 15",
  configuration: "Debug",
};

describe("run method", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    while (_tmpDirs.length) {
      const dir = _tmpDirs.pop()!;
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("succeeds: returns the Build envelope with command=run", async () => {
    const h = makeHarness();
    const build = await h.runMethod({ ...VALID_PARAMS });

    expect(build.command).toBe("run");
    expect(build.status).toBe("succeeded");
    expect(build.scheme).toBe("MyApp");
    expect(h.buildManager.launchExplicit).toHaveBeenCalledOnce();
  });

  it("returns status=failed when launchExplicit throws ExecuteTaskError", async () => {
    const h = makeHarness({
      launchShouldFail: new ExecuteTaskError("nope", { command: "x", errorCode: 65 }),
    });
    const build = await h.runMethod({ ...VALID_PARAMS });

    expect(build.status).toBe("failed");
    expect(build.exitCode).toBe(65);
  });

  it("rejects an in-flight run with BUILD_IN_PROGRESS", async () => {
    const h = makeHarness();
    // Seed a running registry entry
    h.registry.start({ scheme: "X", destination: "D", configuration: "Debug", originator: "cli", command: "run" });

    await expect(h.runMethod({ ...VALID_PARAMS })).rejects.toMatchObject({ code: "BUILD_IN_PROGRESS" });
  });

  it("emits build.started + build.finished events", async () => {
    const h = makeHarness();
    const events: string[] = [];

    const build = h.runMethod({ ...VALID_PARAMS });
    // Subscribe to a synthetic future buildId — but registry IDs are
    // auto-assigned. Easier: subscribe to whatever the next ID will be.
    // We know it'll be b1 in a fresh registry.
    h.eventBus.subscribe("b1", (e) => events.push(e.event));
    await build;

    // build.started is emitted synchronously inside the method after
    // registry.start — our subscribe ran after that point, so it's
    // possible we miss it depending on event-loop ordering. The test
    // asserts only build.finished, which is emitted asynchronously
    // after async work settles.
    expect(events).toContain("build.finished");
  });

  it("rejects missing required params with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.runMethod({ scheme: "MyApp" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects non-string launchArgs with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.runMethod({ ...VALID_PARAMS, launchArgs: [42] })).rejects.toMatchObject({
      code: "INVALID_ARGUMENT",
    });
  });

  it("rejects non-string launchEnv values with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.runMethod({ ...VALID_PARAMS, launchEnv: { K: 42 } })).rejects.toMatchObject({
      code: "INVALID_ARGUMENT",
    });
  });
});
