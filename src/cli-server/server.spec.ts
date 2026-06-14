import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { rpc, RpcError } from "../cli/client";
import { getProjectsIndexFile } from "./paths";
import { projectKey } from "./registry";
import { CliServer } from "./server";

describe("CliServer", () => {
  // A real temp dir as the workspace; an isolated XDG_STATE_HOME so the discovery
  // index never touches the developer's real ~/.local/state. The socket lives in
  // tmpdir (short path).
  let workspacePath: string;
  let stateHome: string;
  let prevStateHome: string | undefined;
  let server: CliServer | undefined;

  beforeEach(async () => {
    workspacePath = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-server-spec-"));
    stateHome = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-state-spec-"));
    prevStateHome = process.env.XDG_STATE_HOME;
    process.env.XDG_STATE_HOME = stateHome;
  });

  afterEach(async () => {
    if (server) await server.dispose();
    server = undefined;
    if (prevStateHome === undefined) delete process.env.XDG_STATE_HOME;
    else process.env.XDG_STATE_HOME = prevStateHome;
    await fs.rm(workspacePath, { recursive: true, force: true });
    await fs.rm(stateHome, { recursive: true, force: true });
  });

  async function indexEntry(): Promise<Record<string, unknown> | undefined> {
    const index = JSON.parse(await fs.readFile(getProjectsIndexFile(), "utf8"));
    return index.projects[await projectKey(workspacePath)];
  }

  it("round-trips a JSON-RPC call end-to-end over the Unix socket", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {
        "echo.test": (params) => ({ received: params }),
      },
    });
    await server.start();

    const result = await rpc<{ received: { hello: string } }>({
      socketPath: server.socket,
      method: "echo.test",
      params: { hello: "world" },
    });
    expect(result.received).toEqual({ hello: "world" });
  });

  it("registers an index entry with correct fields", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "9.9.9",
      handlers: {},
    });
    await server.start();

    const meta = await indexEntry();
    expect(meta?.name).toBe(server.name);
    expect(meta?.socket).toBe(server.socket);
    expect(meta?.workspacePath).toBe(workspacePath);
    expect(meta?.extensionVersion).toBe("9.9.9");
    expect(meta?.protocolVersion).toBe("1.0");
    expect(typeof meta?.pid).toBe("number");
    expect(typeof meta?.startedAt).toBe("string");
  });

  it("removes the socket and the index entry on dispose", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {},
    });
    await server.start();
    const socketPath = server.socket;
    expect(await indexEntry()).toBeDefined();

    await server.dispose();
    server = undefined;

    await expect(fs.access(socketPath)).rejects.toThrow(/ENOENT/);
    expect(await indexEntry()).toBeUndefined();
  });

  it("surfaces RPC errors with the application code in error.data", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {
        "fail.now": () => {
          throw new Error("planned failure");
        },
      },
    });
    await server.start();

    await expect(
      rpc({
        socketPath: server.socket,
        method: "fail.now",
        params: {},
      }),
    ).rejects.toBeInstanceOf(RpcError);
  });

  it("answers an unknown method with JSON-RPC method-not-found (-32601)", async () => {
    server = new CliServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {},
    });
    await server.start();

    const err = await rpc({ socketPath: server.socket, method: "does.not.exist", params: {} }).catch((e) => e);
    expect(err).toBeInstanceOf(RpcError);
    expect((err as RpcError).code).toBe(-32601);
  });
});
