import * as fs from "node:fs";
import * as path from "node:path";

import type { Logger } from "../core/logger/types";
import type { WireEvent } from "../protocol/types";

/**
 * Append-mode writer for a build's `events.jsonl`. Each event is one JSON
 * line; finished builds get their events replayed from this file when an
 * attach client connects after-the-fact. Errors are swallowed — the disk
 * log is best-effort.
 */
export class EventRecorder implements AsyncDisposable {
  private stream: fs.WriteStream | undefined;
  private constructor(
    private readonly logger: Logger,
    readonly eventsPath: string,
  ) {}

  static open(deps: { logger: Logger; eventsPath: string }): EventRecorder {
    const r = new EventRecorder(deps.logger, deps.eventsPath);
    try {
      fs.mkdirSync(path.dirname(deps.eventsPath), { recursive: true });
      r.stream = fs.createWriteStream(deps.eventsPath, { flags: "w", encoding: "utf8" });
      r.stream.on("error", (error) => {
        deps.logger.warn("EventRecorder stream error", { eventsPath: deps.eventsPath, error });
      });
    } catch (error) {
      deps.logger.warn("Failed to open event recorder", { eventsPath: deps.eventsPath, error });
      r.stream = undefined;
    }
    return r;
  }

  record(event: WireEvent): void {
    if (!this.stream) return;
    this.stream.write(`${JSON.stringify(event)}\n`);
  }

  async [Symbol.asyncDispose](): Promise<void> {
    await this.close();
  }

  async close(): Promise<void> {
    if (!this.stream) return;
    await new Promise<void>((resolve) => {
      this.stream!.end(() => resolve());
    });
    this.stream = undefined;
  }
}

/**
 * Read every recorded event in order. Returns an empty array if the file
 * doesn't exist (e.g. for a build whose server died before events flushed)
 * — callers treat that as "no replay available".
 */
export function readRecordedEvents(eventsPath: string): WireEvent[] {
  if (!fs.existsSync(eventsPath)) return [];
  const raw = fs.readFileSync(eventsPath, "utf8");
  if (raw.length === 0) return [];
  const events: WireEvent[] = [];
  for (const line of raw.split("\n")) {
    if (line === "") continue;
    try {
      events.push(JSON.parse(line) as WireEvent);
    } catch {
      // Corrupt line (likely a torn write from a crash) — skip and continue.
    }
  }
  return events;
}
