import { EventEmitter } from "node:events";
import { promises as fs } from "node:fs";
import * as path from "node:path";

import type * as vscode from "vscode";

import type { ParsedDiagnostic } from "../build/diagnostics-parser";
import type { BuildManager, BuildSessionCommand, BuildSessionEnded, BuildSessionStarted } from "../build/manager";
import { commonLogger } from "../common/logger";
import type { DestinationsManager } from "../destination/manager";
import { ensureDir, getBuildDir, getBuildsDir } from "./paths";
import {
  type BuildCommand,
  type BuildEntity,
  type BuildOriginator,
  type BuildStatus,
  type DiagnosticEntity,
} from "./types";

const RETENTION = 10;
// Capped low so build.wait doesn't freeze the agent's tool-use loop; callers
// poll on their own cadence for anything longer.
const WAIT_DEFAULT_TIMEOUT_MS = 10 * 1000;
const WAIT_MAX_TIMEOUT_MS = 30 * 1000;

type SnapshotFile = BuildEntity & {
  diagnostics: DiagnosticEntity[];
};

type ActiveSession = {
  buildId: string;
  command: BuildCommand;
  scheme: string;
  configuration: string | null;
  destination: string | null;
  originator: BuildOriginator;
  caller: string | null;
  startedAt: Date;
  logChunks: string[];
  diagnostics: ParsedDiagnostic[];
  errorCount: number;
  warningCount: number;
};

type PendingClaim = {
  buildId: string;
  originator: BuildOriginator;
  caller: string | null;
  reservedAt: Date;
};

// Per-workspace registry of build sessions. Only one build runs at a time
// (BuildManager.runSchemeTask holds the "sweetpad.build" lock with
// terminateLocked: true), so there's at most one ActiveSession in memory.
export class BuildSessionRegistry implements vscode.Disposable {
  private readonly workspacePath: string;
  private readonly buildManager: BuildManager;
  private readonly destinationsManager: DestinationsManager;

  private nextSeq = 1;
  private current: ActiveSession | undefined;
  private pendingClaim: PendingClaim | undefined;
  private builds = new Map<string, BuildEntity>();
  private waitEmitter = new EventEmitter();
  private disposers: Array<() => void> = [];
  // Chained tail of all session-end I/O so tests can await full drainage.
  private pending: Promise<void> = Promise.resolve();

  constructor(options: {
    workspacePath: string;
    buildManager: BuildManager;
    destinationsManager: DestinationsManager;
  }) {
    this.workspacePath = options.workspacePath;
    this.buildManager = options.buildManager;
    this.destinationsManager = options.destinationsManager;
    this.waitEmitter.setMaxListeners(0);
  }

  async start(): Promise<void> {
    await this.loadPersisted();
    this.subscribe();
  }

  dispose(): void {
    for (const d of this.disposers) d();
    this.disposers = [];
    this.waitEmitter.removeAllListeners();
  }

  async flushPending(): Promise<void> {
    await this.pending;
  }

  // Pre-allocates a buildId so build.start can return one before
  // BuildManager has actually started the underlying scheme task.
  reserveCliBuildId(options?: { caller?: string | null }): string {
    const id = this.allocateId();
    this.pendingClaim = {
      buildId: id,
      originator: "cli",
      caller: options?.caller ?? null,
      reservedAt: new Date(),
    };
    return id;
  }

  getBuild(buildId: string): BuildEntity | undefined {
    return this.builds.get(buildId);
  }

  listBuilds(limit = RETENTION): BuildEntity[] {
    return [...this.builds.values()].toSorted((a, b) => parseSeq(b.buildId) - parseSeq(a.buildId)).slice(0, limit);
  }

  getLatest(): BuildEntity | undefined {
    return this.listBuilds(1)[0];
  }

