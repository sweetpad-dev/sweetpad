import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { ProtocolError } from "../../protocol/errors";
import type {
  BuildResponseData,
  BuildsListResponseData,
  WireErrorResponse,
  WireSuccessResponse,
} from "../../protocol/types";
import { MethodDispatcher } from "../../server/dispatcher";
import { Listener } from "../../server/listener";
import { parseArgv } from "../argv";
import { runBuildsCommand } from "./builds";
import { runErrorsCommand } from "./errors";
import { runShowCommand } from "./show";

/**
 * E2E for the read-only inspect commands: builds.list / build.get on the
 * server side, runBuildsCommand / runShowCommand / runErrorsCommand on the
 * CLI side. Wires both halves with stubs over a real Unix socket.
 */
describe("inspect CLI commands — E2E against a stub server", () => {
  let socketPath: string;
  let listener: Listener;
  let buildsListHandler: (params: unknown) => Promise<BuildsListResponseData>;
  let buildGetHandler: (params: unknown) => Promise<BuildResponseData>;

  beforeEach(async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-inspect-it-"));
    socketPath = path.join(dir, "server.sock");

    const dispatcher = new MethodDispatcher(noopLogger);
    dispatcher.register("builds.list", {
      description: "test",
      handler: (p) => buildsListHandler(p),
    });
    dispatcher.register("build.get", {
      description: "test",
      handler: (p) => buildGetHandler(p),
    });

    listener = new Listener({ socketPath, dispatcher, logger: noopLogger });
    await listener.listen();
  });

  afterEach(async () => {
    await listener.close();
  });

  function args(flags: Record<string, string>, positional: string[] = []): ReturnType<typeof parseArgv> {
    const argv: string[] = [...positional];
    for (const [k, v] of Object.entries(flags)) argv.push(`--${k}=${v}`);
    return parseArgv(argv);
  }

  const env = () => ({ cliEntryDir: "/unused", socketPathOverride: socketPath });

  describe("builds", () => {
    it("returns exit 0 + builds envelope", async () => {
      buildsListHandler = vi.fn().mockResolvedValue({ builds: [makeBuild({ buildId: "b1" })] });

      const result = await runBuildsCommand(args({}), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<BuildsListResponseData>;
      expect(envelope.data.builds[0].buildId).toBe("b1");
    });

    it("forwards --limit and --status to the server", async () => {
      buildsListHandler = vi.fn().mockResolvedValue({ builds: [] });

      await runBuildsCommand(args({ limit: "5", status: "failed" }), env());
      const handler = buildsListHandler as unknown as ReturnType<typeof vi.fn>;
      expect(handler.mock.calls[0][0]).toMatchObject({ limit: 5, status: "failed" });
    });

    it("rejects --limit=foo client-side before touching the socket", async () => {
      buildsListHandler = vi.fn();
      await expect(runBuildsCommand(args({ limit: "foo" }), env())).rejects.toMatchObject({
        code: "INVALID_ARGUMENT",
      });
      expect(buildsListHandler).not.toHaveBeenCalled();
    });

    it("rejects unknown --status client-side", async () => {
      buildsListHandler = vi.fn();
      await expect(runBuildsCommand(args({ status: "bogus" }), env())).rejects.toMatchObject({
        code: "INVALID_ARGUMENT",
      });
      expect(buildsListHandler).not.toHaveBeenCalled();
    });
  });

  describe("show", () => {
    it("requires a positional buildId", async () => {
      buildGetHandler = vi.fn();
      await expect(runShowCommand(args({}), env())).rejects.toMatchObject({ code: "INVALID_ARGUMENT" });
    });

    it("returns exit 0 + the build envelope", async () => {
      buildGetHandler = vi.fn().mockResolvedValue(makeBuild({ buildId: "b1" }));

      const result = await runShowCommand(args({}, ["b1"]), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<BuildResponseData>;
      expect(envelope.data.buildId).toBe("b1");
    });

    it("maps BUILD_NOT_FOUND to exit 2", async () => {
      buildGetHandler = vi.fn().mockImplementation(() => {
        throw new ProtocolError("BUILD_NOT_FOUND", "nope");
      });

      const result = await runShowCommand(args({}, ["bX"]), env());
      expect(result.exitCode).toBe(2);
      expect((result.envelope as WireErrorResponse).error.code).toBe("BUILD_NOT_FOUND");
    });
  });

  describe("errors", () => {
    it("returns the most recent failed build by default", async () => {
      const failed = makeBuild({ buildId: "b3", status: "failed", errorCount: 2 });
      buildsListHandler = vi.fn().mockResolvedValue({ builds: [failed] });
      buildGetHandler = vi.fn();

      const result = await runErrorsCommand(args({}), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<BuildResponseData>;
      expect(envelope.data.buildId).toBe("b3");
      expect(envelope.data.status).toBe("failed");

      // builds.list invoked with status filter + limit, not build.get.
      const handler = buildsListHandler as unknown as ReturnType<typeof vi.fn>;
      expect(handler.mock.calls[0][0]).toMatchObject({ status: "failed", limit: 1 });
      expect(buildGetHandler).not.toHaveBeenCalled();
    });

    it("returns BUILD_NOT_FOUND when there are no failed builds", async () => {
      buildsListHandler = vi.fn().mockResolvedValue({ builds: [] });
      buildGetHandler = vi.fn();

      const result = await runErrorsCommand(args({}), env());
      expect(result.exitCode).toBe(2);
      const envelope = result.envelope as WireErrorResponse;
      expect(envelope.error.code).toBe("BUILD_NOT_FOUND");
    });

    it("--build=<id> routes through build.get instead", async () => {
      buildsListHandler = vi.fn();
      buildGetHandler = vi.fn().mockResolvedValue(makeBuild({ buildId: "b1", status: "succeeded" }));

      const result = await runErrorsCommand(args({ build: "b1" }), env());
      expect(result.exitCode).toBe(0);
      const envelope = result.envelope as WireSuccessResponse<BuildResponseData>;
      expect(envelope.data.buildId).toBe("b1");

      const handler = buildGetHandler as unknown as ReturnType<typeof vi.fn>;
      expect(handler.mock.calls[0][0]).toMatchObject({ buildId: "b1" });
      expect(buildsListHandler).not.toHaveBeenCalled();
    });

    it("--build=<unknown> maps BUILD_NOT_FOUND from the server", async () => {
      buildsListHandler = vi.fn();
      buildGetHandler = vi.fn().mockImplementation(() => {
        throw new ProtocolError("BUILD_NOT_FOUND", "nope");
      });

      const result = await runErrorsCommand(args({ build: "bX" }), env());
      expect(result.exitCode).toBe(2);
      expect((result.envelope as WireErrorResponse).error.code).toBe("BUILD_NOT_FOUND");
    });
  });
});

function makeBuild(overrides: Partial<BuildResponseData>): BuildResponseData {
  return {
    buildId: "b1",
    scheme: "MyApp",
    destination: "iPhone 15",
    config: "Debug",
    command: "build",
    status: "succeeded",
    exitCode: 0,
    originator: "cli",
    startedAt: "2026-01-01T00:00:00.000Z",
    finishedAt: "2026-01-01T00:00:05.000Z",
    durationMs: 5000,
    errorCount: 0,
    warningCount: 0,
    diagnostics: [],
    ...overrides,
  };
}
