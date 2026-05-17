import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { noopLogger } from "../../core/logger/types";
import { ProtocolError } from "../../protocol/errors";
import type { BuildResponseData, WireErrorResponse, WireSuccessResponse } from "../../protocol/types";
import { MethodDispatcher } from "../../server/dispatcher";
import { Listener } from "../../server/listener";
import { parseArgv } from "../argv";
import { runBuildCommand } from "./build";

/**
 * Drives `runBuildCommand` against a real Unix socket served by a stub `build`
 * method. Validates the full CLI path: argv → socket → response → exit-code
 * mapping. The server logic is exercised independently in `methods/build.spec.ts`.
 */
describe("runBuildCommand — E2E against a stub server", () => {
  let socketPath: string;
  let listener: Listener;
  let buildHandler: (params: unknown) => Promise<BuildResponseData>;

  beforeEach(async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "sweetpad-cli-it-"));
    socketPath = path.join(dir, "server.sock");

    const dispatcher = new MethodDispatcher(noopLogger);
    dispatcher.register("build", (params: unknown) => buildHandler(params));

    listener = new Listener({ socketPath, dispatcher, logger: noopLogger });
    await listener.listen();
  });

  afterEach(async () => {
    await listener.close();
  });

  function args(flags: Record<string, string>): ReturnType<typeof parseArgv> {
    const argv: string[] = [];
    for (const [k, v] of Object.entries(flags)) argv.push(`--${k}=${v}`);
    return parseArgv(argv);
  }

  function env() {
    return { cliEntryDir: "/unused", socketPathOverride: socketPath };
  }

  it("returns exit 0 + the Build envelope when the server reports success", async () => {
    buildHandler = vi.fn().mockResolvedValue(makeBuild({ status: "succeeded", exitCode: 0 }));

    const result = await runBuildCommand(
      args({ scheme: "MyApp", destination: "iPhone 15", config: "Debug" }),
      env(),
    );

    expect(result.exitCode).toBe(0);
    const envelope = result.envelope as WireSuccessResponse<BuildResponseData>;
    expect(envelope.ok).toBe(true);
    expect(envelope.data.status).toBe("succeeded");
  });

  it("forwards CLI flags as params to the server", async () => {
    buildHandler = vi.fn().mockResolvedValue(makeBuild({ status: "succeeded", exitCode: 0 }));

    await runBuildCommand(
      args({
        scheme: "MyApp",
        destination: "iPhone 15",
        config: "Debug",
        xcworkspace: "/foo/Bar.xcworkspace",
        debug: "true",
      }),
      env(),
    );

    const handler = buildHandler as unknown as ReturnType<typeof vi.fn>;
    expect(handler.mock.calls[0][0]).toMatchObject({
      scheme: "MyApp",
      destination: "iPhone 15",
      configuration: "Debug",
      xcworkspace: "/foo/Bar.xcworkspace",
    });
  });

  it("returns exit 1 when the server reports status=failed", async () => {
    buildHandler = vi.fn().mockResolvedValue(makeBuild({ status: "failed", exitCode: 65 }));

    const result = await runBuildCommand(
      args({ scheme: "MyApp", destination: "iPhone 15", config: "Debug" }),
      env(),
    );

    expect(result.exitCode).toBe(1);
    const envelope = result.envelope as WireSuccessResponse<BuildResponseData>;
    expect(envelope.data.status).toBe("failed");
  });

  it("returns exit 2 for user-error codes (SCHEME_NOT_FOUND)", async () => {
    buildHandler = vi.fn().mockImplementation(() => {
      throw new ProtocolError("SCHEME_NOT_FOUND", "Scheme 'Ghost' not found");
    });

    const result = await runBuildCommand(
      args({ scheme: "Ghost", destination: "iPhone 15", config: "Debug" }),
      env(),
    );

    expect(result.exitCode).toBe(2);
    const envelope = result.envelope as WireErrorResponse;
    expect(envelope.ok).toBe(false);
    expect(envelope.error.code).toBe("SCHEME_NOT_FOUND");
  });

  it("returns exit 1 for transient error codes (INTERNAL)", async () => {
    buildHandler = vi.fn().mockImplementation(() => {
      throw new ProtocolError("INTERNAL", "boom");
    });

    const result = await runBuildCommand(
      args({ scheme: "MyApp", destination: "iPhone 15", config: "Debug" }),
      env(),
    );

    expect(result.exitCode).toBe(1);
  });

  it("throws INVALID_ARGUMENT before touching the socket when required flags are missing", async () => {
    buildHandler = vi.fn();

    await expect(runBuildCommand(args({ scheme: "MyApp" }), env())).rejects.toMatchObject({
      code: "INVALID_ARGUMENT",
    });

    expect(buildHandler).not.toHaveBeenCalled();
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
