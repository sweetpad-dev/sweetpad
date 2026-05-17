import * as fs from "node:fs";
import { promisify } from "node:util";
import * as child_process from "node:child_process";

import type { Logger } from "../logger/types";
import type { TestCaseStatus, TestCaseSummary } from "../../protocol/types";

const execFile = promisify(child_process.execFile);

export type ParsedTestResults = {
  totalTestCount: number;
  passedTests: number;
  failedTests: number;
  skippedTests: number;
  testCases: TestCaseSummary[];
};

/**
 * Parses an `.xcresult` bundle via `xcrun xcresulttool`. Xcode 16 dropped
 * the legacy single-blob format and split it into separate `test-results`
 * subcommands — we use those and fall back gracefully if the tool is
 * absent or the output schema changes.
 *
 * Returns `undefined` when the bundle can't be parsed (missing tool,
 * malformed output, bundle not at expected path). Callers degrade to
 * counts-from-exit-code rather than failing the whole `test` method.
 */
export async function parseXcresultBundle(
  bundlePath: string,
  deps: { logger: Logger },
): Promise<ParsedTestResults | undefined> {
  // The directory `prepareBundleDir` returns is the path xcodebuild is
  // told to write the .xcresult bundle to. Xcode writes the bundle EITHER
  // at that exact path (treated as a bundle path) or at `<path>.xcresult`
  // depending on version — try both.
  const candidate = [bundlePath, `${bundlePath}.xcresult`].find((p) => fs.existsSync(p));
  if (!candidate) {
    deps.logger.warn("xcresult bundle not found", { bundlePath });
    return undefined;
  }

  try {
    const summary = await runXcresulttool(["get", "test-results", "summary", "--path", candidate, "--format", "json"]);
    const summaryObj = JSON.parse(summary) as XcresultSummary;
    const testCases = await fetchTestCases(candidate, deps);

    return {
      totalTestCount: summaryObj.totalTestCount ?? 0,
      passedTests: summaryObj.passedTests ?? 0,
      failedTests: summaryObj.failedTests ?? 0,
      skippedTests: summaryObj.skippedTests ?? 0,
      testCases,
    };
  } catch (error) {
    deps.logger.warn("Failed to parse xcresult bundle", {
      bundlePath: candidate,
      error: error instanceof Error ? error.message : String(error),
    });
    return undefined;
  }
}

async function fetchTestCases(bundlePath: string, deps: { logger: Logger }): Promise<TestCaseSummary[]> {
  try {
    const raw = await runXcresulttool(["get", "test-results", "tests", "--path", bundlePath, "--format", "json"]);
    const parsed = JSON.parse(raw) as XcresultTests;
    return flattenTestCases(parsed);
  } catch (error) {
    // The summary endpoint is the source of truth for counts; per-test
    // detail is a nice-to-have. Log and return empty rather than failing.
    deps.logger.warn("Failed to fetch xcresult test cases", {
      error: error instanceof Error ? error.message : String(error),
    });
    return [];
  }
}

async function runXcresulttool(args: string[]): Promise<string> {
  const { stdout } = await execFile("xcrun", ["xcresulttool", ...args], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  return stdout;
}

// ---------------------------------------------------------------------------
// Loose shape declarations for the bits of xcresulttool's JSON we care about.
// Marked optional liberally so a minor Xcode update can change a field without
// turning every test response into INTERNAL.
// ---------------------------------------------------------------------------

type XcresultSummary = {
  result?: string;
  totalTestCount?: number;
  passedTests?: number;
  failedTests?: number;
  skippedTests?: number;
};

type XcresultTests = {
  devices?: unknown[];
  testNodes?: XcresultTestNode[];
};

type XcresultTestNode = {
  nodeType?: string;
  name?: string;
  identifier?: string;
  result?: string;
  duration?: string;
  children?: XcresultTestNode[];
  failureMessages?: Array<{ message?: string }>;
};

function flattenTestCases(parsed: XcresultTests): TestCaseSummary[] {
  const out: TestCaseSummary[] = [];
  if (!parsed.testNodes) return out;
  for (const node of parsed.testNodes) walkNode(node, out);
  return out;
}

function walkNode(node: XcresultTestNode, out: TestCaseSummary[]): void {
  if (node.nodeType === "Test Case") {
    out.push({
      identifier: node.identifier ?? node.name ?? "<unknown>",
      status: mapStatus(node.result),
      durationMs: parseDuration(node.duration),
      message: node.failureMessages?.[0]?.message,
    });
    return;
  }
  for (const child of node.children ?? []) walkNode(child, out);
}

function mapStatus(result: string | undefined): TestCaseStatus {
  switch (result) {
    case "Passed":
      return "passed";
    case "Skipped":
      return "skipped";
    case "Failed":
    case "Expected Failure":
    case "Mixed":
      return "failed";
    default:
      return "failed";
  }
}

function parseDuration(raw: string | undefined): number | null {
  if (!raw) return null;
  // xcresulttool emits human-readable durations like "0.234s" or "1.2 sec".
  // Strip non-numeric suffixes; if nothing remains, give up.
  const match = /^([0-9]+(?:\.[0-9]+)?)/.exec(raw);
  if (!match) return null;
  const seconds = Number.parseFloat(match[1]);
  return Number.isFinite(seconds) ? Math.round(seconds * 1000) : null;
}
