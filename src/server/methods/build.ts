import type { BuildManager } from "../../core/build/manager";
import type { ConfigProvider } from "../../core/config/types";
import type { DestinationsManager } from "../../core/destination/manager";
import type { Destination } from "../../core/destination/types";
import type { Logger } from "../../core/logger/types";
import { ExecuteTaskError } from "../../core/tasks/types";
import type { WorkspaceRoot } from "../../core/workspace-root";
import type { ErrorCode } from "../../protocol/error-codes";
import { ProtocolError } from "../../protocol/errors";
import type { WorkspaceState } from "../../core/state/types";
import {
  type BuildEvent,
  type BuildRequestParams,
  type BuildResponseData,
  SCHEMA_VERSION,
} from "../../protocol/types";
import type { JsonDiagnosticsCollector } from "../adapters/json-diagnostics";
import type { EventBus } from "../event-bus";
import { EventRecorder } from "../event-recorder";
import { LogWriter } from "../log-writer";
import type { BuildRegistry } from "../registry";
import { resolveXcworkspace } from "./helpers";

export type BuildMethodDeps = {
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

export function createBuildMethod(deps: BuildMethodDeps) {
  return async (rawParams: unknown): Promise<BuildResponseData> => {
    const params = validateParams(rawParams);

    const running = deps.registry.running();
    if (running.length > 0) {
      throw new ProtocolError("BUILD_IN_PROGRESS", "Another build is already running in this workspace", {
        hint: `sweetpad attach ${running[0].buildId}`,
        extra: { running },
      });
    }

    const xcworkspace = await resolveXcworkspace(
      { workspaceRoot: deps.workspaceRoot, config: deps.config, state: deps.state },
      params.xcworkspace,
    );
    // Persist the resolved path so BuildManager.refreshSchemes() (and any
    // subsequent flow that calls getCurrentXcodeWorkspacePath) picks it up.
    deps.state.update("build.xcodeWorkspacePath", xcworkspace);
    await validateScheme(deps.buildManager, params.scheme);
    const destination = await resolveDestination(deps.destinationsManager, params.destination);

    const build = deps.registry.start({
      scheme: params.scheme,
      destination: destination.label,
      configuration: params.configuration,
      originator: "cli",
    });

    let status: BuildResponseData["status"] = "succeeded";
    let exitCode: number | null = 0;

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
      await deps.buildManager.buildExplicit({
        scheme: params.scheme,
        configuration: params.configuration,
        destination,
        xcworkspace,
        debug: params.debug ?? false,
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
    } catch (error) {
      status = "failed";
      exitCode = error instanceof ExecuteTaskError ? error.errorCode : null;
    } finally {
      await logWriter.close();
    }

    const finalBuild = deps.registry.finish(build.buildId, {
      status,
      exitCode,
      diagnostics: deps.diagnostics.drain(),
    });

    fanOut({
      event: "build.finished",
      schemaVersion: SCHEMA_VERSION,
      ts: new Date().toISOString(),
      buildId: finalBuild.buildId,
      data: { build: finalBuild },
    });
    await eventRecorder.close();

    return finalBuild;
  };
}

function validateParams(raw: unknown): BuildRequestParams {
  if (!raw || typeof raw !== "object") {
    throw invalidArgument("Missing build params");
  }
  const params = raw as Partial<BuildRequestParams>;

  requireString(params.scheme, "scheme");
  requireString(params.destination, "destination");
  requireString(params.configuration, "configuration");
  if (params.xcworkspace !== undefined) requireString(params.xcworkspace, "xcworkspace");
  if (params.debug !== undefined && typeof params.debug !== "boolean") {
    throw invalidArgument("'debug' must be a boolean");
  }

  return {
    scheme: params.scheme!,
    destination: params.destination!,
    configuration: params.configuration!,
    xcworkspace: params.xcworkspace,
    debug: params.debug ?? false,
  };
}

function requireString(value: unknown, name: string): asserts value is string {
  if (typeof value !== "string" || value.length === 0) {
    throw invalidArgument(`'${name}' is required and must be a non-empty string`);
  }
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
