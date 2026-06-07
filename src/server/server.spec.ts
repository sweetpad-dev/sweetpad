import { promises as fs } from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import { rpc, RpcError } from "../cli/client";
import { SocketServer } from "./server";

describe("SocketServer", () => {
  // A real temp dir as the workspace: the connection file lands in
  // <workspace>/.sweetpad/run/. The socket itself lives in tmpdir (short path).
  let workspacePath: string;
  let server: SocketServer | undefined;

  beforeEach(async () => {
    workspacePath = await fs.mkdtemp(path.join(os.tmpdir(), "sweetpad-server-spec-"));
  });

  afterEach(async () => {
    if (server) await server.dispose();
    server = undefined;
    await fs.rm(workspacePath, { recursive: true, force: true });
  });

  function connectionFile(name: string): string {
    return path.join(workspacePath, ".sweetpad", "run", `${name}.json`);
  }

  it("round-trips a JSON-RPC call end-to-end over the Unix socket", async () => {
    server = new SocketServer({
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

  it("writes a connection file with correct fields", async () => {
    server = new SocketServer({
      workspacePath,
      extensionVersion: "9.9.9",
      handlers: {},
    });
    await server.start();

    const meta = JSON.parse(await fs.readFile(connectionFile(server.name), "utf8"));
    expect(meta.name).toBe(server.name);
    expect(meta.kind).toBe("extension");
    expect(meta.socket).toBe(server.socket);
    expect(meta.workspacePath).toBe(workspacePath);
    expect(meta.extensionVersion).toBe("9.9.9");
    expect(meta.protocolVersion).toBe("1.0");
    expect(typeof meta.pid).toBe("number");
    expect(typeof meta.startedAt).toBe("string");
  });

  it("removes the socket and connection file on dispose", async () => {
    server = new SocketServer({
      workspacePath,
      extensionVersion: "test",
      handlers: {},
    });
    await server.start();
    const socketPath = server.socket;
    const connPath = connectionFile(server.name);

    await server.dispose();
    server = undefined;

    await expect(fs.access(socketPath)).rejects.toThrow(/ENOENT/);
    await expect(fs.access(connPath)).rejects.toThrow(/ENOENT/);
  });

  it("surfaces RPC errors with the application code in error.data", async () => {
    server = new SocketServer({
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
    server = new SocketServer({
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
