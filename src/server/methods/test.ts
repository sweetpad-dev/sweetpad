import type { BuildManager } from "../../core/build/manager";
import type { ConfigProvider } from "../../core/config/types";
import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination } from "../../core/destination/types";
import type { Logger } from "../../core/logger/types";
import type { WorkspaceState } from "../../core/state/types";
import { ExecuteTaskError } from "../../core/tasks/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import { parseXcresultBundle } from "../../core/xcresult/parser";
import type { ErrorCode } from "../../protocol/error-codes";
import { ProtocolError } from "../../protocol/errors";
import {
  type BuildEvent,
  type BuildResponseData,
  SCHEMA_VERSION,
  type TestRequestParams,
  type TestResponseData,
} from "../../protocol/types";
import type { JsonDiagnosticsCollector } from "../adapters/json-diagnostics";
import type { EventBus } from "../event-bus";
import { EventRecorder } from "../event-recorder";
import { LogWriter } from "../log-writer";
import type { BuildRegistry } from "../registry";
import { resolveXcworkspace } from "./helpers";

export type TestMethodDeps = {
  buildManager: BuildManager;
  destinationsManager: DestinationsManager;
  registry: BuildRegistry;
  diagnostics: JsonDiagnosticsCollector;
  workspaceRoot: WorkspaceRoot;
  config: ConfigProvider;
  state: WorkspaceState;
  logger: Logger;
  eventBus: EventBus;
};

/**
 * `test` runs `xcodebuild test` for the given scheme/destination, then
 * parses the produced `.xcresult` bundle for counts and per-test
 * outcomes. The build's status reflects xcodebuild's exit code; test
 * counts come from xcresulttool and are zeroed out if the bundle is
 * unreadable.
 */
export function createTestMethod(deps: TestMethodDeps) {
  return async (rawParams: unknown): Promise<TestResponseData> => {
    const params = validateParams(rawParams);

    const running = deps.registry.running();
    if (running.length > 0) {
      throw new ProtocolError("BUILD_IN_PROGRESS", "Another build/run/test is already active in this workspace", {
        hint: `sweetpad attach ${running[0].buildId}`,
        extra: { running },
      });
    }

    const xcworkspace = await resolveXcworkspace(
      { workspaceRoot: deps.workspaceRoot, config: deps.config, state: deps.state },
      params.xcworkspace,
    );
    deps.state.update("build.xcodeWorkspacePath", xcworkspace);
    await validateScheme(deps.buildManager, params.scheme);
    const destination = await resolveDestination(deps.destinationsManager, params.destination);

    const build = deps.registry.start({
      scheme: params.scheme,
      destination: destination.label,
      configuration: params.configuration,
      originator: "cli",
      command: "test",
    });

    let status: BuildResponseData["status"] = "succeeded";
    let exitCode: number | null = 0;
    let bundlePath: string | undefined;

    const logWriter = LogWriter.open({
      logger: deps.logger,
      logPath: deps.registry.getLogPath(build.buildId),
    });
    const eventRecorder = EventRecorder.open({
      logger: deps.logger,
      eventsPath: deps.registry.getEventsPath(build.buildId),
    });

    const fanOut = (event: BuildEvent) => {
      eventRecorder.record(event);
      deps.eventBus.emit(build.buildId, event);
    };

    fanOut({
      event: "build.started",
      schemaVersion: SCHEMA_VERSION,
      ts: new Date().toISOString(),
      buildId: build.buildId,
      data: { build },
    });

    try {
      const result = await deps.buildManager.testExplicit({
        scheme: params.scheme,
        configuration: params.configuration,
        destination,
        xcworkspace,
        onOutputLine: (line) => {
          logWriter.write(line);
          fanOut({
            event: "log.line",
            schemaVersion: SCHEMA_VERSION,
            ts: new Date().toISOString(),
            buildId: build.buildId,
            data: { line },
          });
        },
      });
      bundlePath = result.bundlePath;
    } catch (error) {
      status = "failed";
      exitCode = error instanceof ExecuteTaskError ? error.errorCode : null;
      // testExplicit may have produced the bundle even if it errored
      // (typical for xcodebuild test: exit != 0 when any test fails, but
      // the bundle still exists). Try to parse it below.
      if (error instanceof ExecuteTaskError) {
        // We don't get the bundle path from the error — re-derive it the
        // same way testExplicit does. Cheaper than restructuring the
        // engine's error path to carry it.
        bundlePath = await deriveBundlePath(deps, params.scheme);
      }
    } finally {
      await logWriter.close();
    }

    const baseBuild = deps.registry.finish(build.buildId, {
      status,
      exitCode,
      diagnostics: deps.diagnostics.drain(),
    });

    const testResults = bundlePath
      ? await parseXcresultBundle(bundlePath, { logger: deps.logger })
      : undefined;

    const final: TestResponseData = {
      ...baseBuild,
      testsRun: testResults?.totalTestCount ?? 0,
      testsPassed: testResults?.passedTests ?? 0,
      testsFailed: testResults?.failedTests ?? 0,
      testsSkipped: testResults?.skippedTests ?? 0,
      testCases: testResults?.testCases ?? [],
    };

    fanOut({
      event: "build.finished",
      schemaVersion: SCHEMA_VERSION,
      ts: new Date().toISOString(),
      buildId: final.buildId,
      data: { build: final },
    });
    await eventRecorder.close();

    return final;
  };
}

