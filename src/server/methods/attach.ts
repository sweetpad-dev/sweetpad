import type * as net from "node:net";

import type { Logger } from "../../core/logger/types";
import { errorResponse } from "../../protocol/envelope";
import { ProtocolError } from "../../protocol/errors";
import { encodeMessage } from "../../protocol/framing";
import {
  type AttachCompleteEventData,
  type AttachRequestParams,
  SCHEMA_VERSION,
  type WireEvent,
  type WireRequest,
} from "../../protocol/types";
import { readRecordedEvents } from "../event-recorder";
import type { EventBus } from "../event-bus";
import type { BuildRegistry } from "../registry";

export type AttachHandlerDeps = {
  registry: BuildRegistry;
  eventBus: EventBus;
  logger: Logger;
};

/**
 * Listener handler for streaming `attach` connections. Doesn't go through
 * the dispatcher: it owns the socket lifecycle for the duration of the
 * stream and only ever closes once events have stopped flowing.
 *
 * Wire shape per the protocol contract:
 *  - On validation error: 1 WireResponse (ok:false), then close.
 *  - On success: 0+ WireEvents (`build.started` if recovered live, every
 *    `log.line`, a `build.finished`, finally an `attach.complete`),
 *    then close.
 */
export function createAttachHandler(deps: AttachHandlerDeps) {
  return async function handleAttach(socket: net.Socket, request: WireRequest): Promise<void> {
    let params: AttachRequestParams;
    try {
      params = validateParams(request.params);
    } catch (error) {
      writeErrorAndClose(socket, request.id, error);
      return;
    }

    const build = deps.registry.get(params.buildId);
    if (!build) {
      writeErrorAndClose(
        socket,
        request.id,
        new ProtocolError("BUILD_NOT_FOUND", `No build with id '${params.buildId}'`, {
          hint: "sweetpad builds — list everything in the registry",
        }),
      );
      return;
    }

    const replay = params.replay ?? true;

    // Re-fetch the build before deciding live vs replay: a finish() that
    // raced with our get() above would have updated the snapshot already.
    const isRunning = deps.registry.get(params.buildId)?.status === "running";

    if (isRunning) {
      await streamLive(socket, deps, params.buildId);
    } else if (replay) {
      await streamReplay(socket, deps, params.buildId);
    } else {
      writeEvent(socket, completeEvent(params.buildId, "closed"));
    }

    socket.end();
  };
}

async function streamLive(socket: net.Socket, deps: AttachHandlerDeps, buildId: string): Promise<void> {
  // The live attach subscribes to the event bus and forwards every event
  // to the socket until `build.finished` arrives. The build method emits
  // events synchronously (each output line → emit), so a backpressured
  // socket would buffer in Node — acceptable; the build is short-lived.
  return await new Promise<void>((resolve) => {
    let unsubscribe: (() => void) | undefined;
    let resolved = false;
    const finish = (reason: AttachCompleteEventData["reason"]) => {
      if (resolved) return;
      resolved = true;
      unsubscribe?.();
      writeEvent(socket, completeEvent(buildId, reason));
      resolve();
    };

    unsubscribe = deps.eventBus.subscribe(buildId, (event) => {
      writeEvent(socket, event);
      if (event.event === "build.finished") {
        finish("build.finished");
      }
    });

    // The client going away mid-stream tears the subscription down — the
    // build itself keeps running, the registry/log/events are still
    // recorded for a later `attach` or `show`.
    socket.once("close", () => finish("closed"));
    socket.once("error", () => finish("closed"));

    // Belt-and-suspenders: the build could have transitioned to finished
    // between our `isRunning` check and the subscribe() above. If so, the
    // bus will never emit again — recover by checking and exiting now.
    const fresh = deps.registry.get(buildId);
    if (fresh && fresh.status !== "running") {
      finish("build.finished");
    }
  });
}

async function streamReplay(socket: net.Socket, deps: AttachHandlerDeps, buildId: string): Promise<void> {
  const events = readRecordedEvents(deps.registry.getEventsPath(buildId));
  for (const event of events) {
    writeEvent(socket, event);
  }
  writeEvent(socket, completeEvent(buildId, "replay.complete"));
}

function completeEvent(buildId: string, reason: AttachCompleteEventData["reason"]): WireEvent<AttachCompleteEventData> {
  return {
    event: "attach.complete",
    schemaVersion: SCHEMA_VERSION,
    ts: new Date().toISOString(),
    buildId,
    data: { reason },
  };
}

function writeEvent(socket: net.Socket, event: WireEvent): void {
  if (!socket.writable) return;
  socket.write(encodeMessage(event));
}

function writeErrorAndClose(socket: net.Socket, id: number, error: unknown): void {
  if (!socket.writable) return;
  const protocolError = error instanceof ProtocolError ? error : new ProtocolError("INTERNAL", String(error));
  const envelope = errorResponse(id, protocolError.toPayload(), protocolError.extra);
  socket.write(encodeMessage(envelope));
  socket.end();
}

function validateParams(raw: unknown): AttachRequestParams {
  if (!raw || typeof raw !== "object") {
    throw new ProtocolError("INVALID_ARGUMENT", "Missing attach params");
  }
  const params = raw as Partial<AttachRequestParams>;
  if (typeof params.buildId !== "string" || params.buildId.length === 0) {
    throw new ProtocolError("INVALID_ARGUMENT", "'buildId' is required and must be a non-empty string");
  }
  if (params.replay !== undefined && typeof params.replay !== "boolean") {
    throw new ProtocolError("INVALID_ARGUMENT", "'replay' must be a boolean");
  }
  return { buildId: params.buildId, replay: params.replay };
}
