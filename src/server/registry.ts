import * as fs from "node:fs";
import * as path from "node:path";

import type { ParsedDiagnostic } from "../core/build/diagnostics-parser";
import type { Logger } from "../core/logger/types";
import type { BuildResponseData, BuildStatus } from "../protocol/types";

export type BuildRegistryDeps = {
  /** Directory builds are persisted under, e.g. `<workspace>/.sweetpad/builds`. */
  buildsDir: string;
  logger: Logger;
};

/**
 * Per-build snapshot is written to `<buildsDir>/<buildId>/snapshot.json` on
 * start, then overwritten on finish. The on-disk layout is one directory
 * per build so log.txt and events.jsonl can sit alongside without
 * colliding.
 */
const SNAPSHOT_FILENAME = "snapshot.json";
const LOG_FILENAME = "log.txt";
const EVENTS_FILENAME = "events.jsonl";

/**
 * Build registry backed by `<buildsDir>/<buildId>/snapshot.json`. Allocates
 * monotonic `b1`, `b2`, ... IDs. On construction, `recover()` reads the
 * existing snapshots so a server restart doesn't lose history and the next
 * ID continues past the highest persisted one.
 *
 * Any build still tagged `running` at recovery time is fixed up to
 * `interrupted` — the only way that state survives a restart is if the
 * server that owned it died, since the workspace lockfile prevents
 * concurrent servers.
 */
export class BuildRegistry {
  private nextId = 1;
  private readonly builds = new Map<string, BuildResponseData>();

  constructor(private readonly deps: BuildRegistryDeps) {}

  recover(): void {
    if (!fs.existsSync(this.deps.buildsDir)) return;

    let entries: fs.Dirent[];
    try {
      entries = fs.readdirSync(this.deps.buildsDir, { withFileTypes: true });
    } catch (error) {
      this.deps.logger.warn("Failed to read builds directory", { dir: this.deps.buildsDir, error });
      return;
    }

    let maxId = 0;
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      const snapshotPath = path.join(this.deps.buildsDir, entry.name, SNAPSHOT_FILENAME);
      const build = this.readSnapshot(snapshotPath);
      if (!build) continue;

      // Fix-up running snapshots before exposing them. Without this, a
      // restart would surface BUILD_IN_PROGRESS for a build that nobody is
      // running anymore.
      if (build.status === "running") {
        const fixed: BuildResponseData = {
          ...build,
          status: "interrupted",
          finishedAt: build.finishedAt ?? new Date().toISOString(),
          exitCode: build.exitCode ?? null,
        };
        if (fixed.durationMs === null && fixed.finishedAt) {
          fixed.durationMs = new Date(fixed.finishedAt).getTime() - new Date(build.startedAt).getTime();
        }
        this.persist(fixed);
        this.builds.set(fixed.buildId, fixed);
      } else {
        this.builds.set(build.buildId, build);
      }

      const idNum = parseBuildIdNumber(build.buildId);
      if (idNum !== null && idNum > maxId) maxId = idNum;
    }
    this.nextId = maxId + 1;
  }

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
    this.persist(build);
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
    this.persist(updated);
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

  list(): BuildResponseData[] {
    return Array.from(this.builds.values()).sort((a, b) => {
      const aN = parseBuildIdNumber(a.buildId);
      const bN = parseBuildIdNumber(b.buildId);
      return (aN ?? 0) - (bN ?? 0);
    });
  }

  getBuildDir(buildId: string): string {
    return path.join(this.deps.buildsDir, buildId);
  }

  getLogPath(buildId: string): string {
    return path.join(this.deps.buildsDir, buildId, LOG_FILENAME);
  }

  getEventsPath(buildId: string): string {
    return path.join(this.deps.buildsDir, buildId, EVENTS_FILENAME);
  }

  private persist(build: BuildResponseData): void {
    const dir = path.join(this.deps.buildsDir, build.buildId);
    try {
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(path.join(dir, SNAPSHOT_FILENAME), `${JSON.stringify(build, null, 2)}\n`);
    } catch (error) {
      this.deps.logger.warn("Failed to persist build snapshot", {
        buildId: build.buildId,
        dir,
        error,
      });
    }
  }

  private readSnapshot(snapshotPath: string): BuildResponseData | undefined {
    try {
      if (!fs.existsSync(snapshotPath)) return undefined;
      const raw = fs.readFileSync(snapshotPath, "utf8");
      const parsed = JSON.parse(raw) as BuildResponseData;
      if (typeof parsed.buildId !== "string") return undefined;
      return parsed;
    } catch (error) {
      this.deps.logger.warn("Failed to read build snapshot", { snapshotPath, error });
      return undefined;
    }
  }
}

function parseBuildIdNumber(buildId: string): number | null {
  if (!buildId.startsWith("b")) return null;
  const n = Number.parseInt(buildId.slice(1), 10);
  return Number.isFinite(n) ? n : null;
}
