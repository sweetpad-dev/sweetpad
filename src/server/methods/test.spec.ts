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
import * as xcresultParser from "../../core/xcresult/parser";
import type { TestRequestParams } from "../../protocol/types";
import { JsonDiagnosticsCollector } from "../adapters/json-diagnostics";
import { EventBus } from "../event-bus";
import { BuildRegistry } from "../registry";
import { createTestMethod } from "./test";

vi.mock("../../core/build/utils", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../core/build/utils")>();
  return {
    ...actual,
    findXcodeWorkspaceInDirectory: vi.fn(),
  };
});

vi.mock("../../core/xcresult/parser", () => ({
  parseXcresultBundle: vi.fn(),
}));

const SIM_15 = {
  id: "ios-simulator-A",
  label: "iPhone 15",
  type: "iOSSimulator" as const,
  platform: "iphonesimulator" as const,
} as unknown as Destination;

const _tmpDirs: string[] = [];
function makeTmpBuildsDir(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-test-test-"));
  _tmpDirs.push(dir);
  return dir;
}

function makeHarness(options?: { testShouldFail?: ExecuteTaskError }) {
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
    getSchemes: vi.fn().mockResolvedValue([{ name: "MyApp" }]),
    testExplicit: vi.fn(async () => {
      if (options?.testShouldFail) throw options.testShouldFail;
      return { bundlePath: "/fixture/.sweetpad/storage/bundle/MyApp" };
    }),
  };

  const diagnostics = new JsonDiagnosticsCollector();
  const registry = new BuildRegistry({ buildsDir: makeTmpBuildsDir(), logger: noopLogger });
  const eventBus = new EventBus();

  const testMethod = createTestMethod({
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

  return { testMethod, registry, diagnostics, buildManager };
}

const VALID_PARAMS: TestRequestParams = {
  scheme: "MyApp",
  destination: "iPhone 15",
  configuration: "Debug",
};

describe("test method", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    while (_tmpDirs.length) {
      const dir = _tmpDirs.pop()!;
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("merges xcresult counts into the response", async () => {
    (xcresultParser.parseXcresultBundle as Mock).mockResolvedValue({
      totalTestCount: 5,
      passedTests: 4,
      failedTests: 1,
      skippedTests: 0,
      testCases: [
        { identifier: "A/testA", status: "passed", durationMs: 10 },
        { identifier: "A/testB", status: "failed", durationMs: 20, message: "x" },
      ],
    });

    const h = makeHarness();
    const result = await h.testMethod({ ...VALID_PARAMS });

    expect(result.command).toBe("test");
    expect(result.testsRun).toBe(5);
    expect(result.testsPassed).toBe(4);
    expect(result.testsFailed).toBe(1);
    expect(result.testCases).toHaveLength(2);
  });

  it("zeros counts when xcresult parsing returns undefined", async () => {
    (xcresultParser.parseXcresultBundle as Mock).mockResolvedValue(undefined);
    const h = makeHarness();
    const result = await h.testMethod({ ...VALID_PARAMS });

    expect(result.testsRun).toBe(0);
    expect(result.testsPassed).toBe(0);
    expect(result.testCases).toEqual([]);
    expect(result.status).toBe("succeeded");
  });

  it("still parses xcresult when xcodebuild fails (typical: any test failed)", async () => {
    (xcresultParser.parseXcresultBundle as Mock).mockResolvedValue({
      totalTestCount: 3,
      passedTests: 2,
      failedTests: 1,
      skippedTests: 0,
      testCases: [],
    });

    const h = makeHarness({
      testShouldFail: new ExecuteTaskError("test failed", { command: "xcodebuild", errorCode: 65 }),
    });
    const result = await h.testMethod({ ...VALID_PARAMS });

    expect(result.status).toBe("failed");
    expect(result.exitCode).toBe(65);
    expect(result.testsFailed).toBe(1);
    expect(result.testsPassed).toBe(2);
  });

  it("rejects in-flight runs with BUILD_IN_PROGRESS", async () => {
    const h = makeHarness();
    h.registry.start({ scheme: "X", destination: "D", configuration: "Debug", originator: "cli", command: "test" });
    await expect(h.testMethod({ ...VALID_PARAMS })).rejects.toMatchObject({ code: "BUILD_IN_PROGRESS" });
  });

  it("rejects missing required params with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.testMethod({ scheme: "MyApp" })).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
  });

  it("rejects non-string testIdentifiers with INVALID_ARGUMENT", async () => {
    const h = makeHarness();
    await expect(h.testMethod({ ...VALID_PARAMS, testIdentifiers: [42] })).rejects.toMatchObject({
      code: "INVALID_ARGUMENT",
    });
  });
});
