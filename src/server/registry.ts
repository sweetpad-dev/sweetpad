import type { ParsedDiagnostic } from "../core/build/diagnostics-parser";
import type { BuildResponseData, BuildStatus } from "../protocol/types";

/**
 * In-memory build registry. Allocates monotonic `b1`, `b2`, ... IDs and tracks
 * each build's lifecycle (start, finish, diagnostics). v1 has no disk
 * persistence — a server restart wipes history. Disk-backed registry comes
 * in a follow-up.
 */
export class BuildRegistry {
  private nextId = 1;
  private readonly builds = new Map<string, BuildResponseData>();

  start(spec: {
    scheme: string;
    destination: string;
    configuration: string;
    originator: BuildResponseData["originator"];
  }): BuildResponseData {
    const buildId = `b${this.nextId++}`;
    const startedAt = new Date().toISOString();
    const build: BuildResponseData = {
      buildId,
      scheme: spec.scheme,
      destination: spec.destination,
      config: spec.configuration,
      command: "build",
      status: "running",
      exitCode: null,
      originator: spec.originator,
      startedAt,
      finishedAt: null,
      durationMs: null,
      errorCount: 0,
      warningCount: 0,
      diagnostics: [],
    };
    this.builds.set(buildId, build);
    return build;
  }

  finish(
    buildId: string,
    result: { status: BuildStatus; exitCode: number | null; diagnostics: ParsedDiagnostic[] },
  ): BuildResponseData {
    const build = this.builds.get(buildId);
    if (!build) {
      throw new Error(`Internal: BuildRegistry.finish called for unknown build ${buildId}`);
    }
    const finishedAt = new Date().toISOString();
    const durationMs = new Date(finishedAt).getTime() - new Date(build.startedAt).getTime();

    let errors = 0;
    let warnings = 0;
    for (const d of result.diagnostics) {
      if (d.severity === "error") errors++;
      else if (d.severity === "warning") warnings++;
    }

    const updated: BuildResponseData = {
      ...build,
      status: result.status,
      exitCode: result.exitCode,
      finishedAt,
      durationMs,
      errorCount: errors,
      warningCount: warnings,
      diagnostics: result.diagnostics.map((d) => ({
        file: d.file,
        line: d.line,
        column: d.column,
        severity: d.severity,
        message: d.message,
        source: d.source,
      })),
    };
    this.builds.set(buildId, updated);
    return updated;
  }

  get(buildId: string): BuildResponseData | undefined {
    return this.builds.get(buildId);
  }

  running(): BuildResponseData[] {
    const out: BuildResponseData[] = [];
    for (const b of this.builds.values()) {
      if (b.status === "running") out.push(b);
    }
    return out;
  }
}