  // On timeout, resolves with the still-running entity — never throws — so
  // the agent keeps control of its own scheduling.
  async waitForBuild(buildId: string, timeoutMs: number = WAIT_DEFAULT_TIMEOUT_MS): Promise<BuildEntity> {
    const capped = Math.min(Math.max(0, timeoutMs), WAIT_MAX_TIMEOUT_MS);
    const existing = this.builds.get(buildId);
    if (existing && existing.status !== "running") {
      return existing;
    }
    return await new Promise<BuildEntity>((resolve) => {
      const eventName = `finished:${buildId}`;
      const onFinished = (entity: BuildEntity) => {
        clearTimeout(timer);
        resolve(entity);
      };
      const timer = setTimeout(() => {
        this.waitEmitter.off(eventName, onFinished);
        resolve(this.builds.get(buildId) ?? existing!);
      }, capped);
      timer.unref?.();
      this.waitEmitter.once(eventName, onFinished);
    });
  }

  async readLog(buildId: string): Promise<string> {
    if (this.current?.buildId === buildId) {
      return this.current.logChunks.join("\n");
    }
    const logPath = path.join(getBuildDir(this.workspacePath, buildId), "build.log");
    try {
      return await fs.readFile(logPath, "utf8");
    } catch {
      return "";
    }
  }

  async readDiagnostics(buildId: string): Promise<DiagnosticEntity[]> {
    if (this.current?.buildId === buildId) {
      return this.current.diagnostics.map(toDiagnosticEntity);
    }
    const snapshotPath = path.join(getBuildDir(this.workspacePath, buildId), "snapshot.json");
    try {
      const raw = await fs.readFile(snapshotPath, "utf8");
      const snap = JSON.parse(raw) as SnapshotFile;
      return snap.diagnostics ?? [];
    } catch {
      return [];
    }
  }

  private subscribe(): void {
    const onStarted = (info: BuildSessionStarted) => this.handleStarted(info);
    const onLog = (info: { line: string; diagnostic: ParsedDiagnostic | null }) => this.handleLogLine(info);
    const onEnded = (info: BuildSessionEnded) => {
      this.pending = this.pending
        .then(() => this.handleEnded(info))
        .catch((err) => {
          commonLogger.error("BuildSessionRegistry.handleEnded failed", { error: err });
        });
    };

    this.buildManager.on("buildSessionStarted", onStarted);
    this.buildManager.on("buildLogLine", onLog);
    this.buildManager.on("buildSessionEnded", onEnded);

    this.disposers.push(() => {
      this.buildManager.off("buildSessionStarted", onStarted);
      this.buildManager.off("buildLogLine", onLog);
      this.buildManager.off("buildSessionEnded", onEnded);
    });
  }

  private handleStarted(info: BuildSessionStarted): void {
    let buildId: string;
    let originator: BuildOriginator;
    let caller: string | null;
    if (this.pendingClaim) {
      buildId = this.pendingClaim.buildId;
      originator = this.pendingClaim.originator;
      caller = this.pendingClaim.caller;
      this.pendingClaim = undefined;
    } else {
      buildId = this.allocateId();
      originator = "vscode";
      caller = null;
    }

    const startedAt = new Date();
    const command = sessionCommandToBuildCommand(info.command);
    const configuration = this.buildManager.getDefaultConfigurationForBuild() ?? null;
    const destination = this.destinationsManager.getSelectedXcodeDestinationForBuild()?.name ?? null;

    this.current = {
      buildId,
      command,
      scheme: info.scheme,
      configuration,
      destination,
      originator,
      caller,
      startedAt,
      logChunks: [],
      diagnostics: [],
      errorCount: 0,
      warningCount: 0,
    };
    this.builds.set(buildId, this.snapshot(this.current, "running", null));
  }

  private handleLogLine(info: { line: string; diagnostic: ParsedDiagnostic | null }): void {
    const session = this.current;
    if (!session) return;
    session.logChunks.push(info.line);
    const parsed = info.diagnostic;
    if (!parsed) return;
    session.diagnostics.push(parsed);
    if (parsed.severity === "error") session.errorCount += 1;
    else session.warningCount += 1;
  }

