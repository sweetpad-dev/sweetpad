import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../core/logger/types";
import type { WireEvent } from "../protocol/types";
import { EventRecorder, readRecordedEvents } from "./event-recorder";

function makeEvent(buildId: string, line: string): WireEvent {
  return {
    event: "log.line",
    schemaVersion: "1.0",
    ts: "2026-01-01T00:00:00.000Z",
    buildId,
    data: { line },
  };
}

describe("EventRecorder + readRecordedEvents", () => {
  let dir: string;
  let eventsPath: string;

  beforeEach(() => {
    dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-recorder-"));
    eventsPath = path.join(dir, "events.jsonl");
  });

  afterEach(() => {
    fs.rmSync(dir, { recursive: true, force: true });
  });

  it("writes each event as one JSON line, read back identically", async () => {
    const recorder = EventRecorder.open({ logger: noopLogger, eventsPath });
    recorder.record(makeEvent("b1", "one"));
    recorder.record(makeEvent("b1", "two"));
    await recorder.close();

    const events = readRecordedEvents(eventsPath);
    expect(events).toHaveLength(2);
    expect((events[0].data as { line: string }).line).toBe("one");
  });

  it("returns [] when the file doesn't exist", () => {
    expect(readRecordedEvents(eventsPath)).toEqual([]);
  });

  it("skips a torn final line without throwing", async () => {
    const recorder = EventRecorder.open({ logger: noopLogger, eventsPath });
    recorder.record(makeEvent("b1", "good"));
    await recorder.close();
    fs.appendFileSync(eventsPath, "{not valid json"); // simulate crash mid-write

    const events = readRecordedEvents(eventsPath);
    expect(events).toHaveLength(1);
  });

  it("supports `await using` for auto-close", async () => {
    async function run() {
      await using r = EventRecorder.open({ logger: noopLogger, eventsPath });
      r.record(makeEvent("b1", "auto"));
    }
    await run();
    expect(readRecordedEvents(eventsPath)).toHaveLength(1);
  });
});
