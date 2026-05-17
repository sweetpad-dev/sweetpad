import type { ErrorCode } from "./error-codes";

export const SCHEMA_VERSION = "1.0";

export type WireRequest = {
  id: number;
  method: string;
  params?: Record<string, unknown>;
};

export type WireSuccessResponse<T = unknown> = {
  id: number;
  ok: true;
  schemaVersion: string;
  data: T;
};

export type WireErrorPayload = {
  code: ErrorCode;
  message: string;
  hint?: string;
};

export type WireErrorResponse = {
  id: number;
  ok: false;
  schemaVersion: string;
  error: WireErrorPayload;
  // Per-code extras (e.g. `running` list for BUILD_IN_PROGRESS).
  [extra: string]: unknown;
};

export type WireResponse<T = unknown> = WireSuccessResponse<T> | WireErrorResponse;

/**
 * Server-pushed event. No `id`, no response expected. Used by `attach` (live
 * stream) and replayed from disk for finished builds. Not used in the first
 * slice but the type lives here so server code can emit and CLI can ignore.
 */
export type WireEvent<T = unknown> = {
  event: string;
  schemaVersion: string;
  ts: string;
  buildId?: string;
  data: T;
};

export type WireMessage = WireRequest | WireResponse | WireEvent;

export function isResponse(msg: WireMessage): msg is WireResponse {
  return typeof (msg as WireResponse).id === "number" && "ok" in msg;
}

export function isEvent(msg: WireMessage): msg is WireEvent {
  return typeof (msg as WireEvent).event === "string" && !("id" in msg);
}

export function isRequest(msg: WireMessage): msg is WireRequest {
  return typeof (msg as WireRequest).method === "string" && typeof (msg as WireRequest).id === "number";
}

// ---------------------------------------------------------------------------
// Method: build
// ---------------------------------------------------------------------------

export type BuildRequestParams = {
  scheme: string;
  /** Destination ID or exact name. Server resolves against its destinations manager. */
  destination: string;
  configuration: string;
  /** Override the auto-detected xcworkspace path. Optional. */
  xcworkspace?: string;
  debug?: boolean;
};

export type Diagnostic = {
  file: string;
  line: number;
  column: number;
  endLine?: number;
  endColumn?: number;
  severity: "error" | "warning";
  message: string;
  source: string;
};

export type BuildStatus = "running" | "succeeded" | "failed" | "cancelled" | "interrupted";

export type CommandKind = "build" | "run" | "test";

export type BuildResponseData = {
  buildId: string;
  scheme: string;
  destination: string;
  config: string;
  command: CommandKind;
  status: BuildStatus;
  exitCode: number | null;
  originator: "cli" | "vscode";
  startedAt: string;
  finishedAt: string | null;
  durationMs: number | null;
  errorCount: number;
  warningCount: number;
  diagnostics: Diagnostic[];
};

// ---------------------------------------------------------------------------
// Method: run (build + install + launch; waits for the app to exit)
// ---------------------------------------------------------------------------

export type RunRequestParams = {
  scheme: string;
  destination: string;
  configuration: string;
  xcworkspace?: string;
  debug?: boolean;
  /** Args passed to the launched app's `main()`. */
  launchArgs?: string[];
  /** Env vars merged into the launched app's environment. */
  launchEnv?: Record<string, string>;
};

export type RunResponseData = BuildResponseData;

// ---------------------------------------------------------------------------
// Method: test (build-for-testing + xcodebuild test; returns xcresult summary)
// ---------------------------------------------------------------------------

export type TestRequestParams = {
  scheme: string;
  destination: string;
  configuration: string;
  xcworkspace?: string;
  /** When provided, restrict to these test identifiers (e.g. `MyTests/testFoo`). */
  testIdentifiers?: string[];
};

export type TestCaseStatus = "passed" | "failed" | "skipped";

export type TestCaseSummary = {
  identifier: string;
  status: TestCaseStatus;
  durationMs: number | null;
  /** Failure message if `status === "failed"`. */
  message?: string;
};

export type TestResponseData = BuildResponseData & {
  /** Counts derived from the xcresult bundle. */
  testsRun: number;
  testsPassed: number;
  testsFailed: number;
  testsSkipped: number;
  testCases: TestCaseSummary[];
};