  private async handleEnded(info: BuildSessionEnded): Promise<void> {
    const session = this.current;
    this.current = undefined;
    if (!session) return;

    const status: BuildStatus = info.status;
    const finishedAt = new Date();
    const entity = this.snapshot(session, status, finishedAt);
    this.builds.set(session.buildId, entity);

    try {
      await this.persistSession(session, entity);
    } catch (err) {
      commonLogger.error("Failed to persist build session", {
        buildId: session.buildId,
        error: err,
      });
    }

    try {
      await this.enforceRetention();
    } catch (err) {
      commonLogger.debug("Build retention sweep failed", { error: err });
    }

    this.waitEmitter.emit(`finished:${session.buildId}`, entity);
  }

  private snapshot(session: ActiveSession, status: BuildStatus, finishedAt: Date | null): BuildEntity {
    return {
      buildId: session.buildId,
      command: session.command,
      scheme: session.scheme,
      configuration: session.configuration,
      destination: session.destination,
      status,
      originator: session.originator,
      caller: session.caller,
      startedAt: session.startedAt.toISOString(),
      finishedAt: finishedAt ? finishedAt.toISOString() : null,
      durationMs: finishedAt ? finishedAt.getTime() - session.startedAt.getTime() : null,
      errorCount: session.errorCount,
      warningCount: session.warningCount,
    };
  }

  private async persistSession(session: ActiveSession, entity: BuildEntity): Promise<void> {
    const dir = getBuildDir(this.workspacePath, session.buildId);
    await ensureDir(dir);
    const snapshot: SnapshotFile = {
      ...entity,
      diagnostics: session.diagnostics.map(toDiagnosticEntity),
    };
    await fs.writeFile(path.join(dir, "snapshot.json"), JSON.stringify(snapshot, null, 2));
    await fs.writeFile(path.join(dir, "build.log"), session.logChunks.join("\n"));
  }

  private async loadPersisted(): Promise<void> {
    const buildsDir = getBuildsDir(this.workspacePath);
    let entries: string[];
    try {
      entries = await fs.readdir(buildsDir);
    } catch (err) {
      const code = (err as NodeJS.ErrnoException)?.code;
      if (code === "ENOENT") return;
      throw err;
    }

    const numeric: number[] = [];
    for (const entry of entries) {
      const seq = parseSeq(entry);
      if (!Number.isFinite(seq)) continue;
      numeric.push(seq);
      try {
        const raw = await fs.readFile(path.join(buildsDir, entry, "snapshot.json"), "utf8");
        const snap = JSON.parse(raw) as SnapshotFile;
        // Diagnostics live on disk only; readDiagnostics fetches them lazily.
        const { diagnostics: _diags, ...entity } = snap;
        this.builds.set(entity.buildId, entity);
      } catch {
        // Corrupted snapshot; ignore but still count it for nextSeq.
      }
    }
    if (numeric.length > 0) {
      this.nextSeq = Math.max(...numeric) + 1;
    }
  }

  private async enforceRetention(): Promise<void> {
    const all = [...this.builds.values()]
      .filter((b) => b.status !== "running")
      .toSorted((a, b) => parseSeq(a.buildId) - parseSeq(b.buildId));
    const drop = all.length - RETENTION;
    if (drop <= 0) return;
    const victims = all.slice(0, drop);
    const buildsDir = getBuildsDir(this.workspacePath);
    for (const v of victims) {
      this.builds.delete(v.buildId);
      try {
        await fs.rm(path.join(buildsDir, v.buildId), { recursive: true, force: true });
      } catch (err) {
        commonLogger.debug("Failed to evict build dir", { buildId: v.buildId, error: err });
      }
    }
  }

  private allocateId(): string {
    const id = `b${this.nextSeq}`;
    this.nextSeq += 1;
    return id;
  }
}

function sessionCommandToBuildCommand(c: BuildSessionCommand): BuildCommand {
  if (c === "resolve-deps") return "build";
  return c;
}

function toDiagnosticEntity(p: ParsedDiagnostic): DiagnosticEntity {
  return {
    file: p.file,
    line: p.line,
    column: p.column,
    severity: p.severity,
    message: p.message,
  };
}

function parseSeq(buildId: string): number {
  const m = /^b(\d+)$/.exec(buildId);
  if (!m) return Number.NaN;
  return Number.parseInt(m[1], 10);
}
