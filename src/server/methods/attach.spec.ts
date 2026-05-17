import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { ProtocolClient } from "../../cli/protocol";
import type { BuildEvent } from "../../protocol/types";
import { MethodDispatcher } from "../dispatcher";
import { EventBus } from "../event-bus";
import { EventRecorder } from "../event-recorder";
import { Listener } from "../listener";
import { BuildRegistry } from "../registry";
import { createAttachHandler } from "./attach";

/**
 * Drives `attach` through the real wire: net.Server + framer + ProtocolClient.
 * The build method is not invoked here; instead the registry is seeded
 * directly so we can exercise live vs replay vs not-found paths in isolation.
 */
describe("attach handler — E2E", () => {
  let socketPath: string;
  let listener: Listener;
  let registry: BuildRegistry;
  let eventBus: EventBus;
  let buildsDir: string;

  beforeEach(async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-attach-"));
    socketPath = path.join(dir, "server.sock");
    buildsDir = path.join(dir, "builds");

    registry = new BuildRegistry({ buildsDir, logger: noopLogger });
    eventBus = new EventBus();

    const attachHandler = createAttachHandler({ registry, eventBus, logger: noopLogger });
    listener = new Listener({
      socketPath,
      dispatcher: new MethodDispatcher(noopLogger),
      logger: noopLogger,
      streamingHandlers: { attach: attachHandler },
    });
    await listener.listen();
  });

  afterEach(async () => {
    await listener.close();
    fs.rmSync(path.dirname(socketPath), { recursive: true, force: true });
  });

  it("returns BUILD_NOT_FOUND for an unknown buildId", async () => {
    const client = await ProtocolClient.connect(socketPath);
    const events: BuildEvent[] = [];
    try {
      const errorEnvelope = await client.attach({ buildId: "bX" }, (e) => events.push(e));
      expect(errorEnvelope).not.toBeNull();
      expect(errorEnvelope?.ok).toBe(false);
      if (errorEnvelope?.ok === false) {
        expect(errorEnvelope.error.code).toBe("BUILD_NOT_FOUND");
      }
      expect(events).toHaveLength(0);
    } finally {
      client.close();
    }
  });

  it("replays recorded events for a finished build", async () => {
    // Seed: a finished build with two recorded events.
    const b = registry.start({ scheme: "S", destination: "D", configuration: "Debug", originator: "cli" });
    const recorder = EventRecorder.open({ logger: noopLogger, eventsPath: registry.getEventsPath(b.buildId) });
    recorder.record({
      event: "log.line",
      schemaVersion: "1.0",
      ts: "t0",
      buildId: b.buildId,
      data: { line: "one" },
    });
    recorder.record({
      event: "log.line",
      schemaVersion: "1.0",
      ts: "t1",
      buildId: b.buildId,
      data: { line: "two" },
    });
    await recorder.close();
    registry.finish(b.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });

    const client = await ProtocolClient.connect(socketPath);
    const events: BuildEvent[] = [];
    try {
      const err = await client.attach({ buildId: b.buildId }, (e) => events.push(e));
      expect(err).toBeNull();
    } finally {
      client.close();
    }

    expect(events.map((e) => e.event)).toEqual(["log.line", "log.line", "attach.complete"]);
    expect((events[0].data as { line: string }).line).toBe("one");
    if (events[2].event === "attach.complete") {
      expect(events[2].data.reason).toBe("replay.complete");
    }
  });

  it("with --no-replay (replay=false), emits only attach.complete for finished builds", async () => {
    const b = registry.start({ scheme: "S", destination: "D", configuration: "Debug", originator: "cli" });
    registry.finish(b.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] });

    const client = await ProtocolClient.connect(socketPath);
    const events: BuildEvent[] = [];
    try {
      const err = await client.attach({ buildId: b.buildId, replay: false }, (e) => events.push(e));
      expect(err).toBeNull();
    } finally {
      client.close();
    }

    expect(events.map((e) => e.event)).toEqual(["attach.complete"]);
    if (events[0].event === "attach.complete") {
      expect(events[0].data.reason).toBe("closed");
    }
  });

  it("streams live events for a running build, until build.finished", async () => {
    const b = registry.start({ scheme: "S", destination: "D", configuration: "Debug", originator: "cli" });

    const client = await ProtocolClient.connect(socketPath);
    const events: BuildEvent[] = [];
    const attached = client.attach({ buildId: b.buildId }, (e) => events.push(e));

    // Give the server a tick to subscribe before emitting.
    await new Promise((r) => setTimeout(r, 20));

    eventBus.emit(b.buildId, {
      event: "log.line",
      schemaVersion: "1.0",
      ts: "t0",
      buildId: b.buildId,
      data: { line: "running line" },
    });
    eventBus.emit(b.buildId, {
      event: "build.finished",
      schemaVersion: "1.0",
      ts: "t1",
      buildId: b.buildId,
      data: { build: registry.finish(b.buildId, { status: "succeeded", exitCode: 0, diagnostics: [] }) },
    });

    const err = await attached;
    client.close();

    expect(err).toBeNull();
    expect(events.map((e) => e.event)).toEqual(["log.line", "build.finished", "attach.complete"]);
    if (events[2].event === "attach.complete") {
      expect(events[2].data.reason).toBe("build.finished");
    }
  });

  it("rejects missing buildId with INVALID_ARGUMENT", async () => {
    const client = await ProtocolClient.connect(socketPath);
    try {
      const err = await client.attach({} as never, () => {});
      expect(err?.ok).toBe(false);
      if (err?.ok === false) expect(err.error.code).toBe("INVALID_ARGUMENT");
    } finally {
      client.close();
    }
  });
});