async function deriveBundlePath(deps: TestMethodDeps, scheme: string): Promise<string> {
  const path = await import("node:path");
  const storagePath = await deps.workspaceRoot.getStoragePath();
  return path.join(storagePath, "bundle", scheme);
}

function validateParams(raw: unknown): TestRequestParams {
  if (!raw || typeof raw !== "object") {
    throw invalidArgument("Missing test params");
  }
  const params = raw as Partial<TestRequestParams>;

  requireString(params.scheme, "scheme");
  requireString(params.destination, "destination");
  requireString(params.configuration, "configuration");
  if (params.xcworkspace !== undefined) requireString(params.xcworkspace, "xcworkspace");
  if (params.testIdentifiers !== undefined && !isStringArray(params.testIdentifiers)) {
    throw invalidArgument("'testIdentifiers' must be an array of strings");
  }

  return {
    scheme: params.scheme!,
    destination: params.destination!,
    configuration: params.configuration!,
    xcworkspace: params.xcworkspace,
    testIdentifiers: params.testIdentifiers,
  };
}

function requireString(value: unknown, name: string): asserts value is string {
  if (typeof value !== "string" || value.length === 0) {
    throw invalidArgument(`'${name}' is required and must be a non-empty string`);
  }
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((v) => typeof v === "string");
}

function invalidArgument(message: string): ProtocolError {
  return new ProtocolError("INVALID_ARGUMENT", message);
}

async function validateScheme(buildManager: BuildManager, scheme: string): Promise<void> {
  const schemes = await buildManager.getSchemes();
  if (!schemes.some((s) => s.name === scheme)) {
    const available = schemes.map((s) => s.name);
    throw new ProtocolError("SCHEME_NOT_FOUND", `Scheme '${scheme}' not found`, {
      extra: { availableSchemes: available },
    });
  }
}

async function resolveDestination(
  destinationsManager: DestinationsManager,
  identifier: string,
): Promise<Destination> {
  const all = await destinationsManager.getDestinations();
  const byId = all.filter((d) => d.id === identifier);
  if (byId.length === 1) return byId[0];

  const byLabel = all.filter((d) => d.label === identifier);
  if (byLabel.length === 1) return byLabel[0];
  if (byLabel.length > 1) {
    throw failDestination("DESTINATION_AMBIGUOUS", `Destination '${identifier}' matches ${byLabel.length} entries`, byLabel);
  }

  throw failDestination("DESTINATION_NOT_FOUND", `Destination '${identifier}' not found`, []);
}

function failDestination(code: ErrorCode, message: string, candidates: Destination[]): ProtocolError {
  return new ProtocolError(code, message, {
    extra: {
      candidates: candidates.map((d) => ({ id: d.id, label: d.label, type: d.type })),
    },
  });
}
