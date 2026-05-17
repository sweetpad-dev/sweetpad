import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { ProtocolError } from "../../protocol/errors";
import type {
  DestinationsListResponseData,
  SchemesListResponseData,
  UsageResponseData,
  WireErrorResponse,
  WireSuccessResponse,
} from "../../protocol/types";
import { MethodDispatcher } from "../../server/dispatcher";
import { Listener } from "../../server/listener";
import { parseArgv } from "../argv";
import { runDestinationsCommand } from "./destinations";
import { runSchemesCommand } from "./schemes";
import { runUsageCommand } from "./usage";

/**
 * Drives the discovery CLI commands against a real Unix socket served by stub
 * methods. Validates argv → socket → response → exit-code wiring without
 * hitting xcodebuild or the destinations manager.
 */
describe("discovery CLI commands — E2E against a stub server", () => {
  let socketPath: string;
  let listener: Listener;
  let schemesHandler: (params: unknown) => Promise<SchemesListResponseData>;
  let destinationsHandler: (params: unknown) => Promise<DestinationsListResponseData>;
  let usageHandler: (params: unknown) => Promise<UsageResponseData>;

  beforeEach(async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-discovery-it-"));
    socketPath = path.join(dir, "server.sock");

    const dispatcher = new MethodDispatcher(noopLogger);
    dispatcher.register("schemes.list", {
      description: "test",
      handler: (p) => schemesHandler(p),
    });
    dispatcher.register("destinations.list", {
      description: "test",
      handler: (p) => destinationsHandler(p),
    });
    dispatcher.register("usage", {
      description: "test",
      handler: (p) => usageHandler(p),
    });

    listener = new Listener({ socketPath, dispatcher, logger: noopLogger });
    await listener.listen();
  });

  afterEach(async () => {
    await listener.close();
  });

  function args(flags: Record<string, string | true>): ReturnType<typeof parseArgv> {
    const argv: string[] = [];
    for (const [k, v] of Object.entries(flags)) {
      argv.push(v === true ? `--${k}` : `--${k}=${v}`);
    }
    return parseArgv(argv);
  }

  const env = () => ({ cliEntryDir: "/unused", socketPathOverride: socketPath });

  describe("schemes", () => {
    it("returns exit 0 and the schemes envelope", async () => {
      schemesHandler = vi.fn().mockResolvedValue({
        schemes: [{ name: "App" }],
        xcworkspace: "/p/App.xcworkspace",
      });

      const result = await runSchemesCommand(args({}), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<SchemesListResponseData>;
      expect(envelope.data.schemes).toEqual([{ name: "App" }]);
    });

    it("forwards --xcworkspace to the server", async () => {
      schemesHandler = vi.fn().mockResolvedValue({ schemes: [], xcworkspace: "/x.xcworkspace" });

      await runSchemesCommand(args({ xcworkspace: "/x.xcworkspace" }), env());
      const handler = schemesHandler as unknown as ReturnType<typeof vi.fn>;
      expect(handler.mock.calls[0][0]).toMatchObject({ xcworkspace: "/x.xcworkspace" });
    });

    it("maps WORKSPACE_NOT_DETECTED to exit 2", async () => {
      schemesHandler = vi.fn().mockImplementation(() => {
        throw new ProtocolError("WORKSPACE_NOT_DETECTED", "nope");
      });

      const result = await runSchemesCommand(args({}), env());
      expect(result.exitCode).toBe(2);
      const envelope = result.envelope as WireErrorResponse;
      expect(envelope.error.code).toBe("WORKSPACE_NOT_DETECTED");
    });
  });

  describe("destinations", () => {
    it("returns exit 0 and the destinations envelope", async () => {
      destinationsHandler = vi.fn().mockResolvedValue({
        destinations: [{ id: "macos-1", kind: "macOS", label: "My Mac", platform: "macosx" }],
      });

      const result = await runDestinationsCommand(args({}), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<DestinationsListResponseData>;
      expect(envelope.data.destinations).toHaveLength(1);
    });

    it("forwards --kind and --refresh to the server", async () => {
      destinationsHandler = vi.fn().mockResolvedValue({ destinations: [] });

      await runDestinationsCommand(args({ kind: "iOSSimulator", refresh: true }), env());
      const handler = destinationsHandler as unknown as ReturnType<typeof vi.fn>;
      expect(handler.mock.calls[0][0]).toMatchObject({ kind: "iOSSimulator", refresh: true });
    });

    it("rejects unknown --kind client-side before touching the socket", async () => {
      destinationsHandler = vi.fn();

      await expect(runDestinationsCommand(args({ kind: "bogus" }), env())).rejects.toMatchObject({
        code: "INVALID_ARGUMENT",
      });
      expect(destinationsHandler).not.toHaveBeenCalled();
    });
  });

  describe("usage", () => {
    it("returns exit 0 and the methods list", async () => {
      usageHandler = vi.fn().mockResolvedValue({
        schemaVersion: "1.0",
        methods: [
          { name: "build", description: "Build a scheme" },
          { name: "schemes.list", description: "List schemes" },
        ],
      });

      const result = await runUsageCommand(args({}), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<UsageResponseData>;
      expect(envelope.data.methods.map((m) => m.name)).toEqual(["build", "schemes.list"]);
    });
  });
});