// ---------------------------------------------------------------------------
// Method: builds.list
// ---------------------------------------------------------------------------

export type BuildsListRequestParams = {
  /** Cap on number of builds returned. Most recent first. */
  limit?: number;
  /** Filter to a single status. */
  status?: BuildStatus;
};

export type BuildsListResponseData = {
  builds: BuildResponseData[];
};

// ---------------------------------------------------------------------------
// Method: build.get
// ---------------------------------------------------------------------------

export type BuildGetRequestParams = {
  buildId: string;
};

export type BuildGetResponseData = BuildResponseData;

// ---------------------------------------------------------------------------
// Method: attach (streaming — not in MethodMap; handled by listener directly)
// ---------------------------------------------------------------------------

export type AttachRequestParams = {
  buildId: string;
  /** When the buildId is finished, replay recorded events. Default: true. */
  replay?: boolean;
};

/**
 * Names every event the server can emit. Keep this closed so the CLI's
 * attach loop can switch on type without an `unknown` cast.
 */
export type BuildEventType = "build.started" | "log.line" | "build.finished" | "attach.complete";

export type BuildStartedEventData = { build: BuildResponseData };
export type LogLineEventData = { line: string };
export type BuildFinishedEventData = { build: BuildResponseData };
export type AttachCompleteEventData = {
  reason: "build.finished" | "replay.complete" | "closed";
};

export type BuildEvent =
  | (WireEvent<BuildStartedEventData> & { event: "build.started" })
  | (WireEvent<LogLineEventData> & { event: "log.line" })
  | (WireEvent<BuildFinishedEventData> & { event: "build.finished" })
  | (WireEvent<AttachCompleteEventData> & { event: "attach.complete" });

// ---------------------------------------------------------------------------
// Method: logs.get
// ---------------------------------------------------------------------------

export type LogsGetRequestParams = {
  buildId: string;
  /** Return only the last N lines. Omit for the whole log. */
  tail?: number;
};

export type LogsGetResponseData = {
  buildId: string;
  /** Raw lines joined with `\n`. No trailing newline. */
  content: string;
  /** Total lines in the file, even when `tail` truncated the response. */
  lineCount: number;
  /** True iff the returned content is a subset (tail was set and capped). */
  truncated: boolean;
};

// ---------------------------------------------------------------------------
// Method: schemes.list
// ---------------------------------------------------------------------------

export type SchemesListRequestParams = {
  /** Override the auto-detected xcworkspace path. Optional. */
  xcworkspace?: string;
};

export type SchemeSummary = {
  name: string;
};

export type SchemesListResponseData = {
  schemes: SchemeSummary[];
  xcworkspace: string;
};

// ---------------------------------------------------------------------------
// Method: destinations.list
// ---------------------------------------------------------------------------

// Redeclared here rather than imported from `core/destination/types` so the
// protocol module stays free of engine deps and can be reused by any future
// adapter (MCP, JSON-RPC, etc).
export type DestinationKind =
  | "iOSSimulator"
  | "watchOSSimulator"
  | "tvOSSimulator"
  | "visionOSSimulator"
  | "macOS"
  | "iOSDevice"
  | "watchOSDevice"
  | "tvOSDevice"
  | "visionOSDevice";

export const ALL_DESTINATION_KINDS: DestinationKind[] = [
  "iOSSimulator",
  "watchOSSimulator",
  "tvOSSimulator",
  "visionOSSimulator",
  "macOS",
  "iOSDevice",
  "watchOSDevice",
  "tvOSDevice",
  "visionOSDevice",
];

export type DestinationsListRequestParams = {
  /** Filter to a single destination kind. Omit for everything. */
  kind?: DestinationKind;
  /** Refresh the underlying simctl / xctrace caches before listing. */
  refresh?: boolean;
};

export type DestinationSummary = {
  id: string;
  kind: DestinationKind;
  label: string;
  /** Apple SDK platform name, e.g. "iphonesimulator", "macosx". */
  platform: string;
};

export type DestinationsListResponseData = {
  destinations: DestinationSummary[];
};

// ---------------------------------------------------------------------------
// Method: usage
// ---------------------------------------------------------------------------

export type UsageRequestParams = Record<string, never>;

export type MethodSummary = {
  name: string;
  description: string;
};

export type UsageResponseData = {
  schemaVersion: string;
  methods: MethodSummary[];
};
