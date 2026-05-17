import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { noopLogger } from "../core/logger/types";
import { ProtocolClient } from "../cli/protocol";
import { ProtocolError } from "../protocol/errors";
import type { WireResponse, WireSuccessResponse } from "../protocol/types";
import { MethodDispatcher } from "./dispatcher";
import { Listener } from "./listener";

// The dispatcher's `register` and the client's `request` are statically typed
// against the production `MethodMap`. These tests exercise the wire path with
// synthetic methods that aren't in the map; the casts here are an intentional
// test-only escape from the static contract.
type AnyDispatcher = {
  register: (
    method: string,
    options: { description: string; handler: (params: unknown) => Promise<unknown> },
  ) => void;
};
type AnyClient = {
  request: <T = unknown>(method: string, params: Record<string, unknown>) => Promise<WireResponse<T>>;
  close: () => void;
};

/**
 * Round-trips a request through the same wire path the CLI/server use: a real
 * Unix socket, real framing, real dispatcher. Method handlers are stubs so we
 * don't shell out to xcodebuild — what we're validating is the envelope shape
 * and error mapping.
 */
describe("server wire protocol — integration", () => {
  let socketPath: string;
  let listener: Listener;

  beforeEach(async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-it-"));
    socketPath = path.join(dir, "server.sock");

    const dispatcher = new MethodDispatcher(noopLogger);
    const anyDispatcher = dispatcher as unknown as AnyDispatcher;
    anyDispatcher.register("echo", {
      description: "test: echoes the value back",
      handler: async (params) => ({ echoed: (params as { value: unknown }).value }),
    });
    anyDispatcher.register("explode", {
      description: "test: always throws",
      handler: async () => {
        throw new ProtocolError("BUILD_FAILED", "boom", {
          hint: "sweetpad attach b1",
          extra: { running: [{ buildId: "b1" }] },
        });
      },
    });

    listener = new Listener({ socketPath, dispatcher, logger: noopLogger });
    await listener.listen();
  });

  afterEach(async () => {
    await listener.close();
  });

  it("returns the envelope for a successful method call", async () => {
    const client = (await ProtocolClient.connect(socketPath)) as unknown as AnyClient;
    try {
      const response = await client.request<{ echoed: string }>("echo", { value: "ping" });
      expect(response.ok).toBe(true);
      const data = (response as WireSuccessResponse<{ echoed: string }>).data;
      expect(data.echoed).toBe("ping");
      expect(response.schemaVersion).toBe("1.0");
    } finally {
      client.close();
    }
  });

  it("maps ProtocolError → error envelope with code + hint + extras", async () => {
    const client = (await ProtocolClient.connect(socketPath)) as unknown as AnyClient;
    try {
      const response = await client.request("explode", {});
      expect(response.ok).toBe(false);
      if (response.ok) throw new Error("unreachable");
      expect(response.error.code).toBe("BUILD_FAILED");
      expect(response.error.hint).toBe("sweetpad attach b1");
      expect(response.running).toEqual([{ buildId: "b1" }]);
    } finally {
      client.close();
    }
  });

  it("returns INVALID_ARGUMENT for unknown methods", async () => {
    const client = (await ProtocolClient.connect(socketPath)) as unknown as AnyClient;
    try {
      const response = await client.request("doesNotExist", {});
      expect(response.ok).toBe(false);
      if (response.ok) throw new Error("unreachable");
      expect(response.error.code).toBe("INVALID_ARGUMENT");
    } finally {
      client.close();
    }
  });
});
